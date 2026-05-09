use super::*;
use bacnet_types::primitives::{Date, Time};

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

#[test]
fn trendlog_add_records() {
    let mut tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
    tl.add_record(make_record(10, 72.5));
    tl.add_record(make_record(11, 73.0));
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
        });
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
    tl.write_property(
        PropertyIdentifier::STOP_WHEN_FULL,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    for i in 0..5u8 {
        tl.add_record(make_record(i, i as f32));
    }
    assert_eq!(tl.records().len(), 2);
    assert_eq!(tl.total_record_count, 2); // Only 2 accepted
}

#[test]
fn trendlog_disable_logging() {
    let mut tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
    tl.write_property(
        PropertyIdentifier::LOG_ENABLE,
        None,
        PropertyValue::Boolean(false),
        None,
    )
    .unwrap();
    tl.add_record(make_record(10, 72.5));
    assert_eq!(tl.records().len(), 0);
}

#[test]
fn trendlog_clear_buffer() {
    let mut tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
    tl.add_record(make_record(10, 72.5));
    assert_eq!(tl.records().len(), 1);
    tl.write_property(
        PropertyIdentifier::RECORD_COUNT,
        None,
        PropertyValue::Unsigned(0),
        None,
    )
    .unwrap();
    assert_eq!(tl.records().len(), 0);
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
fn trendlog_read_log_buffer() {
    let mut tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
    tl.add_record(make_record(10, 72.5));
    tl.add_record(make_record(11, 73.0));
    let val = tl
        .read_property(PropertyIdentifier::LOG_BUFFER, None)
        .unwrap();
    if let PropertyValue::List(records) = val {
        assert_eq!(records.len(), 2);
        // First record
        if let PropertyValue::List(fields) = &records[0] {
            assert_eq!(fields.len(), 3);
            assert_eq!(fields[0], PropertyValue::Date(make_record(10, 72.5).date));
            assert_eq!(fields[1], PropertyValue::Time(make_record(10, 72.5).time));
            assert_eq!(fields[2], PropertyValue::Real(72.5));
        } else {
            panic!("Expected List for log record");
        }
        // Second record
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
fn trendlog_log_buffer_empty() {
    let tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
    let val = tl
        .read_property(PropertyIdentifier::LOG_BUFFER, None)
        .unwrap();
    assert_eq!(val, PropertyValue::List(vec![]));
}

#[test]
fn trendlog_log_buffer_overflow_stop_when_full() {
    let mut tl = TrendLogObject::new(1, "TL-1", 3).unwrap();
    tl.write_property(
        PropertyIdentifier::STOP_WHEN_FULL,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    for i in 0..5u8 {
        tl.add_record(make_record(i, i as f32 * 10.0));
    }
    // Buffer capped at 3; only first 3 records accepted
    let val = tl
        .read_property(PropertyIdentifier::LOG_BUFFER, None)
        .unwrap();
    if let PropertyValue::List(records) = val {
        assert_eq!(records.len(), 3);
        if let PropertyValue::List(fields) = &records[0] {
            assert_eq!(fields[2], PropertyValue::Real(0.0));
        } else {
            panic!("Expected List");
        }
        if let PropertyValue::List(fields) = &records[2] {
            assert_eq!(fields[2], PropertyValue::Real(20.0));
        } else {
            panic!("Expected List");
        }
    } else {
        panic!("Expected List for LOG_BUFFER");
    }
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
    tl.set_logging_type(1); // COV
    let val = tl
        .read_property(PropertyIdentifier::LOGGING_TYPE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(1));
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
fn trendlog_log_device_object_property_null_by_default() {
    let tl = TrendLogObject::new(1, "TL-1", 100).unwrap();
    let val = tl
        .read_property(PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Null);
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

    tl.add_record(BACnetLogRecord {
        date,
        time,
        log_datum: LogDatum::BooleanValue(true),
        status_flags: None,
    });
    tl.add_record(BACnetLogRecord {
        date,
        time,
        log_datum: LogDatum::EnumValue(42),
        status_flags: Some(0b0100),
    });
    tl.add_record(BACnetLogRecord {
        date,
        time,
        log_datum: LogDatum::NullValue,
        status_flags: None,
    });

    let val = tl
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
    tlm.add_record(make_record(10, 72.5));
    tlm.add_record(make_record(11, 73.0));
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
        tlm.add_record(BACnetLogRecord {
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
        });
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
fn trendlog_multiple_read_log_buffer() {
    let mut tlm = TrendLogMultipleObject::new(1, "TLM-1", 100).unwrap();
    tlm.add_record(make_record(10, 72.5));
    let val = tlm
        .read_property(PropertyIdentifier::LOG_BUFFER, None)
        .unwrap();
    if let PropertyValue::List(records) = val {
        assert_eq!(records.len(), 1);
        if let PropertyValue::List(fields) = &records[0] {
            assert_eq!(fields[2], PropertyValue::Real(72.5));
        } else {
            panic!("Expected List for log record");
        }
    } else {
        panic!("Expected List for LOG_BUFFER");
    }
}

#[test]
fn trendlog_multiple_property_list() {
    let tlm = TrendLogMultipleObject::new(1, "TLM-1", 100).unwrap();
    let props = tlm.property_list();
    assert!(props.contains(&PropertyIdentifier::LOG_BUFFER));
    assert!(props.contains(&PropertyIdentifier::LOGGING_TYPE));
    assert!(props.contains(&PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY));
    assert!(props.contains(&PropertyIdentifier::OUT_OF_SERVICE));
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
    });
    tlm.add_property_reference(BACnetDeviceObjectPropertyReference {
        object_identifier: oid2,
        property_identifier: pv_raw,
        property_array_index: Some(3),
        device_identifier: None,
    });

    let val = tlm
        .read_property(PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY, None)
        .unwrap();
    if let PropertyValue::List(refs) = val {
        assert_eq!(refs.len(), 2);
        // First reference
        if let PropertyValue::List(fields) = &refs[0] {
            assert_eq!(fields[0], PropertyValue::ObjectIdentifier(oid1));
            assert_eq!(fields[1], PropertyValue::Unsigned(pv_raw as u64));
            assert_eq!(fields[2], PropertyValue::Null);
            assert_eq!(fields[3], PropertyValue::Null);
        } else {
            panic!("Expected List for property reference");
        }
        // Second reference with array index
        if let PropertyValue::List(fields) = &refs[1] {
            assert_eq!(fields[0], PropertyValue::ObjectIdentifier(oid2));
            assert_eq!(fields[1], PropertyValue::Unsigned(pv_raw as u64));
            assert_eq!(fields[2], PropertyValue::Unsigned(3));
            assert_eq!(fields[3], PropertyValue::Null);
        } else {
            panic!("Expected List for property reference");
        }
    } else {
        panic!("Expected List for LOG_DEVICE_OBJECT_PROPERTY");
    }
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
    tlm.add_record(make_record(10, 72.5));
    assert_eq!(tlm.records().len(), 0);
}
