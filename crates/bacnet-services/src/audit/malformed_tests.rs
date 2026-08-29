use super::*;
use bacnet_encoding::{constructed::encode_recipient, primitives, tags};
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::{Date, Time};
use bacnet_types::MacAddr;

fn device(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap()
}

fn minimal_notification(operation: AuditOperation) -> BACnetAuditNotification {
    BACnetAuditNotification {
        source_timestamp: None,
        target_timestamp: None,
        source_device: BACnetRecipient::Device(device(1)),
        source_object: None,
        operation,
        source_comment: None,
        target_comment: None,
        invoke_id: None,
        source_user_id: None,
        source_user_role: None,
        target_device: BACnetRecipient::Device(device(2)),
        target_object: None,
        target_property: None,
        target_priority: None,
        target_value: None,
        current_value: None,
        result: None,
    }
}

fn encode_one(notification: BACnetAuditNotification) -> Vec<u8> {
    let mut encoded = BytesMut::new();
    AuditNotificationRequest {
        notifications: vec![notification],
    }
    .encode(&mut encoded)
    .unwrap();
    encoded.to_vec()
}

fn insert_before_target_device(mut encoded: Vec<u8>, field: &[u8]) -> Vec<u8> {
    let target = encoded
        .iter()
        .position(|byte| *byte == 0xae)
        .expect("minimal notification has opening tag [10]");
    encoded.splice(target..target, field.iter().copied());
    encoded
}

#[test]
fn notification_uses_literal_clause_21_tags_and_round_trips_all_fields() {
    let property = AuditPropertyReference {
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: Some(u64::MAX),
    };
    let notification = BACnetAuditNotification {
        source_timestamp: Some(BACnetTimeStamp::SequenceNumber(1)),
        target_timestamp: Some(BACnetTimeStamp::SequenceNumber(2)),
        source_device: BACnetRecipient::Device(device(1)),
        source_object: Some(ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 4).unwrap()),
        operation: AuditOperation::GENERAL,
        source_comment: Some("source".into()),
        target_comment: Some("target".into()),
        invoke_id: Some(u8::MAX),
        source_user_id: Some(u16::MAX),
        source_user_role: Some(u8::MAX),
        target_device: BACnetRecipient::Device(device(2)),
        target_object: Some(ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 5).unwrap()),
        target_property: Some(property),
        target_priority: Some(16),
        target_value: Some(vec![0x00]),
        current_value: Some(vec![0x11]),
        result: Some((ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)),
    };
    let encoded = encode_one(notification.clone());

    assert_eq!(encoded.first(), Some(&0x0e));
    assert_eq!(encoded.last(), Some(&0x0f));
    assert!(encoded.windows(2).any(|bytes| bytes == [0xfe, 16]));
    assert!(encoded.windows(2).any(|bytes| bytes == [0xff, 16]));
    assert_eq!(
        AuditNotificationRequest::decode(&encoded)
            .unwrap()
            .notifications,
        vec![notification]
    );

    for raw in [0, 15, 32, 63] {
        let notification = minimal_notification(AuditOperation::from_raw(raw));
        assert_eq!(
            AuditNotificationRequest::decode(&encode_one(notification.clone()))
                .unwrap()
                .notifications,
            vec![notification]
        );
    }
}

#[test]
fn minimal_notification_has_no_per_item_sequence_wrapper() {
    let notification = minimal_notification(AuditOperation::READ);
    let encoded = encode_one(notification.clone());
    let mut expected = BytesMut::new();
    tags::encode_opening_tag(&mut expected, 0);
    tags::encode_opening_tag(&mut expected, 2);
    encode_recipient(&mut expected, &notification.source_device);
    tags::encode_closing_tag(&mut expected, 2);
    primitives::encode_ctx_enumerated(&mut expected, 4, 0);
    tags::encode_opening_tag(&mut expected, 10);
    encode_recipient(&mut expected, &notification.target_device);
    tags::encode_closing_tag(&mut expected, 10);
    tags::encode_closing_tag(&mut expected, 0);
    assert_eq!(encoded, expected.as_ref());
}

