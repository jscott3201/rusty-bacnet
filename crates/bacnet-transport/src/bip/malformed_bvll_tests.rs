//! What a running transport does with BVLL datagrams that don't match their
//! Annex J layout, since the receive path moved onto `bacnet-bip-codec`.

use super::*;
use tokio::time::{timeout, Duration};

/// Send `datagram` from a fresh socket and return the reply, if one comes.
async fn exchange(target: &[u8], datagram: &[u8]) -> Option<BvllMessage> {
    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let (ip, port) = decode_bip_mac(target).unwrap();
    socket
        .send_to(datagram, SocketAddrV4::new(Ipv4Addr::from(ip), port))
        .await
        .unwrap();
    let mut buf = [0u8; 2048];
    match timeout(Duration::from_millis(300), socket.recv_from(&mut buf)).await {
        Ok(Ok((len, _))) => Some(decode_bvll(&buf[..len]).unwrap()),
        _ => None,
    }
}

/// A running BBMD that takes foreign devices, its MAC, and its NPDU channel,
/// which the caller keeps open.
async fn start_bbmd() -> (BipTransport, Vec<u8>, mpsc::Receiver<ReceivedNpdu>) {
    let mut bbmd = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    bbmd.enable_bbmd(vec![]);
    bbmd.enable_foreign_device_registration(ForeignDevicePolicy::default());
    let rx = bbmd.start().await.unwrap();
    let mac = bbmd.local_mac().to_vec();
    (bbmd, mac, rx)
}

fn result_code(reply: Option<BvllMessage>) -> BvlcResultCode {
    decode_bvlc_result_code(&reply.expect("a BVLC-Result")).unwrap()
}

/// Annex J.2.3.1 and J.2.7.1 give the table reads no payload. A BBMD used to
/// answer one that carried octets anyway with its table; it now treats the
/// request as malformed and NAKs it, as it does a malformed Register or
/// Delete.
#[tokio::test]
async fn table_reads_with_a_payload_draw_a_nak_from_a_bbmd() {
    let (mut transport, mac, _rx) = start_bbmd().await;
    let cases = [
        (
            BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE,
            BvlcResultCode::READ_BROADCAST_DISTRIBUTION_TABLE_NAK,
        ),
        (
            BvlcFunction::READ_FOREIGN_DEVICE_TABLE,
            BvlcResultCode::READ_FOREIGN_DEVICE_TABLE_NAK,
        ),
    ];
    for (function, nak) in cases {
        let reply = exchange(&mac, &raw_frame(function, &[0x00])).await;
        assert_eq!(result_code(reply), nak, "{function}");
        // The well-formed request still gets its ACK.
        let ack = exchange(&mac, &raw_frame(function, &[])).await.unwrap();
        assert!(ack.message().is_ok(), "{function}");
        assert_ne!(ack.function, BvlcFunction::BVLC_RESULT, "{function}");
    }
    transport.stop().await.unwrap();
}

/// The Length field counts the whole message (Annex J.2). A datagram with
/// octets past it used to be read as the shorter message, so this one
/// registered a foreign device; now it is dropped unanswered.
#[tokio::test]
async fn a_datagram_longer_than_its_length_field_is_dropped() {
    let (mut transport, mac, _rx) = start_bbmd().await;
    let register = [0x81, 0x05, 0x00, 0x06, 0x00, 0x3C, 0xFF];
    assert!(exchange(&mac, &register).await.is_none());
    assert!(bbmd::lock(transport.bbmd_state().unwrap()).fdt().is_empty());

    // Without the trailing octet the same request registers.
    let reply = exchange(&mac, &register[..6]).await;
    assert_eq!(result_code(reply), BvlcResultCode::SUCCESSFUL_COMPLETION);
    assert_eq!(bbmd::lock(transport.bbmd_state().unwrap()).fdt().len(), 1);
    transport.stop().await.unwrap();
}

/// An NPDU in a datagram longer than its Length field never reaches the
/// network layer; the same NPDU in a whole datagram does.
#[tokio::test]
async fn an_npdu_with_trailing_octets_is_not_delivered() {
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut rx = transport.start().await.unwrap();
    let mac = transport.local_mac().to_vec();

    let npdu = [0x01, 0x00, 0x10, 0x08];
    let mut padded = raw_frame(BvlcFunction::ORIGINAL_UNICAST_NPDU, &npdu).to_vec();
    padded.extend_from_slice(&[0xDE, 0xAD]);
    assert!(exchange(&mac, &padded).await.is_none());
    let whole = raw_frame(
        BvlcFunction::ORIGINAL_UNICAST_NPDU,
        &[0x01, 0x00, 0x10, 0x09],
    );
    assert!(exchange(&mac, &whole).await.is_none());

    // Only the whole datagram's NPDU arrives.
    let received = timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.npdu.as_ref(), &[0x01, 0x00, 0x10, 0x09]);
    assert!(rx.try_recv().is_err());
    transport.stop().await.unwrap();
}

/// A Forwarded-NPDU too short for its originating address is dropped, as
/// it was before, and an unknown function draws no reply.
#[tokio::test]
async fn short_forwarded_npdus_and_unknown_functions_draw_nothing() {
    let (mut transport, mac, _rx) = start_bbmd().await;
    let short = raw_frame(BvlcFunction::FORWARDED_NPDU, &[127, 0, 0, 1, 0xBA]);
    assert!(exchange(&mac, &short).await.is_none());
    let unknown = raw_frame(BvlcFunction::from_raw(0x0C), &[0x01, 0x00]);
    assert!(exchange(&mac, &unknown).await.is_none());
    transport.stop().await.unwrap();
}
