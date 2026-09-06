use super::*;
use bacnet_types::enums::ObjectType;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn property_ref(property_identifier: PropertyIdentifier) -> COVReference {
    COVReference {
        monitored_property: PropertyReference {
            property_identifier,
            property_array_index: None,
        },
        cov_increment: None,
        timestamped: false,
    }
}

fn subscription_with(
    property_identifier: PropertyIdentifier,
) -> SubscribeCOVPropertyMultipleRequest {
    SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: 1,
        issue_confirmed_notifications: false,
        lifetime: Some(60),
        max_notification_delay: Some(5),
        list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
            monitored_object_identifier: oid(ObjectType::ANALOG_INPUT, 1),
            list_of_cov_references: vec![property_ref(property_identifier)],
        }],
    }
}

fn notification_with(property_identifier: PropertyIdentifier) -> COVNotificationMultipleRequest {
    COVNotificationMultipleRequest {
        subscriber_process_identifier: 1,
        initiating_device_identifier: oid(ObjectType::DEVICE, 1),
        time_remaining: 1,
        timestamp: None,
        list_of_cov_notifications: vec![COVNotificationItem {
            monitored_object_identifier: oid(ObjectType::ANALOG_INPUT, 1),
            list_of_values: vec![COVNotificationValue {
                property_identifier,
                property_array_index: None,
                value: vec![0],
                time_of_change: None,
            }],
        }],
    }
}

fn raw_subscription(property_identifier: PropertyIdentifier) -> BytesMut {
    let mut encoded = BytesMut::new();
    primitives::encode_ctx_unsigned(&mut encoded, 0, 1);
    primitives::encode_ctx_boolean(&mut encoded, 1, false);
    primitives::encode_ctx_unsigned(&mut encoded, 2, 60);
    primitives::encode_ctx_unsigned(&mut encoded, 3, 5);
    tags::encode_opening_tag(&mut encoded, 4);
    primitives::encode_ctx_object_id(&mut encoded, 0, &oid(ObjectType::ANALOG_INPUT, 1));
    tags::encode_opening_tag(&mut encoded, 1);
    tags::encode_opening_tag(&mut encoded, 0);
    PropertyReference {
        property_identifier,
        property_array_index: None,
    }
    .encode(&mut encoded);
    tags::encode_closing_tag(&mut encoded, 0);
    primitives::encode_ctx_boolean(&mut encoded, 2, false);
    tags::encode_closing_tag(&mut encoded, 1);
    tags::encode_closing_tag(&mut encoded, 4);
    encoded
}

#[test]
fn subscribe_encodes_false_timestamped_and_rejects_missing_required_booleans() {
    let request = subscription_with(PropertyIdentifier::PRESENT_VALUE);
    let mut encoded = BytesMut::new();
    request.encode(&mut encoded);
    assert_eq!(
        encoded.as_ref(),
        &[
            0x09, 0x01, 0x19, 0x00, 0x29, 0x3c, 0x39, 0x05, 0x4e, 0x0c, 0x00, 0x00, 0x00, 0x01,
            0x1e, 0x0e, 0x09, 0x55, 0x0f, 0x29, 0x00, 0x1f, 0x4f,
        ]
    );

    let mut true_request = request.clone();
    true_request.issue_confirmed_notifications = true;
    true_request.list_of_cov_subscription_specifications[0].list_of_cov_references[0].timestamped =
        true;
    encoded.clear();
    true_request.encode(&mut encoded);
    assert_eq!(
        encoded.as_ref(),
        &[
            0x09, 0x01, 0x19, 0x01, 0x29, 0x3c, 0x39, 0x05, 0x4e, 0x0c, 0x00, 0x00, 0x00, 0x01,
            0x1e, 0x0e, 0x09, 0x55, 0x0f, 0x29, 0x01, 0x1f, 0x4f,
        ]
    );

    let mut missing_confirmed = BytesMut::new();
    primitives::encode_ctx_unsigned(&mut missing_confirmed, 0, 1);
    tags::encode_opening_tag(&mut missing_confirmed, 4);
    tags::encode_closing_tag(&mut missing_confirmed, 4);
    assert!(matches!(
        SubscribeCOVPropertyMultipleRequest::decode(&missing_confirmed),
        Err(Error::Reject { reason })
            if reason == RejectReason::MISSING_REQUIRED_PARAMETER.to_raw()
    ));

    let mut missing_timestamped = raw_subscription(PropertyIdentifier::PRESENT_VALUE).to_vec();
    let timestamped = missing_timestamped
        .windows(2)
        .rposition(|window| window == [0x29, 0x00])
        .unwrap();
    missing_timestamped.drain(timestamped..timestamped + 2);
    assert!(matches!(
        SubscribeCOVPropertyMultipleRequest::decode(&missing_timestamped),
        Err(Error::Reject { reason })
            if reason == RejectReason::MISSING_REQUIRED_PARAMETER.to_raw()
    ));

    let malformed_confirmed = [0x09, 0x01, 0x19, 0x02, 0x4e, 0x4f];
    assert!(matches!(
        SubscribeCOVPropertyMultipleRequest::decode(&malformed_confirmed),
        Err(Error::Reject { reason })
            if reason == RejectReason::INVALID_DATA_ENCODING.to_raw()
    ));

    let mut malformed_timestamped = raw_subscription(PropertyIdentifier::PRESENT_VALUE);
    let timestamped = malformed_timestamped
        .windows(2)
        .rposition(|window| window == [0x29, 0x00])
        .unwrap();
    malformed_timestamped[timestamped + 1] = 2;
    assert!(matches!(
        SubscribeCOVPropertyMultipleRequest::decode(&malformed_timestamped),
        Err(Error::Reject { reason })
            if reason == RejectReason::INVALID_DATA_ENCODING.to_raw()
    ));
}

