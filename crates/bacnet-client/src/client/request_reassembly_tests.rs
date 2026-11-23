//! A standalone client reassembles the segmented confirmed requests it
//! executes (#2646): each segment draws what the answering side owes, the
//! whole request is executed once, and nothing of a transfer outlives its
//! end, however it ends.
use super::super::cov_notification_tests::cov_notification;
use super::super::event_notification_tests::payload as event_payload;
use super::*;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_services::cov_multiple::COVNotificationMultipleRequest;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::{SendLog, TestTransport};
use bacnet_types::enums::AbortReason;

const CLIENT: [u8; 1] = [0x61];
const PEER: [u8; 1] = [0x62];
const OTHER: [u8; 1] = [0x63];
const WAIT: Duration = Duration::from_secs(5);
/// The client's segment timeout here: four of them is 4 s of silence.
const TSEG_MS: u64 = 1_000;

const COV: ConfirmedServiceChoice = ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION;
const COV_MULTIPLE: ConfirmedServiceChoice =
    ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE;

/// A client on a link the test feeds and whose sends it reads.
struct Lab {
    client: BACnetClient<TestTransport>,
    input: mpsc::Sender<ReceivedNpdu>,
    log: SendLog,
    /// Frames in the log already looked at.
    seen: usize,
}

async fn lab(segments: bool) -> Lab {
    let (input, rx) = mpsc::channel(512);
    let transport = TestTransport::builder()
        .local_mac(&CLIENT)
        .inbound(rx)
        .build();
    let log = transport.sent();
    let client = BACnetClient::generic_builder()
        .transport(transport)
        .segmented_response_accepted(segments)
        .apdu_segment_timeout_ms(TSEG_MS)
        .build()
        .await
        .unwrap();
    Lab {
        client,
        input,
        log,
        seen: 0,
    }
}

fn npdu_of(apdu: &Apdu, source: Option<NpduAddress>) -> Bytes {
    let mut encoded = BytesMut::new();
    encode_apdu(&mut encoded, apdu).unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            source,
            expecting_reply: matches!(apdu, Apdu::ConfirmedRequest(_)),
            payload: encoded.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    npdu.freeze()
}

fn received(peer: &[u8], apdu: &Apdu, source: Option<NpduAddress>) -> ReceivedNpdu {
    ReceivedNpdu {
        npdu: npdu_of(apdu, source),
        source_mac: MacAddr::from_slice(peer),
        link_layer_group: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        direct_response: None,
        reply_tx: None,
    }
}

impl Lab {
    async fn from(&self, peer: &[u8], apdu: &Apdu) {
        self.input.send(received(peer, apdu, None)).await.unwrap();
    }

    /// The next frame the client sent.
    async fn next_frame(&mut self) -> bacnet_transport::test_transport::SentFrame {
        timeout(WAIT, self.log.wait_for_len(self.seen + 1))
            .await
            .expect("the client sent nothing more");
        let frame = self.log.frame(self.seen);
        self.seen += 1;
        frame
    }

    /// The next APDU the client sent, which went to `peer`.
    async fn next(&mut self, peer: &[u8]) -> Apdu {
        let frame = self.next_frame().await;
        assert_eq!(frame.mac.as_slice(), peer);
        frame.apdu()
    }

    /// Prove the client sent nothing more so far: it answers an
    /// unsegmented ReadProperty, which it doesn't execute, with a Reject,
    /// and that Reject is the next frame.
    async fn quiet(&mut self) {
        let fence = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 480,
            invoke_id: 0xee,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_request: Bytes::from_static(&[0x0c]),
        });
        self.from(&PEER, &fence).await;
        let Apdu::Reject(reject) = self.next(&PEER).await else {
            panic!("the client sent something before the fence's answer");
        };
        assert_eq!(reject.invoke_id, 0xee);
    }

    fn transfers(&self) -> Option<(usize, usize)> {
        self.client.request_transfers.transfers()
    }
}

