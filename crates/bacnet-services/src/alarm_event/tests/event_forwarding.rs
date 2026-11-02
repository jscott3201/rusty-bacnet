//! The Notification Forwarder's view of an event notification (#1225).

use super::*;
use bacnet_encoding::constructed::decode_event_notification;
use bacnet_types::enums::{EventType, RejectReason};
use bacnet_types::primitives::StatusFlags;

fn alarm() -> EventNotificationRequest {
    EventNotificationRequest {
        process_identifier: 5,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 10).unwrap(),
        event_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap(),
        timestamp: BACnetTimeStamp::SequenceNumber(7),
        notification_class: 12,
        priority: 80,
        event_type: EventType::OUT_OF_RANGE,
        message_text: Some("high".into()),
        notify_type: NotifyType::ALARM,
        ack_required: true,
        from_state: EventState::NORMAL,
        to_state: EventState::HIGH_LIMIT,
        event_values: Some(NotificationParameters::OutOfRange {
            exceeding_value: 101.0,
            status_flags: StatusFlags::IN_ALARM,
            deadband: 1.0,
            exceeded_limit: 100.0,
        }),
    }
}

fn encoded(request: &EventNotificationRequest) -> BytesMut {
    let mut buf = BytesMut::new();
    bacnet_encoding::constructed::encode_event_notification(request, &mut buf).unwrap();
    buf
}

#[test]
fn forwarded_notification_reads_its_routing_members_and_swaps_only_the_process() {
    let request = alarm();
    let forwarded = ForwardedEventNotification::decode(&encoded(&request)).unwrap();
    assert_eq!(forwarded.process_identifier, 5);
    assert_eq!(
        forwarded.initiating_device_identifier,
        request.initiating_device_identifier
    );
    assert_eq!(
        forwarded.event_object_identifier,
        request.event_object_identifier
    );
    assert_eq!(forwarded.notification_class, 12);
    assert_eq!(forwarded.priority, 80);
    assert_eq!(forwarded.notify_type, NotifyType::ALARM);
    assert_eq!(forwarded.to_state, EventState::HIGH_LIMIT);

    let mut retargeted = request.clone();
    retargeted.process_identifier = 70_000;
    assert_eq!(forwarded.encode_for(70_000)[..], encoded(&retargeted)[..]);
    assert_eq!(
        ForwardedEventNotification::from_request(&request).unwrap(),
        forwarded
    );
}

#[test]
fn forwarded_notification_keeps_a_message_text_in_any_character_set() {
    // JIS X 0208 (charset 2) text the full decoder refuses to turn into a
    // String; a forwarder passes it on unchanged.
    let mut request = alarm();
    request.message_text = None;
    request.event_values = None;
    let plain = encoded(&request);
    let mut raw = BytesMut::new();
    // Everything up to and including [6] eventType, then the foreign text,
    // then the rest of the request.
    let split = plain
        .windows(2)
        .position(|pair| pair == [0x89, 0x00])
        .expect("[8] notifyType ALARM");
    raw.extend_from_slice(&plain[..split]);
    tags::encode_tag(&mut raw, 7, tags::TagClass::Context, 3);
    raw.extend_from_slice(&[2, 0x30, 0x22]);
    raw.extend_from_slice(&plain[split..]);
    assert!(decode_event_notification(&raw).is_err());

    let forwarded = ForwardedEventNotification::decode(&raw).unwrap();
    let sent = forwarded.encode_for(5);
    assert_eq!(
        sent[..],
        raw[..],
        "the same process identifier sends the same octets"
    );
    assert!(sent.windows(4).any(|run| run == [0x7B, 2, 0x30, 0x22]));
}

#[test]
fn forwarded_acknowledgment_needs_no_from_state() {
    let mut ack = alarm();
    ack.notify_type = NotifyType::ACK_NOTIFICATION;
    ack.event_values = None;
    let forwarded = ForwardedEventNotification::decode(&encoded(&ack)).unwrap();
    assert_eq!(forwarded.notify_type, NotifyType::ACK_NOTIFICATION);
    assert_eq!(forwarded.to_state, EventState::HIGH_LIMIT);
}

