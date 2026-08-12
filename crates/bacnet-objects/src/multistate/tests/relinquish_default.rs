//! #270 — writable Relinquish_Default on the commandable multi-state
//! types (MultiState Output and MultiState Value).

use super::super::*;
// ---------------------------------------------------------------------------
// #270 — writable Relinquish_Default (MSO + MSV)
// ---------------------------------------------------------------------------

/// A MultiState Relinquish_Default write is validated like a commanded
/// Present_Value (Unsigned 1..=Number_Of_States) and — with an all-NULL
/// priority array — Present_Value immediately resolves to the written
/// default.
#[test]
fn mso_msv_relinquish_default_write_recaptures_present_value() {
    for object in [
        &mut MultiStateOutputObject::new(1, "MSO-1", 3).unwrap() as &mut dyn BACnetObject,
        &mut MultiStateValueObject::new(1, "MSV-1", 3).unwrap() as &mut dyn BACnetObject,
    ] {
        assert!(object.is_writable_property(PropertyIdentifier::RELINQUISH_DEFAULT));

        object
            .write_property(
                PropertyIdentifier::RELINQUISH_DEFAULT,
                None,
                PropertyValue::Unsigned(2),
                None,
            )
            .unwrap();
        assert_eq!(
            object
                .read_property(PropertyIdentifier::RELINQUISH_DEFAULT, None)
                .unwrap(),
            PropertyValue::Unsigned(2)
        );
        assert_eq!(
            object
                .read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Unsigned(2),
            "with an empty priority array, PV must resolve to the written default"
        );

        // 0 and Number_Of_States+1 are out of range; u32-overflow refuses at
        // the same pairing as an out-of-range state. None touch the default.
        for value in [
            PropertyValue::Unsigned(0),
            PropertyValue::Unsigned(4),
            PropertyValue::Unsigned(u64::MAX),
            PropertyValue::Real(2.0),
        ] {
            assert!(
                object
                    .write_property(PropertyIdentifier::RELINQUISH_DEFAULT, None, value, None)
                    .is_err(),
                "invalid Relinquish_Default write must refuse"
            );
            assert_eq!(
                object
                    .read_property(PropertyIdentifier::RELINQUISH_DEFAULT, None)
                    .unwrap(),
                PropertyValue::Unsigned(2),
                "refused writes must leave Relinquish_Default untouched"
            );
        }
    }

    // The local setters share the validation.
    let mut mso = MultiStateOutputObject::new(2, "MSO-2", 5).unwrap();
    assert!(mso.set_relinquish_default(6).is_err());
    assert!(mso.set_relinquish_default(0).is_err());
    mso.set_relinquish_default(5).unwrap();
    assert_eq!(
        mso.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Unsigned(5)
    );
}

/// Number_Of_States-shrink interplay, pinned: clause 12.19/12.22 leave any
/// adjustment of Priority_Array / Relinquish_Default / Present_Value /
/// Feedback_Value "a local matter" when the state count shrinks below stored
/// values, and Number_Of_States is constructor-fixed in this implementation,
/// so nothing auto-adjusts — a stale Relinquish_Default simply keeps driving
/// Present_Value until the application resolves the configuration (the
/// CONFIGURATION_ERROR reporting for that condition tracks #226).
#[test]
fn mso_relinquish_default_is_not_range_locked_after_store() {
    let mut mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();
    mso.set_relinquish_default(3).unwrap();
    assert_eq!(
        mso.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Unsigned(3)
    );
    // No Number_Of_States write arm exists: a shrink cannot arrive over the
    // network, and no code path re-validates the stored default.
    assert!(!mso.is_writable_property(PropertyIdentifier::NUMBER_OF_STATES));
}