/// Segment `seq` of a request for `service` under `invoke_id`.
fn segment(
    invoke_id: u8,
    service: ConfirmedServiceChoice,
    seq: u8,
    window: u8,
    more: bool,
    data: Bytes,
) -> Apdu {
    Apdu::ConfirmedRequest(ConfirmedRequestPdu {
        segmented: true,
        more_follows: more,
        segmented_response_accepted: true,
        max_segments: None,
        max_apdu_length: 480,
        invoke_id,
        sequence_number: Some(seq),
        proposed_window_size: Some(window),
        service_choice: service,
        service_request: data,
    })
}

/// `body` cut into segments of `size` octets.
fn segments(
    invoke_id: u8,
    service: ConfirmedServiceChoice,
    body: &Bytes,
    size: usize,
    window: u8,
) -> Vec<Apdu> {
    let count = body.len().div_ceil(size);
    (0..count)
        .map(|n| {
            let data = body.slice(n * size..body.len().min((n + 1) * size));
            segment(invoke_id, service, n as u8, window, n + 1 < count, data)
        })
        .collect()
}

fn cov_body(process: u32) -> Bytes {
    let mut body = BytesMut::new();
    cov_notification(process).encode(&mut body).unwrap();
    body.freeze()
}

/// A COV-multiple about AI 1 to AI `objects`, each with its Present_Value.
fn multiple_body(process: u32, objects: u32) -> (COVNotificationMultipleRequest, Bytes) {
    use bacnet_services::cov_multiple::{COVNotificationItem, COVNotificationValue};
    use bacnet_types::enums::{ObjectType, PropertyIdentifier};
    use bacnet_types::primitives::ObjectIdentifier;
    let request = COVNotificationMultipleRequest {
        subscriber_process_identifier: process,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 100).unwrap(),
        time_remaining: 300,
        timestamp: None,
        list_of_cov_notifications: (1..=objects)
            .map(|instance| COVNotificationItem {
                monitored_object_identifier: ObjectIdentifier::new(
                    ObjectType::ANALOG_INPUT,
                    instance,
                )
                .unwrap(),
                list_of_values: vec![COVNotificationValue {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    value: vec![0x44, 0x42, 0x48, 0x00, 0x00],
                    time_of_change: None,
                }],
            })
            .collect(),
    };
    let mut body = BytesMut::new();
    request.encode(&mut body).unwrap();
    (request, body.freeze())
}

#[track_caller]
fn segment_ack(apdu: Apdu, invoke_id: u8, seq: u8, negative: bool, window: u8) {
    let Apdu::SegmentAck(ack) = apdu else {
        panic!("expected a Segment-ACK, got {apdu:?}");
    };
    assert!(ack.sent_by_server, "the answering side sets the server bit");
    assert_eq!(
        (
            ack.invoke_id,
            ack.sequence_number,
            ack.negative_ack,
            ack.actual_window_size
        ),
        (invoke_id, seq, negative, window)
    );
}

#[track_caller]
fn simple_ack(apdu: Apdu, invoke_id: u8, service: ConfirmedServiceChoice) {
    let Apdu::SimpleAck(ack) = apdu else {
        panic!("expected a Simple-ACK, got {apdu:?}");
    };
    assert_eq!((ack.invoke_id, ack.service_choice), (invoke_id, service));
}

#[track_caller]
fn aborted(apdu: Apdu, invoke_id: u8, reason: AbortReason) {
    let Apdu::Abort(abort) = apdu else {
        panic!("expected an Abort, got {apdu:?}");
    };
    assert!(abort.sent_by_server);
    assert_eq!((abort.invoke_id, abort.abort_reason), (invoke_id, reason));
}

