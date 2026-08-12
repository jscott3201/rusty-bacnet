use super::*;
use crate::primitives::PropertyValue;

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

/// Assert a table of (`Display` name, raw value) pairs against a
/// `bacnet_enum!` type's `ALL_NAMED`: same length, same order, and every
/// entry round-trips `from_raw` / `Display`.
macro_rules! assert_production_values {
    ($Ty:ident, [$($pair:expr),+ $(,)?] $(,)?) => {
        let all = [$($pair),+];
        assert_eq!($Ty::ALL_NAMED.len(), all.len(), "{} count", stringify!($Ty));
        for (i, &(name, raw)) in all.iter().enumerate() {
            let (named_name, value) = $Ty::ALL_NAMED[i];
            assert_eq!(named_name, name, "{} [{i}]", stringify!($Ty));
            assert_eq!(value.to_raw(), raw, "{} [{i}]", stringify!($Ty));
            assert_eq!($Ty::from_raw(raw), value, "{} [{i}]", stringify!($Ty));
            assert_eq!(format!("{value}"), name, "{} [{i}]", stringify!($Ty));
        }
    };
}

/// Standard 135-2020 Clause 21 (`BACnetFileAccessMethod ::= ENUMERATED`)
/// assigns `record-access (0)` and `stream-access (1)`. Guard against a swap
/// regression: the constants were previously reversed (`STREAM_ACCESS = 0`,
/// `RECORD_ACCESS = 1`), which reported the wrong access method for every
/// File object on the wire (#273).
#[test]
fn file_access_method_values_match_clause_21() {
    assert_production_values!(
        FileAccessMethod,
        [("RECORD_ACCESS", 0), ("STREAM_ACCESS", 1)]
    );

    // file-access-method (41) resolves by name with the corrected values.
    assert_eq!(
        ResolvedEnum::from_property(PropertyIdentifier::FILE_ACCESS_METHOD, 0),
        ResolvedEnum::FileAccessMethod(FileAccessMethod::RECORD_ACCESS)
    );
    assert_eq!(
        ResolvedEnum::from_property(PropertyIdentifier::FILE_ACCESS_METHOD, 1),
        ResolvedEnum::FileAccessMethod(FileAccessMethod::STREAM_ACCESS)
    );
    assert_eq!(
        ResolvedEnum::from_property(PropertyIdentifier::FILE_ACCESS_METHOD, 1).to_string(),
        "STREAM_ACCESS"
    );
}

/// Standard 135-2020 Clause 21 (`BACnetDoorAlarmState ::= ENUMERATED`):
/// value 6 is `lock-down`, not a "lock fault" — no such member exists in the
/// production. `LockStatus::LOCK_FAULT` (value 2 of a different production)
/// is correct and must not be conflated with it (#274).
#[test]
fn door_alarm_state_values_match_clause_21() {
    assert_production_values!(
        DoorAlarmState,
        [
            ("NORMAL", 0),
            ("ALARM", 1),
            ("DOOR_OPEN_TOO_LONG", 2),
            ("FORCED_OPEN", 3),
            ("TAMPER", 4),
            ("DOOR_FAULT", 5),
            ("LOCK_DOWN", 6),
            ("FREE_ACCESS", 7),
            ("EGRESS_OPEN", 8),
        ],
    );

    // door-alarm-state (226) resolves by name; 6 displays as LOCK_DOWN.
    assert_eq!(
        ResolvedEnum::from_property(PropertyIdentifier::DOOR_ALARM_STATE, 6),
        ResolvedEnum::DoorAlarmState(DoorAlarmState::LOCK_DOWN)
    );
    assert_eq!(
        ResolvedEnum::from_property(PropertyIdentifier::DOOR_ALARM_STATE, 6).to_string(),
        "LOCK_DOWN"
    );

    // The conflation guard: lock-fault belongs to LockStatus, at value 2.
    assert_eq!(LockStatus::LOCK_FAULT.to_raw(), 2);
    assert!(DoorAlarmState::ALL_NAMED
        .iter()
        .all(|(name, _)| *name != "LOCK_FAULT"));
}

