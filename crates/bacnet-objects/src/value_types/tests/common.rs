//! Common property behaviour, using IntegerValue as the representative type.
//!
//! Split out to keep every file under the 700-LOC cap.

use super::super::*;
use bacnet_types::enums::ObjectType;

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
