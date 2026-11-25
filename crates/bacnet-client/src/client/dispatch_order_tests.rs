//! Orders and refusals on the client's answer path that the split of
//! `dispatch_apdu` and of the segmented receive (#2558) keeps, read from the
//! frames the client sends:
//!
//! - a confirmed COV notification is published before its acknowledgment
//!   goes out;
//! - the final segment's SegmentAck goes out before the request completes;
//! - a final segment sent again after the response completed draws an Abort;
//! - a segment too large to keep aborts the reassembly with
//!   BUFFER_OVERFLOW;
//! - a SegmentAck with the server bit clear leaves a reassembly alone.

use super::super::cov_notification_tests::cov_notification;
use super::*;
use bacnet_encoding::apdu::ComplexAck;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::{SendLog, TestTransport, TestTransportHandle};
use bacnet_types::enums::AbortReason;

const CLIENT: [u8; 1] = [0x71];
const PEER: [u8; 1] = [0x72];
const WAIT: Duration = Duration::from_secs(5);

/// A client on a link the test feeds and whose sends it reads and holds.
struct Lab {
    client: Arc<BACnetClient<TestTransport>>,
    input: mpsc::Sender<ReceivedNpdu>,
    link: TestTransportHandle,
    log: SendLog,
    /// Frames in the log already looked at.
    seen: usize,
}

async fn lab() -> Lab {
    let (input, rx) = mpsc::channel(64);
    let transport = TestTransport::builder()
        .local_mac(&CLIENT)
        .inbound(rx)
        .build();
    let link = transport.handle();
    let log = transport.sent();
    let client = BACnetClient::generic_builder()
        .transport(transport)
        .build()
        .await
        .unwrap();
    Lab {
        client: Arc::new(client),
        input,
        link,
        log,
        seen: 0,
    }
}

fn received(apdu: &Apdu) -> ReceivedNpdu {
    let mut encoded = BytesMut::new();
    encode_apdu(&mut encoded, apdu).unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            expecting_reply: matches!(apdu, Apdu::ConfirmedRequest(_)),
            payload: encoded.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    ReceivedNpdu {
        npdu: npdu.freeze(),
        source_mac: MacAddr::from_slice(&PEER),
        link_layer_group: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        direct_response: None,
        reply_tx: None,
    }
}

/// Segment `seq` of a ReadProperty answer under `invoke_id`.
fn segment(invoke_id: u8, seq: u8, more: bool, data: &[u8]) -> Apdu {
    Apdu::ComplexAck(ComplexAck {
        segmented: true,
        more_follows: more,
        invoke_id,
        sequence_number: Some(seq),
        proposed_window_size: Some(1),
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_ack: Bytes::copy_from_slice(data),
    })
}

impl Lab {
    async fn peer_sends(&self, apdu: &Apdu) {
        self.input.send(received(apdu)).await.unwrap();
    }

    /// The next APDU the client sent, which went to the peer.
    async fn next(&mut self) -> Apdu {
        timeout(WAIT, self.log.wait_for_len(self.seen + 1))
            .await
            .expect("the client sent nothing more");
        let frame = self.log.frame(self.seen);
        self.seen += 1;
        assert_eq!(frame.mac.as_slice(), PEER);
        frame.apdu()
    }

    /// Start a ReadProperty to the peer: its invoke ID, and the task that
    /// awaits its answer.
    async fn request(&mut self) -> (u8, JoinHandle<Result<Bytes, Error>>) {
        let client = Arc::clone(&self.client);
        let task = tokio::spawn(async move {
            client
                .confirmed_request(&PEER, ConfirmedServiceChoice::READ_PROPERTY, &[0x0c; 8])
                .await
        });
        let Apdu::ConfirmedRequest(request) = self.next().await else {
            panic!("the client's first frame is not its request");
        };
        (request.invoke_id, task)
    }
}

fn assert_segment_ack(apdu: Apdu, invoke_id: u8, seq: u8) {
    let Apdu::SegmentAck(ack) = apdu else {
        panic!("expected a SegmentAck, got {apdu:?}");
    };
    assert!(!ack.negative_ack && !ack.sent_by_server);
    assert_eq!((ack.invoke_id, ack.sequence_number), (invoke_id, seq));
}

fn assert_client_abort(apdu: Apdu, invoke_id: u8, reason: AbortReason) {
    let Apdu::Abort(abort) = apdu else {
        panic!("expected an Abort, got {apdu:?}");
    };
    assert!(!abort.sent_by_server, "a client's Abort");
    assert_eq!((abort.invoke_id, abort.abort_reason), (invoke_id, reason));
}

/// Let every task that can run do so, with the clock held.
async fn settle() {
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
}

