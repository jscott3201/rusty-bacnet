//! Correlating inbound responses with the request that is actually waiting.
//!
//! Two independent ways a peer's PDU can be mistaken for this client's answer:
//! it can name a different confirmed service (Clause 20.1.4.2 / 20.1.5.6), or
//! it can carry `'server' = FALSE`, which Clause 20.1.6.2 and 20.1.9.1 define
//! as marking the sender's role and which Clause 5.4.4.2 and 5.4.4.3 then test
//! before accepting the PDU. A third failure needs no misbehaving peer at all:
//! Clause 20.1.5.4 numbers segments modulo 256, so a long enough response wraps
//! onto segments already stored.

use bacnet_encoding::apdu::{self, encode_apdu, AbortPdu, Apdu, ComplexAck, SegmentAck, SimpleAck};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::enums::{AbortReason, ConfirmedServiceChoice};
use bacnet_types::error::Error;
use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration};

use super::{BACnetClient, ClientConfig};

const CLIENT_MAC: [u8; 1] = [0x01];
const PEER_MAC: [u8; 1] = [0x02];

/// How long "the client stayed quiet" is asserted over.
///
/// Comfortably shorter than the 2 s APDU timeout every test here configures, so
/// a retransmission can never masquerade as the reaction under test.
const SILENCE_WINDOW: Duration = Duration::from_millis(300);

/// A test config with retries off and a timeout short enough to keep the
/// deliberately-unanswered cases quick.
fn config() -> ClientConfig {
    ClientConfig {
        apdu_timeout_ms: 2_000,
        apdu_retries: 0,
        proposed_window_size: 1,
        ..ClientConfig::default()
    }
}

/// The far end of an in-process link to a running client.
struct PeerLink {
    transport: LoopbackTransport,
    rx: mpsc::Receiver<ReceivedNpdu>,
    /// Present when the client was started to issue one request.
    request: Option<JoinHandle<Result<Bytes, Error>>>,
}

impl PeerLink {
    /// Start a client that immediately issues one confirmed request.
    async fn issuing(
        config: ClientConfig,
        service_choice: ConfirmedServiceChoice,
        service_data: Vec<u8>,
    ) -> Self {
        let (client_transport, mut transport) =
            LoopbackTransport::pair(CLIENT_MAC.to_vec(), PEER_MAC.to_vec());
        let rx = transport.start().await.expect("peer transport starts");
        let mut client = BACnetClient::start(config, client_transport)
            .await
            .expect("client starts");

        let request = tokio::spawn(async move {
            let result = client
                .confirmed_request(&PEER_MAC, service_choice, &service_data)
                .await;
            client.stop().await.expect("client stops");
            result
        });

        Self {
            transport,
            rx,
            request: Some(request),
        }
    }

    /// Start a client with nothing outstanding. The returned client must be
    /// stopped by the caller.
    async fn idle(config: ClientConfig) -> (Self, BACnetClient<LoopbackTransport>) {
        let (client_transport, mut transport) =
            LoopbackTransport::pair(CLIENT_MAC.to_vec(), PEER_MAC.to_vec());
        let rx = transport.start().await.expect("peer transport starts");
        let client = BACnetClient::start(config, client_transport)
            .await
            .expect("client starts");
        (
            Self {
                transport,
                rx,
                request: None,
            },
            client,
        )
    }

    async fn send(&self, apdu: Apdu) {
        let mut apdu_buf = BytesMut::new();
        encode_apdu(&mut apdu_buf, &apdu).expect("valid APDU encoding");
        let npdu = Npdu {
            payload: apdu_buf.freeze(),
            ..Npdu::default()
        };
        let mut npdu_buf = BytesMut::new();
        encode_npdu(&mut npdu_buf, &npdu).expect("valid NPDU encoding");
        self.transport
            .send_unicast(&npdu_buf, &CLIENT_MAC)
            .await
            .expect("loopback delivers");
    }

