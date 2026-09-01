//! Inbound segmented ConfirmedRequest reassembly (#364, #377, #381).
//!
//! Clause 20.1.2.7 makes the request sequence number modulo 256, so the wire
//! can carry a request longer than the sequence space and only the receiver's
//! bookkeeping stands between that and silent payload corruption. These tests
//! run the real dispatch loop over loopback or injected test transports. The
//! loopback tests play the client in lockstep (window size 1, one ack awaited
//! per segment — also what keeps the loopback channels from filling).
//!
//! Wall-clock discipline: every test must finish well inside the 4 s
//! reassembly reaper, which would otherwise evict the session mid-test and
//! satisfy "the session is gone" assertions for the wrong reason.

use super::*;
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_objects::value_types::CharacterStringValueObject;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_types::enums::Segmentation;
use bacnet_types::primitives::PropertyValue;
use bytes::BytesMut;
use tokio::time::timeout;

const SERVER_MAC: &[u8] = &[0x02];
const CLIENT_MAC: &[u8] = &[0x01];
const CSV_INSTANCE: u32 = 1;

struct RoutedInjectionTransport {
    incoming: Option<mpsc::Receiver<ReceivedNpdu>>,
    sent_unicast: SentFrames,
    local_mac: MacAddr,
}

impl RoutedInjectionTransport {
    fn new(sent_unicast: SentFrames) -> (Self, mpsc::Sender<ReceivedNpdu>) {
        let (incoming_tx, incoming) = mpsc::channel(16);
        (
            Self {
                incoming: Some(incoming),
                sent_unicast,
                local_mac: MacAddr::from_slice(SERVER_MAC),
            },
            incoming_tx,
        )
    }
}

impl TransportPort for RoutedInjectionTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.incoming
            .take()
            .ok_or_else(|| Error::Encoding("routed injection transport already started".into()))
    }

    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.sent_unicast
            .lock()
            .unwrap()
            .push((Bytes::copy_from_slice(npdu), MacAddr::from_slice(mac)));
        Ok(())
    }

    async fn send_broadcast(&self, _npdu: &[u8]) -> Result<(), Error> {
        Ok(())
    }

    fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }
}

async fn start_routed_reassembly_server() -> (
    BACnetServer<RoutedInjectionTransport>,
    mpsc::Sender<ReceivedNpdu>,
    SentFrames,
) {
    let sent = StdArc::new(StdMutex::new(Vec::new()));
    let (transport, incoming) = RoutedInjectionTransport::new(StdArc::clone(&sent));
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        CharacterStringValueObject::new(CSV_INSTANCE, "CSV-1").unwrap(),
    ))
    .unwrap();
    let config = ServerConfig {
        segmentation_supported: Segmentation::BOTH,
        ..ServerConfig::default()
    };
    let server = BACnetServer::start(config, db, transport).await.unwrap();
    (server, incoming, sent)
}

async fn inject_routed_apdu(
    incoming: &mpsc::Sender<ReceivedNpdu>,
    router_mac: &MacAddr,
    routed_source: &NpduAddress,
    apdu: &Apdu,
) {
    let mut apdu_buf = BytesMut::new();
    encode_apdu(&mut apdu_buf, apdu).unwrap();
    let npdu = Npdu {
        source: Some(routed_source.clone()),
        payload: apdu_buf.freeze(),
        ..Npdu::default()
    };
    let mut npdu_buf = BytesMut::new();
    encode_npdu(&mut npdu_buf, &npdu).unwrap();
    incoming
        .send(ReceivedNpdu {
            npdu: npdu_buf.freeze(),
            source_mac: router_mac.clone(),
            link_layer_group: false,
            data_attributes: Vec::new(),
            reply_tx: None,
        })
        .await
        .unwrap();
}

async fn inject_routed_segment(
    incoming: &mpsc::Sender<ReceivedNpdu>,
    router_mac: &MacAddr,
    routed_source: &NpduAddress,
    invoke_id: u8,
    seq: u8,
    more_follows: bool,
    data: &[u8],
) {
    inject_routed_apdu(
        incoming,
        router_mac,
        routed_source,
        &Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: true,
            more_follows,
            segmented_response_accepted: true,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id,
            sequence_number: Some(seq),
            proposed_window_size: Some(1),
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
            service_request: Bytes::copy_from_slice(data),
        }),
    )
    .await;
}

