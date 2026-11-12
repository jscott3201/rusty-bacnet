//! How deep a log record may nest (#2474): every record encoder checks the
//! whole record it wrote, so a body an any-value or an opaque alternative
//! holds can't take the record past what a reader takes back.
//!
//! The check follows the tags only: a record holds as many members as its
//! decoder takes, beyond the item limit a framed list's own check applies.
//!
//! The binding reader is a ReadRange-ACK's: its item-data frame wraps the
//! records, and the tag decoder counts its nesting limit from that frame, so
//! a record may nest one level less deep than the limit. A record one level
//! deeper still decodes alone (from a persisted snapshot, say) but not
//! inside the ACK, and the encoders refuse it.
//!
//! An Audit Log record's binding reader is an AuditLogQuery-ACK's (#2483):
//! its record list and each result's log-record field wrap the record, so
//! it may nest one level less deep again, and its encoder refuses one level
//! more even though a ReadRange-ACK would still carry it.

// The BytesMut encoders and the owned decoders need alloc.
#![cfg(feature = "alloc")]

use bacnet_encoding::constructed::{
    decode_audit_log_record_at, decode_audit_log_record_result_at, decode_event_log_record,
    decode_log_multiple_record, decode_log_record, encode_audit_log_record,
    encode_audit_log_record_result, encode_event_log_record, encode_log_multiple_record,
    encode_log_record, is_record_too_deep,
};
use bacnet_encoding::tags::{
    MAX_CONTEXT_NESTING_DEPTH, decode_tag, encode_closing_tag, encode_opening_tag,
    extract_context_value,
};
use bacnet_types::constructed::{
    BACnetAuditLogDatum, BACnetAuditLogRecord, BACnetAuditLogRecordResult, BACnetAuditNotification,
    BACnetEventLogRecord, BACnetLogMultipleRecord, BACnetLogRecord, BACnetRecipient, EventLogDatum,
    EventNotificationRequest, LogData, LogDatum, LogValue, NotificationParameters, OpaqueChoice,
};
use bacnet_types::enums::{AuditOperation, EventState, EventType, NotifyType, ObjectType};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, Date, ObjectIdentifier, Time};
use bytes::BytesMut;

const DATE: Date = Date {
    year: 126,
    month: 11,
    day: 11,
    day_of_week: 3,
};

const TIME: Time = Time {
    hour: 9,
    minute: 0,
    second: 0,
    hundredths: 0,
};

/// `n` empty constructed values, each inside the last.
fn nested(n: usize) -> Vec<u8> {
    let mut buf = BytesMut::new();
    for _ in 0..n {
        encode_opening_tag(&mut buf, 1);
    }
    for _ in 0..n {
        encode_closing_tag(&mut buf, 1);
    }
    buf.to_vec()
}

/// How many tags are open at once, at most, in `octets`.
fn deepest(octets: &[u8]) -> usize {
    let (mut depth, mut deepest, mut pos) = (0, 0, 0);
    while pos < octets.len() {
        let (tag, next) = decode_tag(octets, pos).unwrap();
        pos = next;
        if tag.is_opening {
            depth += 1;
            deepest = usize::max(deepest, depth);
        } else if tag.is_closing {
            depth -= 1;
        } else {
            pos += tag.length as usize;
        }
    }
    deepest
}

/// Whether a ReadRange-ACK's item-data frame `[5]` takes `record`, as the
/// ACK decoder reads it.
fn in_item_data(record: &[u8]) -> bool {
    let framed = [&[0x5E][..], record, &[0x5F]].concat();
    extract_context_value(&framed, 1, 5).is_ok()
}

fn opaque(tag: u8, body: Vec<u8>) -> OpaqueChoice {
    OpaqueChoice {
        tag,
        constructed: true,
        data: body,
    }
}

fn trend(log_datum: LogDatum) -> BACnetLogRecord {
    BACnetLogRecord {
        date: DATE,
        time: TIME,
        log_datum,
        status_flags: None,
    }
}

fn member(value: LogValue) -> BACnetLogMultipleRecord {
    BACnetLogMultipleRecord {
        date: DATE,
        time: TIME,
        log_data: LogData::Values(vec![LogValue::NullValue, value]),
    }
}

fn event(log_datum: EventLogDatum) -> BACnetEventLogRecord {
    BACnetEventLogRecord {
        date: DATE,
        time: TIME,
        log_datum,
    }
}

/// An alarm whose event values are `[30]` around `body`.
fn alarm(body: Vec<u8>) -> EventNotificationRequest {
    EventNotificationRequest {
        process_identifier: 1,
        initiating_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap(),
        event_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        timestamp: BACnetTimeStamp::SequenceNumber(1),
        notification_class: 0,
        priority: 1,
        event_type: EventType::from_raw(30),
        message_text: None,
        notify_type: NotifyType::ALARM,
        ack_required: false,
        from_state: EventState::NORMAL,
        to_state: EventState::OFFNORMAL,
        event_values: Some(NotificationParameters::Opaque(opaque(30, body))),
    }
}

