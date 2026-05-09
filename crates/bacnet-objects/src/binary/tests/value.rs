use super::super::*;

#[test]
fn bv_read_present_value_default() {
    let bv = BinaryValueObject::new(1, "BV-1").unwrap();
    let val = bv
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(0)); // inactive
}

#[test]
fn bv_write_present_value() {
    let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
    bv.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(1), // active
        Some(8),
    )
    .unwrap();
    let val = bv
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(1));
}

#[test]
fn bv_write_invalid_value_rejected() {
    let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
    let result = bv.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(2), // invalid -- only 0 or 1
        Some(8),
    );
    assert!(result.is_err());
}

#[test]
fn bv_write_wrong_type_rejected() {
    let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
    let result = bv.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(1.0), // wrong type
        Some(8),
    );
    assert!(result.is_err());
}

#[test]
fn bv_read_object_type() {
    let bv = BinaryValueObject::new(1, "BV-1").unwrap();
    let val = bv
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::Enumerated(ObjectType::BINARY_VALUE.to_raw())
    );
}

#[test]
fn bv_read_reliability_default() {
    let bv = BinaryValueObject::new(1, "BV-1").unwrap();
    let val = bv
        .read_property(PropertyIdentifier::RELIABILITY, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(0)); // NO_FAULT_DETECTED
}

// --- BinaryValue commandable tests ---

#[test]
fn bv_write_with_priority() {
    let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
    bv.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(1),
        Some(8),
    )
    .unwrap();
    let val = bv
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(1));
    let slot = bv
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(8))
        .unwrap();
    assert_eq!(slot, PropertyValue::Enumerated(1));
}

#[test]
fn bv_relinquish_falls_to_default() {
    let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
    bv.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(1),
        Some(16),
    )
    .unwrap();
    assert_eq!(
        bv.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Enumerated(1)
    );
    bv.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Null,
        Some(16),
    )
    .unwrap();
    assert_eq!(
        bv.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Enumerated(0) // relinquish_default
    );
}

#[test]
fn bv_read_priority_array_all_none() {
    let bv = BinaryValueObject::new(1, "BV-1").unwrap();
    let val = bv
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, None)
        .unwrap();
    if let PropertyValue::List(elements) = val {
        assert_eq!(elements.len(), 16);
        for elem in &elements {
            assert_eq!(elem, &PropertyValue::Null);
        }
    } else {
        panic!("Expected List for priority array without index");
    }
}

#[test]
fn bv_read_relinquish_default() {
    let bv = BinaryValueObject::new(1, "BV-1").unwrap();
    let val = bv
        .read_property(PropertyIdentifier::RELINQUISH_DEFAULT, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(0));
}

#[test]
fn bv_priority_array_in_property_list() {
    let bv = BinaryValueObject::new(1, "BV-1").unwrap();
    let props = bv.property_list();
    assert!(props.contains(&PropertyIdentifier::PRIORITY_ARRAY));
    assert!(props.contains(&PropertyIdentifier::RELINQUISH_DEFAULT));
}

#[test]
fn bv_direct_priority_array_write() {
    let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
    bv.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(5),
        PropertyValue::Enumerated(1),
        None,
    )
    .unwrap();
    assert_eq!(
        bv.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Enumerated(1)
    );
    assert_eq!(
        bv.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(5))
            .unwrap(),
        PropertyValue::Enumerated(1)
    );
}

#[test]
fn bv_direct_priority_array_relinquish() {
    let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
    bv.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(5),
        PropertyValue::Enumerated(1),
        None,
    )
    .unwrap();
    bv.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(5),
        PropertyValue::Null,
        None,
    )
    .unwrap();
    assert_eq!(
        bv.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Enumerated(0) // relinquish_default
    );
}

// --- Description tests ---

#[test]
fn bv_description_read_write() {
    let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
    // Default is empty
    assert_eq!(
        bv.read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap(),
        PropertyValue::CharacterString(String::new())
    );
    bv.write_property(
        PropertyIdentifier::DESCRIPTION,
        None,
        PropertyValue::CharacterString("Occupied/Unoccupied".into()),
        None,
    )
    .unwrap();
    assert_eq!(
        bv.read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap(),
        PropertyValue::CharacterString("Occupied/Unoccupied".into())
    );
}

#[test]
fn bv_description_in_property_list() {
    let bv = BinaryValueObject::new(1, "BV-1").unwrap();
    assert!(bv
        .property_list()
        .contains(&PropertyIdentifier::DESCRIPTION));
}

#[test]
fn bi_description_read_write() {
    let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
    bi.write_property(
        PropertyIdentifier::DESCRIPTION,
        None,
        PropertyValue::CharacterString("Door contact".into()),
        None,
    )
    .unwrap();
    assert_eq!(
        bi.read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap(),
        PropertyValue::CharacterString("Door contact".into())
    );
}

#[test]
fn bo_description_read_write() {
    let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    bo.write_property(
        PropertyIdentifier::DESCRIPTION,
        None,
        PropertyValue::CharacterString("Fan enable".into()),
        None,
    )
    .unwrap();
    assert_eq!(
        bo.read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap(),
        PropertyValue::CharacterString("Fan enable".into())
    );
}

#[test]
fn bv_higher_priority_wins() {
    let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
    // Write inactive at priority 10
    bv.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(0),
        Some(10),
    )
    .unwrap();
    // Write active at priority 5 (higher)
    bv.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(1),
        Some(5),
    )
    .unwrap();
    assert_eq!(
        bv.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Enumerated(1) // priority 5 wins
    );
    // Relinquish priority 5, falls to priority 10
    bv.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Null,
        Some(5),
    )
    .unwrap();
    assert_eq!(
        bv.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Enumerated(0) // priority 10 value
    );
}

