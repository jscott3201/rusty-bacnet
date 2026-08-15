use super::*;

fn raw_context_value(buf: &mut BytesMut, tag_number: u8, value: &[u8]) {
    tags::encode_tag(buf, tag_number, tags::TagClass::Context, value.len() as u32);
    buf.extend_from_slice(value);
}

fn raw_buffer_ready(values: [&[u8]; 4], field_tags: [u8; 4]) -> BytesMut {
    let mut buf = BytesMut::new();
    tags::encode_opening_tag(&mut buf, 10);
    tags::encode_opening_tag(&mut buf, 0);
    let object = ObjectIdentifier::new(ObjectType::TREND_LOG, 1).unwrap();
    primitives::encode_ctx_object_id(&mut buf, 0, &object);
    raw_context_value(&mut buf, field_tags[0], values[0]);
    raw_context_value(&mut buf, field_tags[1], values[1]);
    tags::encode_closing_tag(&mut buf, 0);
    raw_context_value(&mut buf, field_tags[2], values[2]);
    raw_context_value(&mut buf, field_tags[3], values[3]);
    tags::encode_closing_tag(&mut buf, 10);
    buf
}

fn raw_extended(values: [&[u8]; 2], field_tags: [u8; 2]) -> BytesMut {
    let mut buf = BytesMut::new();
    tags::encode_opening_tag(&mut buf, 9);
    raw_context_value(&mut buf, field_tags[0], values[0]);
    raw_context_value(&mut buf, field_tags[1], values[1]);
    tags::encode_opening_tag(&mut buf, 2);
    tags::encode_closing_tag(&mut buf, 2);
    tags::encode_closing_tag(&mut buf, 9);
    buf
}

fn test_date_time() -> (Date, Time) {
    (
        Date {
            year: 124,
            month: 6,
            day: 15,
            day_of_week: 3,
        },
        Time {
            hour: 10,
            minute: 30,
            second: 0,
            hundredths: 0,
        },
    )
}

fn encode_device_property_reference(buf: &mut BytesMut) {
    tags::encode_opening_tag(buf, 4);
    let object = ObjectIdentifier::new(ObjectType::ACCESS_CREDENTIAL, 1).unwrap();
    primitives::encode_ctx_object_id(buf, 0, &object);
    primitives::encode_ctx_unsigned(buf, 1, 85);
    tags::encode_closing_tag(buf, 4);
}

fn raw_access_event(values: [&[u8]; 2], field_tags: [u8; 2], flags: &[u8]) -> BytesMut {
    let mut buf = BytesMut::new();
    tags::encode_opening_tag(&mut buf, 13);
    raw_context_value(&mut buf, field_tags[0], values[0]);
    raw_context_value(&mut buf, 1, flags);
    raw_context_value(&mut buf, field_tags[1], values[1]);
    primitives::encode_timestamp(
        &mut buf,
        3,
        &BACnetTimeStamp::DateTime {
            date: test_date_time().0,
            time: test_date_time().1,
        },
    )
    .unwrap();
    encode_device_property_reference(&mut buf);
    tags::encode_opening_tag(&mut buf, 5);
    tags::encode_closing_tag(&mut buf, 5);
    tags::encode_closing_tag(&mut buf, 13);
    buf
}

fn raw_change_of_reliability(value: &[u8], field_tag: u8, flags: &[u8]) -> BytesMut {
    let mut buf = BytesMut::new();
    tags::encode_opening_tag(&mut buf, 19);
    raw_context_value(&mut buf, field_tag, value);
    raw_context_value(&mut buf, 1, flags);
    tags::encode_opening_tag(&mut buf, 2);
    tags::encode_closing_tag(&mut buf, 2);
    tags::encode_closing_tag(&mut buf, 19);
    buf
}

fn raw_change_of_timer(values: [&[u8]; 3], field_tags: [u8; 3], flags: &[u8]) -> BytesMut {
    let mut buf = BytesMut::new();
    tags::encode_opening_tag(&mut buf, 22);
    raw_context_value(&mut buf, field_tags[0], values[0]);
    raw_context_value(&mut buf, 1, flags);
    tags::encode_opening_tag(&mut buf, 2);
    primitives::encode_app_date(&mut buf, &test_date_time().0);
    primitives::encode_app_time(&mut buf, &test_date_time().1);
    tags::encode_closing_tag(&mut buf, 2);
    raw_context_value(&mut buf, field_tags[1], values[1]);
    raw_context_value(&mut buf, field_tags[2], values[2]);
    tags::encode_opening_tag(&mut buf, 5);
    primitives::encode_app_date(&mut buf, &test_date_time().0);
    primitives::encode_app_time(&mut buf, &test_date_time().1);
    tags::encode_closing_tag(&mut buf, 5);
    tags::encode_closing_tag(&mut buf, 22);
    buf
}

fn decode_variant(data: &[u8]) -> Result<NotificationParameters, Error> {
    NotificationParameters::decode(data, 0)
}