fn sent_routed_frame(sent: &SentFrames, index: usize) -> (Npdu, MacAddr) {
    let (npdu, link_destination) = {
        let sent = sent.lock().unwrap();
        sent[index].clone()
    };
    (
        decode_npdu(npdu).expect("sent frame should decode as NPDU"),
        link_destination,
    )
}

/// Start a real server on one end of a loopback pair; the test is the client
/// on the other end. The database holds one CharacterString Value object so a
/// reassembled WriteProperty has a writable target.
pub(super) async fn start_reassembly_server(
    segmentation: Segmentation,
) -> (
    BACnetServer<LoopbackTransport>,
    LoopbackTransport,
    mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>,
) {
    let (server_transport, mut client_transport) =
        LoopbackTransport::pair(SERVER_MAC.to_vec(), CLIENT_MAC.to_vec());
    let client_rx = client_transport.start().await.unwrap();

    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        CharacterStringValueObject::new(CSV_INSTANCE, "CSV-1").unwrap(),
    ))
    .unwrap();

    let config = ServerConfig {
        segmentation_supported: segmentation,
        ..ServerConfig::default()
    };
    let server = BACnetServer::start(config, db, server_transport)
        .await
        .unwrap();
    (server, client_transport, client_rx)
}

/// A WriteProperty request whose reassembled form writes `text` to the CSV
/// object's Present_Value — the payload the segments carry, so a corrupt
/// reassembly cannot produce a SimpleAck and a correct one must.
pub(super) fn write_property_payload(text: &str) -> Vec<u8> {
    let mut value = BytesMut::new();
    encode_property_value(
        &mut value,
        &PropertyValue::CharacterString(text.to_string()),
    )
    .unwrap();
    let request = WritePropertyRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::CHARACTERSTRING_VALUE, CSV_INSTANCE)
            .unwrap(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        property_value: value.to_vec(),
        priority: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf);
    buf.to_vec()
}

/// Split `payload` into exactly `count` non-empty chunks.
pub(super) fn split_into(payload: &[u8], count: usize) -> Vec<Vec<u8>> {
    assert!(
        payload.len() >= count,
        "payload of {} bytes cannot fill {count} non-empty segments",
        payload.len()
    );
    let base = payload.len() / count;
    let extra = payload.len() % count;
    let mut chunks = Vec::with_capacity(count);
    let mut at = 0;
    for i in 0..count {
        let len = base + usize::from(i < extra);
        chunks.push(payload[at..at + len].to_vec());
        at += len;
    }
    chunks
}

async fn send_apdu(transport: &LoopbackTransport, apdu: &Apdu) {
    let mut apdu_buf = BytesMut::new();
    encode_apdu(&mut apdu_buf, apdu).unwrap();
    let npdu = Npdu {
        payload: apdu_buf.freeze(),
        ..Npdu::default()
    };
    let mut npdu_buf = BytesMut::new();
    encode_npdu(&mut npdu_buf, &npdu).unwrap();
    transport.send_unicast(&npdu_buf, SERVER_MAC).await.unwrap();
}

async fn send_segment(
    transport: &LoopbackTransport,
    invoke_id: u8,
    seq: u8,
    more_follows: bool,
    data: &[u8],
) {
    send_segment_with_window(transport, invoke_id, seq, 1, more_follows, data).await;
}

pub(super) async fn send_segment_with_window(
    transport: &LoopbackTransport,
    invoke_id: u8,
    seq: u8,
    window_size: u8,
    more_follows: bool,
    data: &[u8],
) {
    send_apdu(
        transport,
        &Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: true,
            more_follows,
            segmented_response_accepted: true,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id,
            sequence_number: Some(seq),
            proposed_window_size: Some(window_size),
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
            service_request: Bytes::copy_from_slice(data),
        }),
    )
    .await;
}

pub(super) async fn recv_apdu(
    rx: &mut mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>,
    context: &str,
) -> Apdu {
    let received = timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap_or_else(|_| panic!("{context}: timed out waiting for a PDU"))
        .unwrap_or_else(|| panic!("{context}: channel closed"));
    let npdu = decode_npdu(received.npdu).unwrap();
    apdu::decode_apdu(npdu.payload).unwrap()
}