#[test]
fn reserved_operations_fail_decode_and_encode_is_atomic() {
    for raw in [16, 31, 64, u32::MAX] {
        let request = AuditNotificationRequest {
            notifications: vec![minimal_notification(AuditOperation::from_raw(raw))],
        };
        let mut output = BytesMut::from(&b"prefix"[..]);
        assert!(request.encode(&mut output).is_err());
        assert_eq!(output.as_ref(), b"prefix");
    }

    let mut encoded = encode_one(minimal_notification(AuditOperation::READ));
    let operation = encoded
        .windows(2)
        .position(|bytes| bytes == [0x49, 0])
        .unwrap();
    encoded[operation + 1] = 16;
    assert!(AuditNotificationRequest::decode(&encoded).is_err());
}

#[test]
fn unsigned8_and_unsigned16_fields_reject_numeric_overflow() {
    let encoded = encode_one(minimal_notification(AuditOperation::READ));
    for (tag_number, contents) in [(7, &[1, 0][..]), (8, &[1, 0, 0][..]), (9, &[1, 0][..])] {
        let mut field = BytesMut::new();
        tags::encode_tag(
            &mut field,
            tag_number,
            tags::TagClass::Context,
            contents.len() as u32,
        );
        field.extend_from_slice(contents);
        let malformed = insert_before_target_device(encoded.clone(), &field);
        assert!(AuditNotificationRequest::decode(&malformed).is_err());
    }
}

#[test]
fn notification_rejects_noncanonical_unsigned_and_enumerated_fields() {
    let encoded = encode_one(minimal_notification(AuditOperation::READ));

    let operation = encoded
        .windows(2)
        .position(|bytes| bytes == [0x49, 0])
        .unwrap();
    let mut noncanonical_operation = encoded.clone();
    noncanonical_operation.splice(operation..operation + 2, [0x4a, 0, 0]);
    assert!(AuditNotificationRequest::decode(&noncanonical_operation).is_err());

    for field in [[0x7a, 0, 1], [0x8a, 0, 1], [0x9a, 0, 1]] {
        let malformed = insert_before_target_device(encoded.clone(), &field);
        assert!(AuditNotificationRequest::decode(&malformed).is_err());
    }

    let mut timestamp_notification = minimal_notification(AuditOperation::READ);
    timestamp_notification.source_timestamp = Some(BACnetTimeStamp::SequenceNumber(1));
    let mut noncanonical_timestamp = encode_one(timestamp_notification);
    let sequence = noncanonical_timestamp
        .windows(2)
        .position(|bytes| bytes == [0x19, 1])
        .unwrap();
    noncanonical_timestamp.splice(sequence..sequence + 2, [0x1a, 0, 1]);
    assert!(AuditNotificationRequest::decode(&noncanonical_timestamp).is_err());

    let mut address_notification = minimal_notification(AuditOperation::READ);
    address_notification.source_device = BACnetRecipient::Address(BACnetAddress {
        network_number: 1,
        mac_address: MacAddr::new(),
    });
    let mut noncanonical_address = encode_one(address_notification);
    let network = noncanonical_address
        .windows(2)
        .position(|bytes| bytes == [0x21, 1])
        .unwrap();
    noncanonical_address.splice(network..network + 2, [0x22, 0, 1]);
    assert!(AuditNotificationRequest::decode(&noncanonical_address).is_err());

    let mut property_notification = minimal_notification(AuditOperation::WRITE);
    property_notification.target_property = Some(AuditPropertyReference {
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: Some(1),
    });
    let canonical_property = encode_one(property_notification);
    for bytes in [[0x09, 0x55], [0x19, 0x01]] {
        let at = canonical_property
            .windows(2)
            .position(|window| window == bytes)
            .unwrap();
        let mut malformed = canonical_property.clone();
        malformed.splice(at..at + 2, [bytes[0] + 1, 0, bytes[1]]);
        assert!(AuditNotificationRequest::decode(&malformed).is_err());
    }

    let mut result_notification = minimal_notification(AuditOperation::WRITE);
    result_notification.result = Some((ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED));
    let canonical_result = encode_one(result_notification);
    for value in [
        ErrorClass::PROPERTY.to_raw(),
        ErrorCode::WRITE_ACCESS_DENIED.to_raw(),
    ] {
        let value = u8::try_from(value).unwrap();
        let at = canonical_result
            .windows(2)
            .position(|bytes| bytes == [0x91, value])
            .unwrap();
        let mut malformed = canonical_result.clone();
        malformed.splice(at..at + 2, [0x92, 0, value]);
        assert!(AuditNotificationRequest::decode(&malformed).is_err());
    }
}

