use super::*;
use crate::clock::{ClockFrame, ClockReader};
use bacnet_encoding::constructed::{decode_log_multiple_record, decode_log_record};
use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::{BACnetLogMultipleRecord, LogData, LogDatum, LogValue};
use bacnet_types::primitives::{Date, Time};
use bytes::BytesMut;
use std::sync::Arc;

/// Each resident record as ReadRange serves it, still encoded.
fn served(object: &dyn BACnetObject) -> Vec<Vec<u8>> {
    let records = object.log_buffer_internal().unwrap();
    (0..records.record_count())
        .map(|index| {
            let mut buf = BytesMut::new();
            records.encode_record(index, &mut buf);
            buf.to_vec()
        })
        .collect()
}

/// A Trend Log's served records, decoded.
fn served_records(tl: &TrendLogObject) -> Vec<BACnetLogRecord> {
    served(tl)
        .iter()
        .map(|bytes| {
            let (record, end) = decode_log_record(bytes, 0).unwrap();
            assert_eq!(end, bytes.len());
            record
        })
        .collect()
}

struct FixedClock;

impl ClockReader for FixedClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(ClockFrame {
            local_date: make_record(9, 0.0).date,
            local_time: make_record(9, 0.0).time,
            utc_offset: 0,
            daylight_savings_status: false,
        })
    }
}

fn bind_clock(object: &mut dyn BACnetObject) {
    object.bind_clock_internal(Some(Arc::new(FixedClock)));
}

fn make_record(hour: u8, value: f32) -> BACnetLogRecord {
    BACnetLogRecord {
        date: Date {
            year: 124,
            month: 3,
            day: 15,
            day_of_week: 5,
        },
        time: Time {
            hour,
            minute: 0,
            second: 0,
            hundredths: 0,
        },
        log_datum: LogDatum::RealValue(value),
        status_flags: None,
    }
}

/// [`make_record`]'s sample as a one-member Trend Log Multiple record.
fn make_multiple(hour: u8, value: f32) -> BACnetLogMultipleRecord {
    let single = make_record(hour, value);
    BACnetLogMultipleRecord {
        date: single.date,
        time: single.time,
        log_data: LogData::Values(vec![LogValue::RealValue(value)]),
    }
}

#[test]
fn trendlog_add_records() {
    let mut tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
    tl.add_record(make_record(10, 72.5)).unwrap();
    tl.add_record(make_record(11, 73.0)).unwrap();
    assert_eq!(tl.records().len(), 2);
    let val = tl
        .read_property(PropertyIdentifier::RECORD_COUNT, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(2));
    let val = tl
        .read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(2));
}

#[test]
fn trendlog_ring_buffer_wraps() {
    let mut tl = TrendLogObject::new(1, "TL-1", 3).unwrap();
    for i in 0..5u8 {
        tl.add_record(BACnetLogRecord {
            date: Date {
                year: 124,
                month: 3,
                day: 15,
                day_of_week: 5,
            },
            time: Time {
                hour: i,
                minute: 0,
                second: 0,
                hundredths: 0,
            },
            log_datum: LogDatum::UnsignedValue(i as u64),
            status_flags: None,
        })
        .unwrap();
    }
    assert_eq!(tl.records().len(), 3);
    // Oldest records should have been evicted; first remaining is hour=2
    assert_eq!(tl.records()[0].time.hour, 2);
    let val = tl
        .read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(5));
}

