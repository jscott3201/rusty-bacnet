//! A ConfirmedEventNotification that comes in segments to a node with both
//! roles (#2743): the server reassembles and answers it, as it always did,
//! and hands the whole notification to the node's client, which publishes it
//! once per transmission, as it publishes one that comes whole.
use super::segmentation::{identity, node};
use super::segmentation_wire::{abort, ack, count, from_peer, peer, sent};
use super::*;
use bacnet_client::client::{
    COVEvent, ClientOptions, EventNotificationDelivery, ReceivedEventNotification,
};
use bacnet_encoding::npdu::NpduAddress;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::cov::COVNotificationRequest;
use bacnet_types::enums::{AbortReason, Segmentation};
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};

pub(super) const EVENT: ConfirmedServiceChoice =
    ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION;

/// The peer's notification for `process`, with a message long enough to
/// split into several segments.
pub(super) fn notification(process: u32) -> EventNotificationRequest {
    EventNotificationRequest {
        process_identifier: process,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, PEER_DEVICE)
            .unwrap(),
        event_object_identifier: ai1(),
        timestamp: BACnetTimeStamp::SequenceNumber(process as u16),
        notification_class: 4,
        priority: 100,
        event_type: EventType::OUT_OF_RANGE,
        message_text: Some(format!("high limit {process}: ").repeat(4)),
        notify_type: NotifyType::ALARM,
        ack_required: false,
        from_state: EventState::NORMAL,
        to_state: EventState::HIGH_LIMIT,
        event_values: None,
    }
}

/// [`notification`]'s service parameters.
pub(super) fn parameters(process: u32) -> Bytes {
    let mut encoded = BytesMut::new();
    encode_event_notification(&notification(process), &mut encoded).unwrap();
    encoded.freeze()
}

/// `body` for `service` in `parts` segments under `invoke_id`, each
/// proposing `window`.
pub(super) fn segments(
    service: ConfirmedServiceChoice,
    invoke_id: u8,
    body: &Bytes,
    parts: usize,
    window: u8,
) -> Vec<Apdu> {
    let size = body.len().div_ceil(parts);
    (0..parts)
        .map(|seq| {
            Apdu::ConfirmedRequest(ConfirmedRequest {
                segmented: true,
                more_follows: seq + 1 < parts,
                segmented_response_accepted: false,
                max_segments: None,
                max_apdu_length: 1476,
                invoke_id,
                sequence_number: Some(seq as u8),
                proposed_window_size: Some(window),
                service_choice: service,
                service_request: body.slice(seq * size..body.len().min((seq + 1) * size)),
            })
        })
        .collect()
}

/// The event notification for `process` in `parts` segments.
pub(super) fn event_segments(invoke_id: u8, process: u32, parts: usize, window: u8) -> Vec<Apdu> {
    segments(EVENT, invoke_id, &parameters(process), parts, window)
}

/// The same notification, whole.
pub(super) fn whole(invoke_id: u8, process: u32) -> Apdu {
    Apdu::ConfirmedRequest(ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: EVENT,
        service_request: parameters(process),
    })
}

/// Put `apdu` in front of the node, from the peer itself, or, with
/// `source`, from a device behind the peer acting as its router.
pub(super) fn deliver(wire: &Wire, apdu: &Apdu, source: Option<&NpduAddress>) {
    let Some(source) = source else {
        return from_peer(wire, apdu);
    };
    let mut payload = BytesMut::new();
    encode_apdu(&mut payload, apdu).unwrap();
    let npdu = Npdu {
        expecting_reply: matches!(apdu, Apdu::ConfirmedRequest(_)),
        source: Some(source.clone()),
        payload: payload.freeze(),
        ..Npdu::default()
    };
    let mut encoded = BytesMut::new();
    encode_npdu(&mut encoded, &npdu).unwrap();
    wire.deliver(
        &Wire::mac(NODE),
        wire::frame(&peer(), encoded.freeze(), false),
    );
}

/// Send `parts` as a requester does: segment zero alone, then a window at
/// a time, each once the node has acknowledged the window before it.
pub(super) async fn send(wire: &Wire, log: &SendLog, parts: &[Apdu]) {
    send_from(wire, log, parts, None).await;
}