/// The final SegmentAck is sent before the transaction completes
/// (Clause 5.4.4.4, the last segment of a ComplexACK): while that send is
/// held, the request is still waiting.
#[tokio::test(start_paused = true)]
async fn final_segment_ack_goes_out_before_the_request_completes() {
    let mut lab = lab().await;
    let (invoke_id, task) = lab.request().await;

    lab.peer_sends(&segment(invoke_id, 0, true, &[0x10])).await;
    assert_segment_ack(lab.next().await, invoke_id, 0);

    lab.link.block_next_send();
    lab.peer_sends(&segment(invoke_id, 1, false, &[0x20])).await;
    timeout(WAIT, lab.link.wait_blocked())
        .await
        .expect("the final SegmentAck's send was not held");
    settle().await;
    assert!(
        !task.is_finished(),
        "the request completed before its final SegmentAck went out"
    );

    lab.link.release_sends(1);
    let answer = timeout(WAIT, task).await.unwrap().unwrap().unwrap();
    assert_eq!(answer.as_ref(), &[0x10, 0x20]);
    assert_segment_ack(lab.next().await, invoke_id, 1);
}

/// A final segment the peer sends again after the response completed (its
/// SegmentAck lost, say) finds no transaction: the client answers it with
/// an Abort, and the answer it already took stands.
#[tokio::test(start_paused = true)]
async fn final_segment_sent_again_after_completion_draws_an_abort() {
    let mut lab = lab().await;
    let (invoke_id, task) = lab.request().await;

    lab.peer_sends(&segment(invoke_id, 0, true, &[0x10])).await;
    assert_segment_ack(lab.next().await, invoke_id, 0);
    lab.peer_sends(&segment(invoke_id, 1, false, &[0x20])).await;
    assert_segment_ack(lab.next().await, invoke_id, 1);
    let answer = timeout(WAIT, task).await.unwrap().unwrap().unwrap();
    assert_eq!(answer.as_ref(), &[0x10, 0x20]);

    lab.peer_sends(&segment(invoke_id, 1, false, &[0x20])).await;
    assert_client_abort(
        lab.next().await,
        invoke_id,
        AbortReason::INVALID_APDU_IN_THIS_STATE,
    );
}

/// A segment longer than the reassembly keeps (1,476 octets) is a local
/// failure to retain it: the client aborts the transfer to the peer and
/// fails the request with the same reason (Clause 5.4.4.4).
#[tokio::test(start_paused = true)]
async fn oversized_segment_aborts_the_reassembly() {
    let mut lab = lab().await;
    let (invoke_id, task) = lab.request().await;

    lab.peer_sends(&segment(invoke_id, 0, true, &[0x10])).await;
    assert_segment_ack(lab.next().await, invoke_id, 0);
    lab.peer_sends(&segment(invoke_id, 1, true, &[0x5a; 1477]))
        .await;
    assert_client_abort(lab.next().await, invoke_id, AbortReason::BUFFER_OVERFLOW);

    let error = timeout(WAIT, task).await.unwrap().unwrap().unwrap_err();
    assert!(
        matches!(
            error,
            Error::Remote(RemoteError::Abort {
                reason: AbortReason::BUFFER_OVERFLOW
            })
        ),
        "{error:?}"
    );
}

/// A confirmed COV notification is published to subscribers before its
/// acknowledgment is sent: while that send is held, the event is already
/// there (#2603).
#[tokio::test(start_paused = true)]
async fn confirmed_cov_notification_is_published_before_its_ack() {
    let mut lab = lab().await;
    let mut events = lab.client.cov_notifications();
    let mut body = BytesMut::new();
    cov_notification(7).encode(&mut body).unwrap();
    let request = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 480,
        invoke_id: 0x21,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
        service_request: body.freeze(),
    });

    lab.link.block_next_send();
    lab.peer_sends(&request).await;
    timeout(WAIT, lab.link.wait_blocked())
        .await
        .expect("the acknowledgment's send was not held");
    let event = events
        .try_recv()
        .expect("the event was not published before the acknowledgment's send");
    assert_eq!(event.subscriber_process_identifier(), 7);

    lab.link.release_sends(1);
    let Apdu::SimpleAck(ack) = lab.next().await else {
        panic!("expected the notification's SimpleAck");
    };
    assert_eq!(ack.invoke_id, 0x21);
}

/// A SegmentAck with the server bit clear comes from a requester, not from
/// the server answering, so it neither aborts the reassembly under way for
/// its invoke ID nor draws anything back: the transfer completes.
#[tokio::test(start_paused = true)]
async fn segment_ack_with_server_bit_clear_leaves_the_reassembly_alone() {
    let mut lab = lab().await;
    let (invoke_id, task) = lab.request().await;

    lab.peer_sends(&segment(invoke_id, 0, true, &[0x10])).await;
    assert_segment_ack(lab.next().await, invoke_id, 0);
    lab.peer_sends(&Apdu::SegmentAck(SegmentAckPdu {
        negative_ack: false,
        sent_by_server: false,
        invoke_id,
        sequence_number: 0,
        actual_window_size: 1,
    }))
    .await;
    lab.peer_sends(&segment(invoke_id, 1, false, &[0x20])).await;
    assert_segment_ack(lab.next().await, invoke_id, 1);
    let answer = timeout(WAIT, task).await.unwrap().unwrap().unwrap();
    assert_eq!(answer.as_ref(), &[0x10, 0x20]);
}