pub(super) async fn expect_positive_ack(
    rx: &mut mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>,
    invoke_id: u8,
    seq: u8,
) {
    match recv_apdu(rx, &format!("ack for segment {seq}")).await {
        Apdu::SegmentAck(ack) => {
            assert!(!ack.negative_ack, "segment {seq}: expected a positive ack");
            assert!(ack.sent_by_server, "SegmentAck must carry 'server' = TRUE");
            assert_eq!(ack.invoke_id, invoke_id);
            assert_eq!(ack.sequence_number, seq);
        }
        other => panic!("segment {seq}: expected SegmentAck, got {other:?}"),
    }
}

async fn expect_abort(
    rx: &mut mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>,
    invoke_id: u8,
    reason: AbortReason,
    context: &str,
) {
    match recv_apdu(rx, context).await {
        Apdu::Abort(abort) => {
            assert!(
                abort.sent_by_server,
                "{context}: server Abort must carry 'server' = TRUE"
            );
            assert_eq!(abort.invoke_id, invoke_id, "{context}");
            assert_eq!(abort.abort_reason, reason, "{context}");
        }
        other => panic!("{context}: expected Abort, got {other:?}"),
    }
}

pub(super) async fn present_value<T: TransportPort + 'static>(server: &BACnetServer<T>) -> String {
    let db = server.database().read().await;
    let oid = ObjectIdentifier::new(ObjectType::CHARACTERSTRING_VALUE, CSV_INSTANCE).unwrap();
    match db
        .get(&oid)
        .unwrap()
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap()
    {
        PropertyValue::CharacterString(s) => s,
        other => panic!("expected CharacterString present value, got {other:?}"),
    }
}

#[tokio::test]
async fn reassembled_request_uses_direct_request_duplicate_admission_boundary() {
    let (server, client, mut rx) = start_reassembly_server(Segmentation::BOTH).await;
    let invoke_id = 5;
    let text = "direct-then-reassembled-duplicate";
    let payload = write_property_payload(text);
    let direct = ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: true,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        service_request: Bytes::copy_from_slice(&payload),
    };

    send_apdu(&client, &Apdu::ConfirmedRequest(direct)).await;
    match recv_apdu(&mut rx, "direct request response").await {
        Apdu::SimpleAck(ack) => assert_eq!(ack.invoke_id, invoke_id),
        other => panic!("expected direct SimpleAck, got {other:?}"),
    }

    let chunks = split_into(&payload, 2);
    for (index, chunk) in chunks.iter().enumerate() {
        send_segment(
            &client,
            invoke_id,
            index as u8,
            index + 1 < chunks.len(),
            chunk,
        )
        .await;
        expect_positive_ack(&mut rx, invoke_id, index as u8).await;
    }
    assert!(
        timeout(Duration::from_millis(250), rx.recv())
            .await
            .is_err(),
        "the reassembled exact duplicate must not produce a service response"
    );
    assert_eq!(present_value(&server).await, text);
}

/// #364: exactly 256 segments — the full sequence space — reassemble to the
/// byte-exact request. Passed before the fix too (`seq + 1` happens to equal
/// the count when nothing has wrapped); this pins the boundary the cap must
/// not break.
#[tokio::test]
async fn two_hundred_fifty_six_segments_reassemble_byte_exact() {
    let (server, client, mut rx) = start_reassembly_server(Segmentation::BOTH).await;
    let text = "x".repeat(300);
    let chunks = split_into(&write_property_payload(&text), 256);

    for (i, chunk) in chunks.iter().enumerate() {
        let seq = i as u8;
        send_segment(&client, 7, seq, i + 1 < chunks.len(), chunk).await;
        expect_positive_ack(&mut rx, 7, seq).await;
    }
    match recv_apdu(&mut rx, "response to the reassembled WriteProperty").await {
        Apdu::SimpleAck(ack) => assert_eq!(ack.invoke_id, 7),
        other => panic!("expected SimpleAck, got {other:?}"),
    }
    assert_eq!(present_value(&server).await, text);
}

