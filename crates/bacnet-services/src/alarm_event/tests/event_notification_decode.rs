use super::*;

const U32_FIELD_VALUE_INDEXES: [usize; 6] = [0, 1, 3, 4, 5, 6];

fn canonical_tags() -> [(u8, tags::TagClass); 9] {
    [
        (0, tags::TagClass::Context),
        (1, tags::TagClass::Context),
        (2, tags::TagClass::Context),
        (4, tags::TagClass::Context),
        (5, tags::TagClass::Context),
        (6, tags::TagClass::Context),
        (8, tags::TagClass::Context),
        (10, tags::TagClass::Context),
        (11, tags::TagClass::Context),
    ]
}

fn canonical_values() -> [&'static [u8]; 7] {
    [&[1], &[5], &[100], &[5], &[0], &[0], &[3]]
}

fn raw_tagged_value(buf: &mut BytesMut, tag: (u8, tags::TagClass), content: &[u8]) {
    tags::encode_tag(buf, tag.0, tag.1, content.len() as u32);
    buf.extend_from_slice(content);
}

fn raw_event_notification(
    values: [&[u8]; 7],
    field_tags: [(u8, tags::TagClass); 9],
    ack_required: Option<&[u8]>,
    unknown_before_notify_type: bool,
) -> BytesMut {
    let mut buf = BytesMut::new();
    raw_tagged_value(&mut buf, field_tags[0], values[0]);

    let initiating_device = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
    raw_tagged_value(&mut buf, field_tags[1], &initiating_device.encode());

    let event_object = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap();
    raw_tagged_value(&mut buf, field_tags[2], &event_object.encode());

    primitives::encode_timestamp(&mut buf, 3, &BACnetTimeStamp::SequenceNumber(7)).unwrap();
    raw_tagged_value(&mut buf, field_tags[3], values[1]);
    raw_tagged_value(&mut buf, field_tags[4], values[2]);
    raw_tagged_value(&mut buf, field_tags[5], values[3]);

    if unknown_before_notify_type {
        primitives::encode_ctx_unsigned(&mut buf, 13, 1);
    }

    raw_tagged_value(&mut buf, field_tags[6], values[4]);
    if let Some(value) = ack_required {
        raw_tagged_value(&mut buf, (9, tags::TagClass::Context), value);
    }
    raw_tagged_value(&mut buf, field_tags[7], values[5]);
    raw_tagged_value(&mut buf, field_tags[8], values[6]);
    buf
}

#[test]
fn event_notification_u32_fields_reject_overflow_aliases() {
    for field in U32_FIELD_VALUE_INDEXES {
        for value in [[1, 0, 0, 0, 0].as_slice(), [0xff; 8].as_slice()] {
            let mut values = canonical_values();
            values[field] = value;
            assert!(EventNotificationRequest::decode(&raw_event_notification(
                values,
                canonical_tags(),
                Some(&[1]),
                false,
            ))
            .is_err());
        }
    }
}

#[test]
fn event_notification_priority_rejects_values_above_u8() {
    for value in [[1, 0].as_slice(), [1, 1].as_slice(), [0xff; 8].as_slice()] {
        let mut values = canonical_values();
        values[2] = value;
        assert!(EventNotificationRequest::decode(&raw_event_notification(
            values,
            canonical_tags(),
            Some(&[1]),
            false,
        ))
        .is_err());
    }
}

#[test]
fn event_notification_numeric_fields_accept_fitting_leading_zero() {
    let max_u32 = [0, 0xff, 0xff, 0xff, 0xff];
    let max_u8 = [0, 0xff];
    let values = [
        max_u32.as_slice(),
        max_u32.as_slice(),
        max_u8.as_slice(),
        max_u32.as_slice(),
        max_u32.as_slice(),
        max_u32.as_slice(),
        max_u32.as_slice(),
    ];
    let decoded = EventNotificationRequest::decode(&raw_event_notification(
        values,
        canonical_tags(),
        Some(&[1]),
        false,
    ))
    .unwrap();

    assert_eq!(decoded.process_identifier, u32::MAX);
    assert_eq!(decoded.notification_class, u32::MAX);
    assert_eq!(decoded.priority, u8::MAX);
    assert_eq!(decoded.event_type, u32::MAX);
    assert_eq!(decoded.notify_type, u32::MAX);
    assert_eq!(decoded.from_state, u32::MAX);
    assert_eq!(decoded.to_state, u32::MAX);
}

