use super::*;

fn raw_context_value(buf: &mut BytesMut, tag_number: u8, value: &[u8]) {
    tags::encode_tag(buf, tag_number, tags::TagClass::Context, value.len() as u32);
    buf.extend_from_slice(value);
}

fn encoded_octet_string(value: &[u8]) -> Vec<u8> {
    let mut encoded = BytesMut::new();
    primitives::encode_app_octet_string(&mut encoded, value);
    encoded.to_vec()
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

fn encode_device_object_reference(buf: &mut BytesMut) {
    tags::encode_opening_tag(buf, 4);
    let object = ObjectIdentifier::new(ObjectType::ACCESS_CREDENTIAL, 1).unwrap();
    primitives::encode_ctx_object_id(buf, 1, &object);
    tags::encode_closing_tag(buf, 4);
}

fn authentication_factor(value: &[u8]) -> Vec<u8> {
    let mut factor = BytesMut::from(&[0x09, 0x01, 0x19, 0x02][..]);
    tags::encode_tag(&mut factor, 2, tags::TagClass::Context, value.len() as u32);
    factor.extend_from_slice(value);
    factor.to_vec()
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
    encode_device_object_reference(&mut buf);
    tags::encode_opening_tag(&mut buf, 5);
    buf.extend_from_slice(&[0x09, 0x01, 0x19, 0x02, 0x28]);
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

fn access_event(authentication_factor: Option<Vec<u8>>) -> NotificationParameters {
    NotificationParameters::AccessEvent {
        access_event: 5,
        status_flags: 0b1000,
        access_event_tag: 10,
        access_event_time: test_date_time(),
        access_credential: BACnetDeviceObjectReference {
            device_identifier: None,
            object_identifier: ObjectIdentifier::new(ObjectType::ACCESS_CREDENTIAL, 1).unwrap(),
        },
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
            last_state_change: Some(u32::MAX),
            initial_timeout: Some(u32::MAX),
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
            command_value: encoded_octet_string(&[0x01]),
            status_flags: 0b1000,
            feedback_value: encoded_octet_string(&[0x2e, 0x2f, 0xcf, 0x3f]),
        },
        NotificationParameters::Extended {
            vendor_id: 42,
            extended_event_type: 7,
            parameters: encoded_octet_string(&[0x2e, 0x2f, 0xcf, 0x9f]),
        },
        access_event(Some(authentication_factor(&[0x5e, 0x5f, 0xcf, 0xdf]))),
        NotificationParameters::ChangeOfReliability {
            reliability: 7,
            status_flags: 0b1000,
            property_values: encoded_octet_string(&[0x2e, 0x2f, 0xcf, 0xff]),
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
            parameters: encoded_octet_string(&[0x01, 0x02]),
        },
        access_event(Some(authentication_factor(&[0xab, 0xcd]))),
        NotificationParameters::ChangeOfTimer {
            new_state: 1,
            status_flags: 0b1000,
            update_time: test_date_time(),
            last_state_change: Some(2),
            initial_timeout: Some(3),
            expiration_time: Some(test_date_time()),
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

#[test]
fn notification_parameters_require_exact_variant_consumption() {
    let buffer_ready = NotificationParameters::BufferReady {
        buffer_property: BACnetDeviceObjectPropertyReference::new_local(
            ObjectIdentifier::new(ObjectType::TREND_LOG, 1).unwrap(),
            131,
        ),
        previous_notification: 1,
        current_notification: 2,
    };

    let mut same_tag_sibling = encode_event(buffer_ready.clone());
    same_tag_sibling.truncate(same_tag_sibling.len() - 1);
    buffer_ready.encode(&mut same_tag_sibling).unwrap();
    tags::encode_closing_tag(&mut same_tag_sibling, 12);
    assert!(EventNotificationRequest::decode(&same_tag_sibling).is_err());

    let mut extra_inner_field = encode_event(buffer_ready);
    extra_inner_field.truncate(extra_inner_field.len() - 2);
    primitives::encode_ctx_unsigned(&mut extra_inner_field, 13, 1);
    tags::encode_closing_tag(&mut extra_inner_field, 10);
    tags::encode_closing_tag(&mut extra_inner_field, 12);
    assert!(EventNotificationRequest::decode(&extra_inner_field).is_err());

    let mut close_in_field_content = raw_buffer_ready([&[1], &[2], &[3], &[0, 0xaf]], [1, 2, 1, 2]);
    close_in_field_content.truncate(close_in_field_content.len() - 1);
    assert!(decode_variant(&close_in_field_content).is_err());

    let mut missing_discrete_status = BytesMut::new();
    tags::encode_opening_tag(&mut missing_discrete_status, 21);
    tags::encode_opening_tag(&mut missing_discrete_status, 0);
    tags::encode_closing_tag(&mut missing_discrete_status, 0);
    tags::encode_closing_tag(&mut missing_discrete_status, 21);
    assert!(decode_variant(&missing_discrete_status).is_err());
}

#[test]
fn public_notification_parameter_decode_preserves_opaque_delimiters() {
    let expected = NotificationParameters::Extended {
        vendor_id: 42,
        extended_event_type: 7,
        parameters: encoded_octet_string(&[0x2f, 0x9f, 0xcf]),
    };
    let mut encoded = BytesMut::new();
    expected.encode(&mut encoded).unwrap();
    assert_eq!(decode_variant(&encoded).unwrap(), expected);
}

#[test]
fn raw_fields_reject_same_tag_siblings_and_truncated_close_aliases() {
    let raw = encoded_octet_string(&[0x2f, 0x9f, 0xcf]);
    let variants = [
        NotificationParameters::CommandFailure {
            command_value: raw.clone(),
            status_flags: 0b1000,
            feedback_value: raw.clone(),
        },
        NotificationParameters::Extended {
            vendor_id: 42,
            extended_event_type: 7,
            parameters: raw.clone(),
        },
        access_event(Some(authentication_factor(&raw))),
        NotificationParameters::ChangeOfReliability {
            reliability: 7,
            status_flags: 0b1000,
            property_values: raw,
        },
    ];

    for variant in variants {
        let mut siblings = encode_event(variant.clone());
        siblings.truncate(siblings.len() - 1);
        variant.encode(&mut siblings).unwrap();
        tags::encode_closing_tag(&mut siblings, 12);
        assert!(EventNotificationRequest::decode(&siblings).is_err());
    }

    let mut truncated = encode_event(NotificationParameters::Extended {
        vendor_id: 42,
        extended_event_type: 7,
        parameters: encoded_octet_string(&[0x2f, 0x9f, 0xcf]),
    });
    truncated.truncate(truncated.len() - 3);
    assert!(EventNotificationRequest::decode(&truncated).is_err());
}

#[test]
fn non_trailing_raw_fields_preserve_delimiters_inside_values() {
    let raw = encoded_octet_string(&[0x0e, 0x0f, 0x1e, 0x1f]);
    let variants = [
        NotificationParameters::CommandFailure {
            command_value: raw.clone(),
            status_flags: 0b1000,
            feedback_value: raw.clone(),
        },
        NotificationParameters::ChangeOfStatusFlags {
            present_value: Some(raw.clone()),
            referenced_flags: 0b1000,
        },
        NotificationParameters::ChangeOfDiscreteValue {
            new_value: raw,
            status_flags: 0b1000,
        },
    ];

    for expected in variants {
        let decoded = EventNotificationRequest::decode(&encode_event(expected.clone())).unwrap();
        assert_eq!(decoded.event_values, Some(expected));
    }
}

#[test]
fn public_notification_parameter_decode_accepts_event_values_close() {
    let expected = NotificationParameters::Extended {
        vendor_id: 42,
        extended_event_type: 7,
        parameters: encoded_octet_string(&[0xcf]),
    };
    let mut wrapped = BytesMut::new();
    tags::encode_opening_tag(&mut wrapped, 12);
    expected.encode(&mut wrapped).unwrap();
    tags::encode_closing_tag(&mut wrapped, 12);
    assert_eq!(
        NotificationParameters::decode(&wrapped, 1).unwrap(),
        expected
    );
}

#[test]
fn raw_fields_require_encoded_bacnet_values() {
    let invalid = NotificationParameters::Extended {
        vendor_id: 42,
        extended_event_type: 7,
        parameters: vec![0x9f],
    };
    let mut untouched = BytesMut::from(&[0xaa][..]);
    assert!(invalid.encode(&mut untouched).is_err());
    assert_eq!(untouched.as_ref(), &[0xaa]);

    let mut raw = BytesMut::new();
    tags::encode_opening_tag(&mut raw, 9);
    primitives::encode_ctx_unsigned(&mut raw, 0, 42);
    primitives::encode_ctx_unsigned(&mut raw, 1, 7);
    tags::encode_opening_tag(&mut raw, 2);
    raw.extend_from_slice(&[0x9f]);
    tags::encode_closing_tag(&mut raw, 2);
    tags::encode_closing_tag(&mut raw, 9);
    assert!(decode_variant(&raw).is_err());
}

#[test]
fn abstract_syntax_values_preserve_empty_aggregates_and_optional_presence() {
    for expected in [
        NotificationParameters::CommandFailure {
            command_value: Vec::new(),
            status_flags: 0,
            feedback_value: Vec::new(),
        },
        NotificationParameters::ChangeOfDiscreteValue {
            new_value: Vec::new(),
            status_flags: 0,
        },
        access_event(None),
        NotificationParameters::ChangeOfStatusFlags {
            present_value: None,
            referenced_flags: 0,
        },
        NotificationParameters::ChangeOfStatusFlags {
            present_value: Some(Vec::new()),
            referenced_flags: 0,
        },
    ] {
        let mut encoded = BytesMut::new();
        expected.encode(&mut encoded).unwrap();
        assert_eq!(decode_variant(&encoded).unwrap(), expected);
        assert_eq!(
            EventNotificationRequest::decode(&encode_event(expected.clone()))
                .unwrap()
                .event_values,
            Some(expected)
        );
    }
}

#[test]
fn event_notification_enforces_total_nesting_on_encode_and_decode() {
    use bacnet_types::constructed::BACnetProprietaryPropertyState;

    let change_of_state = |body_depth| {
        let mut body = vec![0x0e; body_depth];
        body.extend(vec![0x0f; body_depth]);
        NotificationParameters::ChangeOfState {
            new_state: BACnetPropertyStates::Other(
                BACnetProprietaryPropertyState::constructed(64, body).unwrap(),
            ),
            status_flags: 0,
        }
    };

    let accepted = event_request(change_of_state(tags::MAX_CONTEXT_NESTING_DEPTH - 4));
    let mut encoded = BytesMut::new();
    accepted.encode(&mut encoded).unwrap();
    assert!(EventNotificationRequest::decode(&encoded).is_ok());

    let too_deep_parameters = change_of_state(tags::MAX_CONTEXT_NESTING_DEPTH - 3);
    let mut parameters = BytesMut::new();
    too_deep_parameters.encode(&mut parameters).unwrap();

    let mut untouched = BytesMut::from(&[0xaa][..]);
    assert!(event_request(too_deep_parameters.clone())
        .encode(&mut untouched)
        .is_err());
    assert_eq!(untouched.as_ref(), &[0xaa]);

    let mut without_values = event_request(too_deep_parameters);
    without_values.event_values = None;
    let mut raw = BytesMut::new();
    without_values.encode(&mut raw).unwrap();
    tags::encode_opening_tag(&mut raw, 12);
    raw.extend_from_slice(&parameters);
    tags::encode_closing_tag(&mut raw, 12);
    assert!(EventNotificationRequest::decode(&raw).is_err());
}

#[test]
fn primitive_notification_fields_require_their_context_tags() {
    let variants = [
        (
            NotificationParameters::ChangeOfBitstring {
                referenced_bitstring: (0, vec![0x80]),
                status_flags: 0b1000,
            },
            2,
        ),
        (
            NotificationParameters::FloatingLimit {
                reference_value: 1.0,
                status_flags: 0b1000,
                setpoint_value: 2.0,
                error_limit: 3.0,
            },
            4,
        ),
        (
            NotificationParameters::OutOfRange {
                exceeding_value: 1.0,
                status_flags: 0b1000,
                deadband: 2.0,
                exceeded_limit: 3.0,
            },
            4,
        ),
        (
            NotificationParameters::UnsignedRange {
                exceeding_value: 1,
                status_flags: 0b1000,
                exceeded_limit: 2,
            },
            3,
        ),
        (
            NotificationParameters::DoubleOutOfRange {
                exceeding_value: 1.0,
                status_flags: 0b1000,
                deadband: 2.0,
                exceeded_limit: 3.0,
            },
            4,
        ),
        (
            NotificationParameters::SignedOutOfRange {
                exceeding_value: -1,
                status_flags: 0b1000,
                deadband: 2,
                exceeded_limit: -3,
            },
            4,
        ),
        (
            NotificationParameters::UnsignedOutOfRange {
                exceeding_value: 1,
                status_flags: 0b1000,
                deadband: 2,
                exceeded_limit: 3,
            },
            4,
        ),
        (
            NotificationParameters::ChangeOfCharacterstring {
                changed_value: "changed".into(),
                status_flags: 0b1000,
                alarm_value: "alarm".into(),
            },
            3,
        ),
        (
            NotificationParameters::ChangeOfLifeSafety {
                new_state: 1,
                new_mode: 2,
                status_flags: 0b1000,
                operation_expected: 3,
            },
            4,
        ),
    ];

    for (variant, field_count) in variants {
        let mut encoded = BytesMut::new();
        variant.encode(&mut encoded).unwrap();
        let (_, mut field_start) = tags::decode_tag(&encoded, 0).unwrap();
        for expected_tag in 0..field_count {
            let (field, content_start) = tags::decode_tag(&encoded, field_start).unwrap();
            assert!(field.is_context(expected_tag));
            let mut wrong_tag = encoded.clone();
            wrong_tag[field_start] = 0x70 | (wrong_tag[field_start] & 0x0f);
            assert!(decode_variant(&wrong_tag).is_err());
            field_start = content_start + field.length as usize;
        }
    }
}
