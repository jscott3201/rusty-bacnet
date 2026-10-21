//! Trend Log and Event Log records as ReadRange serves them (#1233): each
//! item one record framed as Clause 21's BACnetLogRecord or
//! BACnetEventLogRecord, whatever the range selects.

use super::*;
use bacnet_encoding::constructed::{
    decode_event_log_record, decode_log_record, encode_event_notification,
};
use bacnet_services::alarm_event::EventNotificationRequest;
use bacnet_types::bitstring::LogStatus;
use bacnet_types::enums::{EventState, EventType, NotifyType};
use bacnet_types::primitives::{BACnetTimeStamp, StatusFlags};

/// The timestamp field of a record logged at `time(hour)`.
fn timestamp(hour: u8) -> Vec<u8> {
    vec![0x0E, 0xA4, 126, 8, 31, 1, 0xB4, hour, 2, 3, 4, 0x0F]
}

/// Every item of an ACK, decoded back to back with `decode`.
fn decoded<R>(
    ack: &ReadRangeAck,
    decode: impl Fn(&[u8], usize) -> Result<(R, usize), Error>,
) -> Vec<R> {
    let mut offset = 0;
    let mut records = Vec::new();
    while offset < ack.item_data.len() {
        let (record, next) = decode(&ack.item_data, offset).unwrap();
        records.push(record);
        offset = next;
    }
    records
}

/// Reads of `oid`'s Log_Buffer by position, by sequence number and by time,
/// each with the records it must return (zero-based) and the first sequence
/// number its ACK must carry.
fn assert_every_range(db: &ObjectDatabase, oid: ObjectIdentifier, items: &[Vec<u8>]) {
    for (range, selected, first_sequence_number) in [
        (
            RangeSpec::ByPosition {
                reference_index: 1,
                count: 5,
            },
            0..items.len(),
            None,
        ),
        (
            RangeSpec::BySequenceNumber {
                reference_seq: 2,
                count: 2,
            },
            1..3,
            Some(2),
        ),
        // The records stamped before time(3): hours 1 and 2.
        (
            RangeSpec::ByTime {
                reference_time: (DATE, time(3)),
                count: -2,
            },
            0..2,
            Some(1),
        ),
    ] {
        let ack = call(db, oid, PropertyIdentifier::LOG_BUFFER, Some(range.clone())).unwrap();
        assert_eq!(ack.item_count as usize, selected.len(), "{range:?}");
        assert_eq!(ack.item_data, items[selected].concat(), "{range:?}");
        assert_eq!(ack.first_sequence_number, first_sequence_number);
    }
}

#[test]
fn trend_log_read_range_serves_every_record_kind_with_exact_bytes() {
    let kinds = [
        // real-value [2], then status-flags [2] with OVERRIDDEN (bit 2) set.
        (
            LogDatum::RealValue(72.5),
            Some(StatusFlags::OVERRIDDEN),
            vec![0x1E, 0x2C, 0x42, 0x91, 0x00, 0x00, 0x1F, 0x2A, 0x04, 0x20],
        ),
        // failure [8]: PROPERTY / UNKNOWN_PROPERTY.
        (
            LogDatum::Failure {
                error_class: ErrorClass::PROPERTY,
                error_code: ErrorCode::UNKNOWN_PROPERTY,
            },
            None,
            vec![0x1E, 0x8E, 0x91, 0x02, 0x91, 0x20, 0x8F, 0x1F],
        ),
        // log-status [0]: log-disabled, bit 0, in the top bit.
        (
            LogDatum::LogStatus(LogStatus::LOG_DISABLED),
            None,
            vec![0x1E, 0x0A, 0x05, 0x80, 0x1F],
        ),
        // time-change [9].
        (
            LogDatum::TimeChange(-1.5),
            None,
            vec![0x1E, 0x9C, 0xBF, 0xC0, 0x00, 0x00, 0x1F],
        ),
        // any-value [10] around an application CharacterString "Hi".
        (
            LogDatum::AnyValue(vec![0x73, 0x00, b'H', b'i']),
            None,
            vec![0x1E, 0xAE, 0x73, 0x00, b'H', b'i', 0xAF, 0x1F],
        ),
    ];
    let mut object = TrendLogObject::new(1, "TL-1", 8).unwrap();
    let mut records = Vec::new();
    let mut items = Vec::new();
    for (hour, (log_datum, status_flags, tail)) in (1..).zip(kinds) {
        let record = BACnetLogRecord {
            date: DATE,
            time: time(hour),
            log_datum,
            status_flags,
        };
        object.add_record(record.clone()).unwrap();
        records.push(record);
        items.push([timestamp(hour), tail].concat());
    }
    let oid = object.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(object)).unwrap();

    assert_every_range(&db, oid, &items);
    let whole = call(&db, oid, PropertyIdentifier::LOG_BUFFER, None).unwrap();
    assert_eq!(decoded(&whole, decode_log_record), records);
}

