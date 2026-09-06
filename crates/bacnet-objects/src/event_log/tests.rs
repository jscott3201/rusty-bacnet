use super::*;
use crate::clock::{ClockFrame, ClockReader};
use bacnet_types::constructed::LogDatum;
use bacnet_types::primitives::{Date, Time};
use std::sync::Arc;

struct FixedClock;

impl ClockReader for FixedClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(ClockFrame {
            local_date: make_date(),
            local_time: make_time(9),
            utc_offset: 0,
            daylight_savings_status: false,
        })
    }
}

fn bind_clock(object: &mut EventLogObject) {
    object.bind_clock_internal(Some(Arc::new(FixedClock)));
}

fn make_date() -> Date {
    Date {
        year: 124,
        month: 3,
        day: 15,
        day_of_week: 5,
    }
}

fn make_time(hour: u8) -> Time {
    Time {
        hour,
        minute: 0,
        second: 0,
        hundredths: 0,
    }
}

fn make_record(hour: u8, value: f32) -> BACnetLogRecord {
    BACnetLogRecord {
        date: make_date(),
        time: make_time(hour),
        log_datum: LogDatum::RealValue(value),
        status_flags: None,
    }
}

#[test]
fn create_event_log() {
    let el = EventLogObject::new(1, "EL-1", 100).unwrap();
    assert_eq!(el.object_identifier().object_type(), ObjectType::EVENT_LOG);
    assert_eq!(el.object_identifier().instance_number(), 1);
    assert_eq!(el.object_name(), "EL-1");
}

#[test]
fn read_object_type() {
    let el = EventLogObject::new(1, "EL-1", 100).unwrap();
    let val = el
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::Enumerated(ObjectType::EVENT_LOG.to_raw())
    );
}

#[test]
fn add_records_and_read_count() {
    let mut el = EventLogObject::new(1, "EL-1", 100).unwrap();
    el.add_record(make_record(10, 72.5));
    el.add_record(make_record(11, 73.0));
    assert_eq!(el.records().len(), 2);
    let val = el
        .read_property(PropertyIdentifier::RECORD_COUNT, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(2));
    let val = el
        .read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(2));
}

#[test]
fn read_log_buffer() {
    let mut el = EventLogObject::new(1, "EL-1", 100).unwrap();
    el.add_record(make_record(10, 72.5));
    el.add_record(make_record(11, 73.0));
    let val = el
        .read_property(PropertyIdentifier::LOG_BUFFER, None)
        .unwrap();
    if let PropertyValue::List(records) = val {
        assert_eq!(records.len(), 2);
        if let PropertyValue::List(fields) = &records[0] {
            assert_eq!(fields.len(), 3);
            assert_eq!(fields[2], PropertyValue::Real(72.5));
        } else {
            panic!("Expected List for log record");
        }
        if let PropertyValue::List(fields) = &records[1] {
            assert_eq!(fields[2], PropertyValue::Real(73.0));
        } else {
            panic!("Expected List for log record");
        }
    } else {
        panic!("Expected List for LOG_BUFFER");
    }
}

#[test]
fn read_log_buffer_empty() {
    let el = EventLogObject::new(1, "EL-1", 100).unwrap();
    let val = el
        .read_property(PropertyIdentifier::LOG_BUFFER, None)
        .unwrap();
    assert_eq!(val, PropertyValue::List(vec![]));
}

#[test]
fn ring_buffer_wraps() {
    let mut el = EventLogObject::new(1, "EL-1", 3).unwrap();
    for i in 0..5u8 {
        el.add_record(BACnetLogRecord {
            date: make_date(),
            time: make_time(i),
            log_datum: LogDatum::UnsignedValue(i as u64),
            status_flags: None,
        });
    }
    assert_eq!(el.records().len(), 3);
    // Oldest records evicted; first remaining is hour=2
    assert_eq!(el.records()[0].time.hour, 2);
    let val = el
        .read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(5));
}