/// #364: the 257th in-order segment arrives as a wrapped `seq == 0` and must
/// end the transfer with BUFFER_OVERFLOW — before the fix it silently replaced
/// the session (the wrapped 0 looked like a fresh initial segment) and the
/// request "succeeded" as its own tail.
#[tokio::test]
async fn segment_past_the_sequence_space_aborts_instead_of_corrupting() {
    let (server, client, mut rx) = start_reassembly_server(Segmentation::BOTH).await;
    let text = "y".repeat(310);
    let chunks = split_into(&write_property_payload(&text), 260);

    for (i, chunk) in chunks.iter().enumerate().take(256) {
        let seq = i as u8;
        send_segment(&client, 9, seq, true, chunk).await;
        expect_positive_ack(&mut rx, 9, seq).await;
    }
    // Segment index 256 wraps to sequence 0. In order, new, and one past
    // capacity.
    send_segment(&client, 9, 0, true, &chunks[256]).await;
    expect_abort(
        &mut rx,
        9,
        AbortReason::BUFFER_OVERFLOW,
        "the 257th segment",
    )
    .await;
    // The session is gone: a follow-up segment finds no state and draws the
    // no-session Abort rather than an ack.
    send_segment(&client, 9, 1, true, &chunks[257]).await;
    expect_abort(
        &mut rx,
        9,
        AbortReason::INVALID_APDU_IN_THIS_STATE,
        "a segment after the overflow Abort",
    )
    .await;
    assert_eq!(
        present_value(&server).await,
        "",
        "an overlong request must not write anything"
    );
}

/// A duplicate segment 0 mid-session is out-of-order traffic for the live
/// session (Clause 5.4.5.2), not a fresh initial segment — before the fix it
/// silently reset the session. The transfer must survive it.
#[tokio::test]
async fn duplicate_segment_zero_draws_negative_ack_and_transfer_survives() {
    let (server, client, mut rx) = start_reassembly_server(Segmentation::BOTH).await;
    let text = "z".repeat(40);
    let chunks = split_into(&write_property_payload(&text), 3);

    send_segment(&client, 11, 0, true, &chunks[0]).await;
    expect_positive_ack(&mut rx, 11, 0).await;
    send_segment(&client, 11, 1, true, &chunks[1]).await;
    expect_positive_ack(&mut rx, 11, 1).await;

    // A retransmitted segment 0 (as after a lost ack).
    send_segment(&client, 11, 0, true, &chunks[0]).await;
    match recv_apdu(&mut rx, "reply to the duplicate segment 0").await {
        Apdu::SegmentAck(ack) => {
            assert!(ack.negative_ack, "a duplicate must not be re-acked as new");
            assert_eq!(ack.sequence_number, 1, "NAK names the last accepted seq");
        }
        other => panic!("expected SegmentAck, got {other:?}"),
    }

    send_segment(&client, 11, 2, false, &chunks[2]).await;
    expect_positive_ack(&mut rx, 11, 2).await;
    match recv_apdu(&mut rx, "response after the duplicate").await {
        Apdu::SimpleAck(ack) => assert_eq!(ack.invoke_id, 11),
        other => panic!("expected SimpleAck, got {other:?}"),
    }
    assert_eq!(present_value(&server).await, text);
}

/// #364 companion defect: a segment the receiver cannot store used to be
/// dropped with a warning, leaving the session dangling and the peer waiting.
/// It must end the session and say so.
#[tokio::test]
async fn unsaveable_segment_aborts_and_ends_the_session() {
    let (_server, client, mut rx) = start_reassembly_server(Segmentation::BOTH).await;

    send_segment(&client, 13, 0, true, &[0xAA; 32]).await;
    expect_positive_ack(&mut rx, 13, 0).await;

    // service_request larger than SegmentReceiver's 1476-byte segment cap.
    send_segment(&client, 13, 1, true, &[0xBB; 1500]).await;
    expect_abort(
        &mut rx,
        13,
        AbortReason::BUFFER_OVERFLOW,
        "an oversized segment",
    )
    .await;

    send_segment(&client, 13, 2, true, &[0xCC; 32]).await;
    expect_abort(
        &mut rx,
        13,
        AbortReason::INVALID_APDU_IN_THIS_STATE,
        "a segment after the oversized-segment Abort",
    )
    .await;
}

/// #377: a peer's Abort ('server' = FALSE) ends the reassembly session, per
/// Clause 5.4.5.2 AbortPDU_Received. Before the fix the session survived and
/// the next segment was acked as if nothing happened.
#[tokio::test]
async fn peer_abort_clears_the_reassembly_session() {
    let (_server, client, mut rx) = start_reassembly_server(Segmentation::BOTH).await;

    send_segment(&client, 17, 0, true, &[0x11; 16]).await;
    expect_positive_ack(&mut rx, 17, 0).await;

    send_apdu(
        &client,
        &Apdu::Abort(AbortPdu {
            sent_by_server: false,
            invoke_id: 17,
            abort_reason: AbortReason::OTHER,
        }),
    )
    .await;

    send_segment(&client, 17, 1, true, &[0x22; 16]).await;
    expect_abort(
        &mut rx,
        17,
        AbortReason::INVALID_APDU_IN_THIS_STATE,
        "a segment after the peer's Abort",
    )
    .await;
}