#[test]
fn trendlog_stop_when_full() {
    let mut tl = TrendLogObject::new(1, "TL-1", 2).unwrap();
    bind_clock(&mut tl);
    tl.write_property(
        PropertyIdentifier::STOP_WHEN_FULL,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    for i in 0..5u8 {
        tl.add_record(make_record(i, i as f32)).unwrap();
    }
    assert_eq!(tl.records().len(), 2);
    assert_eq!(
        tl.read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
            .unwrap(),
        PropertyValue::Unsigned(2)
    ); // Only 2 accepted
}

#[test]
fn trendlog_disable_logging() {
    let mut tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
    bind_clock(&mut tl);
    tl.write_property(
        PropertyIdentifier::LOG_ENABLE,
        None,
        PropertyValue::Boolean(false),
        None,
    )
    .unwrap();
    tl.add_record(make_record(10, 72.5)).unwrap();
    assert_eq!(tl.records().len(), 1);
    assert_eq!(
        tl.records()[0].log_datum,
        LogDatum::LogStatus(LogStatus::LOG_DISABLED)
    );
}

#[test]
fn trendlog_clear_buffer() {
    let mut tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
    bind_clock(&mut tl);
    tl.add_record(make_record(10, 72.5)).unwrap();
    assert_eq!(tl.records().len(), 1);
    tl.write_property(
        PropertyIdentifier::RECORD_COUNT,
        None,
        PropertyValue::Unsigned(0),
        None,
    )
    .unwrap();
    assert_eq!(tl.records().len(), 1);
    assert_eq!(
        tl.records()[0].log_datum,
        LogDatum::LogStatus(LogStatus::BUFFER_PURGED)
    );
}

#[test]
fn trendlog_read_object_type() {
    let tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
    let val = tl
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::Enumerated(ObjectType::TREND_LOG.to_raw())
    );
}

#[test]
fn trendlog_description_read_write() {
    let mut tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
    // Default is empty string
    assert_eq!(
        tl.read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap(),
        PropertyValue::CharacterString(String::new())
    );
    tl.write_property(
        PropertyIdentifier::DESCRIPTION,
        None,
        PropertyValue::CharacterString("Zone temperature trend".into()),
        None,
    )
    .unwrap();
    assert_eq!(
        tl.read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap(),
        PropertyValue::CharacterString("Zone temperature trend".into())
    );
}

#[test]
fn trendlog_set_description_convenience() {
    let mut tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
    tl.set_description("Outdoor air temperature log");
    assert_eq!(
        tl.read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap(),
        PropertyValue::CharacterString("Outdoor air temperature log".into())
    );
}

#[test]
fn trendlog_description_in_property_list() {
    let tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
    assert!(tl
        .property_list()
        .contains(&PropertyIdentifier::DESCRIPTION));
}

#[test]
fn trendlog_serves_log_buffer_records_framed() {
    let mut tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
    tl.add_record(make_record(10, 72.5)).unwrap();
    tl.add_record(make_record(11, 73.0)).unwrap();
    assert_eq!(
        served(&tl)[0],
        vec![
            0x0E, 0xA4, 0x7C, 0x03, 0x0F, 0x05, 0xB4, 0x0A, 0x00, 0x00, 0x00,
            0x0F, // timestamp
            0x1E, 0x2C, 0x42, 0x91, 0x00, 0x00, 0x1F, // real-value [2]: 72.5
        ]
    );
    assert_eq!(
        served_records(&tl),
        vec![make_record(10, 72.5), make_record(11, 73.0)]
    );
}

#[test]
fn trendlog_log_buffer_empty() {
    let tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
    assert_eq!(tl.log_buffer_internal().unwrap().record_count(), 0);
    assert!(matches!(
        tl.read_property(PropertyIdentifier::LOG_BUFFER, None),
        Err(Error::Protocol { class, code })
            if class == ErrorClass::PROPERTY
                && code == ErrorCode::READ_ACCESS_DENIED
    ));
}

