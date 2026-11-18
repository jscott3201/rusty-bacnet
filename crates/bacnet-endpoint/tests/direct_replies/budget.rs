use super::*;
use bacnet_transport::port::TransportProvenance;
use bacnet_types::enums::AbortReason;

async fn small_link(
    npdu: u16,
    bvlc: u16,
    source: Option<bacnet_encoding::npdu::NpduAddress>,
    name_len: usize,
) {
    let ca = TestCa::new();
    let (mut f, port) = Fixture::new(&ca).await;
    let mut session = EndpointSession::new(port, SessionRole::Both, SessionConfig::default())
        .unwrap()
        .device(node())
        .with_database(database(&"x".repeat(name_len)));
    session.start().await.unwrap();
    let mut peer = f.peer(ca.tls("small"), Some((npdu, bvlc))).await;
    let admitted = f.capture(&mut peer, &read_name(80), source.clone()).await;
    f.feed(admitted.clone()).await;
    let (reply, apdu) = f.response(&peer).await;
    assert_eq!(reply.destination, source);
    assert_eq!(reply.payload.len(), 3);
    assert!(
        matches!(apdu, Apdu::Abort(a) if a.invoke_id == 80 && a.sent_by_server && a.abort_reason == AbortReason::SEGMENTATION_NOT_SUPPORTED)
    );
    // Ordinary ingress still applies requester APDU480 alone. No new general cap.
    let mut ordinary = admitted;
    ordinary.provenance = TransportProvenance::unverified();
    ordinary.direct_response = None;
    f.feed(ordinary).await;
    let reply = bounded(f.generic.recv()).await.unwrap();
    assert_eq!(reply.destination, source);
    assert_eq!(reply.payload.len(), name_len + 17);
    assert!(
        matches!(bacnet_encoding::apdu::decode_apdu(reply.payload).unwrap(), Apdu::ComplexAck(a) if a.invoke_id == 80)
    );
    f.barrier(&mut peer, 81).await;
    session.stop().await.unwrap();
    f.listener.stop().await;
}
#[tokio::test]
async fn endpoint_saved_npdu_limit_selects_abort_instead_of_oversized_ack() {
    small_link(480, 1500, None, 462).await;
}
#[tokio::test]
async fn endpoint_saved_bvlc_limit_selects_abort_instead_of_oversized_ack() {
    small_link(1497, 484, None, 462).await;
}
#[tokio::test]
async fn endpoint_saved_routed_header_budget_selects_abort() {
    small_link(480, 484, routed(), 452).await;
}

#[tokio::test]
async fn endpoint_too_small_for_abort_fails_without_fallback_and_dispatch_continues() {
    let ca = TestCa::new();
    let (mut f, port) = Fixture::new(&ca).await;
    let mut session = EndpointSession::new(port, SessionRole::Both, SessionConfig::default())
        .unwrap()
        .device(node())
        .with_database(database("x"));
    session.start().await.unwrap();
    let mut a = f.peer(ca.tls("tiny"), Some((4, 8))).await;
    let admitted = f.capture(&mut a, &read_name(90), None).await;
    f.feed(admitted).await;
    bounded(async {
        while session.server_seat_counters().apdus != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(f.generic.try_recv().is_err());
    // The checked sizing/send failure completed; no response was enqueued.
    use bacnet_transport::sc::WebSocketPort;
    let read = a.ws.recv();
    tokio::pin!(read);
    assert!(futures_util::poll!(&mut read).is_pending());
    let mut b = f.peer(ca.tls("normal"), None).await;
    f.barrier(&mut b, 91).await;
    session.stop().await.unwrap();
    f.listener.stop().await;
}

#[tokio::test]
async fn client_too_small_for_fixed_reply_never_falls_back() {
    let ca = TestCa::new();
    let (mut f, port) = Fixture::new(&ca).await;
    let mut client = BACnetClient::start(ClientConfig::default(), port)
        .await
        .unwrap();
    let mut a = f.peer(ca.tls("tiny"), Some((4, 8))).await;
    let held = f.capture(&mut a, &unsupported(92), None).await;
    let mut ordinary = f.capture(&mut a, &unsupported(94), None).await;
    ordinary.provenance = TransportProvenance::unverified();
    ordinary.direct_response = None;
    f.feed(held).await;
    f.feed(ordinary).await;
    // Complete the tiny reply attempt while A is still current, so retirement
    // cannot mask its sizing failure. Only the ordinary marker may use egress.
    let marker = bounded(f.generic.recv()).await.unwrap();
    assert!(
        matches!(bacnet_encoding::apdu::decode_apdu(marker.payload).unwrap(), Apdu::Reject(r) if r.invoke_id == 94)
    );
    let mut b = f.peer(ca.tls("normal"), None).await;
    f.barrier(&mut b, 93).await;
    client.stop().await.unwrap();
    f.listener.stop().await;
}