#[test]
fn stop_when_full() {
    let mut el = EventLogObject::new(1, "EL-1", 2).unwrap();
    bind_clock(&mut el);
    el.write_property(
        PropertyIdentifier::STOP_WHEN_FULL,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    for i in 0..5u8 {
        el.add_record(make_record(i, i as f32));
    }
    assert_eq!(el.records().len(), 2);
    assert_eq!(
        el.read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
            .unwrap(),
        PropertyValue::Unsigned(2)
    ); // Only 2 accepted
}

#[test]
fn disable_logging() {
    let mut el = EventLogObject::new(1, "EL-1", 100).unwrap();
    bind_clock(&mut el);
    el.write_property(
        PropertyIdentifier::LOG_ENABLE,
        None,
        PropertyValue::Boolean(false),
        None,
    )
    .unwrap();
    el.add_record(make_record(10, 72.5));
    assert_eq!(el.records().len(), 1);
    assert_eq!(el.records()[0].log_datum, LogDatum::LogStatus(0b001));
}

#[test]
fn clear_buffer_via_record_count() {
    let mut el = EventLogObject::new(1, "EL-1", 100).unwrap();
    bind_clock(&mut el);
    el.add_record(make_record(10, 72.5));
    assert_eq!(el.records().len(), 1);
    el.write_property(
        PropertyIdentifier::RECORD_COUNT,
        None,
        PropertyValue::Unsigned(0),
        None,
    )
    .unwrap();
    assert_eq!(el.records().len(), 1);
    assert_eq!(el.records()[0].log_datum, LogDatum::LogStatus(0b010));
}

#[test]
fn read_event_state_default() {
    let el = EventLogObject::new(1, "EL-1", 100).unwrap();
    let val = el
        .read_property(PropertyIdentifier::EVENT_STATE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(0)); // normal
}

#[test]
fn property_list_complete() {
    let el = EventLogObject::new(1, "EL-1", 100).unwrap();
    let props = el.property_list();
    assert!(props.contains(&PropertyIdentifier::LOG_ENABLE));
    assert!(props.contains(&PropertyIdentifier::LOG_INTERVAL));
    assert!(props.contains(&PropertyIdentifier::STOP_WHEN_FULL));
    assert!(props.contains(&PropertyIdentifier::BUFFER_SIZE));
    assert!(props.contains(&PropertyIdentifier::LOG_BUFFER));
    assert!(props.contains(&PropertyIdentifier::RECORD_COUNT));
    assert!(props.contains(&PropertyIdentifier::TOTAL_RECORD_COUNT));
    assert!(props.contains(&PropertyIdentifier::STATUS_FLAGS));
    assert!(props.contains(&PropertyIdentifier::EVENT_STATE));
    assert!(props.contains(&PropertyIdentifier::OUT_OF_SERVICE));
    assert!(props.contains(&PropertyIdentifier::RELIABILITY));
}

#[test]
fn write_log_interval() {
    let mut el = EventLogObject::new(1, "EL-1", 100).unwrap();
    el.write_property(
        PropertyIdentifier::LOG_INTERVAL,
        None,
        PropertyValue::Unsigned(60),
        None,
    )
    .unwrap();
    let val = el
        .read_property(PropertyIdentifier::LOG_INTERVAL, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(60));
}

#[test]
fn write_unknown_property_denied() {
    let mut el = EventLogObject::new(1, "EL-1", 100).unwrap();
    let result = el.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(1.0),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn log_buffer_various_datum_types() {
    let mut el = EventLogObject::new(1, "EL-1", 100).unwrap();
    let date = make_date();
    let time = make_time(8);

    el.add_record(BACnetLogRecord {
        date,
        time,
        log_datum: LogDatum::BooleanValue(true),
        status_flags: None,
    });
    el.add_record(BACnetLogRecord {
        date,
        time,
        log_datum: LogDatum::EnumValue(42),
        status_flags: Some(0b0100),
    });
    el.add_record(BACnetLogRecord {
        date,
        time,
        log_datum: LogDatum::NullValue,
        status_flags: None,
    });

    let val = el
        .read_property(PropertyIdentifier::LOG_BUFFER, None)
        .unwrap();
    if let PropertyValue::List(records) = val {
        assert_eq!(records.len(), 3);
        if let PropertyValue::List(fields) = &records[0] {
            assert_eq!(fields[2], PropertyValue::Boolean(true));
        } else {
            panic!("Expected List");
        }
        if let PropertyValue::List(fields) = &records[1] {
            assert_eq!(fields[2], PropertyValue::Enumerated(42));
        } else {
            panic!("Expected List");
        }
        if let PropertyValue::List(fields) = &records[2] {
            assert_eq!(fields[2], PropertyValue::Null);
        } else {
            panic!("Expected List");
        }
    } else {
        panic!("Expected List for LOG_BUFFER");
    }
}

#[test]
fn event_log_identities_align_after_eviction_and_differ_from_position() {
    let mut el = EventLogObject::new(1, "EL-1", 2).unwrap();
    for hour in 1..=3 {
        el.add_record(make_record(hour, hour as f32));
    }

    let identities = el.log_record_identities_internal().unwrap();
    let projected = match el
        .read_property(PropertyIdentifier::LOG_BUFFER, None)
        .unwrap()
    {
        PropertyValue::List(records) => records,
        other => panic!("expected projected log list, got {other:?}"),
    };
    assert_eq!(identities.len(), el.records().len());
    assert_eq!(identities.len(), projected.len());
    assert_eq!(identities[0].sequence_number(), 2);
    assert_ne!(identities[0].sequence_number(), 1);
    for ((identity, raw), wire) in identities.iter().zip(el.records()).zip(projected) {
        assert_eq!(identity.date(), raw.date);
        assert_eq!(identity.time(), raw.time);
        assert_eq!(
            wire,
            PropertyValue::List(vec![
                PropertyValue::Date(raw.date),
                PropertyValue::Time(raw.time),
                PropertyValue::Real(raw.time.hour as f32),
            ])
        );
    }
}

#[test]
fn event_log_clear_preserves_total_and_next_identity() {
    let mut el = EventLogObject::new(1, "EL-1", 2).unwrap();
    assert_eq!(
        el.read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
            .unwrap(),
        PropertyValue::Unsigned(0)
    );
    el.add_record(make_record(1, 1.0));
    el.add_record(make_record(1, 2.0));
    assert_eq!(
        el.log_record_identities_internal()
            .unwrap()
            .iter()
            .map(|identity| identity.sequence_number())
            .collect::<Vec<_>>(),
        vec![1, 2]
    );

    el.clear();
    assert!(el.log_record_identities_internal().unwrap().is_empty());
    assert_eq!(
        el.read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
            .unwrap(),
        PropertyValue::Unsigned(2)
    );
    el.add_record(make_record(1, 3.0));
    assert_eq!(
        el.log_record_identities_internal().unwrap()[0].sequence_number(),
        3
    );
}

#[test]
fn event_log_disabled_ordinary_rejection_does_not_consume_identity() {
    let mut el = EventLogObject::new(1, "EL-1", 1).unwrap();
    bind_clock(&mut el);
    el.write_property(
        PropertyIdentifier::LOG_ENABLE,
        None,
        PropertyValue::Boolean(false),
        None,
    )
    .unwrap();
    let before = el.log_record_identities_internal().unwrap();
    el.add_record(make_record(1, 1.0));

    assert_eq!(el.log_record_identities_internal().unwrap(), before);
    assert_eq!(
        el.read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
            .unwrap(),
        PropertyValue::Unsigned(1)
    );
}

#[test]
fn event_log_total_record_count_is_u32_and_wraps_max_to_one() {
    let mut el = EventLogObject::new(1, "EL-1", 1).unwrap();
    el.log_buffer.set_total_record_count_for_test(u32::MAX);
    assert_eq!(
        el.read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
            .unwrap(),
        PropertyValue::Unsigned(u32::MAX as u64)
    );
    el.add_record(make_record(1, 1.0));
    assert_eq!(
        el.read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
            .unwrap(),
        PropertyValue::Unsigned(1)
    );
    assert_eq!(
        el.log_record_identities_internal().unwrap()[0].sequence_number(),
        1
    );
}

#[test]
fn event_log_retains_raw_flags_but_projects_no_status_or_sequence() {
    let mut el = EventLogObject::new(1, "EL-1", 1).unwrap();
    let mut record = make_record(1, 42.0);
    record.status_flags = Some(0b0100);
    el.add_record(record);

    assert_eq!(el.records()[0].status_flags, Some(0b0100));
    let PropertyValue::List(records) = el
        .read_property(PropertyIdentifier::LOG_BUFFER, None)
        .unwrap()
    else {
        panic!("expected projected log list");
    };
    let PropertyValue::List(fields) = &records[0] else {
        panic!("expected projected record fields");
    };
    assert_eq!(fields.len(), 3);
    assert_eq!(fields[2], PropertyValue::Real(42.0));
}