/// 135-2020 Clause 21 (`BACnetLifeSafetyState ::= ENUMERATED`) runs through
/// test-oeo-unaffected (34): the eleven values past test-supervisory (23)
/// must exist without renumbering anything below them.
#[test]
fn life_safety_state_tail_values_match_clause_21() {
    // Existing numbering is untouched.
    assert_eq!(LifeSafetyState::TEST_SUPERVISORY.to_raw(), 23);
    let tail = [
        (LifeSafetyState::NON_DEFAULT_MODE, 24),
        (LifeSafetyState::OEO_UNAVAILABLE, 25),
        (LifeSafetyState::OEO_ALARM, 26),
        (LifeSafetyState::OEO_PHASE1_RECALL, 27),
        (LifeSafetyState::OEO_EVACUATE, 28),
        (LifeSafetyState::OEO_UNAFFECTED, 29),
        (LifeSafetyState::TEST_OEO_UNAVAILABLE, 30),
        (LifeSafetyState::TEST_OEO_ALARM, 31),
        (LifeSafetyState::TEST_OEO_PHASE1_RECALL, 32),
        (LifeSafetyState::TEST_OEO_EVACUATE, 33),
        (LifeSafetyState::TEST_OEO_UNAFFECTED, 34),
    ];
    for (state, raw) in tail {
        assert_eq!(state.to_raw(), raw);
        assert_eq!(LifeSafetyState::from_raw(raw), state);
        assert_eq!(LifeSafetyState::ALL_NAMED[raw as usize].1, state);
    }
    assert_eq!(LifeSafetyState::ALL_NAMED.len(), 35);
    assert_eq!(
        format!("{}", LifeSafetyState::OEO_PHASE1_RECALL),
        "OEO_PHASE1_RECALL"
    );
    assert_eq!(
        format!("{}", LifeSafetyState::TEST_OEO_UNAFFECTED),
        "TEST_OEO_UNAFFECTED"
    );
}

/// 135-2020 Clause 21 (`BACnetEscalatorOperationDirection ::= ENUMERATED`):
/// the values are direction *and* speed combined (reduced vs rated), not a
/// plain up/down triplet.
#[test]
fn escalator_operation_direction_values_match_clause_21() {
    assert_production_values!(
        EscalatorOperationDirection,
        [
            ("UNKNOWN", 0),
            ("STOPPED", 1),
            ("UP_RATED_SPEED", 2),
            ("UP_REDUCED_SPEED", 3),
            ("DOWN_RATED_SPEED", 4),
            ("DOWN_REDUCED_SPEED", 5),
        ],
    );
}

/// 135-2020 Clause 21 (`BACnetBinaryLightingPV ::= ENUMERATED`): the set is
/// off/on/warn/warn-off/warn-relinquish/stop. There is no "fade-on" value —
/// 4 is warn-relinquish and 5 is stop.
#[test]
fn binary_lighting_pv_values_match_clause_21() {
    assert_production_values!(
        BinaryLightingPV,
        [
            ("OFF", 0),
            ("ON", 1),
            ("WARN", 2),
            ("WARN_OFF", 3),
            ("WARN_RELINQUISH", 4),
            ("STOP", 5),
        ],
    );
}

/// 135-2020 Clause 21 (`BACnetLightingTransition ::= ENUMERATED`).
#[test]
fn lighting_transition_values_match_clause_21() {
    assert_production_values!(LightingTransition, [("NONE", 0), ("FADE", 1), ("RAMP", 2)]);
}

/// 135-2020 Clause 21 (`BACnetDoorValue ::= ENUMERATED`): a closed set of
/// four, exactly matching the domain Access Door writes validate against.
#[test]
fn door_value_values_match_clause_21() {
    assert_production_values!(
        DoorValue,
        [
            ("LOCK", 0),
            ("UNLOCK", 1),
            ("PULSE_UNLOCK", 2),
            ("EXTENDED_PULSE_UNLOCK", 3),
        ],
    );
}

