use super::*;
use bacnet_encoding::constructed::{
    encode_audit_log_record, encode_event_log_record, encode_log_multiple_record, encode_log_record,
};
use bacnet_types::constructed::{
    AuditPropertyReference, BACnetAuditLogRecord, BACnetEventLogRecord, BACnetLogMultipleRecord,
    BACnetLogRecord, LogValue,
};
use bacnet_types::enums::{AuditOperation, EventState, EventType};
use bacnet_types::primitives::BACnetTimeStamp;
use bytes::BytesMut;

const DATE: Date = Date {
    year: 126,
    month: 10,
    day: 3,
    day_of_week: 6,
};

fn time(hour: u8) -> Time {
    Time {
        hour,
        minute: 15,
        second: 30,
        hundredths: 5,
    }
}

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn row(hour: u8, datum: &str, status_flags: Option<&str>) -> LogRecordRow {
    LogRecordRow {
        timestamp: format!("2026-10-03 {hour:02}:15:30.05"),
        datum: datum.to_string(),
        status_flags: status_flags.map(str::to_string),
    }
}

fn ack(object_type: ObjectType, item_count: u32, data: &[u8]) -> ReadRangeAck {
    ReadRangeAck {
        object_identifier: oid(object_type, 1),
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
        result_flags: (true, true, false),
        item_count,
        item_data: data.to_vec(),
        first_sequence_number: None,
    }
}

/// Decode `data`, said to hold `item_count` records, as the log buffer of
/// `object_type`.
fn rows(object_type: ObjectType, item_count: u32, data: &[u8]) -> LogRows {
    log_rows(&ack(object_type, item_count, data)).unwrap()
}

#[test]
fn only_a_log_objects_log_buffer_shows_records() {
    for object_type in [
        ObjectType::TREND_LOG,
        ObjectType::EVENT_LOG,
        ObjectType::TREND_LOG_MULTIPLE,
        ObjectType::AUDIT_LOG,
    ] {
        assert!(
            log_rows(&ack(object_type, 0, &[])).is_some(),
            "{object_type}"
        );
    }
    for object_type in [ObjectType::DEVICE, ObjectType::ANALOG_INPUT] {
        assert!(
            log_rows(&ack(object_type, 0, &[])).is_none(),
            "{object_type}"
        );
    }
    let mut other = ack(ObjectType::TREND_LOG, 0, &[]);
    other.property_identifier = PropertyIdentifier::EVENT_TIME_STAMPS;
    assert!(log_rows(&other).is_none());
}

#[test]
fn trend_log_records_show_datum_and_status_flags() {
    let mut data = BytesMut::new();
    for (hour, log_datum, status_flags) in [
        (8, LogDatum::RealValue(72.5), Some(StatusFlags::IN_ALARM)),
        (9, LogDatum::LogStatus(LogStatus::BUFFER_PURGED), None),
        (
            10,
            LogDatum::Failure {
                error_class: ErrorClass::PROPERTY.to_raw().into(),
                error_code: ErrorCode::VALUE_TOO_LONG.to_raw().into(),
            },
            None,
        ),
        // An any-value holding a CharacterString.
        (
            11,
            LogDatum::AnyValue(vec![0x73, 0x00, b'o', b'k']),
            Some(StatusFlags::empty()),
        ),
    ] {
        let record = BACnetLogRecord {
            date: DATE,
            time: time(hour),
            log_datum,
            status_flags,
        };
        encode_log_record(&record, &mut data).unwrap();
    }
    let records = rows(ObjectType::TREND_LOG, 4, &data);
    assert_eq!(
        records.rows,
        [
            row(8, "72.5", Some("IN_ALARM")),
            row(9, "log-status BUFFER_PURGED", None),
            row(10, "failure PROPERTY/VALUE_TOO_LONG", None),
            row(11, "\"ok\"", Some("")),
        ]
    );
    assert_eq!(records.undecoded, None);
}