#[test]
fn trendlog_log_buffer_overflow_stop_when_full() {
    let mut tl = TrendLogObject::new(1, "TL-1", 3).unwrap();
    bind_clock(&mut tl);
    tl.write_property(
        PropertyIdentifier::STOP_WHEN_FULL,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    for i in 0..5u8 {
        tl.add_record(make_record(i, i as f32 * 10.0)).unwrap();
    }
    // Buffer capped at 3: two samples, then the log-disabled status record.
    let records = served_records(&tl);
    assert_eq!(records.len(), 3);
    assert_eq!(records[0].log_datum, LogDatum::RealValue(0.0));
    assert_eq!(
        records[2].log_datum,
        LogDatum::LogStatus(LogStatus::LOG_DISABLED)
    );
}

#[test]
fn trendlog_read_logging_type() {
    let tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
    let val = tl
        .read_property(PropertyIdentifier::LOGGING_TYPE, None)
        .unwrap();
    // Default is 0 (polled)
    assert_eq!(val, PropertyValue::Enumerated(0));
}

#[test]
fn trendlog_set_logging_type() {
    let mut tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
    tl.set_logging_type(LoggingType::TRIGGERED).unwrap();
    let val = tl
        .read_property(PropertyIdentifier::LOGGING_TYPE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(2));
}

#[test]
fn trendlog_log_buffer_in_property_list() {
    let tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
    let props = tl.property_list();
    assert!(props.contains(&PropertyIdentifier::LOG_BUFFER));
    assert!(props.contains(&PropertyIdentifier::LOGGING_TYPE));
    assert!(props.contains(&PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY));
}

#[test]
fn trendlog_log_device_object_property_unset_by_default() {
    let tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
    let val = tl
        .read_property(PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY, None)
        .unwrap();
    // Analog Input 4194303's Present_Value, the unset form (#1417).
    assert_eq!(
        val,
        PropertyValue::ApplicationData(vec![0x0C, 0x00, 0x3F, 0xFF, 0xFF, 0x19, 0x55])
    );
}

#[test]
fn trendlog_log_buffer_various_datum_types() {
    use bacnet_types::constructed::LogDatum;
    let mut tl = TrendLogObject::new(1, "TL-1", 100).unwrap();

    let date = Date {
        year: 124,
        month: 3,
        day: 15,
        day_of_week: 5,
    };
    let time = Time {
        hour: 8,
        minute: 0,
        second: 0,
        hundredths: 0,
    };

    let records = [
        (LogDatum::BooleanValue(true), None),
        (LogDatum::EnumValue(42), Some(StatusFlags::FAULT)),
        (LogDatum::NullValue, None),
        (LogDatum::AnyValue(vec![0x72, 0x00, b'x']), None),
    ]
    .map(|(log_datum, status_flags)| BACnetLogRecord {
        date,
        time,
        log_datum,
        status_flags,
    });
    for record in &records {
        tl.add_record(record.clone()).unwrap();
    }
    assert_eq!(served_records(&tl), records);
}

// -----------------------------------------------------------------------
// TrendLogMultiple tests
// -----------------------------------------------------------------------

#[test]
fn trendlog_multiple_create() {
    let tlm = TrendLogMultipleObject::new(1, "TLM-1", 200).unwrap();
    assert_eq!(
        tlm.read_property(PropertyIdentifier::OBJECT_NAME, None)
            .unwrap(),
        PropertyValue::CharacterString("TLM-1".into())
    );
    assert_eq!(
        tlm.read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap(),
        PropertyValue::Enumerated(ObjectType::TREND_LOG_MULTIPLE.to_raw())
    );
    assert_eq!(
        tlm.read_property(PropertyIdentifier::BUFFER_SIZE, None)
            .unwrap(),
        PropertyValue::Unsigned(200)
    );
}

#[test]
fn trendlog_multiple_add_records() {
    let mut tlm = TrendLogMultipleObject::new(1, "TLM-1", 100).unwrap();
    tlm.add_record(make_multiple(10, 72.5)).unwrap();
    tlm.add_record(make_multiple(11, 73.0)).unwrap();
    assert_eq!(tlm.records().len(), 2);
    assert_eq!(
        tlm.read_property(PropertyIdentifier::RECORD_COUNT, None)
            .unwrap(),
        PropertyValue::Unsigned(2)
    );
    assert_eq!(
        tlm.read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
            .unwrap(),
        PropertyValue::Unsigned(2)
    );
}

#[test]
fn trendlog_multiple_ring_buffer() {
    let mut tlm = TrendLogMultipleObject::new(1, "TLM-1", 3).unwrap();
    for i in 0..5u8 {
        tlm.add_record(BACnetLogMultipleRecord {
            log_data: LogData::Values(vec![LogValue::UnsignedValue(i as u64)]),
            ..make_multiple(i, 0.0)
        })
        .unwrap();
    }
    assert_eq!(tlm.records().len(), 3);
    assert_eq!(tlm.records()[0].time.hour, 2);
    assert_eq!(
        tlm.read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
            .unwrap(),
        PropertyValue::Unsigned(5)
    );
}

#[test]
fn trendlog_multiple_serves_log_buffer_records_framed() {
    let mut tlm = TrendLogMultipleObject::new(1, "TLM-1", 100).unwrap();
    tlm.add_record(make_multiple(10, 72.5)).unwrap();
    // Each record is framed as Clause 21's BACnetLogMultipleRecord (#1203).
    let served = served(&tlm);
    assert_eq!(served.len(), 1);
    assert_eq!(
        decode_log_multiple_record(&served[0], 0).unwrap(),
        (make_multiple(10, 72.5), served[0].len())
    );
}

#[test]
fn trendlog_multiple_property_list() {
    let tlm = TrendLogMultipleObject::new(1, "TLM-1", 100).unwrap();
    let props = tlm.property_list();
    assert!(props.contains(&PropertyIdentifier::LOG_BUFFER));
    assert!(props.contains(&PropertyIdentifier::LOGGING_TYPE));
    assert!(props.contains(&PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY));
    assert!(!props.contains(&PropertyIdentifier::OUT_OF_SERVICE));
    assert!(props.contains(&PropertyIdentifier::RELIABILITY));
}

#[test]
fn trendlog_multiple_add_property_references() {
    let mut tlm = TrendLogMultipleObject::new(1, "TLM-1", 100).unwrap();

    let oid1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let oid2 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2).unwrap();
    let pv_raw = PropertyIdentifier::PRESENT_VALUE.to_raw();

    tlm.add_property_reference(BACnetDeviceObjectPropertyReference {
        object_identifier: oid1,
        property_identifier: pv_raw,
        property_array_index: None,
        device_identifier: None,
    })
    .unwrap();
    tlm.add_property_reference(BACnetDeviceObjectPropertyReference {
        object_identifier: oid2,
        property_identifier: pv_raw,
        property_array_index: Some(3),
        device_identifier: None,
    })
    .unwrap();

    // One Clause 21 encoding per element (#1234).
    assert_eq!(
        tlm.read_property(PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY, None)
            .unwrap(),
        PropertyValue::List(vec![
            PropertyValue::ApplicationData(vec![0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55]),
            PropertyValue::ApplicationData(vec![
                0x0C, 0x00, 0x00, 0x00, 0x02, 0x19, 0x55, 0x29, 0x03
            ]),
        ])
    );
}