/// [`send`], from `source` behind the peer when there is one. Each
/// acknowledgment waited for is a new one, so a transfer sent again waits
/// for its own.
pub(super) async fn send_from(
    wire: &Wire,
    log: &SendLog,
    parts: &[Apdu],
    source: Option<&NpduAddress>,
) {
    let mut acknowledged = 0;
    for (seq, part) in parts.iter().enumerate() {
        let Apdu::ConfirmedRequest(segment) = part else {
            unreachable!()
        };
        let window = usize::from(segment.proposed_window_size.unwrap());
        let want = ack(true, segment.invoke_id, seq as u8);
        let before = count(log, &want);
        deliver(wire, part, source);
        if seq == 0 || seq == acknowledged + window || seq + 1 == parts.len() {
            eventually(WAIT, "the node's Segment-ACK", async || {
                count(log, &want) > before
            })
            .await;
            acknowledged = seq;
        }
    }
}

/// The node's Simple-ACK of an event notification under `invoke_id`.
pub(super) fn acknowledged(invoke_id: u8) -> impl Fn(&Apdu) -> bool {
    move |apdu| {
        matches!(apdu, Apdu::SimpleAck(ack)
            if ack.invoke_id == invoke_id && ack.service_choice == EVENT)
    }
}

/// The next notification `events` publishes, failing rather than hanging.
pub(super) async fn heard(
    events: &mut Receiver<ReceivedEventNotification>,
) -> ReceivedEventNotification {
    tokio::time::timeout(WAIT, events.recv())
        .await
        .expect("no event notification published")
        .unwrap()
}

/// `events` publishes nothing more, a second later.
pub(super) async fn nothing_more(events: &mut Receiver<ReceivedEventNotification>) {
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(matches!(events.try_recv(), Err(TryRecvError::Empty)));
}

/// Every Abort, Reject and Error the node sent the peer.
pub(super) fn refusals(log: &SendLog) -> usize {
    count(log, |apdu| {
        matches!(apdu, Apdu::Abort(_) | Apdu::Reject(_) | Apdu::Error(_))
    })
}

/// A node with both roles that segments both ways.
pub(super) async fn both(wire: &Wire) -> (EndpointSession<TestTransport>, SendLog) {
    let mode = Segmentation::SEGMENTED_BOTH;
    let (session, log, _link) = node(wire, SessionRole::Both, identity(NODE_DEVICE, mode)).await;
    (session, log)
}

/// At windows of one and of four: the server acknowledges each window and
/// answers once, and the client publishes the whole notification once.
#[tokio::test(start_paused = true)]
async fn the_server_answers_and_the_client_publishes_once() {
    let wire = Wire::new();
    let (mut session, log) = both(&wire).await;
    let client = session.client().unwrap();
    let mut events = client.event_notifications();

    for (id, process, parts, window) in [(0x51, 11, 3, 1), (0x52, 12, 6, 4)] {
        send(&wire, &log, &event_segments(id, process, parts, window)).await;
        sent(&log, "the server's Simple-ACK", acknowledged(id)).await;
        let event = heard(&mut events).await;
        assert_eq!(event.notification, notification(process));
        assert_eq!(event.source_mac, peer());
        assert_eq!(event.delivery, EventNotificationDelivery::Confirmed);
        nothing_more(&mut events).await;
        assert_eq!(count(&log, acknowledged(id)), 1, "one answer");
    }
    // Segment zero and each window's last: every one of 3 at a window of
    // one; 0, 4 and 5 of 6 at a window of four.
    let acks = |id| {
        count(
            &log,
            |apdu| matches!(apdu, Apdu::SegmentAck(a) if a.invoke_id == id),
        )
    };
    assert_eq!((acks(0x51), acks(0x52)), (3, 3));
    assert_eq!(refusals(&log), 0);
    assert_eq!(session.client_seat_counters().full_drops, 0);

    drop(client);
    session.stop().await.unwrap();
}

