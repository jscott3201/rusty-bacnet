use super::*;
use bacnet_types::enums::ObjectType;

// -----------------------------------------------------------------------
// IntegerValueObject
// -----------------------------------------------------------------------

#[test]
fn integer_value_construct_and_read_object_type() {
    let obj = IntegerValueObject::new(1, "IV-1").unwrap();
    let ot = obj
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        ot,
        PropertyValue::Enumerated(ObjectType::INTEGER_VALUE.to_raw())
    );
}

#[test]
fn integer_value_read_write_pv() {
    let mut obj = IntegerValueObject::new(1, "IV-1").unwrap();
    // Default PV is 0
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::Signed(0));

    // Write via priority 8
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Signed(-42),
        Some(8),
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::Signed(-42));
}

#[test]
fn integer_value_priority_array() {
    let mut obj = IntegerValueObject::new(1, "IV-1").unwrap();
    // Write at priority 10
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Signed(100),
        Some(10),
    )
    .unwrap();
    // Write at priority 5 (should win)
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Signed(50),
        Some(5),
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::Signed(50));

    // Relinquish priority 5 — priority 10 takes over
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Null,
        Some(5),
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::Signed(100));

    // Read priority array size via array_index 0
    let pa_size = obj
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(0))
        .unwrap();
    assert_eq!(pa_size, PropertyValue::Unsigned(16));
}

#[test]
fn integer_value_invalid_data_type() {
    let mut obj = IntegerValueObject::new(1, "IV-1").unwrap();
    let result = obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::CharacterString("bad".into()),
        Some(16),
    );
    assert!(result.is_err());
}

// -----------------------------------------------------------------------
// PositiveIntegerValueObject
// -----------------------------------------------------------------------

#[test]
fn positive_integer_value_read_write() {
    let mut obj = PositiveIntegerValueObject::new(1, "PIV-1").unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::Unsigned(0));

    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Unsigned(9999),
        Some(16),
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::Unsigned(9999));
}

#[test]
fn positive_integer_value_object_type() {
    let obj = PositiveIntegerValueObject::new(1, "PIV-1").unwrap();
    let ot = obj
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        ot,
        PropertyValue::Enumerated(ObjectType::POSITIVE_INTEGER_VALUE.to_raw())
    );
}

// -----------------------------------------------------------------------
// LargeAnalogValueObject
// -----------------------------------------------------------------------

#[test]
fn large_analog_value_read_write() {
    let mut obj = LargeAnalogValueObject::new(1, "LAV-1").unwrap();
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Double(1.23456789012345),
        Some(16),
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::Double(1.23456789012345));
}

#[test]
fn large_analog_value_object_type() {
    let obj = LargeAnalogValueObject::new(1, "LAV-1").unwrap();
    let ot = obj
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        ot,
        PropertyValue::Enumerated(ObjectType::LARGE_ANALOG_VALUE.to_raw())
    );
}

// -----------------------------------------------------------------------
// CharacterStringValueObject
// -----------------------------------------------------------------------

#[test]
fn characterstring_value_read_write() {
    let mut obj = CharacterStringValueObject::new(1, "CSV-1").unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::CharacterString(String::new()));

    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::CharacterString("hello world".into()),
        Some(16),
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::CharacterString("hello world".into()));
}

#[test]
fn characterstring_value_priority_array() {
    let mut obj = CharacterStringValueObject::new(1, "CSV-1").unwrap();
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::CharacterString("low".into()),
        Some(16),
    )
    .unwrap();
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::CharacterString("high".into()),
        Some(1),
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::CharacterString("high".into()));

    // Relinquish priority 1 — low takes over
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Null,
        Some(1),
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::CharacterString("low".into()));
}

// -----------------------------------------------------------------------
// OctetStringValueObject
// -----------------------------------------------------------------------

#[test]
fn octetstring_value_read_write() {
    let mut obj = OctetStringValueObject::new(1, "OSV-1").unwrap();
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::OctetString(vec![0xDE, 0xAD, 0xBE, 0xEF]),
        Some(16),
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::OctetString(vec![0xDE, 0xAD, 0xBE, 0xEF]));
}

