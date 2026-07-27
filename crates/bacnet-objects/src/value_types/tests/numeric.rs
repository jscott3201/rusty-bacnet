//! IntegerValue, PositiveIntegerValue and LargeAnalogValue tests.
//!
//! Split out to keep every file under the 700-LOC cap.

use super::super::*;
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
