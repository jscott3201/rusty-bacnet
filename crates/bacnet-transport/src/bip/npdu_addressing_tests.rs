use super::*;
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu, NpduAddress};
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};
use tokio::time::{timeout, Duration};

fn encode_broadcast_npdu(network: u16, hop_count: u8, payload: &'static [u8]) -> Bytes {
    let mut npdu_buf = BytesMut::new();
    encode_npdu(
        &mut npdu_buf,
        &Npdu {
            destination: Some(NpduAddress {
                network,
                mac_address: MacAddr::new(),
            }),
            hop_count,
            payload: Bytes::from_static(payload),
            ..Default::default()
        },
    )
    .unwrap();
    npdu_buf.freeze()
}

async fn send_and_receive_original_broadcast(npdu: &Bytes) -> BvllMessage {
    let sink = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let sink_port = sink.local_addr().unwrap().port();
    let source_socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, sink_port, Ipv4Addr::LOCALHOST);
    transport.socket = Some(Arc::new(source_socket));

    transport.send_broadcast(npdu).await.unwrap();

    let mut recv_buf = [0u8; 2048];
    let (len, _addr) = timeout(Duration::from_secs(2), sink.recv_from(&mut recv_buf))
        .await
        .expect("timed out waiting for Original-Broadcast-NPDU")
        .unwrap();

    decode_bvll(&recv_buf[..len]).unwrap()
}

#[tokio::test]
async fn original_broadcast_npdu_preserves_global_npdu_destination() {
    let npdu = encode_broadcast_npdu(0xFFFF, 255, &[0x10, 0x08]);
    let frame = send_and_receive_original_broadcast(&npdu).await;

    assert_eq!(frame.function, BvlcFunction::ORIGINAL_BROADCAST_NPDU);
    assert_eq!(
        frame.payload.as_ref(),
        npdu.as_ref(),
        "Original-Broadcast-NPDU must preserve global NPDU addressing bytes"
    );

    let decoded = decode_npdu(frame.payload).unwrap();
    let destination = decoded
        .destination
        .expect("global broadcast NPDU must keep DNET/DADR destination");
    assert_eq!(destination.network, 0xFFFF);
    assert!(destination.mac_address.is_empty());
    assert_eq!(decoded.hop_count, 255);
    assert_eq!(decoded.payload.as_ref(), &[0x10, 0x08]);
}

#[tokio::test]
async fn original_broadcast_npdu_preserves_remote_npdu_destination() {
    let npdu = encode_broadcast_npdu(0x1234, 64, &[0x10, 0x09]);
    let frame = send_and_receive_original_broadcast(&npdu).await;

    assert_eq!(frame.function, BvlcFunction::ORIGINAL_BROADCAST_NPDU);
    assert_eq!(
        frame.payload.as_ref(),
        npdu.as_ref(),
        "Original-Broadcast-NPDU must preserve remote NPDU addressing bytes"
    );

    let decoded = decode_npdu(frame.payload).unwrap();
    let destination = decoded
        .destination
        .expect("remote broadcast NPDU must keep DNET/DADR destination");
    assert_eq!(destination.network, 0x1234);
    assert!(destination.mac_address.is_empty());
    assert_eq!(decoded.hop_count, 64);
    assert_eq!(decoded.payload.as_ref(), &[0x10, 0x09]);
}