#[test]
fn octetstring_value_object_type() {
    let obj = OctetStringValueObject::new(1, "OSV-1").unwrap();
    let ot = obj
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        ot,
        PropertyValue::Enumerated(ObjectType::OCTETSTRING_VALUE.to_raw())
    );
}

// -----------------------------------------------------------------------
// BitStringValueObject
// -----------------------------------------------------------------------

#[test]
fn bitstring_value_read_write() {
    let mut obj = BitStringValueObject::new(1, "BSV-1").unwrap();
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::BitString {
            unused_bits: 3,
            data: vec![0b11010000],
        },
        Some(16),
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(
        pv,
        PropertyValue::BitString {
            unused_bits: 3,
            data: vec![0b11010000],
        }
    );
}

#[test]
fn bitstring_value_object_type() {
    let obj = BitStringValueObject::new(1, "BSV-1").unwrap();
    let ot = obj
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        ot,
        PropertyValue::Enumerated(ObjectType::BITSTRING_VALUE.to_raw())
    );
}

// -----------------------------------------------------------------------
// DateValueObject
// -----------------------------------------------------------------------

#[test]
fn date_value_read_write() {
    let mut obj = DateValueObject::new(1, "DV-1").unwrap();
    let d = Date {
        year: 124,
        month: 3,
        day: 15,
        day_of_week: 5,
    };
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Date(d),
        Some(16),
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::Date(d));
}

#[test]
fn date_value_priority_array() {
    let mut obj = DateValueObject::new(1, "DV-1").unwrap();
    let d1 = Date {
        year: 124,
        month: 1,
        day: 1,
        day_of_week: 1,
    };
    let d2 = Date {
        year: 124,
        month: 12,
        day: 25,
        day_of_week: 3,
    };
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Date(d1),
        Some(16),
    )
    .unwrap();
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Date(d2),
        Some(8),
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::Date(d2));
}

// -----------------------------------------------------------------------
// TimeValueObject
// -----------------------------------------------------------------------

#[test]
fn time_value_read_write() {
    let mut obj = TimeValueObject::new(1, "TV-1").unwrap();
    let t = Time {
        hour: 14,
        minute: 30,
        second: 0,
        hundredths: 0,
    };
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Time(t),
        Some(16),
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::Time(t));
}

#[test]
fn time_value_object_type() {
    let obj = TimeValueObject::new(1, "TV-1").unwrap();
    let ot = obj
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        ot,
        PropertyValue::Enumerated(ObjectType::TIME_VALUE.to_raw())
    );
}

// -----------------------------------------------------------------------
// DateTimeValueObject
// -----------------------------------------------------------------------

#[test]
fn datetime_value_read_write() {
    let mut obj = DateTimeValueObject::new(1, "DTV-1").unwrap();
    let d = Date {
        year: 124,
        month: 6,
        day: 15,
        day_of_week: 6,
    };
    let t = Time {
        hour: 12,
        minute: 0,
        second: 0,
        hundredths: 0,
    };
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::List(vec![PropertyValue::Date(d), PropertyValue::Time(t)]),
        Some(16),
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(
        pv,
        PropertyValue::List(vec![PropertyValue::Date(d), PropertyValue::Time(t)])
    );
}

#[test]
fn datetime_value_object_type() {
    let obj = DateTimeValueObject::new(1, "DTV-1").unwrap();
    let ot = obj
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        ot,
        PropertyValue::Enumerated(ObjectType::DATETIME_VALUE.to_raw())
    );
}