#[tokio::test]
async fn a_segmented_cov_notification_is_acknowledged_segment_by_segment_and_published_once() {
    let mut lab = lab(true).await;
    let mut events = lab.client.cov_notifications();
    let parts = segments(5, COV, &cov_body(40), 8, 1);
    assert!(parts.len() >= 3);
    for (seq, part) in parts.iter().enumerate() {
        lab.from(&PEER, part).await;
        segment_ack(lab.next(&PEER).await, 5, seq as u8, false, 1);
    }
    simple_ack(lab.next(&PEER).await, 5, COV);
    let COVEvent::Object(event) = events.recv().await.unwrap() else {
        panic!("expected the one-object form");
    };
    assert_eq!(event.notification, cov_notification(40));
    assert_eq!(event.source_mac.as_slice(), PEER);
    assert_eq!(event.delivery, COVNotificationDelivery::Confirmed);
    lab.quiet().await;
    assert!(events.try_recv().is_err(), "published once");
    assert_eq!(lab.transfers(), Some((0, 0)));
    lab.client.stop().await.unwrap();
}

#[tokio::test]
async fn a_segmented_cov_multiple_is_acknowledged_once_a_window() {
    let mut lab = lab(true).await;
    let mut events = lab.client.cov_notifications();
    let (request, body) = multiple_body(41, 12);
    let parts = segments(6, COV_MULTIPLE, &body, 16, 3);
    assert!(parts.len() >= 6, "{} segments", parts.len());
    let last = parts.len() - 1;
    for (seq, part) in parts.iter().enumerate() {
        lab.from(&PEER, part).await;
        // Segment zero at once, then each third, then the last.
        if seq == 0 || seq % 3 == 0 || seq == last {
            segment_ack(lab.next(&PEER).await, 6, seq as u8, false, 3);
        }
    }
    simple_ack(lab.next(&PEER).await, 6, COV_MULTIPLE);
    let COVEvent::Multiple(event) = events.recv().await.unwrap() else {
        panic!("expected the several-object form");
    };
    assert_eq!(event.notification, request);
    lab.quiet().await;
    assert!(events.try_recv().is_err(), "published once");
    lab.client.stop().await.unwrap();
}

#[tokio::test]
async fn a_duplicate_is_dropped_and_a_gap_draws_a_negative_ack() {
    let mut lab = lab(true).await;
    let mut events = lab.client.cov_notifications();
    let (request, body) = multiple_body(42, 6);
    let parts = segments(7, COV_MULTIPLE, &body, 16, 3);
    let last = parts.len() - 1;
    assert!(last >= 5);
    lab.from(&PEER, &parts[0]).await;
    segment_ack(lab.next(&PEER).await, 7, 0, false, 3);
    lab.from(&PEER, &parts[1]).await;
    // A duplicate is dropped without a word.
    lab.from(&PEER, &parts[1]).await;
    // Segment 2 is lost: segment 3 draws a negative Segment-ACK for 1.
    lab.from(&PEER, &parts[3]).await;
    segment_ack(lab.next(&PEER).await, 7, 1, true, 3);
    // The requester goes again from segment 2; the window now counts from
    // segment 1, so it ends at 4, 7 and so on, and at the last.
    for (seq, part) in parts.iter().enumerate().skip(2) {
        lab.from(&PEER, part).await;
        if seq % 3 == 1 || seq == last {
            segment_ack(lab.next(&PEER).await, 7, seq as u8, false, 3);
        }
    }
    simple_ack(lab.next(&PEER).await, 7, COV_MULTIPLE);
    let COVEvent::Multiple(event) = events.recv().await.unwrap() else {
        panic!("expected the several-object form");
    };
    assert_eq!(event.notification, request);
    lab.quiet().await;
    assert!(events.try_recv().is_err());
    lab.client.stop().await.unwrap();
}