/// 135-2020 Clause 21 (`BACnetAuthenticationStatus ::= ENUMERATED`): 1 is
/// ready, *not* "waiting" — the wait-for-* states start at 3.
#[test]
fn authentication_status_values_match_clause_21() {
    assert_production_values!(
        AuthenticationStatus,
        [
            ("NOT_READY", 0),
            ("READY", 1),
            ("DISABLED", 2),
            ("WAITING_FOR_AUTHENTICATION_FACTOR", 3),
            ("WAITING_FOR_ACCOMPANIMENT", 4),
            ("WAITING_FOR_VERIFICATION", 5),
            ("IN_PROGRESS", 6),
        ],
    );
}

/// 135-2020 Clause 21 (`BACnetAuthorizationExemption ::= ENUMERATED`).
#[test]
fn authorization_exemption_values_match_clause_21() {
    assert_production_values!(
        AuthorizationExemption,
        [
            ("PASSBACK", 0),
            ("OCCUPANCY_CHECK", 1),
            ("ACCESS_RIGHTS", 2),
            ("LOCKOUT", 3),
            ("DENY", 4),
            ("VERIFICATION", 5),
            ("AUTHORIZATION_DELAY", 6),
        ],
    );
}

/// 135-2020 Clause 21 (`BACnetAccessZoneOccupancyState ::= ENUMERATED`).
#[test]
fn access_zone_occupancy_state_values_match_clause_21() {
    assert_production_values!(
        AccessZoneOccupancyState,
        [
            ("NORMAL", 0),
            ("BELOW_LOWER_LIMIT", 1),
            ("AT_LOWER_LIMIT", 2),
            ("AT_UPPER_LIMIT", 3),
            ("ABOVE_UPPER_LIMIT", 4),
            ("DISABLED", 5),
            ("NOT_SUPPORTED", 6),
        ],
    );
}

/// 135-2020 Clause 21 (`BACnetProgramError ::= ENUMERATED`), the type of the
/// Program object's Reason_For_Halt.
#[test]
fn program_error_values_match_clause_21() {
    assert_production_values!(
        ProgramError,
        [
            ("NORMAL", 0),
            ("LOAD_FAILED", 1),
            ("INTERNAL", 2),
            ("PROGRAM", 3),
            ("OTHER", 4),
        ],
    );
}

/// 135-2020 Clause 21 (`BACnetRestartReason ::= ENUMERATED`), the type of the
/// Device object's Last_Restart_Reason.
#[test]
fn restart_reason_values_match_clause_21() {
    assert_production_values!(
        RestartReason,
        [
            ("UNKNOWN", 0),
            ("COLDSTART", 1),
            ("WARMSTART", 2),
            ("DETECTED_POWER_LOST", 3),
            ("DETECTED_POWERED_OFF", 4),
            ("HARDWARE_WATCHDOG", 5),
            ("SOFTWARE_WATCHDOG", 6),
            ("SUSPENDED", 7),
            ("ACTIVATE_CHANGES", 8),
        ],
    );
}

/// 135-2020 Clause 21 (`BACnetMaintenance ::= ENUMERATED`).
#[test]
fn maintenance_values_match_clause_21() {
    assert_production_values!(
        Maintenance,
        [
            ("NONE", 0),
            ("PERIODIC_TEST", 1),
            ("NEED_SERVICE_OPERATIONAL", 2),
            ("NEED_SERVICE_INOPERATIVE", 3),
        ],
    );
}