#[test]
fn subscribe_rejects_empty_reference_lists_and_special_property_identifiers() {
    let empty = SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: 1,
        issue_confirmed_notifications: false,
        lifetime: None,
        max_notification_delay: None,
        list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
            monitored_object_identifier: oid(ObjectType::ANALOG_INPUT, 1),
            list_of_cov_references: Vec::new(),
        }],
    };
    let mut encoded = BytesMut::new();
    assert!(empty.try_encode(&mut encoded).is_err());

    for property_identifier in [
        PropertyIdentifier::ALL,
        PropertyIdentifier::OPTIONAL,
        PropertyIdentifier::REQUIRED,
    ] {
        let request = subscription_with(property_identifier);
        let mut encoded = BytesMut::new();
        assert!(request.try_encode(&mut encoded).is_err());
        assert!(
            SubscribeCOVPropertyMultipleRequest::decode(&raw_subscription(property_identifier))
                .is_err()
        );
    }

    let proprietary = PropertyIdentifier::from_raw(512);
    let request = subscription_with(proprietary);
    encoded.clear();
    request.try_encode(&mut encoded).unwrap();
    assert_eq!(
        SubscribeCOVPropertyMultipleRequest::decode(&encoded)
            .unwrap()
            .list_of_cov_subscription_specifications[0]
            .list_of_cov_references[0]
            .monitored_property
            .property_identifier,
        proprietary
    );
}

#[test]
fn notification_decodes_standard_datetime_and_primitive_time_of_change() {
    let date = bacnet_types::primitives::Date {
        year: 126,
        month: 8,
        day: 20,
        day_of_week: 4,
    };
    let time = bacnet_types::primitives::Time {
        hour: 12,
        minute: 34,
        second: 56,
        hundredths: 78,
    };
    let mut encoded = BytesMut::new();
    primitives::encode_ctx_unsigned(&mut encoded, 0, 1);
    primitives::encode_ctx_object_id(&mut encoded, 1, &oid(ObjectType::DEVICE, 1));
    primitives::encode_ctx_unsigned(&mut encoded, 2, 1);
    tags::encode_opening_tag(&mut encoded, 3);
    primitives::encode_app_date(&mut encoded, &date);
    primitives::encode_app_time(&mut encoded, &time);
    tags::encode_closing_tag(&mut encoded, 3);
    tags::encode_opening_tag(&mut encoded, 4);
    primitives::encode_ctx_object_id(&mut encoded, 0, &oid(ObjectType::ANALOG_INPUT, 1));
    tags::encode_opening_tag(&mut encoded, 1);
    primitives::encode_ctx_unsigned(
        &mut encoded,
        0,
        PropertyIdentifier::PRESENT_VALUE.to_raw() as u64,
    );
    tags::encode_opening_tag(&mut encoded, 2);
    encoded.extend_from_slice(&[0]);
    tags::encode_closing_tag(&mut encoded, 2);
    tags::encode_tag(&mut encoded, 3, tags::TagClass::Context, 4);
    encoded.extend_from_slice(&time.encode());
    tags::encode_closing_tag(&mut encoded, 1);
    tags::encode_closing_tag(&mut encoded, 4);

    let decoded = COVNotificationMultipleRequest::decode(&encoded).unwrap();
    assert_eq!(decoded.timestamp, Some((date, time)));
    assert_eq!(
        decoded.list_of_cov_notifications[0].list_of_values[0].time_of_change,
        Some(time)
    );
    let mut reencoded = BytesMut::new();
    decoded.encode(&mut reencoded).unwrap();
    assert_eq!(reencoded, encoded);
}