/// A notification as a server sends it, the body an Event Log record holds.
fn notification() -> EventNotificationRequest {
    EventNotificationRequest {
        process_identifier: 1,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap(),
        event_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        timestamp: BACnetTimeStamp::SequenceNumber(5),
        notification_class: 0,
        priority: 100,
        event_type: EventType::CHANGE_OF_STATE,
        message_text: None,
        notify_type: NotifyType::ALARM,
        ack_required: false,
        from_state: EventState::NORMAL,
        to_state: EventState::OFFNORMAL,
        event_values: None,
    }
}

#[test]
fn event_log_read_range_serves_every_record_kind_with_exact_bytes() {
    // The notification's own fields, process identifier through to-state.
    let parameters = vec![
        0x09, 0x01, 0x1C, 0x02, 0x00, 0x00, 0x01, 0x2C, 0x00, 0x00, 0x00, 0x01, 0x3E, 0x19, 0x05,
        0x3F, 0x49, 0x00, 0x59, 0x64, 0x69, 0x01, 0x89, 0x00, 0x99, 0x00, 0xA9, 0x00, 0xB9, 0x02,
    ];
    let mut encoded = BytesMut::new();
    encode_event_notification(&notification(), &mut encoded).unwrap();
    assert_eq!(encoded.as_ref(), &parameters[..]);

    let kinds = [
        // log-status [0]: log-disabled, bit 0, in the top bit.
        (
            EventLogDatum::LogStatus(LogStatus::LOG_DISABLED),
            vec![0x1E, 0x0A, 0x05, 0x80, 0x1F],
        ),
        // notification [1] around the request's fields.
        (
            EventLogDatum::Notification(notification()),
            [&[0x1E, 0x1E][..], &parameters, &[0x1F, 0x1F]].concat(),
        ),
        // time-change [2], amount unknown.
        (
            EventLogDatum::TimeChange(0.0),
            vec![0x1E, 0x2C, 0x00, 0x00, 0x00, 0x00, 0x1F],
        ),
    ];
    let mut object = EventLogObject::new(1, "EL-1", 8).unwrap();
    let mut records = Vec::new();
    let mut items = Vec::new();
    for (hour, (log_datum, tail)) in (1..).zip(kinds) {
        let record = BACnetEventLogRecord {
            date: DATE,
            time: time(hour),
            log_datum,
        };
        object.add_record(record.clone()).unwrap();
        records.push(record);
        items.push([timestamp(hour), tail].concat());
    }
    let oid = object.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(object)).unwrap();

    assert_every_range(&db, oid, &items);
    let whole = call(&db, oid, PropertyIdentifier::LOG_BUFFER, None).unwrap();
    let served = decoded(&whole, decode_event_log_record);
    assert_eq!(served, records);
    assert_eq!(
        served[1].log_datum,
        EventLogDatum::Notification(notification())
    );
}

/// One ReadRange of a Trend Log's buffer under `bytes` of service ACK.
fn budgeted(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    range: RangeSpec,
    bytes: usize,
) -> Result<ReadRangeAck, ReadRangeFailure> {
    let mut request = BytesMut::new();
    ReadRangeRequest {
        object_identifier: oid,
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
        range: Some(range),
    }
    .encode(&mut request)
    .unwrap();
    let mut response = BytesMut::new();
    handle_read_range_budgeted(
        db,
        &request,
        &mut response,
        crate::server::ReadRangeBudget {
            max_returned_items: 256,
            max_service_ack_bytes: bytes,
        },
    )?;
    assert!(response.len() <= bytes);
    Ok(ReadRangeAck::decode(&response).unwrap())
}