#[test]
fn trendlog_multiple_empty_property_references() {
    let tlm = TrendLogMultipleObject::new(1, "TLM-1", 100).unwrap();
    let val = tlm
        .read_property(PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY, None)
        .unwrap();
    assert_eq!(val, PropertyValue::List(vec![]));
}

#[test]
fn trendlog_multiple_write_log_enable() {
    let mut tlm = TrendLogMultipleObject::new(1, "TLM-1", 100).unwrap();
    bind_clock(&mut tlm);
    tlm.write_property(
        PropertyIdentifier::LOG_ENABLE,
        None,
        PropertyValue::Boolean(false),
        None,
    )
    .unwrap();
    assert_eq!(
        tlm.read_property(PropertyIdentifier::LOG_ENABLE, None)
            .unwrap(),
        PropertyValue::Boolean(false)
    );
    // Records should not be added when disabled
    tlm.add_record(make_multiple(10, 72.5)).unwrap();
    assert_eq!(tlm.records().len(), 1);
    assert_eq!(
        tlm.records()[0].log_data,
        LogData::LogStatus(LogStatus::LOG_DISABLED)
    );
}

// ──────────────────────────────────────────────────────────────────────────
// Reliability writability pins (#240 sweep)
// ──────────────────────────────────────────────────────────────────────────