fn event_request(event_values: NotificationParameters) -> EventNotificationRequest {
    EventNotificationRequest {
        process_identifier: 1,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap(),
        event_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap(),
        timestamp: BACnetTimeStamp::SequenceNumber(7),
        notification_class: 1,
        priority: 1,
        event_type: 1,
        message_text: None,
        notify_type: 0,
        ack_required: true,
        from_state: 0,
        to_state: 1,
        event_values: Some(event_values),
    }
}

fn encode_event(event_values: NotificationParameters) -> BytesMut {
    let mut encoded = BytesMut::new();
    event_request(event_values).encode(&mut encoded).unwrap();
    encoded
}

fn access_event(authentication_factor: Vec<u8>) -> NotificationParameters {
    NotificationParameters::AccessEvent {
        access_event: 5,
        status_flags: 0b1000,
        access_event_tag: 10,
        access_event_time: test_date_time(),
        access_credential: BACnetDeviceObjectPropertyReference::new_local(
            ObjectIdentifier::new(ObjectType::ACCESS_CREDENTIAL, 1).unwrap(),
            85,
        ),
        authentication_factor,
    }
}

#[test]
fn notification_parameter_values_reject_overflow_aliases() {
    let too_wide_u32 = [1, 0, 0, 0, 0];
    let too_wide_u16 = [1, 0, 0];
    let u64_max = [0xff; 8];

    for value in [too_wide_u32.as_slice(), u64_max.as_slice()] {
        for field in 0..4 {
            let mut values = [&[1][..], &[2], &[3], &[4]];
            values[field] = value;
            assert!(decode_variant(&raw_buffer_ready(values, [1, 2, 1, 2])).is_err());
        }

        assert!(decode_variant(&raw_extended([&[1], value], [0, 1])).is_err());
        for field in 0..2 {
            let mut values = [&[1][..], &[2]];
            values[field] = value;
            assert!(decode_variant(&raw_access_event(values, [0, 2], &[4, 0x80])).is_err());
        }
        assert!(decode_variant(&raw_change_of_reliability(value, 0, &[4, 0x80])).is_err());
        for field in 0..3 {
            let mut values = [&[1][..], &[2], &[3]];
            values[field] = value;
            assert!(decode_variant(&raw_change_of_timer(values, [0, 3, 4], &[4, 0x80],)).is_err());
        }
    }

    for value in [too_wide_u16.as_slice(), u64_max.as_slice()] {
        assert!(decode_variant(&raw_extended([value, &[1]], [0, 1])).is_err());
    }
}

#[test]
fn notification_parameter_values_accept_fitting_leading_zero() {
    let max_u32 = [0, 0xff, 0xff, 0xff, 0xff];
    let max_u16 = [0, 0xff, 0xff];

    let buffer = decode_variant(&raw_buffer_ready([max_u32.as_slice(); 4], [1, 2, 1, 2])).unwrap();
    let NotificationParameters::BufferReady {
        buffer_property,
        previous_notification,
        current_notification,
    } = buffer
    else {
        panic!("expected BufferReady");
    };
    assert_eq!(buffer_property.property_identifier, u32::MAX);
    assert_eq!(buffer_property.property_array_index, Some(u32::MAX));
    assert_eq!(previous_notification, u32::MAX);
    assert_eq!(current_notification, u32::MAX);

    assert!(matches!(
        decode_variant(&raw_extended(
            [max_u16.as_slice(), max_u32.as_slice()],
            [0, 1]
        )),
        Ok(NotificationParameters::Extended {
            vendor_id: u16::MAX,
            extended_event_type: u32::MAX,
            ..
        })
    ));
    assert!(matches!(
        decode_variant(&raw_access_event(
            [max_u32.as_slice(); 2],
            [0, 2],
            &[4, 0x80]
        )),
        Ok(NotificationParameters::AccessEvent {
            access_event: u32::MAX,
            access_event_tag: u32::MAX,
            ..
        })
    ));
    assert!(matches!(
        decode_variant(&raw_change_of_reliability(
            max_u32.as_slice(),
            0,
            &[4, 0x80]
        )),
        Ok(NotificationParameters::ChangeOfReliability {
            reliability: u32::MAX,
            ..
        })
    ));
    assert!(matches!(
        decode_variant(&raw_change_of_timer(
            [max_u32.as_slice(); 3],
            [0, 3, 4],
            &[4, 0x80]
        )),
        Ok(NotificationParameters::ChangeOfTimer {
            new_state: u32::MAX,
            last_state_change: u32::MAX,
            initial_timeout: u32::MAX,
            ..
        })
    ));
}

