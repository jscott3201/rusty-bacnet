use super::super::*;

// --- MultiStateInput ---

#[test]
fn msi_read_present_value_default() {
    let msi = MultiStateInputObject::new(1, "MSI-1", 4).unwrap();
    let val = msi
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(1));
}

#[test]
fn msi_read_number_of_states() {
    let msi = MultiStateInputObject::new(1, "MSI-1", 4).unwrap();
    let val = msi
        .read_property(PropertyIdentifier::NUMBER_OF_STATES, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(4));
}

#[test]
fn msi_write_denied_when_in_service() {
    let mut msi = MultiStateInputObject::new(1, "MSI-1", 4).unwrap();
    let result = msi.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Unsigned(2),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn msi_write_allowed_when_out_of_service() {
    let mut msi = MultiStateInputObject::new(1, "MSI-1", 4).unwrap();
    msi.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    msi.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Unsigned(3),
        None,
    )
    .unwrap();
    let val = msi
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(3));
}

#[test]
fn msi_write_out_of_range_rejected() {
    let mut msi = MultiStateInputObject::new(1, "MSI-1", 4).unwrap();
    msi.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    assert!(msi
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(0),
            None
        )
        .is_err());
    assert!(msi
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(5),
            None
        )
        .is_err());
}

#[test]
fn msi_read_reliability_default() {
    let msi = MultiStateInputObject::new(1, "MSI-1", 4).unwrap();
    let val = msi
        .read_property(PropertyIdentifier::RELIABILITY, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(0)); // NO_FAULT_DETECTED
}

// --- MultiStateOutput ---

#[test]
fn mso_write_with_priority() {
    let mut mso = MultiStateOutputObject::new(1, "MSO-1", 5).unwrap();
    mso.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Unsigned(3),
        Some(8),
    )
    .unwrap();
    let val = mso
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(3));
    let slot = mso
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(8))
        .unwrap();
    assert_eq!(slot, PropertyValue::Unsigned(3));
}

#[test]
fn mso_relinquish_falls_to_default() {
    let mut mso = MultiStateOutputObject::new(1, "MSO-1", 5).unwrap();
    mso.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Unsigned(4),
        Some(16),
    )
    .unwrap();
    assert_eq!(
        mso.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Unsigned(4)
    );
    mso.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Null,
        Some(16),
    )
    .unwrap();
    assert_eq!(
        mso.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Unsigned(1)
    ); // default
}

#[test]
fn mso_out_of_range_rejected() {
    let mut mso = MultiStateOutputObject::new(1, "MSO-1", 5).unwrap();
    assert!(mso
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(0),
            None
        )
        .is_err());
    assert!(mso
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(6),
            None
        )
        .is_err());
}

#[test]
fn mso_read_reliability_default() {
    let mso = MultiStateOutputObject::new(1, "MSO-1", 5).unwrap();
    let val = mso
        .read_property(PropertyIdentifier::RELIABILITY, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(0)); // NO_FAULT_DETECTED
}

// --- MultiStateValue ---

#[test]
fn msv_read_present_value_default() {
    let msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
    let val = msv
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(1));
}

#[test]
fn msv_write_with_priority() {
    let mut msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
    msv.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Unsigned(2),
        Some(8),
    )
    .unwrap();
    let val = msv
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(2));
    let slot = msv
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(8))
        .unwrap();
    assert_eq!(slot, PropertyValue::Unsigned(2));
}

#[test]
fn msv_relinquish_falls_to_default() {
    let mut msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
    msv.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Unsigned(3),
        Some(16),
    )
    .unwrap();
    assert_eq!(
        msv.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Unsigned(3)
    );
    msv.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Null,
        Some(16),
    )
    .unwrap();
    assert_eq!(
        msv.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Unsigned(1)
    ); // relinquish_default
}

