use super::*;
use bacnet_encoding::constructed::{
    encode_audit_log_record, encode_event_log_record, encode_log_multiple_record, encode_log_record,
};
use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::{BACnetAuditLogDatum, EventLogDatum, LogData, LogDatum, LogValue};
use bacnet_types::primitives::{Date, ObjectIdentifier, StatusFlags, Time};
use bytes::BytesMut;

const DATE: Date = Date {
    year: 126,
    month: 10,
    day: 5,
    day_of_week: 1,
};

fn time(second: u8) -> Time {
    Time {
        hour: 9,
        minute: 30,
        second,
        hundredths: 0,
    }
}

fn trend(value: f32) -> BACnetLogRecord {
    BACnetLogRecord {
        date: DATE,
        time: time(value as u8),
        log_datum: LogDatum::RealValue(value),
        status_flags: Some(StatusFlags::IN_ALARM),
    }
}

fn ack(object_type: ObjectType, item_count: u32, item_data: Vec<u8>) -> ReadRangeAck {
    ReadRangeAck {
        object_identifier: ObjectIdentifier::new(object_type, 1).unwrap(),
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
        result_flags: (true, true, false),
        item_count,
        item_data,
        first_sequence_number: None,
    }
}

fn trend_data(records: &[BACnetLogRecord]) -> Vec<u8> {
    let mut buf = BytesMut::new();
    for record in records {
        encode_log_record(record, &mut buf).unwrap();
    }
    buf.to_vec()
}

#[test]
fn trend_log_records_decode_every_record_in_order() {
    let records = vec![trend(1.0), trend(2.0), trend(3.0)];
    let ack = ack(ObjectType::TREND_LOG, 3, trend_data(&records));
    assert_eq!(ack.trend_log_records().unwrap(), records);
    assert_eq!(
        ack.log_records().unwrap().unwrap(),
        LogRecords::TrendLog(records)
    );
}

#[test]
fn empty_item_data_is_no_records() {
    let ack = ack(ObjectType::TREND_LOG, 0, Vec::new());
    assert!(ack.trend_log_records().unwrap().is_empty());
    assert!(ack.log_records().unwrap().unwrap().is_empty());
}

#[test]
fn a_record_that_fails_names_its_index_and_offset_and_keeps_the_ones_before() {
    let records = vec![trend(1.0), trend(2.0)];
    let mut data = trend_data(&records);
    let second = trend_data(&records[..1]).len();
    // Cut the second record's closing tag off: it no longer decodes.
    data.truncate(data.len() - 1);
    let ack = ack(ObjectType::TREND_LOG, 2, data);
    let error = ack.trend_log_records().unwrap_err();
    assert_eq!(error.index, 1);
    assert_eq!(error.offset, second);
    assert_eq!(error.decoded, records[..1]);
    match Error::from(error) {
        Error::Decoding {
            offset, message, ..
        } => {
            assert_eq!(offset, second);
            assert!(
                message.starts_with("log record 1 at item-data offset"),
                "{message}"
            );
        }
        other => panic!("expected a decoding error, got {other:?}"),
    }
}

#[test]
fn item_count_must_match_the_records_in_the_data() {
    let records = vec![trend(1.0), trend(2.0)];
    let data = trend_data(&records);
    let first_len = trend_data(&records[..1]).len();

    let fewer = ack(ObjectType::TREND_LOG, 3, data.clone())
        .trend_log_records()
        .unwrap_err();
    assert_eq!((fewer.index, fewer.offset), (2, data.len()));
    assert_eq!(fewer.decoded, records);
    assert!(matches!(
        fewer.error,
        Error::Decoding {
            kind: DecodingKind::Missing,
            ..
        }
    ));

    let more = ack(ObjectType::TREND_LOG, 1, data)
        .trend_log_records()
        .unwrap_err();
    assert_eq!((more.index, more.offset), (1, first_len));
    assert_eq!(more.decoded, records[..1]);
    assert!(matches!(
        more.error,
        Error::Decoding {
            kind: DecodingKind::Trailing,
            ..
        }
    ));
}

#[test]
fn a_huge_item_count_does_not_reserve_memory_up_front() {
    let error = ack(ObjectType::TREND_LOG, u32::MAX, Vec::new())
        .trend_log_records()
        .unwrap_err();
    assert_eq!((error.index, error.offset), (0, 0));
}

#[test]
fn event_trend_multiple_and_audit_records_decode_by_object_type() {
    let event = BACnetEventLogRecord {
        date: DATE,
        time: time(1),
        log_datum: EventLogDatum::LogStatus(LogStatus::BUFFER_PURGED),
    };
    let mut buf = BytesMut::new();
    encode_event_log_record(&event, &mut buf).unwrap();
    let event_ack = ack(ObjectType::EVENT_LOG, 1, buf.to_vec());
    assert_eq!(event_ack.event_log_records().unwrap(), vec![event.clone()]);
    assert_eq!(
        event_ack.log_records().unwrap().unwrap(),
        LogRecords::EventLog(vec![event])
    );

    let multiple = BACnetLogMultipleRecord {
        date: DATE,
        time: time(2),
        log_data: LogData::Values(vec![LogValue::RealValue(1.5), LogValue::NullValue]),
    };
    let mut buf = BytesMut::new();
    encode_log_multiple_record(&multiple, &mut buf).unwrap();
    let multiple_ack = ack(ObjectType::TREND_LOG_MULTIPLE, 1, buf.to_vec());
    assert_eq!(
        multiple_ack.trend_log_multiple_records().unwrap(),
        vec![multiple.clone()]
    );
    assert_eq!(
        multiple_ack.log_records().unwrap().unwrap(),
        LogRecords::TrendLogMultiple(vec![multiple])
    );

    let audit = BACnetAuditLogRecord {
        timestamp: (DATE, time(3)),
        datum: BACnetAuditLogDatum::TimeChange(-2.0),
    };
    let mut buf = BytesMut::new();
    encode_audit_log_record(&audit, &mut buf).unwrap();
    encode_audit_log_record(&audit, &mut buf).unwrap();
    let audit_ack = ack(ObjectType::AUDIT_LOG, 2, buf.to_vec());
    assert_eq!(
        audit_ack.audit_log_records().unwrap(),
        vec![audit.clone(), audit.clone()]
    );
    assert_eq!(
        audit_ack.log_records().unwrap().unwrap(),
        LogRecords::AuditLog(vec![audit.clone(), audit])
    );
}

#[test]
fn log_records_is_none_off_a_log_buffer() {
    let data = trend_data(&[trend(1.0)]);
    let mut other_property = ack(ObjectType::TREND_LOG, 1, data.clone());
    other_property.property_identifier = PropertyIdentifier::EVENT_TIME_STAMPS;
    assert!(other_property.log_records().is_none());
    assert!(ack(ObjectType::ANALOG_INPUT, 1, data)
        .log_records()
        .is_none());
}

#[test]
fn a_failure_in_dynamic_decoding_keeps_the_decoded_records_typed() {
    let records = vec![trend(1.0)];
    let mut data = trend_data(&records);
    data.push(0xFF);
    let error = ack(ObjectType::TREND_LOG, 2, data)
        .log_records()
        .unwrap()
        .unwrap_err();
    assert_eq!(error.decoded, LogRecords::TrendLog(records));
    assert_eq!(error.index, 1);
}

#[test]
fn empty_for_names_each_log_kind() {
    assert_eq!(
        LogRecords::empty_for(ObjectType::AUDIT_LOG),
        Some(LogRecords::AuditLog(Vec::new()))
    );
    assert_eq!(LogRecords::empty_for(ObjectType::DEVICE), None);
}