#[test]
fn target_priority_enforces_inclusive_one_to_sixteen_range() {
    for priority in [1, 16] {
        let mut notification = minimal_notification(AuditOperation::WRITE);
        notification.target_priority = Some(priority);
        let encoded = encode_one(notification.clone());
        assert_eq!(
            AuditNotificationRequest::decode(&encoded)
                .unwrap()
                .notifications,
            vec![notification]
        );
    }

    for priority in [0, 17] {
        let mut notification = minimal_notification(AuditOperation::WRITE);
        notification.target_priority = Some(priority);
        let request = AuditNotificationRequest {
            notifications: vec![notification],
        };
        let mut output = BytesMut::from(&b"prefix"[..]);
        assert!(request.encode(&mut output).is_err());
        assert_eq!(output.as_ref(), b"prefix");
    }

    let mut notification = minimal_notification(AuditOperation::WRITE);
    notification.target_priority = Some(1);
    let mut encoded = encode_one(notification);
    let priority = encoded
        .windows(2)
        .position(|bytes| bytes == [0xd9, 1])
        .unwrap();
    encoded[priority + 1] = 0;
    assert!(AuditNotificationRequest::decode(&encoded).is_err());
}

#[test]
fn raw_values_must_be_nonempty_structural_tlv_and_encoding_is_atomic() {
    for raw in [Vec::new(), vec![0x21]] {
        let mut notification = minimal_notification(AuditOperation::WRITE);
        notification.target_value = Some(raw);
        let request = AuditNotificationRequest {
            notifications: vec![notification],
        };
        let mut output = BytesMut::from(&b"prefix"[..]);
        assert!(request.encode(&mut output).is_err());
        assert_eq!(output.as_ref(), b"prefix");
    }

    let mut notification = minimal_notification(AuditOperation::WRITE);
    notification.target_value = Some(vec![0x00]);
    let mut empty = encode_one(notification);
    let raw = empty
        .windows(3)
        .position(|bytes| bytes == [0xee, 0x00, 0xef])
        .unwrap();
    empty.remove(raw + 1);
    assert!(AuditNotificationRequest::decode(&empty).is_err());

    let mut notification = minimal_notification(AuditOperation::WRITE);
    notification.target_value = Some(vec![0x00]);
    let mut truncated = encode_one(notification);
    let raw = truncated
        .windows(3)
        .position(|bytes| bytes == [0xee, 0x00, 0xef])
        .unwrap();
    truncated[raw + 1] = 0x21;
    assert!(AuditNotificationRequest::decode(&truncated).is_err());
}