#[test]
fn msv_read_priority_array_all_none() {
    let msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
    let val = msv
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
fn msv_read_relinquish_default() {
    let msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
    let val = msv
        .read_property(PropertyIdentifier::RELINQUISH_DEFAULT, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(1));
}

#[test]
fn msv_write_out_of_range_rejected() {
    let mut msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
    assert!(msv
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(0),
            None
        )
        .is_err());
    assert!(msv
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(4),
            None
        )
        .is_err());
}

#[test]
fn msv_write_wrong_type_rejected() {
    let mut msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
    let result = msv.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(1.0),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn msv_read_object_type() {
    let msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
    let val = msv
        .read_property(PropertyIdentifier::OBJECT_TYPE, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::Enumerated(ObjectType::MULTI_STATE_VALUE.to_raw())
    );
}

#[test]
fn msv_read_reliability_default() {
    let msv = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
    let val = msv
        .read_property(PropertyIdentifier::RELIABILITY, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Enumerated(0)); // NO_FAULT_DETECTED
}

// --- MultiStateValue direct PRIORITY_ARRAY writes ---

#[test]
fn msv_direct_priority_array_write_value() {
    let mut msv = MultiStateValueObject::new(1, "MSV-1", 5).unwrap();
    msv.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(5),
        PropertyValue::Unsigned(3),
        None,
    )
    .unwrap();
    assert_eq!(
        msv.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Unsigned(3)
    );
    assert_eq!(
        msv.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(5))
            .unwrap(),
        PropertyValue::Unsigned(3)
    );
}

#[test]
fn msv_direct_priority_array_relinquish() {
    let mut msv = MultiStateValueObject::new(1, "MSV-1", 5).unwrap();
    msv.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(5),
        PropertyValue::Unsigned(3),
        None,
    )
    .unwrap();
    msv.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(5),
        PropertyValue::Null,
        None,
    )
    .unwrap();
    assert_eq!(
        msv.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Unsigned(1)
    ); // relinquish_default
}

#[test]
fn msv_direct_priority_array_no_index_error() {
    let mut msv = MultiStateValueObject::new(1, "MSV-1", 5).unwrap();
    let result = msv.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        None,
        PropertyValue::Unsigned(3),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn msv_direct_priority_array_index_zero_error() {
    let mut msv = MultiStateValueObject::new(1, "MSV-1", 5).unwrap();
    let result = msv.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(0),
        PropertyValue::Unsigned(3),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn msv_direct_priority_array_index_17_error() {
    let mut msv = MultiStateValueObject::new(1, "MSV-1", 5).unwrap();
    let result = msv.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(17),
        PropertyValue::Unsigned(3),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn msv_direct_priority_array_range_validation() {
    let mut msv = MultiStateValueObject::new(1, "MSV-1", 5).unwrap();
    // Value 0 is out of range (valid: 1..=5)
    assert!(msv
        .write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(1),
            PropertyValue::Unsigned(0),
            None
        )
        .is_err());
    // Value 6 is out of range
    assert!(msv
        .write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(1),
            PropertyValue::Unsigned(6),
            None
        )
        .is_err());
    // Value 5 is valid
    msv.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(1),
        PropertyValue::Unsigned(5),
        None,
    )
    .unwrap();
}

// --- Direct PRIORITY_ARRAY writes ---

#[test]
fn mso_direct_priority_array_write_value() {
    let mut mso = MultiStateOutputObject::new(1, "MSO-1", 5).unwrap();
    mso.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(5),
        PropertyValue::Unsigned(3),
        None,
    )
    .unwrap();
    assert_eq!(
        mso.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Unsigned(3)
    );
    assert_eq!(
        mso.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(5))
            .unwrap(),
        PropertyValue::Unsigned(3)
    );
}

