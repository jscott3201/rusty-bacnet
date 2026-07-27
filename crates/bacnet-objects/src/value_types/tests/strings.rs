//! CharacterStringValue, OctetStringValue and BitStringValue tests.
//!
//! Split out to keep every file under the 700-LOC cap.

use super::super::*;
use bacnet_types::enums::ObjectType;

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