#[test]
fn timestamp_and_time_of_change_presence_must_agree() {
    let date = bacnet_types::primitives::Date {
        year: 126,
        month: 8,
        day: 20,
        day_of_week: 4,
    };
    let time = bacnet_types::primitives::Time {
        hour: 12,
        minute: 34,
        second: 56,
        hundredths: 78,
    };

    let mut request = notification_with(PropertyIdentifier::PRESENT_VALUE);
    let mut output = BytesMut::from(&b"prefix"[..]);
    request.timestamp = Some((date, time));
    assert!(request.encode(&mut output).is_err());
    assert_eq!(output.as_ref(), b"prefix");

    request.timestamp = None;
    request.list_of_cov_notifications[0].list_of_values[0].time_of_change = Some(time);
    assert!(request.encode(&mut output).is_err());
    assert_eq!(output.as_ref(), b"prefix");

    request.timestamp = Some((date, time));
    request.encode(&mut output).unwrap();
    assert!(output.len() > b"prefix".len());
}

#[test]
fn notification_requires_specific_actual_date_and_time_values() {
    let valid_date = bacnet_types::primitives::Date {
        year: 126,
        month: 8,
        day: 20,
        day_of_week: 4,
    };
    let valid_time = bacnet_types::primitives::Time {
        hour: 12,
        minute: 34,
        second: 56,
        hundredths: 78,
    };

    let mut invalid_date = notification_with(PropertyIdentifier::PRESENT_VALUE);
    invalid_date.timestamp = Some((
        bacnet_types::primitives::Date {
            month: 0,
            ..valid_date
        },
        valid_time,
    ));
    invalid_date.list_of_cov_notifications[0].list_of_values[0].time_of_change = Some(valid_time);
    let mut output = BytesMut::from(&b"prefix"[..]);
    assert!(invalid_date.encode(&mut output).is_err());
    assert_eq!(output.as_ref(), b"prefix");

    invalid_date.timestamp = Some((
        bacnet_types::primitives::Date {
            year: 123,
            month: 2,
            day: 29,
            ..valid_date
        },
        valid_time,
    ));
    assert!(invalid_date.encode(&mut output).is_err());
    assert_eq!(output.as_ref(), b"prefix");

    invalid_date.timestamp = Some((
        bacnet_types::primitives::Date {
            year: 124,
            month: 2,
            day: 29,
            ..valid_date
        },
        valid_time,
    ));
    let mut leap_output = BytesMut::new();
    invalid_date.encode(&mut leap_output).unwrap();

    let mut invalid_time = notification_with(PropertyIdentifier::PRESENT_VALUE);
    let invalid_actual_time = bacnet_types::primitives::Time {
        hour: 24,
        ..valid_time
    };
    invalid_time.timestamp = Some((valid_date, invalid_actual_time));
    invalid_time.list_of_cov_notifications[0].list_of_values[0].time_of_change =
        Some(invalid_actual_time);
    assert!(invalid_time.encode(&mut output).is_err());
    assert_eq!(output.as_ref(), b"prefix");

    let mut encoded = BytesMut::new();
    let mut valid = notification_with(PropertyIdentifier::PRESENT_VALUE);
    valid.timestamp = Some((valid_date, valid_time));
    valid.list_of_cov_notifications[0].list_of_values[0].time_of_change = Some(valid_time);
    valid.encode(&mut encoded).unwrap();
    let date_tag = encoded.iter().position(|byte| *byte == 0xa4).unwrap();
    encoded[date_tag + 2] = 0;
    assert!(matches!(
        COVNotificationMultipleRequest::decode(&encoded),
        Err(Error::Reject { reason })
            if reason == RejectReason::INVALID_DATA_ENCODING.to_raw()
    ));

    encoded.clear();
    valid.encode(&mut encoded).unwrap();
    let date_tag = encoded.iter().position(|byte| *byte == 0xa4).unwrap();
    encoded[date_tag + 2] = 2;
    encoded[date_tag + 3] = 30;
    assert!(matches!(
        COVNotificationMultipleRequest::decode(&encoded),
        Err(Error::Reject { reason })
            if reason == RejectReason::INVALID_DATA_ENCODING.to_raw()
    ));

    encoded.clear();
    valid.encode(&mut encoded).unwrap();
    let time_of_change = encoded.iter().position(|byte| *byte == 0x3c).unwrap();
    encoded[time_of_change + 1] = 24;
    assert!(matches!(
        COVNotificationMultipleRequest::decode(&encoded),
        Err(Error::Reject { reason })
            if reason == RejectReason::INVALID_DATA_ENCODING.to_raw()
    ));
}