#[test]
fn event_log_records_show_the_typed_notification() {
    let notification = EventNotificationRequest {
        process_identifier: 1,
        initiating_device_identifier: oid(ObjectType::DEVICE, 1),
        event_object_identifier: oid(ObjectType::ANALOG_INPUT, 3),
        timestamp: BACnetTimeStamp::SequenceNumber(5),
        notification_class: 0,
        priority: 100,
        event_type: EventType::OUT_OF_RANGE,
        message_text: Some("too hot".into()),
        notify_type: NotifyType::ALARM,
        ack_required: true,
        from_state: EventState::NORMAL,
        to_state: EventState::HIGH_LIMIT,
        event_values: None,
    };
    let acknowledgment = EventNotificationRequest {
        notify_type: NotifyType::ACK_NOTIFICATION,
        message_text: None,
        ..notification.clone()
    };
    let mut data = BytesMut::new();
    for (hour, log_datum) in [
        (8, EventLogDatum::Notification(notification)),
        (9, EventLogDatum::Notification(acknowledgment)),
        (10, EventLogDatum::TimeChange(-1.5)),
    ] {
        let record = BACnetEventLogRecord {
            date: DATE,
            time: time(hour),
            log_datum,
        };
        encode_event_log_record(&record, &mut data).unwrap();
    }
    assert_eq!(
        rows(ObjectType::EVENT_LOG, 3, &data).rows,
        [
            row(
                8,
                "ALARM OUT_OF_RANGE ANALOG_INPUT:3 NORMAL -> HIGH_LIMIT \"too hot\"",
                None
            ),
            row(
                9,
                "ACK_NOTIFICATION OUT_OF_RANGE ANALOG_INPUT:3 HIGH_LIMIT",
                None
            ),
            row(10, "time-change -1.5 s", None),
        ]
    );
}

#[test]
fn trend_log_multiple_records_show_every_value() {
    let mut data = BytesMut::new();
    for (hour, log_data) in [
        (
            8,
            LogData::Values(vec![
                LogValue::RealValue(21.5),
                LogValue::BooleanValue(true),
                LogValue::NullValue,
                LogValue::EnumValue(3),
            ]),
        ),
        (9, LogData::LogStatus(LogStatus::LOG_INTERRUPTED)),
    ] {
        let record = BACnetLogMultipleRecord {
            date: DATE,
            time: time(hour),
            log_data,
        };
        encode_log_multiple_record(&record, &mut data).unwrap();
    }
    assert_eq!(
        rows(ObjectType::TREND_LOG_MULTIPLE, 2, &data).rows,
        [
            row(8, "[21.5, true, null, enumerated(3)]", None),
            row(9, "log-status LOG_INTERRUPTED", None),
        ]
    );
}

#[test]
fn audit_log_records_show_the_operation_and_its_target() {
    let notification = BACnetAuditNotification {
        source_timestamp: None,
        target_timestamp: None,
        source_device: BACnetRecipient::Device(oid(ObjectType::DEVICE, 1)),
        source_object: None,
        operation: AuditOperation::WRITE,
        source_comment: None,
        target_comment: None,
        invoke_id: None,
        source_user_id: None,
        source_user_role: None,
        target_device: BACnetRecipient::Device(oid(ObjectType::DEVICE, 2)),
        target_object: Some(oid(ObjectType::ANALOG_VALUE, 4)),
        target_property: Some(AuditPropertyReference {
            property_identifier: PropertyIdentifier::PRIORITY_ARRAY,
            property_array_index: Some(8),
        }),
        target_priority: None,
        target_value: None,
        current_value: None,
        result: Some((ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)),
    };
    let mut data = BytesMut::new();
    for (hour, datum) in [
        (8, BACnetAuditLogDatum::AuditNotification(notification)),
        (9, BACnetAuditLogDatum::TimeChange(2.0)),
    ] {
        let record = BACnetAuditLogRecord {
            timestamp: (DATE, time(hour)),
            datum,
        };
        encode_audit_log_record(&record, &mut data).unwrap();
    }
    assert_eq!(
        rows(ObjectType::AUDIT_LOG, 2, &data).rows,
        [
            row(
                8,
                "WRITE DEVICE:2 ANALOG_VALUE:4 PRIORITY_ARRAY[8] by DEVICE:1 \
                 failed PROPERTY/WRITE_ACCESS_DENIED",
                None
            ),
            row(9, "time-change 2 s", None),
        ]
    );
}

/// Records decode up to the first one that fails; the rest is kept as hex,
/// so a record of another kind still shows something.
#[test]
fn records_after_an_undecodable_one_fall_back_to_hex() {
    let mut data = BytesMut::new();
    let record = BACnetLogRecord {
        date: DATE,
        time: time(8),
        log_datum: LogDatum::UnsignedValue(7),
        status_flags: None,
    };
    encode_log_record(&record, &mut data).unwrap();
    data.extend_from_slice(&[0x0E, 0xA4]);
    let records = rows(ObjectType::TREND_LOG, 2, &data);
    assert_eq!(records.rows, [row(8, "7", None)]);
    assert_eq!(records.undecoded.as_deref(), Some("0e a4"));

    // A Trend Log record read as a Trend Log Multiple record fails whole.
    let records = rows(ObjectType::TREND_LOG_MULTIPLE, 1, &data[..data.len() - 2]);
    assert!(records.rows.is_empty());
    assert!(records.undecoded.unwrap().starts_with("0e a4 7e 0a 03"));
}