type Encode<R> = fn(&R, &mut BytesMut) -> Result<(), Error>;
type Decode<R> = fn(&[u8], usize) -> Result<(R, usize), Error>;

/// A record kind: its name, the levels the record's own tags take above
/// the body, and a record holding a body.
struct Case<R> {
    name: &'static str,
    levels: usize,
    record: fn(Vec<u8>) -> R,
}

/// The body as deep as `case` leaves room for encodes, reads back alone
/// and inside the ACK's item data; one level deeper is refused, leaving the
/// buffer as it was, though the record it would be still decodes alone.
fn check<R: PartialEq + std::fmt::Debug>(case: &Case<R>, encode: Encode<R>, decode: Decode<R>) {
    let room = MAX_CONTEXT_NESTING_DEPTH - 1 - case.levels;
    let record = (case.record)(nested(room));
    let mut octets = BytesMut::new();
    encode(&record, &mut octets).unwrap_or_else(|e| panic!("{}: {e:?}", case.name));
    assert_eq!(
        deepest(&octets),
        MAX_CONTEXT_NESTING_DEPTH - 1,
        "{}",
        case.name
    );
    assert_eq!(
        decode(&octets, 0).unwrap(),
        (record, octets.len()),
        "{}",
        case.name
    );
    assert!(in_item_data(&octets), "{}", case.name);

    let record = (case.record)(nested(room + 1));
    let mut buf = BytesMut::from(&b"kept"[..]);
    assert!(encode(&record, &mut buf).is_err(), "{}", case.name);
    assert_eq!(&buf[..], b"kept", "{}", case.name);
    // What the encoder would have written decodes alone, but not in the
    // item data, which is why it's refused.
    let deeper = deepen(&octets);
    assert_eq!(decode(&deeper, 0).unwrap().0, record, "{}", case.name);
    assert!(!in_item_data(&deeper), "{}", case.name);
}

/// `octets` with an empty `[1]` inside its innermost constructed value:
/// one level deeper, everything else as it was.
fn deepen(octets: &[u8]) -> Vec<u8> {
    let innermost = deepest(octets);
    let (mut depth, mut pos) = (0, 0);
    loop {
        let (tag, next) = decode_tag(octets, pos).unwrap();
        if tag.is_closing && depth == innermost {
            return [&octets[..pos], &[0x1E, 0x1F], &octets[pos..]].concat();
        }
        if tag.is_opening {
            depth += 1;
            pos = next;
        } else if tag.is_closing {
            depth -= 1;
            pos = next;
        } else {
            pos = next + tag.length as usize;
        }
    }
}

#[test]
fn a_record_nests_as_deep_as_a_readrange_ack_carries_and_no_deeper() {
    // A Trend Log datum: field [1], then the any-value [10] or opaque [11].
    for case in [
        Case {
            name: "trend any-value",
            levels: 2,
            record: |body| trend(LogDatum::AnyValue(body)),
        },
        Case {
            name: "trend opaque",
            levels: 2,
            record: |body| trend(LogDatum::Opaque(opaque(11, body))),
        },
    ] {
        check(&case, encode_log_record, decode_log_record);
    }
    // A Trend Log Multiple member: field [1], log-data [1], then the
    // any-value [8] or opaque [9].
    for case in [
        Case {
            name: "member any-value",
            levels: 3,
            record: |body| member(LogValue::AnyValue(body)),
        },
        Case {
            name: "member opaque",
            levels: 3,
            record: |body| member(LogValue::Opaque(opaque(9, body))),
        },
    ] {
        check(
            &case,
            encode_log_multiple_record,
            decode_log_multiple_record,
        );
    }
    // An Event Log datum: field [1], then the opaque [3]; or a logged
    // notification, [1], its event values [12] and their choice [30].
    for case in [
        Case {
            name: "event opaque",
            levels: 2,
            record: |body| event(EventLogDatum::Opaque(opaque(3, body))),
        },
        Case {
            name: "event notification",
            levels: 4,
            record: |body| event(EventLogDatum::Notification(alarm(body))),
        },
    ] {
        check(&case, encode_event_log_record, decode_event_log_record);
    }
}

/// The whole-record check counts depth, not items: a Trend Log Multiple
/// record with more members than a framed list's item limit (10,000)
/// encodes and reads back, as its decoder takes it.
#[test]
fn a_record_with_more_members_than_a_framed_list_holds_still_encodes() {
    let record = BACnetLogMultipleRecord {
        date: DATE,
        time: TIME,
        log_data: LogData::Values(vec![LogValue::NullValue; 10_001]),
    };
    let mut octets = BytesMut::new();
    encode_log_multiple_record(&record, &mut octets).unwrap();
    assert_eq!(
        decode_log_multiple_record(&octets, 0).unwrap(),
        (record, octets.len())
    );
}

fn audit(datum: BACnetAuditLogDatum) -> BACnetAuditLogRecord {
    BACnetAuditLogRecord {
        timestamp: (DATE, TIME),
        datum,
    }
}

