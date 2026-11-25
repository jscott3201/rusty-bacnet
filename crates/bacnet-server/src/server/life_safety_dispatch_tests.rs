//! A LifeSafetyOperation takes the dispatch loop's one confirmed-request
//! spawn with its own admission (#2558): admitted once, decided once and
//! answered, and a retransmission inside the replay window draws the same
//! bytes without being admitted or decided again, whether the request came
//! whole or in segments. Under the ordinary admission, a retransmission
//! after the answer would run again.

use super::*;
use bacnet_encoding::npdu::{Npdu, decode_npdu, encode_npdu};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_types::enums::Segmentation;
use std::sync::atomic::AtomicUsize;

const SERVER_MAC: &[u8] = &[0x02];
const CLIENT_MAC: &[u8] = &[0x01];
const INVOKE: u8 = 0x2C;

type Replies = mpsc::Receiver<ReceivedNpdu>;

/// A running server that takes segments and lets every LifeSafetyOperation
/// through, counting each it decides, and the client end of its link.
async fn start(
    decided: &Arc<AtomicUsize>,
) -> (BACnetServer<LoopbackTransport>, LoopbackTransport, Replies) {
    let (server_link, mut client) =
        LoopbackTransport::pair(SERVER_MAC.to_vec(), CLIENT_MAC.to_vec());
    let replies = client.start().await.unwrap();
    let counted = Arc::clone(decided);
    let config = ServerConfig {
        device: bacnet_objects::DeviceIdentity::default()
            .with_segmentation(Segmentation::SEGMENTED_BOTH),
        policy: Some(Arc::new(crate::FnPolicy::new().life_safety_operation(
            move |_| {
                counted.fetch_add(1, Ordering::Relaxed);
                true
            },
        ))),
        ..ServerConfig::default()
    };
    let server = BACnetServer::start(config, ObjectDatabase::new(), server_link)
        .await
        .unwrap();
    (server, client, replies)
}

/// A targetless SILENCE.
fn life_safety_operation() -> ConfirmedRequestPdu {
    let mut body = BytesMut::new();
    bacnet_services::life_safety::LifeSafetyOperationRequest {
        requesting_process_identifier: 41,
        requesting_source: "operator".into(),
        request: bacnet_types::enums::LifeSafetyOperation::SILENCE,
        object_identifier: None,
    }
    .encode(&mut body)
    .unwrap();
    ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: true,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id: INVOKE,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::LIFE_SAFETY_OPERATION,
        service_request: body.freeze(),
    }
}

/// `request` in two segments at a window of one.
fn in_two_segments(request: &ConfirmedRequestPdu) -> [ConfirmedRequestPdu; 2] {
    let body = &request.service_request;
    let (first, second) = body.split_at(body.len() / 2);
    [(0, first, true), (1, second, false)].map(|(seq, chunk, more_follows)| ConfirmedRequestPdu {
        segmented: true,
        more_follows,
        sequence_number: Some(seq),
        proposed_window_size: Some(1),
        service_request: Bytes::copy_from_slice(chunk),
        ..request.clone()
    })
}

async fn send(client: &LoopbackTransport, request: ConfirmedRequestPdu) {
    let mut apdu = BytesMut::new();
    encode_apdu(&mut apdu, &Apdu::ConfirmedRequest(request)).unwrap();
    let npdu = Npdu {
        payload: apdu.freeze(),
        ..Npdu::default()
    };
    let mut frame = BytesMut::new();
    encode_npdu(&mut frame, &npdu).unwrap();
    client.send_unicast(&frame, SERVER_MAC).await.unwrap();
}

/// The next reply's APDU, as sent.
async fn reply(replies: &mut Replies) -> Bytes {
    let received = tokio::time::timeout(Duration::from_secs(2), replies.recv())
        .await
        .expect("a reply")
        .expect("the link is open");
    decode_npdu(received.npdu).unwrap().payload
}

/// Send the LifeSafetyOperation whole, or in segments taking each one's
/// Segment-ACK, and return its answer.
async fn exchange(client: &LoopbackTransport, replies: &mut Replies, segmented: bool) -> Bytes {
    let request = life_safety_operation();
    if !segmented {
        send(client, request).await;
        return reply(replies).await;
    }
    for (seq, segment) in (0..).zip(in_two_segments(&request)) {
        send(client, segment).await;
        match apdu::decode_apdu(reply(replies).await).unwrap() {
            Apdu::SegmentAck(ack) => {
                assert_eq!(ack.invoke_id, INVOKE);
                assert_eq!(ack.sequence_number, seq);
                assert!(!ack.negative_ack && ack.sent_by_server);
            }
            other => panic!("segment {seq}: expected a Segment-ACK, got {other:?}"),
        }
    }
    reply(replies).await
}

async fn answered_once_then_replayed(segmented: bool) {
    let decided = Arc::new(AtomicUsize::new(0));
    let (mut server, client, mut replies) = start(&decided).await;
    let answer = exchange(&client, &mut replies, segmented).await;
    match apdu::decode_apdu(answer.clone()).unwrap() {
        Apdu::SimpleAck(ack) => {
            assert_eq!(ack.invoke_id, INVOKE);
            assert_eq!(
                ack.service_choice,
                ConfirmedServiceChoice::LIFE_SAFETY_OPERATION
            );
        }
        other => panic!("expected a SimpleACK, got {other:?}"),
    }
    assert_eq!(decided.load(Ordering::Relaxed), 1);
    let counters = server.request_admission_counters();
    assert_eq!(counters.confirmed_admitted_total, 1);
    // The same request again, inside the replay window: the same bytes,
    // with no second admission or decision.
    assert_eq!(exchange(&client, &mut replies, segmented).await, answer);
    assert_eq!(decided.load(Ordering::Relaxed), 1, "decided once");
    let counters = server.request_admission_counters();
    assert_eq!(counters.confirmed_admitted_total, 1, "admitted once");
    server.stop().await.unwrap();
}

// Paused: the replay window is measured on tokio's clock (#1556).
#[tokio::test(start_paused = true)]
async fn a_whole_life_safety_operation_is_answered_once_then_replayed() {
    answered_once_then_replayed(false).await;
}

#[tokio::test(start_paused = true)]
async fn a_segmented_life_safety_operation_is_answered_once_then_replayed() {
    answered_once_then_replayed(true).await;
}