/// One that comes whole is published as before, and the client hears whole
/// and segmented notifications from one peer in the order the server
/// answers them.
#[tokio::test(start_paused = true)]
async fn whole_and_segmented_notifications_are_published_in_order() {
    let wire = Wire::new();
    let (mut session, log) = both(&wire).await;
    let client = session.client().unwrap();
    let mut events = client.event_notifications();

    from_peer(&wire, &whole(0x61, 21));
    sent(&log, "the first answer", acknowledged(0x61)).await;
    send(&wire, &log, &event_segments(0x62, 22, 3, 1)).await;
    sent(&log, "the second answer", acknowledged(0x62)).await;
    from_peer(&wire, &whole(0x63, 23));
    sent(&log, "the third answer", acknowledged(0x63)).await;

    for process in [21, 22, 23] {
        assert_eq!(heard(&mut events).await.notification, notification(process));
    }
    nothing_more(&mut events).await;
    assert_eq!(refusals(&log), 0);

    drop(client);
    session.stop().await.unwrap();
}

/// Parameters that don't decode draw the server's Reject, whole or in
/// segments, with the same reason; the client publishes neither.
#[tokio::test(start_paused = true)]
async fn a_notification_that_does_not_decode_is_rejected_and_not_published() {
    let wire = Wire::new();
    let (mut session, log) = both(&wire).await;
    let client = session.client().unwrap();
    let mut events = client.event_notifications();
    // A process identifier, then a tag no notification has there.
    let broken = Bytes::from_static(&[0x09, 0x01, 0x7e, 0x01, 0x02, 0x03, 0x7f, 0x04]);

    let mut whole_one = whole(0x71, 0);
    if let Apdu::ConfirmedRequest(request) = &mut whole_one {
        request.service_request = broken.clone();
    }
    from_peer(&wire, &whole_one);
    let rejected = |id| move |apdu: &Apdu| matches!(apdu, Apdu::Reject(r) if r.invoke_id == id);
    let Apdu::Reject(first) = sent(&log, "the Reject of the whole one", rejected(0x71)).await
    else {
        unreachable!()
    };
    send(&wire, &log, &segments(EVENT, 0x72, &broken, 2, 1)).await;
    let Apdu::Reject(second) = sent(&log, "the Reject of the segmented one", rejected(0x72)).await
    else {
        unreachable!()
    };
    assert_eq!(first.reject_reason, second.reject_reason);
    nothing_more(&mut events).await;
    assert_eq!(count(&log, |apdu| matches!(apdu, Apdu::SimpleAck(_))), 0);
    // One Reject each, and a Segment-ACK for each of the two segments.
    assert_eq!(
        (count(&log, rejected(0x71)), count(&log, rejected(0x72))),
        (1, 1)
    );
    let segment_acks = count(&log, |apdu| matches!(apdu, Apdu::SegmentAck(_)));
    assert_eq!((segment_acks, refusals(&log)), (2, 2));

    drop(client);
    session.stop().await.unwrap();
}

/// A subscriber that falls behind loses the oldest notification, as with
/// whole ones; the server answers each all the same.
#[tokio::test(start_paused = true)]
async fn a_subscriber_that_falls_behind_holds_nothing_up() {
    let wire = Wire::new();
    let (mut session, log) = both(&wire).await;
    let options = ClientOptions::default().with_event_channel_capacity(1);
    let client = session.client_with(options).unwrap();
    let mut events = client.event_notifications();

    for (id, process) in [(0x81, 31), (0x82, 32)] {
        send(&wire, &log, &event_segments(id, process, 3, 1)).await;
        sent(&log, "the server's Simple-ACK", acknowledged(id)).await;
    }
    // Each was handed on before the next arrived: the first was published
    // and then pushed out by the second.
    eventually(WAIT, "both published", async || events.len() == 2).await;
    assert!(matches!(events.recv().await, Err(RecvError::Lagged(1))));
    assert_eq!(heard(&mut events).await.notification, notification(32));
    nothing_more(&mut events).await;
    assert_eq!(refusals(&log), 0);

    drop(client);
    session.stop().await.unwrap();
}