#[test]
fn encoder_validation_is_atomic_and_caps_total_nested_items() {
    let mut invalid_subscription = subscription_with(PropertyIdentifier::ALL);
    let mut output = BytesMut::from(&b"prefix"[..]);
    assert!(invalid_subscription.try_encode(&mut output).is_err());
    assert_eq!(output.as_ref(), b"prefix");

    let mut inconsistent_timing = subscription_with(PropertyIdentifier::PRESENT_VALUE);
    inconsistent_timing.max_notification_delay = None;
    assert!(inconsistent_timing.try_encode(&mut output).is_err());
    assert_eq!(output.as_ref(), b"prefix");

    let mut out_of_range_timing = subscription_with(PropertyIdentifier::PRESENT_VALUE);
    out_of_range_timing.lifetime = Some(0);
    assert!(out_of_range_timing.try_encode(&mut output).is_err());
    assert_eq!(output.as_ref(), b"prefix");

    invalid_subscription.list_of_cov_subscription_specifications[0].list_of_cov_references =
        vec![property_ref(PropertyIdentifier::PRESENT_VALUE); MAX_DECODED_ITEMS + 1];
    assert!(invalid_subscription.try_encode(&mut output).is_err());
    assert_eq!(output.as_ref(), b"prefix");

    let mut exact = subscription_with(PropertyIdentifier::PRESENT_VALUE);
    exact.list_of_cov_subscription_specifications[0].list_of_cov_references =
        vec![property_ref(PropertyIdentifier::PRESENT_VALUE); MAX_DECODED_ITEMS / 2];
    let mut second_specification = exact.list_of_cov_subscription_specifications[0].clone();
    second_specification.monitored_object_identifier = oid(ObjectType::ANALOG_INPUT, 2);
    second_specification.list_of_cov_references =
        vec![property_ref(PropertyIdentifier::PRESENT_VALUE); MAX_DECODED_ITEMS / 2];
    exact
        .list_of_cov_subscription_specifications
        .push(second_specification);
    let mut encoded = BytesMut::new();
    exact.try_encode(&mut encoded).unwrap();
    let decoded = SubscribeCOVPropertyMultipleRequest::decode(&encoded).unwrap();
    assert_eq!(
        decoded.list_of_cov_subscription_specifications[0]
            .list_of_cov_references
            .len()
            + decoded.list_of_cov_subscription_specifications[1]
                .list_of_cov_references
                .len(),
        MAX_DECODED_ITEMS
    );
    let mut extra_reference = BytesMut::new();
    tags::encode_opening_tag(&mut extra_reference, 0);
    PropertyReference {
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    }
    .encode(&mut extra_reference);
    tags::encode_closing_tag(&mut extra_reference, 0);
    primitives::encode_ctx_boolean(&mut extra_reference, 2, false);
    let mut over_cap = encoded.to_vec();
    let insert_at = over_cap.len() - 2;
    over_cap.splice(insert_at..insert_at, extra_reference);
    assert!(matches!(
        SubscribeCOVPropertyMultipleRequest::decode(&over_cap),
        Err(Error::Reject { reason }) if reason == RejectReason::BUFFER_OVERFLOW.to_raw()
    ));

    let value = notification_with(PropertyIdentifier::PRESENT_VALUE).list_of_cov_notifications[0]
        .list_of_values[0]
        .clone();
    let mut notification = notification_with(PropertyIdentifier::PRESENT_VALUE);
    notification.list_of_cov_notifications[0].list_of_values =
        vec![value.clone(); MAX_DECODED_ITEMS / 2];
    notification
        .list_of_cov_notifications
        .push(COVNotificationItem {
            monitored_object_identifier: oid(ObjectType::ANALOG_INPUT, 2),
            list_of_values: vec![value.clone(); MAX_DECODED_ITEMS / 2],
        });
    encoded.clear();
    notification.encode(&mut encoded).unwrap();
    let decoded = COVNotificationMultipleRequest::decode(&encoded).unwrap();
    assert_eq!(
        decoded
            .list_of_cov_notifications
            .iter()
            .map(|item| item.list_of_values.len())
            .sum::<usize>(),
        MAX_DECODED_ITEMS,
    );
    let mut extra_value = BytesMut::new();
    primitives::encode_ctx_unsigned(
        &mut extra_value,
        0,
        PropertyIdentifier::PRESENT_VALUE.to_raw() as u64,
    );
    tags::encode_opening_tag(&mut extra_value, 2);
    extra_value.extend_from_slice(&[0]);
    tags::encode_closing_tag(&mut extra_value, 2);
    let mut over_cap = encoded.to_vec();
    let insert_at = over_cap.len() - 2;
    over_cap.splice(insert_at..insert_at, extra_value);
    assert!(matches!(
        COVNotificationMultipleRequest::decode(&over_cap),
        Err(Error::Reject { reason }) if reason == RejectReason::BUFFER_OVERFLOW.to_raw()
    ));
    notification.list_of_cov_notifications[1]
        .list_of_values
        .push(value);
    output.truncate(b"prefix".len());
    assert!(notification.encode(&mut output).is_err());
    assert_eq!(output.as_ref(), b"prefix");
}