/// 135-2020 Clause 21 (`BACnetRelationship ::= ENUMERATED`): after
/// unknown/default, every value is one half of an even/odd forward/reverse
/// pair, so `n ^ 1` must always name the opposite relationship.
#[test]
fn relationship_values_match_clause_21() {
    assert_production_values!(
        Relationship,
        [
            ("UNKNOWN", 0),
            ("DEFAULT", 1),
            ("CONTAINS", 2),
            ("CONTAINED_BY", 3),
            ("USES", 4),
            ("USED_BY", 5),
            ("COMMANDS", 6),
            ("COMMANDED_BY", 7),
            ("ADJUSTS", 8),
            ("ADJUSTED_BY", 9),
            ("INGRESS", 10),
            ("EGRESS", 11),
            ("SUPPLIES_AIR", 12),
            ("RECEIVES_AIR", 13),
            ("SUPPLIES_HOT_AIR", 14),
            ("RECEIVES_HOT_AIR", 15),
            ("SUPPLIES_COOL_AIR", 16),
            ("RECEIVES_COOL_AIR", 17),
            ("SUPPLIES_POWER", 18),
            ("RECEIVES_POWER", 19),
            ("SUPPLIES_GAS", 20),
            ("RECEIVES_GAS", 21),
            ("SUPPLIES_WATER", 22),
            ("RECEIVES_WATER", 23),
            ("SUPPLIES_HOT_WATER", 24),
            ("RECEIVES_HOT_WATER", 25),
            ("SUPPLIES_COOL_WATER", 26),
            ("RECEIVES_COOL_WATER", 27),
            ("SUPPLIES_STEAM", 28),
            ("RECEIVES_STEAM", 29),
        ],
    );
    // The forward/reverse pairing is structural in the production.
    for pair in Relationship::ALL_NAMED[2..].chunks_exact(2) {
        let (fwd_name, fwd) = pair[0];
        let (rev_name, rev) = pair[1];
        assert_eq!(fwd.to_raw() ^ 1, rev.to_raw(), "{fwd_name} / {rev_name}");
        assert!(
            fwd_name.starts_with("SUPPLIES") || rev_name.ends_with("_BY") || rev_name == "EGRESS"
        );
    }
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
// ResolvedEnum arms added for #253
// ---------------------------------------------------------------------------

/// Each property identifier mapped in `resolve.rs` by the #253 additions
/// promotes a raw number to its named enumeration.
#[test]
fn resolves_253_properties_by_name() {
    let cases: [(PropertyIdentifier, u32, ResolvedEnum, &str); 12] = [
        (
            PropertyIdentifier::OPERATION_DIRECTION,
            2,
            ResolvedEnum::EscalatorOperationDirection(EscalatorOperationDirection::UP_RATED_SPEED),
            "UP_RATED_SPEED",
        ),
        (
            PropertyIdentifier::LAST_RESTART_REASON,
            5,
            ResolvedEnum::RestartReason(RestartReason::HARDWARE_WATCHDOG),
            "HARDWARE_WATCHDOG",
        ),
        (
            PropertyIdentifier::REASON_FOR_HALT,
            1,
            ResolvedEnum::ProgramError(ProgramError::LOAD_FAILED),
            "LOAD_FAILED",
        ),
        (
            PropertyIdentifier::AUTHENTICATION_STATUS,
            3,
            ResolvedEnum::AuthenticationStatus(
                AuthenticationStatus::WAITING_FOR_AUTHENTICATION_FACTOR,
            ),
            "WAITING_FOR_AUTHENTICATION_FACTOR",
        ),
        (
            PropertyIdentifier::MAINTENANCE_REQUIRED,
            2,
            ResolvedEnum::Maintenance(Maintenance::NEED_SERVICE_OPERATIONAL),
            "NEED_SERVICE_OPERATIONAL",
        ),
        (
            PropertyIdentifier::SUBORDINATE_RELATIONSHIPS,
            12,
            ResolvedEnum::Relationship(Relationship::SUPPLIES_AIR),
            "SUPPLIES_AIR",
        ),
        (
            PropertyIdentifier::DEFAULT_SUBORDINATE_RELATIONSHIP,
            1,
            ResolvedEnum::Relationship(Relationship::DEFAULT),
            "DEFAULT",
        ),
        (
            PropertyIdentifier::OCCUPANCY_STATE,
            4,
            ResolvedEnum::AccessZoneOccupancyState(AccessZoneOccupancyState::ABOVE_UPPER_LIMIT),
            "ABOVE_UPPER_LIMIT",
        ),
        (
            PropertyIdentifier::AUTHORIZATION_EXEMPTIONS,
            0,
            ResolvedEnum::AuthorizationExemption(AuthorizationExemption::PASSBACK),
            "PASSBACK",
        ),
        (
            PropertyIdentifier::TRANSITION,
            1,
            ResolvedEnum::LightingTransition(LightingTransition::FADE),
            "FADE",
        ),
        (
            PropertyIdentifier::TRACKING_VALUE,
            25,
            ResolvedEnum::LifeSafetyState(LifeSafetyState::OEO_UNAVAILABLE),
            "OEO_UNAVAILABLE",
        ),
        (
            PropertyIdentifier::TRACKING_VALUE,
            33,
            ResolvedEnum::LifeSafetyState(LifeSafetyState::TEST_OEO_EVACUATE),
            "TEST_OEO_EVACUATE",
        ),
    ];
    for (property, raw, expected, display) in cases {
        let resolved = ResolvedEnum::from_property(property, raw);
        assert_eq!(resolved, expected, "{property}");
        assert_eq!(resolved.to_string(), display, "{property}");
    }
}

/// Out-of-production values stay named-but-numeric through the arms (the
/// newtype keeps the raw number; Display falls back to it).
#[test]
fn resolves_253_properties_unknown_values_stay_numeric() {
    let r = ResolvedEnum::from_property(PropertyIdentifier::OPERATION_DIRECTION, 42);
    assert_eq!(
        r,
        ResolvedEnum::EscalatorOperationDirection(EscalatorOperationDirection::from_raw(42))
    );
    assert_eq!(r.to_string(), "42");
}

/// The two list-valued properties (subordinate-relationships is a
/// BACnetARRAY, authorization-exemptions a BACnetLIST) resolve each element
/// through the same arm via `resolve_value`'s recursion.
#[test]
fn resolves_253_list_properties_elementwise() {
    let list = PropertyValue::List(vec![
        PropertyValue::Enumerated(2),  // contains
        PropertyValue::Enumerated(28), // supplies-steam
    ]);
    let ResolvedValue::List(items) =
        resolve_value(PropertyIdentifier::SUBORDINATE_RELATIONSHIPS, list)
    else {
        panic!("expected list");
    };
    assert_eq!(
        items,
        [
            ResolvedValue::Enumerated(ResolvedEnum::Relationship(Relationship::CONTAINS)),
            ResolvedValue::Enumerated(ResolvedEnum::Relationship(Relationship::SUPPLIES_STEAM)),
        ]
    );

    let list = PropertyValue::List(vec![
        PropertyValue::Enumerated(3), // lockout
        PropertyValue::Enumerated(5), // verification
    ]);
    let ResolvedValue::List(items) =
        resolve_value(PropertyIdentifier::AUTHORIZATION_EXEMPTIONS, list)
    else {
        panic!("expected list");
    };
    assert_eq!(
        items,
        [
            ResolvedValue::Enumerated(ResolvedEnum::AuthorizationExemption(
                AuthorizationExemption::LOCKOUT
            )),
            ResolvedValue::Enumerated(ResolvedEnum::AuthorizationExemption(
                AuthorizationExemption::VERIFICATION
            )),
        ]
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
        EscalatorOperationDirection,
        BinaryLightingPV,
        LightingTransition,
        DoorValue,
        AuthenticationStatus,
        AuthorizationExemption,
        AccessZoneOccupancyState,
        ProgramError,
        RestartReason,
        Maintenance,
        Relationship,
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