/// A requester that goes quiet: four segment timeouts later the transfer
/// is gone, with nothing sent (Clause 5.4.5.2), and a late segment finds
/// no transfer.
#[tokio::test(start_paused = true)]
async fn a_silent_transfer_ends_without_a_word_and_a_late_segment_is_aborted() {
    let mut lab = lab(true).await;
    let parts = segments(8, COV, &cov_body(43), 8, 1);
    lab.from(&PEER, &parts[0]).await;
    segment_ack(lab.next(&PEER).await, 8, 0, false, 1);
    assert_eq!(lab.transfers().map(|(open, _)| open), Some(1));
    tokio::time::sleep(Duration::from_millis(4 * TSEG_MS - 1)).await;
    assert_eq!(lab.transfers().map(|(open, _)| open), Some(1));
    // The timer rounds up to the next millisecond and fires first.
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(lab.transfers(), Some((0, 0)));
    lab.quiet().await;
    lab.from(&PEER, &parts[1]).await;
    aborted(
        lab.next(&PEER).await,
        8,
        AbortReason::INVALID_APDU_IN_THIS_STATE,
    );
    lab.client.stop().await.unwrap();
}

/// A requester that only repeats a gap keeps the segment timer running but
/// saves nothing: the transfer ends with an Abort (OTHER) after the
/// progress limit, as the server's does.
#[tokio::test(start_paused = true)]
async fn a_transfer_that_makes_no_progress_is_aborted() {
    let mut lab = lab(true).await;
    let parts = segments(9, COV, &cov_body(44), 8, 1);
    lab.from(&PEER, &parts[0]).await;
    segment_ack(lab.next(&PEER).await, 9, 0, false, 1);
    let started = tokio::time::Instant::now();
    loop {
        tokio::time::sleep(Duration::from_millis(TSEG_MS)).await;
        lab.from(&PEER, &parts[2]).await;
        let apdu = lab.next(&PEER).await;
        if matches!(apdu, Apdu::Abort(_)) {
            aborted(apdu, 9, AbortReason::OTHER);
            break;
        }
        segment_ack(apdu, 9, 0, true, 1);
        assert!(started.elapsed() < Duration::from_secs(17));
    }
    assert!(started.elapsed() >= Duration::from_secs(16));
    assert_eq!(lab.transfers(), Some((0, 0)));
    lab.client.stop().await.unwrap();
}

#[tokio::test]
async fn a_requesters_abort_ends_its_transfer_and_draws_nothing() {
    let mut lab = lab(true).await;
    let parts = segments(10, COV, &cov_body(45), 8, 1);
    lab.from(&PEER, &parts[0]).await;
    segment_ack(lab.next(&PEER).await, 10, 0, false, 1);
    lab.from(
        &PEER,
        &Apdu::Abort(AbortPdu {
            sent_by_server: false,
            invoke_id: 10,
            abort_reason: AbortReason::OTHER,
        }),
    )
    .await;
    lab.quiet().await;
    assert_eq!(lab.transfers(), Some((0, 0)));
    lab.from(&PEER, &parts[1]).await;
    aborted(
        lab.next(&PEER).await,
        10,
        AbortReason::INVALID_APDU_IN_THIS_STATE,
    );
    lab.client.stop().await.unwrap();
}