#[test]
fn datetime_value_priority_array() {
    let mut obj = DateTimeValueObject::new(1, "DTV-1").unwrap();
    let d1 = Date {
        year: 124,
        month: 1,
        day: 1,
        day_of_week: 1,
    };
    let t1 = Time {
        hour: 0,
        minute: 0,
        second: 0,
        hundredths: 0,
    };
    let d2 = Date {
        year: 124,
        month: 12,
        day: 31,
        day_of_week: 2,
    };
    let t2 = Time {
        hour: 23,
        minute: 59,
        second: 59,
        hundredths: 99,
    };
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::List(vec![PropertyValue::Date(d1), PropertyValue::Time(t1)]),
        Some(16),
    )
    .unwrap();
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::List(vec![PropertyValue::Date(d2), PropertyValue::Time(t2)]),
        Some(4),
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(
        pv,
        PropertyValue::List(vec![PropertyValue::Date(d2), PropertyValue::Time(t2)])
    );
}

// -----------------------------------------------------------------------
// DatePatternValueObject (non-commandable)
// -----------------------------------------------------------------------

#[test]
fn date_pattern_value_read_write() {
    let mut obj = DatePatternValueObject::new(1, "DPV-1").unwrap();
    let d = Date {
        year: 0xFF,
        month: 0xFF,
        day: 25,
        day_of_week: 0xFF,
    };
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Date(d),
        None,
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::Date(d));
}

#[test]
fn date_pattern_value_object_type() {
    let obj = DatePatternValueObject::new(1, "DPV-1").unwrap();
    let ot = obj
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        ot,
        PropertyValue::Enumerated(ObjectType::DATEPATTERN_VALUE.to_raw())
    );
}

#[test]
fn date_pattern_value_has_priority_array() {
    let obj = DatePatternValueObject::new(1, "DPV-1").unwrap();
    let props = obj.property_list();
    assert!(props.contains(&PropertyIdentifier::PRIORITY_ARRAY));
    assert!(props.contains(&PropertyIdentifier::RELINQUISH_DEFAULT));
}

// -----------------------------------------------------------------------
// TimePatternValueObject (non-commandable)
// -----------------------------------------------------------------------

#[test]
fn time_pattern_value_read_write() {
    let mut obj = TimePatternValueObject::new(1, "TPV-1").unwrap();
    let t = Time {
        hour: 12,
        minute: 0xFF,
        second: 0xFF,
        hundredths: 0xFF,
    };
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Time(t),
        None,
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::Time(t));
}

#[test]
fn time_pattern_value_object_type() {
    let obj = TimePatternValueObject::new(1, "TPV-1").unwrap();
    let ot = obj
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        ot,
        PropertyValue::Enumerated(ObjectType::TIMEPATTERN_VALUE.to_raw())
    );
}

// -----------------------------------------------------------------------
// DateTimePatternValueObject (non-commandable)
// -----------------------------------------------------------------------

#[test]
fn datetime_pattern_value_read_write() {
    let mut obj = DateTimePatternValueObject::new(1, "DTPV-1").unwrap();
    let d = Date {
        year: 0xFF,
        month: 12,
        day: 25,
        day_of_week: 0xFF,
    };
    let t = Time {
        hour: 0xFF,
        minute: 0xFF,
        second: 0xFF,
        hundredths: 0xFF,
    };
    obj.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::List(vec![PropertyValue::Date(d), PropertyValue::Time(t)]),
        None,
    )
    .unwrap();
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(
        pv,
        PropertyValue::List(vec![PropertyValue::Date(d), PropertyValue::Time(t)])
    );
}

#[test]
fn datetime_pattern_value_object_type() {
    let obj = DateTimePatternValueObject::new(1, "DTPV-1").unwrap();
    let ot = obj
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        ot,
        PropertyValue::Enumerated(ObjectType::DATETIMEPATTERN_VALUE.to_raw())
    );
}

#[test]
fn datetime_pattern_value_has_priority_array() {
    let obj = DateTimePatternValueObject::new(1, "DTPV-1").unwrap();
    let props = obj.property_list();
    assert!(props.contains(&PropertyIdentifier::PRIORITY_ARRAY));
    assert!(props.contains(&PropertyIdentifier::RELINQUISH_DEFAULT));
}

// -----------------------------------------------------------------------
// Common property tests (using IntegerValue as representative)
// -----------------------------------------------------------------------