#[test]
fn notification_rejects_legacy_timestamp_and_time_shapes() {
    let mut primitive_timestamp = BytesMut::new();
    primitives::encode_ctx_unsigned(&mut primitive_timestamp, 0, 1);
    primitives::encode_ctx_object_id(&mut primitive_timestamp, 1, &oid(ObjectType::DEVICE, 1));
    primitives::encode_ctx_unsigned(&mut primitive_timestamp, 2, 1);
    primitives::encode_ctx_unsigned(&mut primitive_timestamp, 3, 1);
    tags::encode_opening_tag(&mut primitive_timestamp, 4);
    tags::encode_closing_tag(&mut primitive_timestamp, 4);
    assert!(COVNotificationMultipleRequest::decode(&primitive_timestamp).is_err());

    let mut constructed_time = BytesMut::new();
    primitives::encode_ctx_unsigned(&mut constructed_time, 0, 1);
    primitives::encode_ctx_object_id(&mut constructed_time, 1, &oid(ObjectType::DEVICE, 1));
    primitives::encode_ctx_unsigned(&mut constructed_time, 2, 1);
    tags::encode_opening_tag(&mut constructed_time, 4);
    primitives::encode_ctx_object_id(&mut constructed_time, 0, &oid(ObjectType::ANALOG_INPUT, 1));
    tags::encode_opening_tag(&mut constructed_time, 1);
    primitives::encode_ctx_unsigned(
        &mut constructed_time,
        0,
        PropertyIdentifier::PRESENT_VALUE.to_raw() as u64,
    );
    tags::encode_opening_tag(&mut constructed_time, 2);
    constructed_time.extend_from_slice(&[0]);
    tags::encode_closing_tag(&mut constructed_time, 2);
    tags::encode_opening_tag(&mut constructed_time, 3);
    primitives::encode_ctx_unsigned(&mut constructed_time, 1, 42);
    tags::encode_closing_tag(&mut constructed_time, 3);
    tags::encode_closing_tag(&mut constructed_time, 1);
    tags::encode_closing_tag(&mut constructed_time, 4);
    assert!(COVNotificationMultipleRequest::decode(&constructed_time).is_err());

    let mut malformed_value = notification_with(PropertyIdentifier::PRESENT_VALUE);
    malformed_value.list_of_cov_notifications[0].list_of_values[0].value = vec![0x2e];
    let mut output = BytesMut::from(&b"prefix"[..]);
    assert!(malformed_value.encode(&mut output).is_err());
    assert_eq!(output.as_ref(), b"prefix");
}