#[test]
fn forwarded_acknowledgment_refuses_alarm_only_fields() {
    // #2247: an ALARM's octets with the notify type turned to
    // ACK_NOTIFICATION carry ackRequired, fromState and eventValues, which an
    // acknowledgment leaves out. The forwarder refuses them as the full
    // decoder does, rather than passing on a request that decoder refuses.
    let mut ack = encoded(&alarm()).to_vec();
    let notify_type = ack
        .windows(2)
        .position(|pair| pair == [0x89, 0x00])
        .expect("[8] notifyType ALARM")
        + 1;
    ack[notify_type] = 0x02;
    assert!(decode_event_notification(&ack).is_err());
    assert!(ForwardedEventNotification::decode(&ack).is_err());

    // Each field alone: [9] and [10] where To State is due, [12] after it.
    let mut plain = alarm();
    plain.notify_type = NotifyType::ACK_NOTIFICATION;
    let plain = encoded(&plain).to_vec();
    let to_state = plain.len() - 2;
    let values = &encoded(&alarm())[..];
    let values = &values[values.len() - 22..];
    for (between, after) in [
        (&[0x99, 0x00][..], &[][..]),
        (&[0xA9, 0x00], &[]),
        (&[], values),
    ] {
        let data = [&plain[..to_state], between, &plain[to_state..], after].concat();
        assert!(
            ForwardedEventNotification::decode(&data).is_err(),
            "{data:02X?} must not decode"
        );
    }
}

#[test]
fn forwarded_notification_refuses_malformed_requests() {
    let whole = encoded(&alarm());
    // Truncated anywhere before the end.
    for len in [0, 1, 4, whole.len() / 2, whole.len() - 1] {
        assert!(
            ForwardedEventNotification::decode(&whole[..len]).is_err(),
            "{len} octets must not decode"
        );
    }
    // Something after the event values: a context tag, a second empty
    // event-values member, and a context-0 primitive whose value octet reads
    // as a closing tag 12.
    for extra in [&[0xD9, 0x01][..], &[0xCE, 0xCF][..], &[0x09, 0xCF][..]] {
        let mut trailing = whole.clone();
        trailing.extend_from_slice(extra);
        assert!(
            ForwardedEventNotification::decode(&trailing).is_err(),
            "{extra:02X?} after the event values must not decode"
        );
    }
    // Empty event values.
    let mut request = alarm();
    request.event_values = None;
    let mut empty = encoded(&request);
    empty.extend_from_slice(&[0xCE, 0xCF]);
    assert!(ForwardedEventNotification::decode(&empty).is_err());

    // An alarm with no fromState.
    let mut request = alarm();
    request.event_values = None;
    let plain = encoded(&request);
    let from_state = plain
        .windows(2)
        .rposition(|pair| pair == [0xA9, 0x00])
        .expect("[10] fromState NORMAL");
    let mut missing = BytesMut::from(&plain[..from_state]);
    missing.extend_from_slice(&plain[from_state + 2..]);
    assert!(ForwardedEventNotification::decode(&missing).is_err());

    // A priority past u8.
    let mut wide = request.clone();
    wide.priority = 0;
    let wide = encoded(&wide);
    let priority = wide
        .windows(2)
        .position(|pair| pair == [0x59, 0x00])
        .expect("[5] priority");
    let mut overflow = BytesMut::from(&wide[..priority]);
    overflow.extend_from_slice(&[0x5A, 0x01, 0x00]);
    overflow.extend_from_slice(&wide[priority + 2..]);
    assert!(ForwardedEventNotification::decode(&overflow).is_err());
}

/// Where an alarm's fromState is due, a later member or the end of the data
/// means it is missing, and an application tag is the wrong tag, as the
/// client's own decoder answers.
#[test]
fn forwarded_from_state_faults_name_their_reasons() {
    let mut request = alarm();
    request.event_values = None;
    let plain = encoded(&request);
    let from_state = plain
        .windows(2)
        .rposition(|pair| pair == [0xA9, 0x00])
        .expect("[10] fromState NORMAL");
    let with = |member: &[u8], rest: bool| {
        let mut body = BytesMut::from(&plain[..from_state]);
        body.extend_from_slice(member);
        if rest {
            body.extend_from_slice(&plain[from_state + 2..]);
        }
        body
    };
    let reason = |body: &[u8]| {
        ForwardedEventNotification::decode(body)
            .unwrap_err()
            .reject_reason()
    };
    // toState [11] where fromState is due.
    assert_eq!(
        reason(&with(&[], true)),
        Some(RejectReason::MISSING_REQUIRED_PARAMETER)
    );
    // The data ends where fromState is due.
    assert_eq!(
        reason(&with(&[], false)),
        Some(RejectReason::MISSING_REQUIRED_PARAMETER)
    );
    // fromState as an application ENUMERATED.
    assert_eq!(
        reason(&with(&[0x91, 0x00], true)),
        Some(RejectReason::INVALID_TAG)
    );
}