#[tokio::test]
async fn two_peers_reassemble_at_once_and_one_peer_is_capped() {
    let mut lab = lab(true).await;
    let mut events = lab.client.cov_notifications();
    let ours = segments(11, COV, &cov_body(46), 8, 1);
    let theirs = segments(11, COV, &cov_body(47), 8, 1);
    let last = ours.len() - 1;
    for (seq, (ours, theirs)) in ours.iter().zip(&theirs).enumerate() {
        for (peer, part) in [(&PEER, ours), (&OTHER, theirs)] {
            lab.from(peer, part).await;
            segment_ack(lab.next(peer).await, 11, seq as u8, false, 1);
            if seq == last {
                simple_ack(lab.next(peer).await, 11, COV);
            }
        }
    }
    let mut sources = Vec::new();
    for _ in 0..2 {
        let event = events.recv().await.unwrap();
        sources.push((
            event.source_mac().to_vec(),
            event.subscriber_process_identifier(),
        ));
    }
    sources.sort();
    assert_eq!(sources, [(PEER.to_vec(), 46), (OTHER.to_vec(), 47)]);

    // One peer holds at most sixteen transfers at once; another still
    // opens one.
    let per_peer = bacnet_endpoint_core::segmented_request::MAX_TRANSFERS_PER_PEER as u8;
    for invoke_id in 0..per_peer {
        lab.from(
            &PEER,
            &segments(20 + invoke_id, COV, &cov_body(48), 8, 1)[0],
        )
        .await;
        segment_ack(lab.next(&PEER).await, 20 + invoke_id, 0, false, 1);
    }
    lab.from(&PEER, &segments(60, COV, &cov_body(48), 8, 1)[0])
        .await;
    aborted(lab.next(&PEER).await, 60, AbortReason::OUT_OF_RESOURCES);
    lab.from(&OTHER, &segments(60, COV, &cov_body(48), 8, 1)[0])
        .await;
    segment_ack(lab.next(&OTHER).await, 60, 0, false, 1);
    assert_eq!(lab.transfers().map(|(open, _)| open), Some(17));
    lab.client.stop().await.unwrap();
}

/// A client that takes no segments, as an endpoint's does on a Device that
/// declares NO_SEGMENTATION or SEGMENTED_TRANSMIT, answers the first
/// segment with SEGMENTATION_NOT_SUPPORTED and keeps nothing.
#[tokio::test]
async fn a_client_that_takes_no_segments_aborts_the_first_segment() {
    let mut lab = lab(false).await;
    let mut events = lab.client.cov_notifications();
    let (_, body) = multiple_body(49, 4);
    let parts = segments(12, COV_MULTIPLE, &body, 16, 1);
    lab.from(&PEER, &parts[0]).await;
    aborted(
        lab.next(&PEER).await,
        12,
        AbortReason::SEGMENTATION_NOT_SUPPORTED,
    );
    lab.quiet().await;
    assert!(events.try_recv().is_err());
    assert_eq!(lab.transfers(), None, "no table was opened");
    lab.client.stop().await.unwrap();
}

/// A segmented request for a service the client doesn't execute keeps the
/// Abort it always drew.
#[tokio::test]
async fn a_segmented_request_the_client_does_not_execute_is_aborted() {
    let mut lab = lab(true).await;
    lab.from(
        &PEER,
        &segment(
            13,
            ConfirmedServiceChoice::READ_PROPERTY,
            0,
            1,
            true,
            Bytes::from_static(&[0x0c]),
        ),
    )
    .await;
    aborted(
        lab.next(&PEER).await,
        13,
        AbortReason::SEGMENTATION_NOT_SUPPORTED,
    );
    assert_eq!(lab.transfers(), None);
    lab.client.stop().await.unwrap();
}

#[tokio::test]
async fn a_segmented_event_notification_is_published_once() {
    let mut lab = lab(true).await;
    let mut events = lab.client.event_notifications();
    let body = event_payload(Some(&[0, 0xff]));
    let parts = segments(
        14,
        ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
        &body,
        20,
        2,
    );
    assert!(parts.len() >= 2);
    for part in &parts {
        lab.from(&PEER, part).await;
    }
    let last = parts.len() - 1;
    segment_ack(lab.next(&PEER).await, 14, 0, false, 2);
    for seq in (2..last).step_by(2) {
        segment_ack(lab.next(&PEER).await, 14, seq as u8, false, 2);
    }
    segment_ack(lab.next(&PEER).await, 14, last as u8, false, 2);
    simple_ack(
        lab.next(&PEER).await,
        14,
        ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
    );
    let event = events.recv().await.unwrap();
    assert_eq!(event.notification.process_identifier, 42);
    assert_eq!(event.source_mac.as_slice(), PEER);
    lab.quiet().await;
    assert!(events.try_recv().is_err());
    lab.client.stop().await.unwrap();
}

