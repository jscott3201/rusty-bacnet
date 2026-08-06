use super::*;

#[test]
fn object_type_round_trip() {
    assert_eq!(ObjectType::DEVICE.to_raw(), 8);
    assert_eq!(ObjectType::from_raw(8), ObjectType::DEVICE);
}

#[test]
fn object_type_vendor_proprietary() {
    let vendor = ObjectType::from_raw(128);
    assert_eq!(vendor.to_raw(), 128);
    assert_eq!(format!("{}", vendor), "128");
    assert_eq!(format!("{:?}", vendor), "ObjectType(128)");
}

#[test]
fn object_type_display_known() {
    assert_eq!(format!("{}", ObjectType::ANALOG_INPUT), "ANALOG_INPUT");
    assert_eq!(format!("{:?}", ObjectType::DEVICE), "ObjectType::DEVICE");
}

/// Standard 135-2020 Clause 21.6 (`BACnetObjectType ::= ENUMERATED`) assigns
/// `audit-log (61)` and `audit-reporter (62)`. Guard against a swap regression:
/// the constants were previously reversed (`AUDIT_REPORTER = 61`, `AUDIT_LOG
/// = 62`), which encoded the wrong object type on the wire.
#[test]
fn object_type_audit_codes_match_clause_21_6() {
    assert_eq!(ObjectType::AUDIT_LOG.to_raw(), 61);
    assert_eq!(ObjectType::AUDIT_REPORTER.to_raw(), 62);
    assert_eq!(ObjectType::from_raw(61), ObjectType::AUDIT_LOG);
    assert_eq!(ObjectType::from_raw(62), ObjectType::AUDIT_REPORTER);

    // Lock the on-wire ObjectIdentifier encoding, not just the enum value:
    // the high 10 bits of a 4-byte OID carry the object type, so an AuditLog
    // (type 61) instance 1 must encode as `(61 << 22) | 1` big-endian.
    let audit_log = crate::primitives::ObjectIdentifier::new(ObjectType::AUDIT_LOG, 1).unwrap();
    assert_eq!(audit_log.encode(), ((61u32 << 22) | 1u32).to_be_bytes());
    let audit_reporter =
        crate::primitives::ObjectIdentifier::new(ObjectType::AUDIT_REPORTER, 1).unwrap();
    assert_eq!(
        audit_reporter.encode(),
        ((62u32 << 22) | 1u32).to_be_bytes()
    );
}

#[test]
fn property_identifier_round_trip() {
    assert_eq!(PropertyIdentifier::PRESENT_VALUE.to_raw(), 85);
    assert_eq!(
        PropertyIdentifier::from_raw(85),
        PropertyIdentifier::PRESENT_VALUE
    );
}

#[test]
fn property_identifier_vendor() {
    let vendor = PropertyIdentifier::from_raw(512);
    assert_eq!(vendor.to_raw(), 512);
}

#[test]
fn pdu_type_values() {
    assert_eq!(PduType::CONFIRMED_REQUEST.to_raw(), 0);
    assert_eq!(PduType::ABORT.to_raw(), 7);
}

#[test]
fn confirmed_service_choice_values() {
    assert_eq!(ConfirmedServiceChoice::READ_PROPERTY.to_raw(), 12);
    assert_eq!(ConfirmedServiceChoice::WRITE_PROPERTY.to_raw(), 15);
}

#[test]
fn unconfirmed_service_choice_values() {
    assert_eq!(UnconfirmedServiceChoice::WHO_IS.to_raw(), 8);
    assert_eq!(UnconfirmedServiceChoice::I_AM.to_raw(), 0);
}

#[test]
fn bvlc_function_values() {
    assert_eq!(BvlcFunction::ORIGINAL_UNICAST_NPDU.to_raw(), 0x0A);
    assert_eq!(BvlcFunction::ORIGINAL_BROADCAST_NPDU.to_raw(), 0x0B);
}

#[test]
fn engineering_units_round_trip() {
    assert_eq!(EngineeringUnits::DEGREES_CELSIUS.to_raw(), 62);
    assert_eq!(
        EngineeringUnits::from_raw(62),
        EngineeringUnits::DEGREES_CELSIUS
    );
}

#[test]
fn engineering_units_ashrae_extended() {
    assert_eq!(
        EngineeringUnits::STANDARD_CUBIC_FEET_PER_DAY.to_raw(),
        47808
    );
}

#[test]
fn segmentation_values() {
    assert_eq!(Segmentation::BOTH.to_raw(), 0);
    assert_eq!(Segmentation::NONE.to_raw(), 3);
}

#[test]
fn network_message_type_values() {
    assert_eq!(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK.to_raw(), 0x00);
    assert_eq!(NetworkMessageType::NETWORK_NUMBER_IS.to_raw(), 0x13);
}