/// A write to analog-value:1 whose written value is `target` and whose
/// held value is `current`.
fn write(target: Option<Vec<u8>>, current: Option<Vec<u8>>) -> BACnetAuditNotification {
    let device = BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap());
    BACnetAuditNotification {
        source_timestamp: None,
        target_timestamp: None,
        source_device: device.clone(),
        source_object: None,
        operation: AuditOperation::WRITE,
        source_comment: None,
        target_comment: None,
        invoke_id: None,
        source_user_id: None,
        source_user_role: None,
        target_device: device,
        target_object: Some(ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap()),
        target_property: None,
        target_priority: None,
        target_value: target,
        current_value: current,
        result: None,
    }
}

/// Whether an AuditLogQuery-ACK takes `record` as its one result, read as
/// its decoder reads it: the record list `[1]` first, then the result's
/// sequence number `[0]` and log-record field `[1]` inside it.
fn in_audit_query_ack(record: &BACnetAuditLogRecord) -> bool {
    let mut result = BytesMut::new();
    encode_audit_log_record_result(
        &BACnetAuditLogRecordResult {
            sequence_number: 1,
            record: record.clone(),
        },
        &mut result,
    )
    .unwrap();
    in_record_list(&result)
}

/// Whether the AuditLogQuery-ACK record list `[1]` takes `results`.
fn in_record_list(results: &[u8]) -> bool {
    let framed = [&[0x1E][..], results, &[0x1F]].concat();
    extract_context_value(&framed, 1, 1)
        .and_then(|(list, _)| decode_audit_log_record_result_at(list, 0))
        .is_ok()
}

/// The body as deep as an Audit Log record leaves room for encodes, reads
/// back alone, in an AuditLogQuery-ACK and in ReadRange item data; one
/// level deeper is refused as too deep, leaving the buffer as it was,
/// though that record still decodes alone and in the item data, just not in
/// the query's ACK.
fn check_audit(case: &Case<BACnetAuditLogRecord>) {
    let room = MAX_CONTEXT_NESTING_DEPTH - 2 - case.levels;
    let record = (case.record)(nested(room));
    let mut octets = BytesMut::new();
    encode_audit_log_record(&record, &mut octets)
        .unwrap_or_else(|e| panic!("{}: {e:?}", case.name));
    assert_eq!(
        deepest(&octets),
        MAX_CONTEXT_NESTING_DEPTH - 2,
        "{}",
        case.name
    );
    assert_eq!(
        decode_audit_log_record_at(&octets, 0).unwrap(),
        (record.clone(), octets.len()),
        "{}",
        case.name
    );
    assert!(in_audit_query_ack(&record), "{}", case.name);
    assert!(in_item_data(&octets), "{}", case.name);

    let record = (case.record)(nested(room + 1));
    let mut buf = BytesMut::from(&b"kept"[..]);
    let error = encode_audit_log_record(&record, &mut buf).unwrap_err();
    assert!(is_record_too_deep(&error), "{}: {error:?}", case.name);
    assert_eq!(&buf[..], b"kept", "{}", case.name);
    // Nor does the query result around it leave anything behind.
    let result = BACnetAuditLogRecordResult {
        sequence_number: 1,
        record: record.clone(),
    };
    assert!(encode_audit_log_record_result(&result, &mut buf).is_err());
    assert_eq!(&buf[..], b"kept", "{}", case.name);
    let deeper = deepen(&octets);
    assert_eq!(
        decode_audit_log_record_at(&deeper, 0).unwrap().0,
        record,
        "{}",
        case.name
    );
    assert!(in_item_data(&deeper), "{}", case.name);
    let mut deeper_result = BytesMut::new();
    result_around(&mut deeper_result, &deeper);
    assert!(!in_record_list(&deeper_result), "{}", case.name);
}

/// A result numbered 1 around record octets the encoder didn't write.
fn result_around(buf: &mut BytesMut, record: &[u8]) {
    buf.extend_from_slice(&[0x09, 0x01]);
    encode_opening_tag(buf, 1);
    buf.extend_from_slice(record);
    encode_closing_tag(buf, 1);
}

#[test]
fn an_audit_record_nests_as_deep_as_an_audit_log_query_ack_carries_and_no_deeper() {
    // The datum field [1], then the opaque [3]; or the notification [1] and
    // its target-value [14] or current-value [15].
    for case in [
        Case {
            name: "audit opaque",
            levels: 2,
            record: |body| audit(BACnetAuditLogDatum::Opaque(opaque(3, body))),
        },
        Case {
            name: "audit target value",
            levels: 3,
            record: |body| {
                audit(BACnetAuditLogDatum::AuditNotification(write(
                    Some(body),
                    None,
                )))
            },
        },
        Case {
            name: "audit current value",
            levels: 3,
            record: |body| {
                audit(BACnetAuditLogDatum::AuditNotification(write(
                    None,
                    Some(body),
                )))
            },
        },
    ] {
        check_audit(&case);
    }
}