// --- active_text / inactive_text tests ---

#[test]
fn bi_active_inactive_text_defaults() {
    let bi = BinaryInputObject::new(1, "BI-1").unwrap();
    assert_eq!(
        bi.read_property(PropertyIdentifier::ACTIVE_TEXT, None)
            .unwrap(),
        PropertyValue::CharacterString("Active".into())
    );
    assert_eq!(
        bi.read_property(PropertyIdentifier::INACTIVE_TEXT, None)
            .unwrap(),
        PropertyValue::CharacterString("Inactive".into())
    );
}

#[test]
fn bi_active_inactive_text_write_read() {
    let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
    bi.write_property(
        PropertyIdentifier::ACTIVE_TEXT,
        None,
        PropertyValue::CharacterString("On".into()),
        None,
    )
    .unwrap();
    bi.write_property(
        PropertyIdentifier::INACTIVE_TEXT,
        None,
        PropertyValue::CharacterString("Off".into()),
        None,
    )
    .unwrap();
    assert_eq!(
        bi.read_property(PropertyIdentifier::ACTIVE_TEXT, None)
            .unwrap(),
        PropertyValue::CharacterString("On".into())
    );
    assert_eq!(
        bi.read_property(PropertyIdentifier::INACTIVE_TEXT, None)
            .unwrap(),
        PropertyValue::CharacterString("Off".into())
    );
}

#[test]
fn bi_active_text_wrong_type_rejected() {
    let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
    assert!(bi
        .write_property(
            PropertyIdentifier::ACTIVE_TEXT,
            None,
            PropertyValue::Enumerated(1),
            None,
        )
        .is_err());
}

#[test]
fn bi_inactive_text_wrong_type_rejected() {
    let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
    assert!(bi
        .write_property(
            PropertyIdentifier::INACTIVE_TEXT,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .is_err());
}

#[test]
fn bi_active_inactive_text_in_property_list() {
    let bi = BinaryInputObject::new(1, "BI-1").unwrap();
    let props = bi.property_list();
    assert!(props.contains(&PropertyIdentifier::ACTIVE_TEXT));
    assert!(props.contains(&PropertyIdentifier::INACTIVE_TEXT));
}

#[test]
fn bo_active_inactive_text_defaults() {
    let bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    assert_eq!(
        bo.read_property(PropertyIdentifier::ACTIVE_TEXT, None)
            .unwrap(),
        PropertyValue::CharacterString("Active".into())
    );
    assert_eq!(
        bo.read_property(PropertyIdentifier::INACTIVE_TEXT, None)
            .unwrap(),
        PropertyValue::CharacterString("Inactive".into())
    );
}

#[test]
fn bo_active_inactive_text_write_read() {
    let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    bo.write_property(
        PropertyIdentifier::ACTIVE_TEXT,
        None,
        PropertyValue::CharacterString("Running".into()),
        None,
    )
    .unwrap();
    bo.write_property(
        PropertyIdentifier::INACTIVE_TEXT,
        None,
        PropertyValue::CharacterString("Stopped".into()),
        None,
    )
    .unwrap();
    assert_eq!(
        bo.read_property(PropertyIdentifier::ACTIVE_TEXT, None)
            .unwrap(),
        PropertyValue::CharacterString("Running".into())
    );
    assert_eq!(
        bo.read_property(PropertyIdentifier::INACTIVE_TEXT, None)
            .unwrap(),
        PropertyValue::CharacterString("Stopped".into())
    );
}

#[test]
fn bo_active_inactive_text_in_property_list() {
    let bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    let props = bo.property_list();
    assert!(props.contains(&PropertyIdentifier::ACTIVE_TEXT));
    assert!(props.contains(&PropertyIdentifier::INACTIVE_TEXT));
}

#[test]
fn bv_active_inactive_text_defaults() {
    let bv = BinaryValueObject::new(1, "BV-1").unwrap();
    assert_eq!(
        bv.read_property(PropertyIdentifier::ACTIVE_TEXT, None)
            .unwrap(),
        PropertyValue::CharacterString("Active".into())
    );
    assert_eq!(
        bv.read_property(PropertyIdentifier::INACTIVE_TEXT, None)
            .unwrap(),
        PropertyValue::CharacterString("Inactive".into())
    );
}

#[test]
fn bv_active_inactive_text_write_read() {
    let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
    bv.write_property(
        PropertyIdentifier::ACTIVE_TEXT,
        None,
        PropertyValue::CharacterString("Occupied".into()),
        None,
    )
    .unwrap();
    bv.write_property(
        PropertyIdentifier::INACTIVE_TEXT,
        None,
        PropertyValue::CharacterString("Unoccupied".into()),
        None,
    )
    .unwrap();
    assert_eq!(
        bv.read_property(PropertyIdentifier::ACTIVE_TEXT, None)
            .unwrap(),
        PropertyValue::CharacterString("Occupied".into())
    );
    assert_eq!(
        bv.read_property(PropertyIdentifier::INACTIVE_TEXT, None)
            .unwrap(),
        PropertyValue::CharacterString("Unoccupied".into())
    );
}

#[test]
fn bv_active_inactive_text_in_property_list() {
    let bv = BinaryValueObject::new(1, "BV-1").unwrap();
    let props = bv.property_list();
    assert!(props.contains(&PropertyIdentifier::ACTIVE_TEXT));
    assert!(props.contains(&PropertyIdentifier::INACTIVE_TEXT));
}

#[test]
fn bv_active_text_wrong_type_rejected() {
    let mut bv = BinaryValueObject::new(1, "BV-1").unwrap();
    assert!(bv
        .write_property(
            PropertyIdentifier::ACTIVE_TEXT,
            None,
            PropertyValue::Real(1.0),
            None,
        )
        .is_err());
}