/// Clause 12.25 Table 12-29 lists Reliability as plain O with no writability
/// footnote. The Trend Log Reliability_Evaluation_Inhibit paragraph requires
/// NO_FAULT_DETECTED while evaluation is inhibited, without the Schedule
/// (Clause 12.24) and intrinsic-reporting exception for a client-supplied
/// Reliability value while Out_Of_Service is TRUE. The log owns the property as a logging
/// status/fault indication, so no-write is the conformant posture; a client
/// write is refused PROPERTY / WRITE_ACCESS_DENIED.
#[test]
fn trendlog_reliability_is_not_network_writable() {
    let mut tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
    assert!(!tl.is_writable_property(PropertyIdentifier::RELIABILITY));

    let result = tl.write_property(
        PropertyIdentifier::RELIABILITY,
        None,
        PropertyValue::Enumerated(1),
        None,
    );
    match result.expect_err("Reliability write must be refused") {
        Error::Protocol { class, code } => {
            assert_eq!(class, ErrorClass::PROPERTY);
            assert_eq!(code, ErrorCode::WRITE_ACCESS_DENIED);
        }
        other => panic!("expected PROPERTY / WRITE_ACCESS_DENIED, got {other:?}"),
    }
    assert_eq!(
        tl.read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap(),
        PropertyValue::Enumerated(0),
        "a refused write must leave Reliability untouched"
    );
}

/// Tables 12-29 and 12-35 define no Out_Of_Service for Trend Log or Trend Log
/// Multiple (#985, as #984 did for Calendar): the row is gone from
/// Property_List and the metadata, and reads and writes find no property.
#[test]
fn trend_logs_have_no_out_of_service_property() {
    let objects: [Box<dyn BACnetObject>; 2] = [
        Box::new(TrendLogObject::new(1, "TL-1", 100).unwrap()),
        Box::new(TrendLogMultipleObject::new(1, "TLM-1", 100).unwrap()),
    ];
    for mut object in objects {
        let kind = object.object_identifier().object_type();
        assert!(
            !object
                .property_list()
                .contains(&PropertyIdentifier::OUT_OF_SERVICE),
            "{kind:?}"
        );
        assert!(!object
            .property_metadata()
            .iter()
            .any(|row| row.property_identifier == PropertyIdentifier::OUT_OF_SERVICE));
        assert!(!object.is_writable_property(PropertyIdentifier::OUT_OF_SERVICE));
        let read = object
            .read_property(PropertyIdentifier::OUT_OF_SERVICE, None)
            .map(|_| ());
        let write = object.write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        );
        for result in [read, write] {
            match result.expect_err("no Out_Of_Service property") {
                Error::Protocol { class, code } => {
                    assert_eq!(class, ErrorClass::PROPERTY, "{kind:?}");
                    assert_eq!(code, ErrorCode::UNKNOWN_PROPERTY, "{kind:?}");
                }
                other => panic!("expected PROPERTY / UNKNOWN_PROPERTY, got {other:?}"),
            }
        }
    }
}

/// Clause 12.30 Table 12-35 lists Reliability as plain O with the same
/// no-provision Reliability_Evaluation_Inhibit text as the Trend Log, so
/// Trend Log Multiple keeps the trait-default denial and this pin documents
/// that the absence of a write arm is deliberate, not an omission.
#[test]
fn trendlog_multiple_reliability_is_not_network_writable() {
    let mut tlm = TrendLogMultipleObject::new(1, "TLM-1", 100).unwrap();
    assert!(!tlm.is_writable_property(PropertyIdentifier::RELIABILITY));

    let result = tlm.write_property(
        PropertyIdentifier::RELIABILITY,
        None,
        PropertyValue::Enumerated(1),
        None,
    );
    match result.expect_err("Reliability write must be refused") {
        Error::Protocol { class, code } => {
            assert_eq!(class, ErrorClass::PROPERTY);
            assert_eq!(code, ErrorCode::WRITE_ACCESS_DENIED);
        }
        other => panic!("expected PROPERTY / WRITE_ACCESS_DENIED, got {other:?}"),
    }
    assert_eq!(
        tlm.read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap(),
        PropertyValue::Enumerated(0),
        "a refused write must leave Reliability untouched"
    );
}