    async fn recv(&mut self, context: &str) -> Apdu {
        match timeout(Duration::from_secs(5), self.rx.recv()).await {
            Ok(Some(received)) => {
                let npdu = decode_npdu(received.npdu).expect("valid NPDU");
                apdu::decode_apdu(npdu.payload).expect("valid APDU")
            }
            Ok(None) => panic!("{context}: {}", self.epitaph().await),
            Err(_) => panic!("{context}: timed out waiting for an APDU"),
        }
    }

    /// Assert the client transmits nothing for [`SILENCE_WINDOW`].
    ///
    /// A closed channel is a failure, not silence: a client that shut down
    /// early is quiet for the wrong reason and would pass the assertion these
    /// tests are actually making.
    async fn expect_silence(&mut self, context: &str) {
        match timeout(SILENCE_WINDOW, self.rx.recv()).await {
            Err(_) => {}
            Ok(None) => panic!("{context}: {}", self.epitaph().await),
            Ok(Some(received)) => {
                let npdu = decode_npdu(received.npdu).expect("valid NPDU");
                let decoded = apdu::decode_apdu(npdu.payload).expect("valid APDU");
                panic!("{context}: expected no transmission, got {decoded:?}");
            }
        }
    }

    /// Why the client stopped, for a failure message.
    async fn epitaph(&mut self) -> String {
        match self.request.take() {
            Some(handle) => format!(
                "client shut down, its request returned {:?}",
                handle.await.expect("request task did not panic")
            ),
            None => "client shut down".into(),
        }
    }

    /// Consume the client's request and return its invoke ID.
    async fn request_invoke_id(&mut self) -> u8 {
        match self.recv("initial request").await {
            Apdu::ConfirmedRequest(req) => req.invoke_id,
            other => panic!("expected ConfirmedRequest, got {other:?}"),
        }
    }

    async fn finish(mut self) -> Result<Bytes, Error> {
        let result = self
            .request
            .take()
            .expect("link was created with a request")
            .await
            .expect("request task did not panic");
        self.transport.stop().await.expect("peer transport stops");
        result
    }
}