/// Behind a router: each answer goes back to it, with the peer's network
/// and address as its destination.
#[tokio::test]
async fn a_routed_requesters_segments_are_answered_through_its_router() {
    let mut lab = lab(true).await;
    let behind = NpduAddress {
        network: 300,
        mac_address: MacAddr::from_slice(&[0x31, 0x32]),
    };
    for part in segments(15, COV, &cov_body(50), 8, 1) {
        lab.input
            .send(received(&PEER, &part, Some(behind.clone())))
            .await
            .unwrap();
        let frame = lab.next_frame().await;
        assert_eq!(frame.mac.as_slice(), PEER);
        assert_eq!(frame.decode_npdu().destination, Some(behind.clone()));
        assert!(matches!(frame.apdu(), Apdu::SegmentAck(_)));
    }
    let frame = lab.next_frame().await;
    assert_eq!(frame.decode_npdu().destination, Some(behind));
    simple_ack(frame.apdu(), 15, COV);
    lab.client.stop().await.unwrap();
}

/// On MS/TP a segment that expects a reply gets its Segment-ACK as that
/// reply. One that draws nothing lets the reply sender go, so the link
/// answers Reply Postponed; the whole request's answer goes out on its own.
#[tokio::test]
async fn on_mstp_a_segment_ack_is_the_reply_frame() {
    let mut lab = lab(true).await;
    let parts = segments(16, COV, &cov_body(51), 8, 2);
    assert!(parts.len() >= 3);
    let mut replies = Vec::new();
    for part in &parts {
        let (reply_tx, reply_rx) = oneshot::channel();
        let mut frame = received(&PEER, part, None);
        frame.reply_tx = Some(reply_tx);
        lab.input.send(frame).await.unwrap();
        replies.push(reply_rx);
    }
    let last = parts.len() - 1;
    for (seq, reply) in replies.into_iter().enumerate() {
        let answered = timeout(WAIT, reply).await.unwrap();
        // Segment zero, the window's end and the last are acknowledged.
        if seq == 0 || seq % 2 == 0 || seq == last {
            let npdu = decode_npdu(answered.expect("a Segment-ACK as the reply")).unwrap();
            let apdu = apdu::decode_apdu(npdu.payload).unwrap();
            segment_ack(apdu, 16, seq as u8, false, 2);
        } else {
            assert!(answered.is_err(), "segment {seq} postpones its reply");
        }
    }
    simple_ack(lab.next(&PEER).await, 16, COV);
    lab.client.stop().await.unwrap();
}

/// A stop midway leaves nothing: the table and its timer go with the
/// dispatch task, and no Abort follows, even past every timer.
#[tokio::test(start_paused = true)]
async fn a_stop_midway_leaves_nothing_behind() {
    let mut lab = lab(true).await;
    lab.from(&PEER, &segments(17, COV, &cov_body(52), 8, 1)[0])
        .await;
    segment_ack(lab.next(&PEER).await, 17, 0, false, 1);
    let hook = Arc::clone(&lab.client.request_transfers);
    lab.client.stop().await.unwrap();
    assert!(hook.ended(), "the table is gone once stop returns");
    let sent = lab.log.len();
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(lab.log.len(), sent);
}

/// So does a drop, once the runtime has ended the aborted tasks.
#[tokio::test(start_paused = true)]
async fn a_drop_midway_leaves_nothing_behind() {
    let mut lab = lab(true).await;
    lab.from(&PEER, &segments(18, COV, &cov_body(53), 8, 1)[0])
        .await;
    segment_ack(lab.next(&PEER).await, 18, 0, false, 1);
    let hook = Arc::clone(&lab.client.request_transfers);
    let Lab { client, log, .. } = lab;
    drop(client);
    timeout(WAIT, async {
        while !hook.ended() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the table outlived the client");
    let sent = log.len();
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(log.len(), sent);
}
