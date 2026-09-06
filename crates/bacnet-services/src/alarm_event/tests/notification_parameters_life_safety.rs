use super::*;

fn raw_context_value(buf: &mut BytesMut, tag_number: u8, value: &[u8]) {
    tags::encode_tag(buf, tag_number, tags::TagClass::Context, value.len() as u32);
    buf.extend_from_slice(value);
}

fn raw_change_of_life_safety(
    values: [&[u8]; 3],
    field_tags: [u8; 4],
    status_flags: &[u8],
    closing_tag: Option<u8>,
) -> BytesMut {
    let mut buf = BytesMut::new();
    tags::encode_opening_tag(&mut buf, 8);
    raw_context_value(&mut buf, field_tags[0], values[0]);
    raw_context_value(&mut buf, field_tags[1], values[1]);
    raw_context_value(&mut buf, field_tags[2], status_flags);
    raw_context_value(&mut buf, field_tags[3], values[2]);
    if let Some(tag) = closing_tag {
        tags::encode_closing_tag(&mut buf, tag);
    }
    buf
}

fn decode_params(data: &[u8]) -> Result<NotificationParameters, Error> {
    NotificationParameters::decode(data, 0)
}

fn encoded_event_notification() -> (BytesMut, usize) {
    let request = EventNotificationRequest {
        process_identifier: 1,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap(),
        event_object_identifier: ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 1).unwrap(),
        timestamp: BACnetTimeStamp::SequenceNumber(1),
        notification_class: 1,
        priority: 1,
        event_type: 8,
        message_text: None,
        notify_type: 0,
        ack_required: true,
        from_state: 0,
        to_state: 1,
        event_values: None,
    };
    let mut buf = BytesMut::new();
    request.encode(&mut buf).unwrap();
    let event_values_offset = buf.len();
    tags::encode_opening_tag(&mut buf, 12);
    NotificationParameters::ChangeOfLifeSafety {
        new_state: 1,
        new_mode: 1,
        status_flags: 8,
        operation_expected: 1,
    }
    .encode(&mut buf)
    .unwrap();
    tags::encode_closing_tag(&mut buf, 12);
    (buf, event_values_offset)
}

#[test]
fn change_of_life_safety_accepts_u32_max_with_leading_zero() {
    let max = [0, 0xff, 0xff, 0xff, 0xff];
    let decoded = decode_params(&raw_change_of_life_safety(
        [&max, &max, &max],
        [0, 1, 2, 3],
        &[4, 0xf0],
        Some(8),
    ))
    .unwrap();

    assert_eq!(
        decoded,
        NotificationParameters::ChangeOfLifeSafety {
            new_state: u32::MAX,
            new_mode: u32::MAX,
            status_flags: 0x0f,
            operation_expected: u32::MAX,
        }
    );
}

#[test]
fn change_of_life_safety_rejects_values_above_u32() {
    let one = [1];
    let overflow_alias = [1, 0, 0, 0, 1];
    let u64_max = [0xff; 8];

    for field in 0..3 {
        for invalid in [&overflow_alias[..], &u64_max] {
            let mut values: [&[u8]; 3] = [&one, &one, &one];
            values[field] = invalid;
            assert!(decode_params(&raw_change_of_life_safety(
                values,
                [0, 1, 2, 3],
                &[4, 0x80],
                Some(8),
            ))
            .is_err());
        }
    }
}

#[test]
fn change_of_life_safety_requires_exact_field_tags() {
    for field in 0..4 {
        let mut field_tags = [0, 1, 2, 3];
        field_tags[field] = 7;
        assert!(decode_params(&raw_change_of_life_safety(
            [&[1], &[1], &[1]],
            field_tags,
            &[4, 0x80],
            Some(8),
        ))
        .is_err());
    }

    let mut application_tagged =
        raw_change_of_life_safety([&[1], &[1], &[1]], [0, 1, 2, 3], &[4, 0x80], Some(8));
    application_tagged[1] &= !0x08;
    assert!(decode_params(&application_tagged).is_err());
}

#[test]
fn change_of_life_safety_requires_four_status_bits_with_zero_padding() {
    for invalid in [
        &[][..],
        &[4][..],
        &[3, 0xf0][..],
        &[4, 0xf1][..],
        &[4, 0xf0, 0][..],
    ] {
        assert!(decode_params(&raw_change_of_life_safety(
            [&[1], &[1], &[1]],
            [0, 1, 2, 3],
            invalid,
            Some(8),
        ))
        .is_err());
    }
}

#[test]
fn change_of_life_safety_requires_its_immediate_closing_tag() {
    assert!(decode_params(&raw_change_of_life_safety(
        [&[1], &[1], &[1]],
        [0, 1, 2, 3],
        &[4, 0x80],
        None,
    ))
    .is_err());
    assert!(decode_params(&raw_change_of_life_safety(
        [&[1], &[1], &[1]],
        [0, 1, 2, 3],
        &[4, 0x80],
        Some(9),
    ))
    .is_err());

    let mut extra_field =
        raw_change_of_life_safety([&[1], &[1], &[1]], [0, 1, 2, 3], &[4, 0x80], None);
    raw_context_value(&mut extra_field, 4, &[1]);
    tags::encode_closing_tag(&mut extra_field, 8);
    assert!(decode_params(&extra_field).is_err());
}

#[test]
fn change_of_life_safety_rejects_every_truncated_prefix() {
    let encoded = raw_change_of_life_safety([&[1], &[1], &[1]], [0, 1, 2, 3], &[4, 0x80], Some(8));

    for end in 0..encoded.len() {
        assert!(decode_params(&encoded[..end]).is_err(), "prefix {end}");
    }
}

#[test]
fn event_notification_requires_event_values_outer_framing() {
    let (encoded, event_values_offset) = encoded_event_notification();
    assert!(EventNotificationRequest::decode(&encoded).is_ok());

    let mut missing_opening = encoded.to_vec();
    missing_opening.remove(event_values_offset);
    assert!(EventNotificationRequest::decode(&missing_opening).is_err());

    let mut wrong_opening = encoded.clone();
    let mut tag = BytesMut::new();
    tags::encode_opening_tag(&mut tag, 11);
    wrong_opening[event_values_offset] = tag[0];
    assert!(EventNotificationRequest::decode(&wrong_opening).is_err());

    let mut missing_closing = encoded.clone();
    missing_closing.truncate(missing_closing.len() - 1);
    assert!(EventNotificationRequest::decode(&missing_closing).is_err());

    let mut wrong_closing = missing_closing.clone();
    tags::encode_closing_tag(&mut wrong_closing, 9);
    assert!(EventNotificationRequest::decode(&wrong_closing).is_err());

    let mut extra_sibling = missing_closing;
    raw_context_value(&mut extra_sibling, 4, &[1]);
    tags::encode_closing_tag(&mut extra_sibling, 12);
    assert!(EventNotificationRequest::decode(&extra_sibling).is_err());

    let mut trailing_data = encoded;
    raw_context_value(&mut trailing_data, 13, &[1]);
    assert!(EventNotificationRequest::decode(&trailing_data).is_err());
}
