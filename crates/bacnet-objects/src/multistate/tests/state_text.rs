use super::super::*;

// --- state_text tests ---

#[test]
fn msi_state_text_defaults() {
    let msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
    let val = msi
        .read_property(PropertyIdentifier::STATE_TEXT, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::List(vec![
            PropertyValue::CharacterString("State 1".into()),
            PropertyValue::CharacterString("State 2".into()),
            PropertyValue::CharacterString("State 3".into()),
        ])
    );
}

#[test]
fn msi_state_text_index_zero_returns_length() {
    let msi = MultiStateInputObject::new(1, "MSI-1", 4).unwrap();
    let val = msi
        .read_property(PropertyIdentifier::STATE_TEXT, Some(0))
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(4));
}

#[test]
fn msi_state_text_valid_index() {
    let msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
    let val = msi
        .read_property(PropertyIdentifier::STATE_TEXT, Some(2))
        .unwrap();
    assert_eq!(val, PropertyValue::CharacterString("State 2".into()));
}

#[test]
fn msi_state_text_invalid_index_error() {
    let msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
    assert!(msi
        .read_property(PropertyIdentifier::STATE_TEXT, Some(4))
        .is_err());
    assert!(msi
        .read_property(PropertyIdentifier::STATE_TEXT, Some(100))
        .is_err());
}

#[test]
fn msi_state_text_write_at_index() {
    let mut msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
    msi.write_property(
        PropertyIdentifier::STATE_TEXT,
        Some(2),
        PropertyValue::CharacterString("Occupied".into()),
        None,
    )
    .unwrap();
    let val = msi
        .read_property(PropertyIdentifier::STATE_TEXT, Some(2))
        .unwrap();
    assert_eq!(val, PropertyValue::CharacterString("Occupied".into()));
}

#[test]
fn msi_state_text_write_wrong_type_rejected() {
    let mut msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
    assert!(msi
        .write_property(
            PropertyIdentifier::STATE_TEXT,
            Some(1),
            PropertyValue::Unsigned(42),
            None,
        )
        .is_err());
}

#[test]
fn msi_state_text_write_bad_index_rejected() {
    let mut msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
    // index 0 is invalid for write
    assert!(msi
        .write_property(
            PropertyIdentifier::STATE_TEXT,
            Some(0),
            PropertyValue::CharacterString("X".into()),
            None,
        )
        .is_err());
    // out-of-range index
    assert!(msi
        .write_property(
            PropertyIdentifier::STATE_TEXT,
            Some(4),
            PropertyValue::CharacterString("X".into()),
            None,
        )
        .is_err());
    // no index
    assert!(msi
        .write_property(
            PropertyIdentifier::STATE_TEXT,
            None,
            PropertyValue::CharacterString("X".into()),
            None,
        )
        .is_err());
}

#[test]
fn msi_state_text_in_property_list() {
    let msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
    assert!(msi
        .property_list()
        .contains(&PropertyIdentifier::STATE_TEXT));
}

#[test]
fn mso_state_text_defaults() {
    let mso = MultiStateOutputObject::new(1, "MSO-1", 2).unwrap();
    let val = mso
        .read_property(PropertyIdentifier::STATE_TEXT, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::List(vec![
            PropertyValue::CharacterString("State 1".into()),
            PropertyValue::CharacterString("State 2".into()),
        ])
    );
}

#[test]
fn mso_state_text_index_zero_returns_length() {
    let mso = MultiStateOutputObject::new(1, "MSO-1", 5).unwrap();
    let val = mso
        .read_property(PropertyIdentifier::STATE_TEXT, Some(0))
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(5));
}

#[test]
fn mso_state_text_valid_index() {
    let mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();
    let val = mso
        .read_property(PropertyIdentifier::STATE_TEXT, Some(3))
        .unwrap();
    assert_eq!(val, PropertyValue::CharacterString("State 3".into()));
}

#[test]
fn mso_state_text_invalid_index_error() {
    let mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();
    assert!(mso
        .read_property(PropertyIdentifier::STATE_TEXT, Some(4))
        .is_err());
}

#[test]
fn mso_state_text_write_at_index() {
    let mut mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();
    mso.write_property(
        PropertyIdentifier::STATE_TEXT,
        Some(1),
        PropertyValue::CharacterString("Low".into()),
        None,
    )
    .unwrap();
    assert_eq!(
        mso.read_property(PropertyIdentifier::STATE_TEXT, Some(1))
            .unwrap(),
        PropertyValue::CharacterString("Low".into())
    );
}

#[test]
fn mso_state_text_in_property_list() {
    let mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();
    assert!(mso
        .property_list()
        .contains(&PropertyIdentifier::STATE_TEXT));
}

#[test]
fn msv_state_text_defaults() {
    let msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
    let val = msv
        .read_property(PropertyIdentifier::STATE_TEXT, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::List(vec![
            PropertyValue::CharacterString("State 1".into()),
            PropertyValue::CharacterString("State 2".into()),
            PropertyValue::CharacterString("State 3".into()),
        ])
    );
}

#[test]
fn msv_state_text_index_zero_returns_length() {
    let msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
    let val = msv
        .read_property(PropertyIdentifier::STATE_TEXT, Some(0))
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(3));
}

#[test]
fn msv_state_text_valid_index() {
    let msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
    let val = msv
        .read_property(PropertyIdentifier::STATE_TEXT, Some(1))
        .unwrap();
    assert_eq!(val, PropertyValue::CharacterString("State 1".into()));
}

#[test]
fn msv_state_text_invalid_index_error() {
    let msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
    assert!(msv
        .read_property(PropertyIdentifier::STATE_TEXT, Some(4))
        .is_err());
    assert!(msv
        .read_property(PropertyIdentifier::STATE_TEXT, Some(0xFF))
        .is_err());
}

#[test]
fn msv_state_text_write_at_index() {
    let mut msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
    msv.write_property(
        PropertyIdentifier::STATE_TEXT,
        Some(2),
        PropertyValue::CharacterString("Comfort".into()),
        None,
    )
    .unwrap();
    assert_eq!(
        msv.read_property(PropertyIdentifier::STATE_TEXT, Some(2))
            .unwrap(),
        PropertyValue::CharacterString("Comfort".into())
    );
}

#[test]
fn msv_state_text_write_bad_index_rejected() {
    let mut msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
    assert!(msv
        .write_property(
            PropertyIdentifier::STATE_TEXT,
            None,
            PropertyValue::CharacterString("X".into()),
            None,
        )
        .is_err());
    assert!(msv
        .write_property(
            PropertyIdentifier::STATE_TEXT,
            Some(0),
            PropertyValue::CharacterString("X".into()),
            None,
        )
        .is_err());
    assert!(msv
        .write_property(
            PropertyIdentifier::STATE_TEXT,
            Some(4),
            PropertyValue::CharacterString("X".into()),
            None,
        )
        .is_err());
}

#[test]
fn msv_state_text_write_wrong_type_rejected() {
    let mut msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
    assert!(msv
        .write_property(
            PropertyIdentifier::STATE_TEXT,
            Some(1),
            PropertyValue::Unsigned(1),
            None,
        )
        .is_err());
}

#[test]
fn msv_state_text_in_property_list() {
    let msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
    assert!(msv
        .property_list()
        .contains(&PropertyIdentifier::STATE_TEXT));
}