#[tokio::test]
async fn routed_reassembly_survives_immediate_router_change_and_replies_on_current_path() {
    let (server, incoming, sent) = start_routed_reassembly_server().await;
    let router_a = test_mac(30);
    let router_b = test_mac(31);
    let remote = routed_address(400, 0x40);
    let invoke_id = 29;
    let text = "routed-segment-identity";
    let chunks = split_into(&write_property_payload(text), 2);

    inject_routed_segment(
        &incoming, &router_a, &remote, invoke_id, 0, true, &chunks[0],
    )
    .await;
    wait_for_sent_len(&sent, 1).await;
    inject_routed_segment(
        &incoming, &router_b, &remote, invoke_id, 1, false, &chunks[1],
    )
    .await;
    wait_for_sent_len(&sent, 3).await;

    for (index, expected_router) in [(0, &router_a), (1, &router_b), (2, &router_b)] {
        let (npdu, link_destination) = sent_routed_frame(&sent, index);
        assert_eq!(&link_destination, expected_router);
        assert_eq!(npdu.destination.as_ref(), Some(&remote));
        assert_eq!(
            npdu.destination.as_ref().map(|address| address.network),
            Some(remote.network)
        );
        assert_eq!(
            npdu.destination
                .as_ref()
                .map(|address| &address.mac_address),
            Some(&remote.mac_address)
        );
        match (index, apdu::decode_apdu(npdu.payload).unwrap()) {
            (0, Apdu::SegmentAck(ack)) => {
                assert_eq!(ack.invoke_id, invoke_id);
                assert_eq!(ack.sequence_number, 0);
                assert!(!ack.negative_ack);
            }
            (1, Apdu::SegmentAck(ack)) => {
                assert_eq!(ack.invoke_id, invoke_id);
                assert_eq!(ack.sequence_number, 1);
                assert!(!ack.negative_ack);
            }
            (2, Apdu::SimpleAck(ack)) => {
                assert_eq!(ack.invoke_id, invoke_id);
                assert_eq!(ack.service_choice, ConfirmedServiceChoice::WRITE_PROPERTY);
            }
            (index, other) => panic!("unexpected response {index}: {other:?}"),
        }
    }

    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(sent_count(&sent), 3, "request must complete exactly once");
    assert_eq!(present_value(&server).await, text);
}

#[tokio::test]
async fn routed_peer_abort_through_new_router_removes_existing_reassembly_session() {
    let (_server, incoming, sent) = start_routed_reassembly_server().await;
    let router_a = test_mac(32);
    let router_b = test_mac(33);
    let remote = routed_address(401, 0x50);
    let invoke_id = 30;

    inject_routed_segment(
        &incoming,
        &router_a,
        &remote,
        invoke_id,
        0,
        true,
        &[0x11; 16],
    )
    .await;
    wait_for_sent_len(&sent, 1).await;
    inject_routed_apdu(
        &incoming,
        &router_b,
        &remote,
        &Apdu::Abort(AbortPdu {
            sent_by_server: false,
            invoke_id,
            abort_reason: AbortReason::OTHER,
        }),
    )
    .await;
    inject_routed_segment(
        &incoming,
        &router_a,
        &remote,
        invoke_id,
        1,
        true,
        &[0x22; 16],
    )
    .await;
    wait_for_sent_len(&sent, 2).await;

    let (npdu, link_destination) = sent_routed_frame(&sent, 1);
    assert_eq!(link_destination, router_a);
    assert_eq!(npdu.destination, Some(remote));
    match apdu::decode_apdu(npdu.payload).unwrap() {
        Apdu::Abort(abort) => {
            assert_eq!(abort.invoke_id, invoke_id);
            assert_eq!(abort.abort_reason, AbortReason::INVALID_APDU_IN_THIS_STATE);
        }
        other => panic!("expected no-session Abort after routed peer Abort, got {other:?}"),
    }
}