/// The last segment, sent again before the answer, is acknowledged again
/// (#2751), and again after the answer it draws the Abort as before; the
/// client publishes the notification once.
#[tokio::test(start_paused = true)]
async fn a_repeated_segment_is_acknowledged_again_and_published_once() {
    let wire = Wire::new();
    let (mut session, log) = both(&wire).await;
    let client = session.client().unwrap();
    let mut events = client.event_notifications();
    let parts = event_segments(0x91, 41, 3, 1);

    send(&wire, &log, &parts[..2]).await;
    // The last segment and its repeat reach the server together, so the
    // repeat finds the request whole and not yet answered.
    let node_mac = Wire::mac(NODE);
    wire.hold(&node_mac);
    from_peer(&wire, &parts[2]);
    from_peer(&wire, &parts[2]);
    wire.release(&node_mac);
    sent(&log, "the server's Simple-ACK", acknowledged(0x91)).await;
    eventually(WAIT, "the repeat acknowledged again", async || {
        count(&log, ack(true, 0x91, 2)) == 2
    })
    .await;
    assert_eq!(heard(&mut events).await.notification, notification(41));

    from_peer(&wire, &parts[2]);
    let Apdu::Abort(late) = sent(&log, "the Abort of a late repeat", abort(true, 0x91)).await
    else {
        unreachable!()
    };
    assert_eq!(late.abort_reason, AbortReason::INVALID_APDU_IN_THIS_STATE);
    nothing_more(&mut events).await;
    assert_eq!(count(&log, acknowledged(0x91)), 1);

    drop(client);
    session.stop().await.unwrap();
}

/// The three notifications a client executes, each in segments on one
/// node: the client answers the COV notifications of either form and the
/// server the event notification, and each is published once.
#[tokio::test(start_paused = true)]
async fn every_segmented_notification_is_published_once() {
    let wire = Wire::new();
    let (mut session, log) = both(&wire).await;
    let client = session.client().unwrap();
    let mut events = client.event_notifications();
    let mut covs = client.cov_notifications();

    let mut cov = BytesMut::new();
    COVNotificationRequest {
        subscriber_process_identifier: 51,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, PEER_DEVICE)
            .unwrap(),
        monitored_object_identifier: ai1(),
        time_remaining: 300,
        list_of_values: vec![BACnetPropertyValue {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            value: vec![0x44, 0x42, 0x28, 0x00, 0x00],
            priority: None,
        }],
    }
    .encode(&mut cov)
    .unwrap();
    let single = ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION;
    let multiple = ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE;
    let mut cov_multiple = BytesMut::new();
    super::segmented_notifications::notification(52)
        .encode(&mut cov_multiple)
        .unwrap();
    let sent_whole = [
        (0xa1, single, cov.freeze()),
        (0xa2, multiple, cov_multiple.freeze()),
        (0xa3, EVENT, parameters(53)),
    ];
    for (id, service, body) in &sent_whole {
        send(&wire, &log, &segments(*service, *id, body, 3, 1)).await;
        let answered = move |apdu: &Apdu| matches!(apdu, Apdu::SimpleAck(a) if a.invoke_id == *id && a.service_choice == *service);
        sent(&log, "the Simple-ACK", answered).await;
        assert_eq!(count(&log, answered), 1, "{service:?}");
    }

    let next_cov = async |covs: &mut Receiver<COVEvent>| {
        tokio::time::timeout(WAIT, covs.recv())
            .await
            .unwrap()
            .unwrap()
    };
    let COVEvent::Object(first) = next_cov(&mut covs).await else {
        panic!("the one-object form first");
    };
    assert_eq!(first.notification.subscriber_process_identifier, 51);
    let COVEvent::Multiple(second) = next_cov(&mut covs).await else {
        panic!("then the several-object form");
    };
    assert_eq!(second.notification.subscriber_process_identifier, 52);
    assert_eq!(heard(&mut events).await.notification, notification(53));
    nothing_more(&mut events).await;
    assert!(matches!(covs.try_recv(), Err(TryRecvError::Empty)));
    assert_eq!(refusals(&log), 0);

    drop(client);
    session.stop().await.unwrap();
}
