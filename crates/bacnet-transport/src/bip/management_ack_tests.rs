use super::*;
use tokio::time::{timeout, Duration};

async fn malformed_management_ack(function: BvlcFunction, payload_len: usize) -> Error {
    let server = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let server_port = server.local_addr().unwrap().port();
    let server_mac = encode_bip_mac(Ipv4Addr::LOCALHOST.octets(), server_port);

    let responder = tokio::spawn(async move {
        let mut recv_buf = [0u8; 2048];
        let (_len, client_addr) = timeout(Duration::from_secs(2), server.recv_from(&mut recv_buf))
            .await
            .expect("timed out waiting for management request")
            .unwrap();

        let payload = vec![0; payload_len];
        let mut response = BytesMut::with_capacity(4 + payload.len());
        encode_bvll(&mut response, function, &payload).unwrap();
        server.send_to(&response, client_addr).await.unwrap();
    });

    let mut client = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _client_rx = client.start().await.unwrap();

    let err = match function {
        BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK => {
            client.read_bdt(&server_mac).await.unwrap_err()
        }
        BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK => {
            client.read_fdt(&server_mac).await.unwrap_err()
        }
        _ => unreachable!("test helper only supports BDT/FDT management ACKs"),
    };

    client.stop().await.unwrap();
    responder.await.unwrap();

    err
}

async fn raw_bvlc_request(target: &[u8], function: BvlcFunction, payload: &[u8]) -> BvllMessage {
    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let (ip, port) = decode_bip_mac(target).unwrap();
    let dest = SocketAddrV4::new(Ipv4Addr::from(ip), port);

    let mut buf = BytesMut::new();
    encode_bvll(&mut buf, function, payload).unwrap();
    socket.send_to(&buf, dest).await.unwrap();

    let mut recv_buf = [0u8; 2048];
    let (len, _addr) = timeout(Duration::from_secs(2), socket.recv_from(&mut recv_buf))
        .await
        .expect("timed out waiting for BVLC management response")
        .unwrap();

    decode_bvll(&recv_buf[..len]).unwrap()
}

#[tokio::test]
async fn read_bdt_ack_encodes_bdt_entries_as_n10_payload() {
    let configured_entry = BdtEntry {
        ip: [192, 168, 40, 10],
        port: 0xBAC0,
        broadcast_mask: [255, 255, 255, 0],
    };
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd_transport.enable_bbmd(vec![configured_entry.clone()]);
    let _bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();

    let response = raw_bvlc_request(
        &bbmd_mac,
        BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE,
        &[],
    )
    .await;

    assert_eq!(
        response.function,
        BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK
    );
    assert_eq!(response.payload.len() % bbmd::BDT_ENTRY_SIZE, 0);
    assert!(response.payload.len() >= bbmd::BDT_ENTRY_SIZE);

    let configured_wire = [192, 168, 40, 10, 0xBA, 0xC0, 255, 255, 255, 0];
    assert!(
        response
            .payload
            .chunks_exact(bbmd::BDT_ENTRY_SIZE)
            .any(|chunk| chunk == configured_wire),
        "Read-BDT-Ack must include configured BDT entry bytes"
    );

    let entries = BbmdState::decode_bdt(&response.payload).unwrap();
    assert!(entries.iter().any(|entry| entry == &configured_entry));

    bbmd_transport.stop().await.unwrap();
}

#[tokio::test]
async fn read_fdt_ack_encodes_fdt_entries_as_n10_payload() {
    let mut bbmd_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd_transport.enable_bbmd(vec![]);
    let _bbmd_rx = bbmd_transport.start().await.unwrap();
    let bbmd_mac = bbmd_transport.local_mac().to_vec();

    let mut client_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _client_rx = client_transport.start().await.unwrap();
    let result = client_transport
        .register_foreign_device_bvlc(&bbmd_mac, 120)
        .await
        .unwrap();
    assert_eq!(result, BvlcResultCode::SUCCESSFUL_COMPLETION);
    let (fd_ip, fd_port) = decode_bip_mac(client_transport.local_mac()).unwrap();

    let response = raw_bvlc_request(&bbmd_mac, BvlcFunction::READ_FOREIGN_DEVICE_TABLE, &[]).await;

    assert_eq!(
        response.function,
        BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK
    );
    assert_eq!(response.payload.len(), bbmd::FDT_ENTRY_SIZE);
    assert_eq!(&response.payload[0..4], &fd_ip);
    assert_eq!(
        u16::from_be_bytes([response.payload[4], response.payload[5]]),
        fd_port
    );
    assert_eq!(
        u16::from_be_bytes([response.payload[6], response.payload[7]]),
        120
    );
    let remaining = u16::from_be_bytes([response.payload[8], response.payload[9]]);
    assert!((120..=150).contains(&remaining));

    let entries = bbmd::decode_fdt(&response.payload).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].ip, fd_ip);
    assert_eq!(entries[0].port, fd_port);
    assert_eq!(entries[0].ttl, 120);
    assert_eq!(entries[0].seconds_remaining, remaining);

    client_transport.stop().await.unwrap();
    bbmd_transport.stop().await.unwrap();
}

#[tokio::test]
async fn read_bdt_rejects_malformed_ack_payload_length() {
    let err = malformed_management_ack(
        BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK,
        bbmd::BDT_ENTRY_SIZE - 1,
    )
    .await;

    assert!(
        format!("{err}").contains("BDT data length 9 not a multiple of 10"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn read_fdt_rejects_malformed_ack_payload_length() {
    let err = malformed_management_ack(
        BvlcFunction::READ_FOREIGN_DEVICE_TABLE_ACK,
        bbmd::FDT_ENTRY_SIZE - 1,
    )
    .await;

    assert!(
        format!("{err}").contains("FDT data length 9 not a multiple of 10"),
        "unexpected error: {err}"
    );
}