/// Feed `count` in-order segments carrying `[0, 1, 2, ...]`, one byte each,
/// consuming the SegmentACK the client returns for every one of them.
///
/// `terminates` sets `more-follows` FALSE on the last segment. A window size of
/// 1 keeps the exchange in lockstep, so this works at any length.
async fn feed_segments(
    link: &mut PeerLink,
    invoke_id: u8,
    service_choice: ConfirmedServiceChoice,
    count: usize,
    terminates: bool,
) {
    for i in 0..count {
        let is_last = i == count - 1;
        link.send(Apdu::ComplexAck(ComplexAck {
            segmented: true,
            more_follows: !(is_last && terminates),
            invoke_id,
            sequence_number: Some(i as u8),
            proposed_window_size: Some(1),
            service_choice,
            service_ack: Bytes::from(vec![i as u8]),
        }))
        .await;

        if is_last && terminates {
            break;
        }
        match link.recv("segment ack").await {
            Apdu::SegmentAck(ack) => {
                assert!(!ack.negative_ack, "segment {i} was negatively acked");
                assert_eq!(
                    ack.sequence_number, i as u8,
                    "segment {i} acked out of step"
                );
            }
            other => panic!("segment {i}: expected SegmentAck, got {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Service-choice correlation (Clause 20.1.4.2, 20.1.5.6)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn complex_ack_for_a_different_service_does_not_answer_the_request() {
    let mut link =
        PeerLink::issuing(config(), ConfirmedServiceChoice::READ_PROPERTY, vec![0x01]).await;
    let invoke_id = link.request_invoke_id().await;

    // Same invoke ID, wrong service-ack-choice: not this transaction's answer.
    link.send(Apdu::ComplexAck(ComplexAck {
        segmented: false,
        more_follows: false,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        service_ack: Bytes::from_static(b"imposter"),
    }))
    .await;

    // The transaction must still be pending, so the real answer still lands.
    link.send(Apdu::ComplexAck(ComplexAck {
        segmented: false,
        more_follows: false,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_ack: Bytes::from_static(b"genuine"),
    }))
    .await;

    let payload = link.finish().await.expect("request succeeds");
    assert_eq!(
        payload.as_ref(),
        b"genuine",
        "the mismatched ComplexACK was delivered to the caller"
    );
}

#[tokio::test]
async fn simple_ack_for_a_different_service_does_not_answer_the_request() {
    let mut link =
        PeerLink::issuing(config(), ConfirmedServiceChoice::READ_PROPERTY, vec![0x01]).await;
    let invoke_id = link.request_invoke_id().await;

    // A SimpleACK is the wrong shape of answer for READ_PROPERTY as well as
    // the wrong service, and it carries no data of its own — so the genuine
    // answer below has to be the ComplexACK to make the difference visible.
    // Asserting "the result was an empty payload" would hold either way.
    link.send(Apdu::SimpleAck(SimpleAck {
        invoke_id,
        service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
    }))
    .await;
    link.send(Apdu::ComplexAck(ComplexAck {
        segmented: false,
        more_follows: false,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_ack: Bytes::from_static(b"genuine"),
    }))
    .await;

    let payload = link.finish().await.expect("request succeeds");
    assert_eq!(
        payload.as_ref(),
        b"genuine",
        "the mismatched SimpleACK completed the transaction"
    );
}

#[tokio::test]
async fn reassembled_ack_for_a_different_service_is_discarded() {
    let mut link =
        PeerLink::issuing(config(), ConfirmedServiceChoice::READ_PROPERTY, vec![0x01]).await;
    let invoke_id = link.request_invoke_id().await;

    // A complete, well-formed three-segment response — for the wrong service.
    // It reassembles (the client acks every segment) and is then dropped.
    feed_segments(
        &mut link,
        invoke_id,
        ConfirmedServiceChoice::ATOMIC_READ_FILE,
        3,
        true,
    )
    .await;

    link.send(Apdu::ComplexAck(ComplexAck {
        segmented: false,
        more_follows: false,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_ack: Bytes::from_static(b"genuine"),
    }))
    .await;

    let payload = link.finish().await.expect("request succeeds");
    assert_eq!(
        payload.as_ref(),
        b"genuine",
        "the reassembled mismatched response reached the caller"
    );
}

// ---------------------------------------------------------------------------
// The 'server' parameter (Clause 5.4.4.2, 5.4.4.3)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn abort_with_server_false_does_not_end_the_transaction() {
    let mut link =
        PeerLink::issuing(config(), ConfirmedServiceChoice::READ_PROPERTY, vec![0x01]).await;
    let invoke_id = link.request_invoke_id().await;

    // Clause 5.4.4.3 gates every AWAIT_CONFIRMATION abort transition on
    // 'server' = TRUE; this one is what a *client* emits, echoed back.
    link.send(Apdu::Abort(AbortPdu {
        sent_by_server: false,
        invoke_id,
        abort_reason: AbortReason::BUFFER_OVERFLOW,
    }))
    .await;
    link.send(Apdu::SimpleAck(SimpleAck {
        invoke_id,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
    }))
    .await;

    link.finish()
        .await
        .expect("the server=FALSE Abort aborted the transaction");
}

#[tokio::test]
async fn segment_ack_with_server_false_does_not_advance_the_send_window() {
    let mut config = config();
    // Force segmentation of the outbound request and a two-segment window.
    config.max_apdu_length = 50;
    config.proposed_window_size = 2;
    // Long enough that the second window really does carry two more segments.
    let service_data: Vec<u8> = (0..260).map(|i| (i % 251) as u8).collect();
    let mut link =
        PeerLink::issuing(config, ConfirmedServiceChoice::WRITE_PROPERTY, service_data).await;

    let invoke_id = link.request_invoke_id().await;
    match link.recv("second segment of initial window").await {
        Apdu::ConfirmedRequest(req) => assert_eq!(req.sequence_number, Some(1)),
        other => panic!("expected ConfirmedRequest, got {other:?}"),
    }

    // A SegmentACK a *receiver* emits. Clause 5.4.4.2's NewACK_Received
    // requires 'server' = TRUE, so this must not open the next window.
    link.send(Apdu::SegmentAck(SegmentAck {
        negative_ack: false,
        sent_by_server: false,
        invoke_id,
        sequence_number: 1,
        actual_window_size: 2,
    }))
    .await;
    link.expect_silence("after a server=FALSE SegmentACK").await;

    // The genuine acknowledgment still moves the transfer along.
    link.send(Apdu::SegmentAck(SegmentAck {
        negative_ack: false,
        sent_by_server: true,
        invoke_id,
        sequence_number: 1,
        actual_window_size: 2,
    }))
    .await;
    for expected_seq in [2, 3] {
        match link.recv("second window").await {
            Apdu::ConfirmedRequest(req) => {
                assert_eq!(req.sequence_number, Some(expected_seq));
            }
            other => panic!("expected ConfirmedRequest, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn segmented_ack_for_an_unknown_invoke_id_is_aborted_not_reassembled() {
    let (mut link, mut client) = PeerLink::idle(config()).await;

    // Clause 5.4.4.1's IDLE state has no transition into SEGMENTED_CONF, so a
    // segment for a transaction nobody issued must not open a reassembly
    // session — that is how a peer would occupy every slot at will. What it
    // gets instead is UnexpectedSegmentInfoReceived's Abort, not silence.
    link.send(Apdu::ComplexAck(ComplexAck {
        segmented: true,
        more_follows: true,
        invoke_id: 42,
        sequence_number: Some(0),
        proposed_window_size: Some(1),
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_ack: Bytes::from_static(&[0xAA]),
    }))
    .await;

    match link.recv("unsolicited segment").await {
        Apdu::Abort(abort) => {
            assert_eq!(abort.invoke_id, 42);
            assert!(!abort.sent_by_server, "this client is not the server");
            assert_eq!(
                abort.abort_reason,
                AbortReason::INVALID_APDU_IN_THIS_STATE,
                "Clause 5.4.4.1 names this reason specifically"
            );
        }
        other => panic!("expected Abort, got {other:?}"),
    }
    // And crucially not a SegmentACK, which would mean a session was opened.
    link.expect_silence("after the unsolicited-segment Abort")
        .await;

    client.stop().await.expect("client stops");
    link.transport.stop().await.expect("peer transport stops");
}

/// A stale SegmentACK must not abort a live request that reused its invoke ID.
///
/// `release_invoke_id` drops a peer's allocator once every ID is free, so the
/// next request to an idle peer reuses invoke ID 0. A duplicated SegmentACK
/// from a finished segmented transfer then lands on a live *unsegmented*
/// request, which has no `seg_ack_senders` entry — and Clause 5.4.4.3
/// `SegmentACK_Received` says to "discard the PDU as a duplicate", precisely
/// so that this client does not abort its own healthy request at the peer.
#[tokio::test]
async fn segment_ack_during_an_outstanding_request_is_discarded_not_aborted() {
    let mut link =
        PeerLink::issuing(config(), ConfirmedServiceChoice::READ_PROPERTY, vec![0x01]).await;
    let invoke_id = link.request_invoke_id().await;

    link.send(Apdu::SegmentAck(SegmentAck {
        negative_ack: false,
        sent_by_server: true,
        invoke_id,
        sequence_number: 0,
        actual_window_size: 1,
    }))
    .await;
    link.expect_silence("stale SegmentACK during AWAIT_CONFIRMATION")
        .await;

    // The request must still be alive and answerable.
    link.send(Apdu::ComplexAck(ComplexAck {
        segmented: false,
        more_follows: false,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_ack: Bytes::from_static(b"genuine"),
    }))
    .await;

    let payload = link.finish().await.expect("request survives the stale ack");
    assert_eq!(payload.as_ref(), b"genuine");
}

/// Mid-reassembly, an unexpected SegmentACK aborts rather than being discarded.
///
/// SEGMENTED_CONF and AWAIT_CONFIRMATION both have a transaction outstanding,
/// so "is a transaction pending" cannot tell them apart — but Clause 5.4.4.4
/// `UnexpectedPDU_Received` and Clause 5.4.4.3 `SegmentACK_Received` give them
/// opposite answers. Reassembly in progress is the distinguishing fact.
#[tokio::test]
async fn segment_ack_during_reassembly_aborts_the_transaction() {
    let mut link =
        PeerLink::issuing(config(), ConfirmedServiceChoice::READ_PROPERTY, vec![0x01]).await;
    let invoke_id = link.request_invoke_id().await;

    // One segment in, so the client is reassembling: SEGMENTED_CONF.
    feed_segments(
        &mut link,
        invoke_id,
        ConfirmedServiceChoice::READ_PROPERTY,
        1,
        false,
    )
    .await;

    link.send(Apdu::SegmentAck(SegmentAck {
        negative_ack: false,
        sent_by_server: true,
        invoke_id,
        sequence_number: 0,
        actual_window_size: 1,
    }))
    .await;

    match link.recv("unexpected SegmentACK during reassembly").await {
        Apdu::Abort(abort) => {
            assert_eq!(abort.invoke_id, invoke_id);
            assert!(!abort.sent_by_server);
            assert_eq!(abort.abort_reason, AbortReason::INVALID_APDU_IN_THIS_STATE);
        }
        other => panic!("expected Abort, got {other:?}"),
    }

    // "send ABORT.indication ... to the local application program": the
    // caller must be told, not left waiting for a reassembly that is over.
    match link.finish().await {
        Err(Error::Abort { reason }) => {
            assert_eq!(reason, AbortReason::INVALID_APDU_IN_THIS_STATE.to_raw());
        }
        other => panic!("expected the caller to be aborted, got {other:?}"),
    }
}

#[tokio::test]
async fn segment_ack_for_an_unknown_invoke_id_is_aborted() {
    let (mut link, mut client) = PeerLink::idle(config()).await;

    // The other half of Clause 5.4.4.1 UnexpectedSegmentInfoReceived: a
    // SegmentACK with 'server' = TRUE that answers no outstanding send.
    link.send(Apdu::SegmentAck(SegmentAck {
        negative_ack: false,
        sent_by_server: true,
        invoke_id: 7,
        sequence_number: 0,
        actual_window_size: 1,
    }))
    .await;

    match link.recv("unsolicited SegmentACK").await {
        Apdu::Abort(abort) => {
            assert_eq!(abort.invoke_id, 7);
            assert!(!abort.sent_by_server);
            assert_eq!(abort.abort_reason, AbortReason::INVALID_APDU_IN_THIS_STATE);
        }
        other => panic!("expected Abort, got {other:?}"),
    }

    client.stop().await.expect("client stops");
    link.transport.stop().await.expect("peer transport stops");
}

// ---------------------------------------------------------------------------
// Reassembly capacity (Clause 20.1.5.4, 5.4.4.4, 20.1.2.4)
// ---------------------------------------------------------------------------

/// The one test here that is meant to pass before the fix as well as after:
/// 256 segments were always legal, and the new cap must not take them away.
#[tokio::test]
async fn exactly_256_segments_reassemble() {
    let mut link =
        PeerLink::issuing(config(), ConfirmedServiceChoice::READ_PROPERTY, vec![0x01]).await;
    let invoke_id = link.request_invoke_id().await;

    // 256 fills the modulo-256 sequence space exactly and must still work.
    feed_segments(
        &mut link,
        invoke_id,
        ConfirmedServiceChoice::READ_PROPERTY,
        256,
        true,
    )
    .await;

    let payload = link.finish().await.expect("request succeeds");
    let expected: Vec<u8> = (0..=255).collect();
    assert_eq!(payload.as_ref(), expected.as_slice());
}

#[tokio::test]
async fn segment_257_aborts_with_buffer_overflow() {
    let mut link =
        PeerLink::issuing(config(), ConfirmedServiceChoice::READ_PROPERTY, vec![0x01]).await;
    let invoke_id = link.request_invoke_id().await;

    // 256 segments, all still promising more.
    feed_segments(
        &mut link,
        invoke_id,
        ConfirmedServiceChoice::READ_PROPERTY,
        256,
        false,
    )
    .await;

    // The 257th wraps to sequence number 0. Storing it would overwrite the
    // first segment and silently corrupt the reassembled payload, so Clause
    // 5.4.4.4 NewSegmentReceived_NoSpace applies instead.
    link.send(Apdu::ComplexAck(ComplexAck {
        segmented: true,
        more_follows: false,
        invoke_id,
        sequence_number: Some(0),
        proposed_window_size: Some(1),
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_ack: Bytes::from_static(&[0xFF]),
    }))
    .await;

    match link.recv("overflow abort").await {
        Apdu::Abort(abort) => {
            assert_eq!(abort.invoke_id, invoke_id);
            assert!(
                !abort.sent_by_server,
                "Clause 5.4.4.4 requires 'server' = FALSE"
            );
            assert_eq!(abort.abort_reason, AbortReason::BUFFER_OVERFLOW);
        }
        other => panic!("expected Abort, got {other:?}"),
    }

    match link.finish().await {
        Err(Error::Abort { reason }) => {
            assert_eq!(reason, AbortReason::BUFFER_OVERFLOW.to_raw());
        }
        other => panic!("expected a BUFFER_OVERFLOW abort, got {other:?}"),
    }
}

#[tokio::test]
async fn non_rung_max_segments_rounds_down_in_header_and_bounds_reassembly() {
    let mut config = config();
    // Five has no Clause 20.1.2.4 encoding. Conservatively advertise the
    // B'010' rung (four) and enforce the same receive bound locally.
    config.max_segments = Some(5);
    let mut link =
        PeerLink::issuing(config, ConfirmedServiceChoice::READ_PROPERTY, vec![0x01]).await;
    let invoke_id = match link.recv("initial request").await {
        Apdu::ConfirmedRequest(req) => {
            assert_eq!(req.max_segments, Some(4));
            req.invoke_id
        }
        other => panic!("expected ConfirmedRequest, got {other:?}"),
    };

    feed_segments(
        &mut link,
        invoke_id,
        ConfirmedServiceChoice::READ_PROPERTY,
        4,
        false,
    )
    .await;

    link.send(Apdu::ComplexAck(ComplexAck {
        segmented: true,
        more_follows: false,
        invoke_id,
        sequence_number: Some(4),
        proposed_window_size: Some(1),
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_ack: Bytes::from_static(&[0xFF]),
    }))
    .await;

    match link.recv("overflow abort").await {
        Apdu::Abort(abort) => assert_eq!(abort.abort_reason, AbortReason::BUFFER_OVERFLOW),
        other => panic!("expected Abort, got {other:?}"),
    }
    assert!(matches!(link.finish().await, Err(Error::Abort { .. })));
}

#[tokio::test]
async fn max_segments_above_the_encodable_rungs_advertises_no_bound() {
    // Clause 20.1.2.4 has no rung for 100; it encodes as B'111', "Greater than
    // 64 segments accepted", which promises the peer nothing. Capping
    // reassembly at 100 would enforce a limit that was never advertised.
    assert_eq!(apdu::advertised_max_segments(Some(100)), None);
    assert_eq!(apdu::advertised_max_segments(None), None);
    assert_eq!(apdu::advertised_max_segments(Some(255)), None);
    assert_eq!(apdu::advertised_max_segments(Some(4)), Some(4));
    assert_eq!(apdu::advertised_max_segments(Some(64)), Some(64));
}