#[test]
fn notification_decoder_rejects_timestamp_presence_mismatch_and_accepts_proprietary_id() {
    let date = bacnet_types::primitives::Date {
        year: 126,
        month: 8,
        day: 20,
        day_of_week: 4,
    };
    let time = bacnet_types::primitives::Time {
        hour: 12,
        minute: 34,
        second: 56,
        hundredths: 78,
    };
    let mut request = notification_with(PropertyIdentifier::PRESENT_VALUE);
    request.timestamp = Some((date, time));
    request.list_of_cov_notifications[0].list_of_values[0].time_of_change = Some(time);
    let mut encoded = BytesMut::new();
    request.encode(&mut encoded).unwrap();
    let time_of_change = encoded
        .windows(5)
        .rposition(|window| window == [0x3c, 12, 34, 56, 78])
        .unwrap();
    let mut mismatched = encoded.to_vec();
    mismatched.drain(time_of_change..time_of_change + 5);
    assert!(matches!(
        COVNotificationMultipleRequest::decode(&mismatched),
        Err(Error::Reject { reason }) if reason == RejectReason::PARAMETER_OUT_OF_RANGE.to_raw()
    ));

    let proprietary = PropertyIdentifier::from_raw(512);
    let request = notification_with(proprietary);
    encoded.clear();
    request.encode(&mut encoded).unwrap();
    assert_eq!(
        COVNotificationMultipleRequest::decode(&encoded)
            .unwrap()
            .list_of_cov_notifications[0]
            .list_of_values[0]
            .property_identifier,
        proprietary,
    );
}

#[test]
fn notification_rejects_empty_lists_and_special_property_identifiers() {
    let empty_outer = COVNotificationMultipleRequest {
        subscriber_process_identifier: 1,
        initiating_device_identifier: oid(ObjectType::DEVICE, 1),
        time_remaining: 1,
        timestamp: None,
        list_of_cov_notifications: Vec::new(),
    };
    let mut encoded = BytesMut::new();
    primitives::encode_ctx_unsigned(&mut encoded, 0, 1);
    primitives::encode_ctx_object_id(&mut encoded, 1, &oid(ObjectType::DEVICE, 1));
    primitives::encode_ctx_unsigned(&mut encoded, 2, 1);
    tags::encode_opening_tag(&mut encoded, 4);
    tags::encode_closing_tag(&mut encoded, 4);
    assert!(COVNotificationMultipleRequest::decode(&encoded).is_err());

    let mut empty_values = notification_with(PropertyIdentifier::PRESENT_VALUE);
    empty_values.list_of_cov_notifications[0]
        .list_of_values
        .clear();
    encoded.clear();
    assert!(empty_values.encode(&mut encoded).is_err());

    for property_identifier in [
        PropertyIdentifier::ALL,
        PropertyIdentifier::OPTIONAL,
        PropertyIdentifier::REQUIRED,
    ] {
        let request = notification_with(property_identifier);
        encoded.clear();
        assert!(request.encode(&mut encoded).is_err());

        let valid = notification_with(PropertyIdentifier::PRESENT_VALUE);
        valid.encode(&mut encoded).unwrap();
        let property_tag = encoded
            .windows(2)
            .rposition(|window| window == [0x09, 0x55])
            .unwrap();
        encoded[property_tag + 1] = property_identifier.to_raw() as u8;
        assert!(COVNotificationMultipleRequest::decode(&encoded).is_err());
    }
}