#[test]
fn bacnet_sc_error_code_values() {
    assert_eq!(ErrorClass::COMMUNICATION.to_raw(), 7);
    assert_eq!(ErrorCode::MESSAGE_INCOMPLETE.to_raw(), 147);
    assert_eq!(ErrorCode::NODE_DUPLICATE_VMAC.to_raw(), 151);
}

#[test]
fn event_state_values() {
    assert_eq!(EventState::NORMAL.to_raw(), 0);
    assert_eq!(EventState::LIFE_SAFETY_ALARM.to_raw(), 5);
}

#[test]
fn reliability_gap_at_11() {
    // Value 11 is intentionally missing from the standard
    assert_eq!(Reliability::CONFIGURATION_ERROR.to_raw(), 10);
    assert_eq!(Reliability::COMMUNICATION_FAILURE.to_raw(), 12);
}

#[test]
fn reliability_multi_state_out_of_range() {
    assert_eq!(Reliability::MULTI_STATE_OUT_OF_RANGE.to_raw(), 25);
    assert_eq!(
        Reliability::from_raw(25),
        Reliability::MULTI_STATE_OUT_OF_RANGE
    );
    assert_eq!(
        format!("{}", Reliability::MULTI_STATE_OUT_OF_RANGE),
        "MULTI_STATE_OUT_OF_RANGE"
    );
}

#[test]
fn backup_and_restore_state_failure_values() {
    // A device reporting either failure state during a Clause 19.1
    // backup/restore procedure now displays by name, not as a bare number.
    assert_eq!(BackupAndRestoreState::BACKUP_FAILURE.to_raw(), 5);
    assert_eq!(BackupAndRestoreState::RESTORE_FAILURE.to_raw(), 6);
    assert_eq!(
        format!("{}", BackupAndRestoreState::from_raw(5)),
        "BACKUP_FAILURE"
    );
    assert_eq!(
        format!("{}", BackupAndRestoreState::from_raw(6)),
        "RESTORE_FAILURE"
    );
}

// ---------------------------------------------------------------------------
// FromStr
// ---------------------------------------------------------------------------

#[test]
fn from_str_accepts_every_case_style() {
    for input in [
        "analoginput",
        "ANALOGINPUT",
        "AnalogInput",
        "analogInput",
        "analog_input",
        "ANALOG_INPUT",
        "analog-input",
        "ANALOG-INPUT",
    ] {
        assert_eq!(
            input.parse::<ObjectType>(),
            Ok(ObjectType::ANALOG_INPUT),
            "input: {input}"
        );
    }
}

/// Every named constant of every enum must survive a `Display` -> `parse`
/// round trip; the two directions share `ALL_NAMED`, so this also proves the
/// name table itself is unambiguous (no two constants normalize alike).
#[test]
fn from_str_round_trips_display_for_all_named() {
    macro_rules! assert_round_trips {
        ($($Ty:ident),+ $(,)?) => {
            $(
                for &(name, value) in $Ty::ALL_NAMED {
                    assert_eq!(name.parse::<$Ty>(), Ok(value), "{}::{name}", stringify!($Ty));
                    assert_eq!(
                        format!("{value}").parse::<$Ty>(),
                        Ok(value),
                        "{}::{name} via Display",
                        stringify!($Ty),
                    );
                }
            )+
        };
    }

    assert_round_trips!(
        ObjectType,
        PropertyIdentifier,
        EngineeringUnits,
        EventState,
        EventType,
        Reliability,
        BackupAndRestoreState,
        LifeSafetyState,
    );
}

#[test]
fn from_str_rejects_unknown_and_numeric_names() {
    let err = "not-an-object".parse::<ObjectType>().unwrap_err();
    assert_eq!(err.type_name(), "ObjectType");
    assert_eq!(format!("{err}"), "not a known ObjectType name");

    // Raw wire values are `from_raw`'s job, including vendor-proprietary ones.
    assert!("8".parse::<ObjectType>().is_err());
    assert!("128".parse::<ObjectType>().is_err());

    // Separators are ignored, but they cannot bridge distinct names.
    assert!("analog input".parse::<ObjectType>().is_err());
    assert!("analoginputs".parse::<ObjectType>().is_err());
    assert!("".parse::<ObjectType>().is_err());
}

/// `AUDIT_LOG` is 61 and `AUDIT_REPORTER` is 62 per Clause 21.6; parsing by
/// name must agree with the constants (see
/// [`object_type_audit_codes_match_clause_21_6`]).
#[test]
fn from_str_audit_names_are_not_swapped() {
    assert_eq!("audit-log".parse::<ObjectType>(), Ok(ObjectType::AUDIT_LOG));
    assert_eq!(
        "audit-log".parse::<ObjectType>().map(ObjectType::to_raw),
        Ok(61)
    );
    assert_eq!(
        "audit-reporter".parse::<ObjectType>(),
        Ok(ObjectType::AUDIT_REPORTER)
    );
    assert_eq!(
        "audit-reporter"
            .parse::<ObjectType>()
            .map(ObjectType::to_raw),
        Ok(62)
    );
}