#[test]
fn notification_list_requires_one_to_ten_thousand_items() {
    let empty = AuditNotificationRequest {
        notifications: Vec::new(),
    };
    let mut output = BytesMut::from(&b"prefix"[..]);
    assert!(empty.encode(&mut output).is_err());
    assert_eq!(output.as_ref(), b"prefix");
    assert!(AuditNotificationRequest::decode(&[0x0e, 0x0f]).is_err());

    let one = encode_one(minimal_notification(AuditOperation::READ));
    let item = &one[1..one.len() - 1];
    let mut at_limit = BytesMut::new();
    tags::encode_opening_tag(&mut at_limit, 0);
    for _ in 0..crate::common::MAX_DECODED_ITEMS {
        at_limit.extend_from_slice(item);
    }
    tags::encode_closing_tag(&mut at_limit, 0);
    assert_eq!(
        AuditNotificationRequest::decode(&at_limit)
            .unwrap()
            .notifications
            .len(),
        crate::common::MAX_DECODED_ITEMS
    );

    let at_limit_request = AuditNotificationRequest {
        notifications: vec![
            minimal_notification(AuditOperation::READ);
            crate::common::MAX_DECODED_ITEMS
        ],
    };
    let mut encoded_limit = BytesMut::new();
    at_limit_request.encode(&mut encoded_limit).unwrap();
    assert_eq!(
        AuditNotificationRequest::decode(&encoded_limit)
            .unwrap()
            .notifications
            .len(),
        crate::common::MAX_DECODED_ITEMS
    );

    let mut over_limit = at_limit;
    over_limit.truncate(over_limit.len() - 1);
    over_limit.extend_from_slice(item);
    tags::encode_closing_tag(&mut over_limit, 0);
    assert!(AuditNotificationRequest::decode(&over_limit).is_err());

    let request = AuditNotificationRequest {
        notifications: vec![
            minimal_notification(AuditOperation::READ);
            crate::common::MAX_DECODED_ITEMS + 1
        ],
    };
    let mut output = BytesMut::from(&b"prefix"[..]);
    assert!(request.encode(&mut output).is_err());
    assert_eq!(output.as_ref(), b"prefix");
}

#[test]
fn decoder_requires_full_service_consumption() {
    let mut encoded = encode_one(minimal_notification(AuditOperation::READ));
    encoded.push(0x00);
    assert!(AuditNotificationRequest::decode(&encoded).is_err());

    let mut missing_close = encode_one(minimal_notification(AuditOperation::READ));
    missing_close.pop();
    assert!(AuditNotificationRequest::decode(&missing_close).is_err());

    let wrong_wrapper = [0x1e, 0x1f];
    assert!(AuditNotificationRequest::decode(&wrong_wrapper).is_err());
}

fn query_ack(datum: BACnetAuditLogDatum) -> AuditLogQueryAck {
    AuditLogQueryAck {
        audit_log: ObjectIdentifier::new(ObjectType::AUDIT_LOG, 1).unwrap(),
        records: vec![BACnetAuditLogRecordResult {
            sequence_number: 1,
            record: BACnetAuditLogRecord {
                timestamp: (
                    Date {
                        year: 126,
                        month: 8,
                        day: 29,
                        day_of_week: 6,
                    },
                    Time {
                        hour: 12,
                        minute: 34,
                        second: 56,
                        hundredths: 78,
                    },
                ),
                datum,
            },
        }],
        no_more_items: false,
    }
}

fn encode_query_ack(datum: BACnetAuditLogDatum) -> Vec<u8> {
    let mut encoded = BytesMut::new();
    query_ack(datum).encode(&mut encoded).unwrap();
    encoded.to_vec()
}