/// A Trend Log's records page under the byte budget like any list: a page
/// stops at the last whole record that fits and sets MORE_ITEMS, a backward
/// page keeps the newest records, and a budget smaller than one record
/// fails instead of splitting it.
#[test]
fn trend_log_read_range_pages_records_under_the_byte_budget() {
    let mut object = TrendLogObject::new(1, "TL-1", 8).unwrap();
    for value in 1..=5 {
        object.add_record(record(value)).unwrap();
    }
    let oid = object.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(object)).unwrap();
    let items: Vec<Vec<u8>> = (1..=5)
        .map(|value| {
            let PropertyValue::ApplicationData(bytes) = projected(LogFamily::Trend, value) else {
                unreachable!()
            };
            bytes
        })
        .collect();

    // The exact size of an ACK carrying the first two records.
    let two = budgeted(
        &db,
        oid,
        RangeSpec::ByPosition {
            reference_index: 1,
            count: 2,
        },
        usize::MAX,
    )
    .unwrap();
    let mut two_bytes = BytesMut::new();
    two.encode(&mut two_bytes);
    let cap = two_bytes.len();

    let forward = budgeted(
        &db,
        oid,
        RangeSpec::ByPosition {
            reference_index: 1,
            count: 5,
        },
        cap,
    )
    .unwrap();
    assert_eq!(forward.item_count, 2);
    assert_eq!(forward.item_data, items[..2].concat());
    assert_eq!(forward.result_flags, (true, false, true));
    assert_eq!(forward.first_sequence_number, None);

    // A By Sequence ACK also carries First Sequence Number, so its two-record
    // size differs.
    let mut newest_two = BytesMut::new();
    budgeted(
        &db,
        oid,
        RangeSpec::BySequenceNumber {
            reference_seq: 5,
            count: -2,
        },
        usize::MAX,
    )
    .unwrap()
    .encode(&mut newest_two);
    let backward = budgeted(
        &db,
        oid,
        RangeSpec::BySequenceNumber {
            reference_seq: 5,
            count: -5,
        },
        newest_two.len(),
    )
    .unwrap();
    assert_eq!(backward.item_count, 2);
    assert_eq!(backward.item_data, items[3..].concat());
    assert_eq!(backward.result_flags, (false, true, true));
    assert_eq!(backward.first_sequence_number, Some(4));

    assert!(matches!(
        budgeted(
            &db,
            oid,
            RangeSpec::ByPosition {
                reference_index: 1,
                count: 1,
            },
            cap - items[0].len() - items[1].len() + 1,
        ),
        Err(ReadRangeFailure::Bytes)
    ));
}

/// A record refused at admission never reaches the buffer, so every
/// ReadRange window over the log still answers.
#[test]
fn read_range_serves_the_records_left_after_a_refused_add() {
    let mut object = TrendLogObject::new(1, "TL-1", 8).unwrap();
    object.add_record(record(1)).unwrap();
    let mut bad = record(2);
    bad.log_datum = LogDatum::AnyValue(vec![0x21, 0x01, 0x0F]);
    assert!(object.add_record(bad).is_err());
    object.add_record(record(3)).unwrap();
    let oid = object.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(object)).unwrap();

    let ack = call(&db, oid, PropertyIdentifier::LOG_BUFFER, None).unwrap();
    assert_eq!(decoded(&ack, decode_log_record), vec![record(1), record(3)]);
    let by_sequence = call(
        &db,
        oid,
        PropertyIdentifier::LOG_BUFFER,
        Some(RangeSpec::BySequenceNumber {
            reference_seq: 2,
            count: 1,
        }),
    )
    .unwrap();
    assert_eq!(decoded(&by_sequence, decode_log_record), vec![record(3)]);
}