#[test]
fn mso_direct_priority_array_relinquish() {
    let mut mso = MultiStateOutputObject::new(1, "MSO-1", 5).unwrap();
    mso.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(5),
        PropertyValue::Unsigned(3),
        None,
    )
    .unwrap();
    mso.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(5),
        PropertyValue::Null,
        None,
    )
    .unwrap();
    // Fall back to relinquish default (1)
    assert_eq!(
        mso.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Unsigned(1)
    );
}

#[test]
fn mso_direct_priority_array_no_index_error() {
    let mut mso = MultiStateOutputObject::new(1, "MSO-1", 5).unwrap();
    let result = mso.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        None,
        PropertyValue::Unsigned(3),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn mso_direct_priority_array_index_zero_error() {
    let mut mso = MultiStateOutputObject::new(1, "MSO-1", 5).unwrap();
    let result = mso.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(0),
        PropertyValue::Unsigned(3),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn mso_direct_priority_array_index_17_error() {
    let mut mso = MultiStateOutputObject::new(1, "MSO-1", 5).unwrap();
    let result = mso.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(17),
        PropertyValue::Unsigned(3),
        None,
    );
    assert!(result.is_err());
}

// ── Trait capability method tests (issue #115 shared truth source) ──────────

#[test]
fn msi_is_createable_and_writable_match_factory() {
    use crate::traits::BACnetObject;
    let msi = MultiStateInputObject::new(1, "msi-1", 2).unwrap();
    assert!(
        msi.is_createable(),
        "MultiStateInput is factory-constructable"
    );
    // PRESENT_VALUE (when OOS) + STATE_TEXT + common.
    assert!(msi.is_writable_property(PropertyIdentifier::PRESENT_VALUE));
    assert!(msi.is_writable_property(PropertyIdentifier::STATE_TEXT));
    assert!(msi.is_writable_property(PropertyIdentifier::OUT_OF_SERVICE));
    assert!(msi.is_writable_property(PropertyIdentifier::OBJECT_NAME));
    // Not commandable.
    assert!(!msi.is_writable_property(PropertyIdentifier::PRIORITY_ARRAY));
    // EVENT_ENABLE read-only on MSI.
    assert!(!msi.is_writable_property(PropertyIdentifier::EVENT_ENABLE));
}

#[test]
fn mso_is_createable_and_writable_match_factory() {
    use crate::traits::BACnetObject;
    let mso = MultiStateOutputObject::new(1, "mso-1", 2).unwrap();
    assert!(
        mso.is_createable(),
        "MultiStateOutput is factory-constructable"
    );
    // Commandable + STATE_TEXT + common.
    assert!(mso.is_writable_property(PropertyIdentifier::PRIORITY_ARRAY));
    assert!(mso.is_writable_property(PropertyIdentifier::PRESENT_VALUE));
    assert!(mso.is_writable_property(PropertyIdentifier::STATE_TEXT));
    assert!(mso.is_writable_property(PropertyIdentifier::OUT_OF_SERVICE));
    // EVENT_ENABLE read-only on MSO.
    assert!(!mso.is_writable_property(PropertyIdentifier::EVENT_ENABLE));
}

#[test]
fn msv_is_createable_and_writable_match_factory() {
    use crate::traits::BACnetObject;
    let msv = MultiStateValueObject::new(1, "msv-1", 2).unwrap();
    assert!(
        msv.is_createable(),
        "MultiStateValue is factory-constructable"
    );
    // Commandable + STATE_TEXT + common.
    assert!(msv.is_writable_property(PropertyIdentifier::PRIORITY_ARRAY));
    assert!(msv.is_writable_property(PropertyIdentifier::PRESENT_VALUE));
    assert!(msv.is_writable_property(PropertyIdentifier::STATE_TEXT));
    assert!(msv.is_writable_property(PropertyIdentifier::OUT_OF_SERVICE));
}

// --- number_of_states boundary (#119) ---

#[test]
fn msi_new_rejects_zero_states() {
    let err = MultiStateInputObject::new(1, "MSI-1", 0);
    assert!(err.is_err(), "expected an error, got Ok");
    if let Err(e) = err {
        assert!(matches!(e, Error::OutOfRange(_)), "got {e:?}");
    }
}

#[test]
fn mso_new_rejects_zero_states() {
    let err = MultiStateOutputObject::new(1, "MSO-1", 0);
    assert!(err.is_err(), "expected an error, got Ok");
    if let Err(e) = err {
        assert!(matches!(e, Error::OutOfRange(_)), "got {e:?}");
    }
}

#[test]
fn msv_new_rejects_zero_states() {
    let err = MultiStateValueObject::new(1, "MSV-1", 0);
    assert!(err.is_err(), "expected an error, got Ok");
    if let Err(e) = err {
        assert!(matches!(e, Error::OutOfRange(_)), "got {e:?}");
    }
}

/// A single-state object is the minimum valid configuration: the initial
/// `PRESENT_VALUE` (1) is within `1..=number_of_states`, and `STATE_TEXT`
/// holds exactly one entry.
#[test]
fn msi_one_state_is_valid_initial_value_in_range() {
    let msi = MultiStateInputObject::new(1, "MSI-1", 1).unwrap();
    assert_eq!(
        msi.read_property(PropertyIdentifier::NUMBER_OF_STATES, None)
            .unwrap(),
        PropertyValue::Unsigned(1)
    );
    assert_eq!(
        msi.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Unsigned(1)
    );
    assert_eq!(
        msi.read_property(PropertyIdentifier::STATE_TEXT, Some(1))
            .unwrap(),
        PropertyValue::CharacterString("State 1".into())
    );
    // No second state exists.
    assert!(msi
        .read_property(PropertyIdentifier::STATE_TEXT, Some(2))
        .is_err());
}

#[test]
fn mso_one_state_initial_and_relinquish_default_in_range() {
    let mut mso = MultiStateOutputObject::new(1, "MSO-1", 1).unwrap();
    assert_eq!(
        mso.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Unsigned(1)
    );
    assert_eq!(
        mso.read_property(PropertyIdentifier::RELINQUISH_DEFAULT, None)
            .unwrap(),
        PropertyValue::Unsigned(1)
    );
    // Command the only valid state (1) at priority 8, then relinquish slot 8 so
    // the effective value genuinely falls back to the in-range default (not a
    // no-op relinquish of an already-empty slot).
    mso.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(8),
        PropertyValue::Unsigned(1),
        None,
    )
    .unwrap();
    assert_eq!(
        mso.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Unsigned(1)
    );
    mso.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(8),
        PropertyValue::Null,
        None,
    )
    .unwrap();
    assert_eq!(
        mso.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Unsigned(1)
    );
}

#[test]
fn msv_one_state_initial_and_relinquish_default_in_range() {
    let mut msv = MultiStateValueObject::new(1, "MSV-1", 1).unwrap();
    assert_eq!(
        msv.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Unsigned(1)
    );
    assert_eq!(
        msv.read_property(PropertyIdentifier::RELINQUISH_DEFAULT, None)
            .unwrap(),
        PropertyValue::Unsigned(1)
    );
    // Command the only valid state (1) at priority 8, then relinquish slot 8 so
    // the effective value genuinely falls back to the in-range default.
    msv.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(8),
        PropertyValue::Unsigned(1),
        None,
    )
    .unwrap();
    assert_eq!(
        msv.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Unsigned(1)
    );
    msv.write_property(
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(8),
        PropertyValue::Null,
        None,
    )
    .unwrap();
    assert_eq!(
        msv.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Unsigned(1)
    );
}

/// A write outside the valid range is still rejected for a one-state object
/// (state 2 must be refused when only state 1 exists).
#[test]
fn msi_one_state_write_out_of_range_rejected() {
    let mut msi = MultiStateInputObject::new(1, "MSI-1", 1).unwrap();
    msi.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    assert!(msi
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(2),
            None
        )
        .is_err());
}