#[test]
fn notification_parameter_fields_require_owned_tags_and_status_flags() {
    for field in 0..4 {
        let mut tags = [1, 2, 1, 2];
        tags[field] = 7;
        assert!(decode_variant(&raw_buffer_ready([&[1], &[2], &[3], &[4]], tags)).is_err());
    }
    for field in 0..2 {
        let mut tags = [0, 1];
        tags[field] = 7;
        assert!(decode_variant(&raw_extended([&[1], &[2]], tags)).is_err());

        let mut tags = [0, 2];
        tags[field] = 7;
        assert!(decode_variant(&raw_access_event([&[1], &[2]], tags, &[4, 0x80])).is_err());
    }
    for field in 0..3 {
        let mut tags = [0, 3, 4];
        tags[field] = 7;
        assert!(
            decode_variant(&raw_change_of_timer([&[1], &[2], &[3]], tags, &[4, 0x80],)).is_err()
        );
    }
    assert!(decode_variant(&raw_change_of_reliability(&[1], 7, &[4, 0x80])).is_err());

    for flags in [&[][..], &[3, 0x80], &[4, 0x81]] {
        assert!(decode_variant(&raw_access_event([&[1], &[2]], [0, 2], flags)).is_err());
        assert!(decode_variant(&raw_change_of_reliability(&[1], 0, flags)).is_err());
        assert!(
            decode_variant(&raw_change_of_timer([&[1], &[2], &[3]], [0, 3, 4], flags,)).is_err()
        );
    }

    let mut application_vendor = raw_extended([&[1], &[2]], [0, 1]);
    application_vendor[1] &= !0x08;
    assert!(decode_variant(&application_vendor).is_err());

    let mut wrong_object_tag = raw_buffer_ready([&[1], &[2], &[3], &[4]], [1, 2, 1, 2]);
    wrong_object_tag[2] = 0x3c;
    assert!(decode_variant(&wrong_object_tag).is_err());

    let mut wrong_timer_date = raw_change_of_timer([&[1], &[2], &[3]], [0, 3, 4], &[4, 0x80]);
    let date_tag = wrong_timer_date
        .iter()
        .position(|byte| *byte == 0xa4)
        .unwrap();
    wrong_timer_date[date_tag] = 0xb4;
    assert!(decode_variant(&wrong_timer_date).is_err());
}

#[test]
fn event_notification_preserves_trailing_opaque_payload_bytes() {
    let variants = [
        NotificationParameters::CommandFailure {
            command_value: vec![0x01],
            status_flags: 0b1000,
            feedback_value: vec![0x2e, 0x2f, 0xcf, 0x3f],
        },
        NotificationParameters::Extended {
            vendor_id: 42,
            extended_event_type: 7,
            parameters: vec![0x2e, 0x2f, 0xcf, 0x9f],
        },
        access_event(vec![0x5e, 0x5f, 0xcf, 0xdf]),
        NotificationParameters::ChangeOfReliability {
            reliability: 7,
            status_flags: 0b1000,
            property_values: vec![0x2e, 0x2f, 0xcf, 0xff],
        },
    ];

    for expected in variants {
        let encoded = encode_event(expected.clone());
        let decoded = EventNotificationRequest::decode(&encoded).unwrap();
        assert_eq!(decoded.event_values, Some(expected));
    }
}

#[test]
fn event_notification_requires_exact_event_values_suffix() {
    let variants = [
        NotificationParameters::BufferReady {
            buffer_property: BACnetDeviceObjectPropertyReference::new_local(
                ObjectIdentifier::new(ObjectType::TREND_LOG, 1).unwrap(),
                131,
            ),
            previous_notification: 1,
            current_notification: 2,
        },
        NotificationParameters::Extended {
            vendor_id: 42,
            extended_event_type: 7,
            parameters: vec![0x01, 0x02],
        },
        access_event(vec![0xab, 0xcd]),
        NotificationParameters::ChangeOfTimer {
            new_state: 1,
            status_flags: 0b1000,
            update_time: test_date_time(),
            last_state_change: 2,
            initial_timeout: 3,
            expiration_time: test_date_time(),
        },
    ];

    for variant in variants {
        let encoded = encode_event(variant);

        let mut missing_outer = encoded.clone();
        missing_outer.truncate(missing_outer.len() - 1);
        assert!(EventNotificationRequest::decode(&missing_outer).is_err());

        let mut wrong_outer = encoded.clone();
        *wrong_outer.last_mut().unwrap() = 0xbf;
        assert!(EventNotificationRequest::decode(&wrong_outer).is_err());

        let mut trailing = encoded.clone();
        primitives::encode_ctx_unsigned(&mut trailing, 13, 1);
        assert!(EventNotificationRequest::decode(&trailing).is_err());

        let mut sibling = encoded.clone();
        sibling.truncate(sibling.len() - 1);
        primitives::encode_ctx_unsigned(&mut sibling, 13, 1);
        tags::encode_closing_tag(&mut sibling, 12);
        assert!(EventNotificationRequest::decode(&sibling).is_err());

        let mut fake_final_outer = encoded.clone();
        primitives::encode_ctx_unsigned(&mut fake_final_outer, 13, 1);
        tags::encode_closing_tag(&mut fake_final_outer, 12);
        assert!(EventNotificationRequest::decode(&fake_final_outer).is_err());

        let mut wrong_variant_close = encoded;
        let close_number_index = wrong_variant_close.len() - 2;
        if wrong_variant_close[close_number_index] == 22 {
            wrong_variant_close[close_number_index] = 21;
        } else {
            wrong_variant_close[close_number_index] = 0x7f;
        }
        assert!(EventNotificationRequest::decode(&wrong_variant_close).is_err());
    }
}