#[test]
fn value_object_read_common_properties() {
    let obj = IntegerValueObject::new(42, "TestObj").unwrap();

    // OBJECT_NAME
    let name = obj
        .read_property(PropertyIdentifier::OBJECT_NAME, None)
        .unwrap();
    assert_eq!(name, PropertyValue::CharacterString("TestObj".into()));

    // OBJECT_IDENTIFIER
    let oid = obj
        .read_property(PropertyIdentifier::OBJECT_IDENTIFIER, None)
        .unwrap();
    assert!(matches!(oid, PropertyValue::ObjectIdentifier(_)));

    // STATUS_FLAGS
    let sf = obj
        .read_property(PropertyIdentifier::STATUS_FLAGS, None)
        .unwrap();
    assert!(matches!(sf, PropertyValue::BitString { .. }));

    // OUT_OF_SERVICE
    let oos = obj
        .read_property(PropertyIdentifier::OUT_OF_SERVICE, None)
        .unwrap();
    assert_eq!(oos, PropertyValue::Boolean(false));

    // RELIABILITY
    let rel = obj
        .read_property(PropertyIdentifier::RELIABILITY, None)
        .unwrap();
    assert_eq!(rel, PropertyValue::Enumerated(0));
}

#[test]
fn value_object_write_description() {
    let mut obj = IntegerValueObject::new(1, "IV-1").unwrap();
    obj.write_property(
        PropertyIdentifier::DESCRIPTION,
        None,
        PropertyValue::CharacterString("A test integer".into()),
        None,
    )
    .unwrap();
    let desc = obj
        .read_property(PropertyIdentifier::DESCRIPTION, None)
        .unwrap();
    assert_eq!(
        desc,
        PropertyValue::CharacterString("A test integer".into())
    );
}

#[test]
fn value_object_write_out_of_service() {
    let mut obj = IntegerValueObject::new(1, "IV-1").unwrap();
    obj.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    let oos = obj
        .read_property(PropertyIdentifier::OUT_OF_SERVICE, None)
        .unwrap();
    assert_eq!(oos, PropertyValue::Boolean(true));
}

#[test]
fn value_object_relinquish_default() {
    let obj = IntegerValueObject::new(1, "IV-1").unwrap();
    let rd = obj
        .read_property(PropertyIdentifier::RELINQUISH_DEFAULT, None)
        .unwrap();
    assert_eq!(rd, PropertyValue::Signed(0));
}

#[test]
fn value_object_priority_array_direct_write() {
    let mut obj = IntegerValueObject::new(1, "IV-1").unwrap();

    // Write directly to priority array slot 5
    obj.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(5),
        PropertyValue::Signed(77),
        None,
    )
    .unwrap();

    // Read back slot 5
    let slot = obj
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(5))
        .unwrap();
    assert_eq!(slot, PropertyValue::Signed(77));

    // PV should reflect it
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::Signed(77));

    // Relinquish slot 5
    obj.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(5),
        PropertyValue::Null,
        None,
    )
    .unwrap();

    // PV falls back to relinquish default
    let pv = obj
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::Signed(0));
}

#[test]
fn value_object_unknown_property() {
    let obj = IntegerValueObject::new(1, "IV-1").unwrap();
    let result = obj.read_property(PropertyIdentifier::UNITS, None);
    assert!(result.is_err());
}

#[test]
fn value_object_write_object_name() {
    let mut obj = IntegerValueObject::new(1, "IV-1").unwrap();
    let result = obj.write_property(
        PropertyIdentifier::OBJECT_NAME,
        None,
        PropertyValue::CharacterString("new-name".into()),
        None,
    );
    assert!(result.is_ok());
    assert_eq!(obj.object_name(), "new-name");
}

#[test]
fn value_object_write_access_denied() {
    // OBJECT_TYPE is never writable
    let mut obj = IntegerValueObject::new(1, "IV-1").unwrap();
    let result = obj.write_property(
        PropertyIdentifier::OBJECT_TYPE,
        None,
        PropertyValue::Enumerated(0),
        None,
    );
    assert!(result.is_err());
}