#[test]
fn event_notification_fields_require_owned_context_tags() {
    for field in 0..canonical_tags().len() {
        let mut field_tags = canonical_tags();
        field_tags[field].0 = 13;
        assert!(EventNotificationRequest::decode(&raw_event_notification(
            canonical_values(),
            field_tags,
            Some(&[1]),
            false,
        ))
        .is_err());

        let mut field_tags = canonical_tags();
        field_tags[field].1 = tags::TagClass::Application;
        assert!(EventNotificationRequest::decode(&raw_event_notification(
            canonical_values(),
            field_tags,
            Some(&[1]),
            false,
        ))
        .is_err());
    }
}

#[test]
fn event_notification_rejects_unknown_fields_before_notify_type() {
    assert!(EventNotificationRequest::decode(&raw_event_notification(
        canonical_values(),
        canonical_tags(),
        Some(&[1]),
        true,
    ))
    .is_err());
}

#[test]
fn event_notification_ack_required_must_be_boolean() {
    for value in [&[][..], &[2], &[0, 1]] {
        assert!(EventNotificationRequest::decode(&raw_event_notification(
            canonical_values(),
            canonical_tags(),
            Some(value),
            false,
        ))
        .is_err());
    }
}

#[test]
fn event_notification_preserves_optional_envelope_fields() {
    let request = EventNotificationRequest {
        process_identifier: 1,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap(),
        event_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap(),
        timestamp: BACnetTimeStamp::SequenceNumber(7),
        notification_class: 5,
        priority: 100,
        event_type: 5,
        message_text: Some("high limit".into()),
        notify_type: 0,
        ack_required: true,
        from_state: 0,
        to_state: 3,
        event_values: None,
    };
    let mut encoded = BytesMut::new();
    request.encode(&mut encoded).unwrap();
    let decoded = EventNotificationRequest::decode(&encoded).unwrap();
    assert_eq!(decoded.message_text.as_deref(), Some("high limit"));
    assert!(decoded.ack_required);

    let mut ack_values = canonical_values();
    ack_values[4] = &[2];
    let decoded = EventNotificationRequest::decode(&raw_event_notification(
        ack_values,
        canonical_tags(),
        None,
        false,
    ))
    .unwrap();
    assert!(!decoded.ack_required);
}

#[test]
fn ack_notification_omits_ack_from_state_and_event_values_exactly() {
    let request = EventNotificationRequest {
        process_identifier: 1,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap(),
        event_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap(),
        timestamp: BACnetTimeStamp::SequenceNumber(7),
        notification_class: 5,
        priority: 100,
        event_type: 5,
        message_text: None,
        notify_type: 2,
        ack_required: true,
        from_state: 4,
        to_state: 3,
        event_values: Some(NotificationParameters::OutOfRange {
            exceeding_value: 85.0,
            status_flags: 0b1000,
            deadband: 2.0,
            exceeded_limit: 80.0,
        }),
    };

    let mut encoded = BytesMut::new();
    request.encode(&mut encoded).unwrap();
    assert_eq!(
        encoded.as_ref(),
        &[
            0x09, 0x01, 0x1c, 0x02, 0x00, 0x00, 0x01, 0x2c, 0x00, 0x00, 0x00, 0x03, 0x3e, 0x19,
            0x07, 0x3f, 0x49, 0x05, 0x59, 0x64, 0x69, 0x05, 0x89, 0x02, 0xb9, 0x03,
        ],
        "ACK_NOTIFICATION must end with To State [11] and omit [9], [10], and [12]"
    );

    let decoded = EventNotificationRequest::decode(&encoded).unwrap();
    assert_eq!(decoded.notify_type, 2);
    assert!(!decoded.ack_required);
    assert_eq!(
        decoded.from_state, 0,
        "absent From State uses the neutral default"
    );
    assert_eq!(decoded.to_state, 3);
    assert!(decoded.event_values.is_none());
}

#[test]
fn event_notification_rejects_every_truncated_prefix() {
    let request = EventNotificationRequest {
        process_identifier: 1,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap(),
        event_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap(),
        timestamp: BACnetTimeStamp::SequenceNumber(7),
        notification_class: 5,
        priority: 100,
        event_type: 5,
        message_text: Some("high limit".into()),
        notify_type: 0,
        ack_required: true,
        from_state: 0,
        to_state: 3,
        event_values: None,
    };
    let mut encoded = BytesMut::new();
    request.encode(&mut encoded).unwrap();

    for end in 0..encoded.len() {
        assert!(EventNotificationRequest::decode(&encoded[..end]).is_err());
    }
}