/// #381: a device that does not support segmented reception answers segment
/// traffic with SEGMENTATION_NOT_SUPPORTED (Clause 5.4.5.1) instead of
/// reassembling. `ServerConfig::default()` advertises NO_SEGMENTATION.
#[tokio::test]
async fn default_config_aborts_segmented_requests_as_unsupported() {
    let (_server, client, mut rx) = start_reassembly_server(Segmentation::NONE).await;

    send_segment(&client, 19, 0, true, &[0x33; 16]).await;
    expect_abort(
        &mut rx,
        19,
        AbortReason::SEGMENTATION_NOT_SUPPORTED,
        "segment 0 at a NO_SEGMENTATION device",
    )
    .await;
}

/// SEGMENTED_TRANSMIT can send but not receive; reception draws the same
/// Clause 5.4.5.1 Abort as NO_SEGMENTATION.
#[tokio::test]
async fn transmit_only_config_aborts_segmented_requests_as_unsupported() {
    let (_server, client, mut rx) = start_reassembly_server(Segmentation::TRANSMIT).await;

    send_segment(&client, 21, 0, true, &[0x44; 16]).await;
    expect_abort(
        &mut rx,
        21,
        AbortReason::SEGMENTATION_NOT_SUPPORTED,
        "segment 0 at a SEGMENTED_TRANSMIT device",
    )
    .await;
}

/// The 129th concurrent session is refused with BUFFER_OVERFLOW. Pins the
/// session-count cap after its guard lost the (now dead) same-key exemption
/// in the branch reorder.
#[tokio::test]
async fn session_count_cap_refuses_the_129th_session() {
    let (_server, client, mut rx) = start_reassembly_server(Segmentation::BOTH).await;

    for invoke_id in 0u8..128 {
        send_segment(&client, invoke_id, 0, true, &[invoke_id; 8]).await;
        expect_positive_ack(&mut rx, invoke_id, 0).await;
    }
    send_segment(&client, 128, 0, true, &[0x55; 8]).await;
    expect_abort(
        &mut rx,
        128,
        AbortReason::BUFFER_OVERFLOW,
        "the 129th concurrent session",
    )
    .await;
}

/// Clause 5.4.5.3 case (a): a device that does not support transmitting
/// segments must Abort an oversized response rather than segmenting it —
/// the receive-side #381 gate's mirror. RECEIVE may reassemble but not
/// transmit; BOTH is the positive control proving the gate keys on the
/// configured advertisement.
#[tokio::test]
async fn transmit_gate_aborts_oversized_response_under_receive_only() {
    for (segmentation, expects_segmented) in
        [(Segmentation::RECEIVE, false), (Segmentation::BOTH, true)]
    {
        let (server, client, mut rx) = start_reassembly_server(segmentation).await;
        {
            let db = server.database();
            let mut db = db.write().await;
            let oid =
                ObjectIdentifier::new(ObjectType::CHARACTERSTRING_VALUE, CSV_INSTANCE).unwrap();
            db.get_mut(&oid)
                .unwrap()
                .write_property(
                    PropertyIdentifier::PRESENT_VALUE,
                    None,
                    PropertyValue::CharacterString("v".repeat(300)),
                    None,
                )
                .unwrap();
        }

        let mut service_buf = BytesMut::new();
        bacnet_services::read_property::ReadPropertyRequest {
            object_identifier: ObjectIdentifier::new(
                ObjectType::CHARACTERSTRING_VALUE,
                CSV_INSTANCE,
            )
            .unwrap(),
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
        }
        .encode(&mut service_buf);
        send_apdu(
            &client,
            &Apdu::ConfirmedRequest(ConfirmedRequestPdu {
                segmented: false,
                more_follows: false,
                segmented_response_accepted: true,
                max_segments: None,
                max_apdu_length: 50,
                invoke_id: 23,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: ConfirmedServiceChoice::READ_PROPERTY,
                service_request: service_buf.freeze(),
            }),
        )
        .await;

        if expects_segmented {
            match recv_apdu(&mut rx, "segmented response under BOTH").await {
                Apdu::ComplexAck(ack) => {
                    assert!(ack.segmented, "an oversized response must segment");
                    assert_eq!(ack.sequence_number, Some(0));
                }
                other => panic!("expected segmented ComplexAck, got {other:?}"),
            }
        } else {
            expect_abort(
                &mut rx,
                23,
                AbortReason::SEGMENTATION_NOT_SUPPORTED,
                "an oversized response at a receive-only device",
            )
            .await;
        }
    }
}