#[test]
fn query_ack_rejects_unsigned_boolean_and_top_level_malformations() {
    let canonical = encode_query_ack(BACnetAuditLogDatum::LogStatus(0));
    let sequence = canonical
        .windows(2)
        .position(|bytes| bytes == [0x09, 1])
        .unwrap();

    let mut noncanonical = canonical.clone();
    noncanonical.splice(sequence..sequence + 2, [0x0a, 0, 1]);
    assert!(AuditLogQueryAck::decode(&noncanonical).is_err());

    let mut over_wide = canonical.clone();
    over_wide.splice(sequence..sequence + 2, [0x0d, 9, 1, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert!(AuditLogQueryAck::decode(&over_wide).is_err());

    let mut non_boolean = canonical.clone();
    *non_boolean.last_mut().unwrap() = 2;
    assert!(AuditLogQueryAck::decode(&non_boolean).is_err());

    let mut missing_boolean = canonical.clone();
    missing_boolean.truncate(missing_boolean.len() - 2);
    assert!(AuditLogQueryAck::decode(&missing_boolean).is_err());

    let mut trailing = canonical.clone();
    trailing.push(0);
    assert!(AuditLogQueryAck::decode(&trailing).is_err());

    let mut wrong_list = canonical.clone();
    wrong_list[5] = 0x2e;
    assert!(AuditLogQueryAck::decode(&wrong_list).is_err());

    let mut mismatched_list = canonical;
    let list_close = mismatched_list.len() - 3;
    mismatched_list[list_close] = 0x0f;
    assert!(AuditLogQueryAck::decode(&mismatched_list).is_err());
}

#[test]
fn query_ack_rejects_malformed_date_time_and_datum_choices() {
    let canonical = encode_query_ack(BACnetAuditLogDatum::LogStatus(0b010));

    let date = canonical.iter().position(|byte| *byte == 0xa4).unwrap();
    let mut invalid_date = canonical.clone();
    invalid_date[date + 2] = 0;
    assert!(AuditLogQueryAck::decode(&invalid_date).is_err());

    let time = canonical.iter().position(|byte| *byte == 0xb4).unwrap();
    let mut invalid_time = canonical.clone();
    invalid_time[time + 1] = 24;
    assert!(AuditLogQueryAck::decode(&invalid_time).is_err());

    let status = canonical
        .windows(3)
        .position(|bytes| bytes == [0x0a, 5, 0x40])
        .unwrap();
    let mut bad_unused = canonical.clone();
    bad_unused[status + 1] = 4;
    assert!(AuditLogQueryAck::decode(&bad_unused).is_err());

    let mut bad_padding = canonical.clone();
    bad_padding[status + 2] |= 1;
    assert!(AuditLogQueryAck::decode(&bad_padding).is_err());

    let mut wrong_class = canonical;
    wrong_class[status] = 0x82;
    assert!(AuditLogQueryAck::decode(&wrong_class).is_err());

    let mut real = encode_query_ack(BACnetAuditLogDatum::TimeChange(1.5));
    let real_tag = real.iter().position(|byte| *byte == 0x2c).unwrap();
    real[real_tag] = 0x2b;
    assert!(AuditLogQueryAck::decode(&real).is_err());
}

#[test]
fn query_ack_nested_notification_requires_complete_consumption() {
    let notification = minimal_notification(AuditOperation::WRITE);
    let mut encoded = encode_query_ack(BACnetAuditLogDatum::AuditNotification(notification));
    let target_close = encoded.iter().position(|byte| *byte == 0xaf).unwrap();
    encoded.insert(target_close + 1, 0x00);
    assert!(AuditLogQueryAck::decode(&encoded).is_err());
}

#[test]
fn query_ack_record_list_accepts_limit_and_rejects_one_more() {
    let record = BACnetAuditLogRecordResult {
        sequence_number: 0,
        record: query_ack(BACnetAuditLogDatum::LogStatus(0))
            .records
            .pop()
            .unwrap()
            .record,
    };
    let at_limit = AuditLogQueryAck {
        audit_log: ObjectIdentifier::new(ObjectType::AUDIT_LOG, 1).unwrap(),
        records: vec![record.clone(); crate::common::MAX_DECODED_ITEMS],
        no_more_items: true,
    };
    let mut encoded = BytesMut::new();
    at_limit.encode(&mut encoded).unwrap();
    assert_eq!(
        AuditLogQueryAck::decode(&encoded).unwrap().records.len(),
        crate::common::MAX_DECODED_ITEMS
    );

    let item = encoded[6..encoded.len() - 3].to_vec();
    let mut over_limit = encoded;
    over_limit.truncate(over_limit.len() - 3);
    over_limit.extend_from_slice(&item[..item.len() / crate::common::MAX_DECODED_ITEMS]);
    tags::encode_closing_tag(&mut over_limit, 1);
    primitives::encode_ctx_boolean(&mut over_limit, 2, true);
    assert!(AuditLogQueryAck::decode(&over_limit).is_err());

    let over_limit_model = AuditLogQueryAck {
        audit_log: ObjectIdentifier::new(ObjectType::AUDIT_LOG, 1).unwrap(),
        records: vec![record; crate::common::MAX_DECODED_ITEMS + 1],
        no_more_items: false,
    };
    let mut output = BytesMut::from(&b"prefix"[..]);
    assert!(over_limit_model.encode(&mut output).is_err());
    assert_eq!(output.as_ref(), b"prefix");
}
