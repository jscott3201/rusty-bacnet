// ===========================================================================
// Enumerated resolution: raw number -> named enum, keyed by property
// ===========================================================================
//
// On the wire a BACnet `Enumerated` is just a number (see
// `PropertyValue::Enumerated(u32)`). The number carries no information about
// *which* enumeration it belongs to; that is determined by the property being
// read. ASHRAE 135 Clause 12 (object properties) and Clause 21 (application
// types) fix, per property, the concrete `BACnetXxx` ENUMERATED type. This
// module encodes that property -> enum-type mapping so a decoded value can be
// promoted from a bare number to its named variant.
//
// Properties whose ENUMERATED type depends on the *object type* rather than the
// property alone (e.g. `present-value`, plural `alarm-values`, and
// `fault-values`, whose type varies between Binary/Multi-state/Life-Safety
// objects) are intentionally
// left unmapped: the property identifier alone is insufficient to name them
// correctly, so they resolve to `Unknown`.
//
// One deliberate exception: `tracking-value` (164) is object-type-dependent
// (Lighting Output types it REAL), but `BACnetLifeSafetyState` is its only
// ENUMERATED form in the standard, so promoting the `Enumerated` case is
// unambiguous and the arm is kept.

#[cfg(not(feature = "std"))]
use alloc::{string::String, vec::Vec};

use crate::bitstring::{EventTransitionBits, LimitEnable, ObjectTypesSupported, ServicesSupported};
use crate::primitives::{Date, ObjectIdentifier, PropertyValue, StatusFlags, Time};

use super::*;

macro_rules! resolved_enum {
    ($($variant:ident($ty:ty)),+ $(,)?) => {
        /// A BACnet `Enumerated` value promoted to its named enumeration.
        ///
        /// Produced by [`ResolvedEnum::from_property`], which uses the property
        /// identifier to decide which `BACnetXxx` enum a raw number denotes.
        /// Values whose enumeration cannot be determined from the property alone
        /// (vendor-proprietary or object-type-dependent properties) become
        /// [`ResolvedEnum::Unknown`], preserving the raw number.
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub enum ResolvedEnum {
            $(
                #[doc = concat!("A [`", stringify!($ty), "`] value.")]
                $variant($ty),
            )+
            /// No known enumeration for this property; the raw wire value is kept.
            Unknown(u32),
        }

        impl core::fmt::Display for ResolvedEnum {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                match self {
                    $( Self::$variant(v) => core::fmt::Display::fmt(v, f), )+
                    Self::Unknown(n) => write!(f, "{n}"),
                }
            }
        }
    };
}

resolved_enum! {
    ObjectType(ObjectType),
    EventState(EventState),
    EventType(EventType),
    NotifyType(NotifyType),
    Reliability(Reliability),
    DeviceStatus(DeviceStatus),
    Segmentation(Segmentation),
    EngineeringUnits(EngineeringUnits),
    Polarity(Polarity),
    BinaryPV(BinaryPV),
    ProgramState(ProgramState),
    ProgramChange(ProgramChange),
    ProgramError(ProgramError),
    NodeType(NodeType),
    LoggingType(LoggingType),
    FileAccessMethod(FileAccessMethod),
    BackupAndRestoreState(BackupAndRestoreState),
    RestartReason(RestartReason),
    LifeSafetyMode(LifeSafetyMode),
    LifeSafetyState(LifeSafetyState),
    LifeSafetyOperation(LifeSafetyOperation),
    SilencedState(SilencedState),
    Maintenance(Maintenance),
    DoorStatus(DoorStatus),
    DoorAlarmState(DoorAlarmState),
    DoorSecuredStatus(DoorSecuredStatus),
    LockStatus(LockStatus),
    AccessEvent(AccessEvent),
    AuthorizationMode(AuthorizationMode),
    AccessPassbackMode(AccessPassbackMode),
    AccessUserType(AccessUserType),
    AccessCredentialDisable(AccessCredentialDisable),
    AccessCredentialDisableReason(AccessCredentialDisableReason),
    AuthenticationStatus(AuthenticationStatus),
    AuthorizationExemption(AuthorizationExemption),
    AccessZoneOccupancyState(AccessZoneOccupancyState),
    Relationship(Relationship),
    NetworkType(NetworkType),
    NetworkNumberQuality(NetworkNumberQuality),
    IPMode(IPMode),
    NetworkPortCommand(NetworkPortCommand),
    ProtocolLevel(ProtocolLevel),
    LightingInProgress(LightingInProgress),
    LightingTransition(LightingTransition),
    TimerState(TimerState),
    TimerTransition(TimerTransition),
    WriteStatus(WriteStatus),
    LiftCarMode(LiftCarMode),
    LiftGroupMode(LiftGroupMode),
    EscalatorMode(EscalatorMode),
    EscalatorOperationDirection(EscalatorOperationDirection),
    LiftCarDriveStatus(LiftCarDriveStatus),
    LiftCarDirection(LiftCarDirection),
    LiftCarDoorCommand(LiftCarDoorCommand),
    AuditLevel(AuditLevel),
}

impl ResolvedEnum {
    /// Promote a raw `Enumerated` wire value to its named enumeration using the
    /// property it was read from.
    ///
    /// The property numbers below are the identifiers defined in
    /// [`PropertyIdentifier`] (ASHRAE 135 Clause 21). A property with no known
    /// scalar ENUMERATED type — including vendor-proprietary properties and ones
    /// whose type depends on the object type — yields [`ResolvedEnum::Unknown`].
    pub fn from_property(property: PropertyIdentifier, value: u32) -> Self {
        match property.to_raw() {
            79 => Self::ObjectType(ObjectType::from_raw(value)), // object-type
            36 => Self::EventState(EventState::from_raw(value)), // event-state
            37 => Self::EventType(EventType::from_raw(value)),   // event-type
            72 => Self::NotifyType(NotifyType::from_raw(value)), // notify-type
            103 => Self::Reliability(Reliability::from_raw(value)), // reliability
            112 => Self::DeviceStatus(DeviceStatus::from_raw(value)), // system-status
            // segmentation-supported: Segmentation is the one u8-backed enum,
            // so out-of-range wire values fall through to `Unknown` instead of
            // wrapping into a valid-looking named variant.
            107 if value <= u8::MAX as u32 => {
                Self::Segmentation(Segmentation::from_raw(value as u8))
            }

            // engineering-units: units + the various *-units properties.
            20 | 27 | 50 | 82 | 94 | 117 | 455 => {
                Self::EngineeringUnits(EngineeringUnits::from_raw(value))
            }

            84 => Self::Polarity(Polarity::from_raw(value)), // polarity
            // alarm-value is BACnetBinaryPV on both object types that define it.
            6 => Self::BinaryPV(BinaryPV::from_raw(value)),
            92 => Self::ProgramState(ProgramState::from_raw(value)), // program-state
            90 => Self::ProgramChange(ProgramChange::from_raw(value)), // program-change
            100 => Self::ProgramError(ProgramError::from_raw(value)), // reason-for-halt
            208 => Self::NodeType(NodeType::from_raw(value)),        // node-type
            197 => Self::LoggingType(LoggingType::from_raw(value)),  // logging-type
            41 => Self::FileAccessMethod(FileAccessMethod::from_raw(value)), // file-access-method
            338 => Self::BackupAndRestoreState(BackupAndRestoreState::from_raw(value)), // backup-and-restore-state
            196 => Self::RestartReason(RestartReason::from_raw(value)), // last-restart-reason

            // Life safety.
            160 | 175 => Self::LifeSafetyMode(LifeSafetyMode::from_raw(value)), // mode / accepted-modes
            // tracking-value: BACnetLifeSafetyState is its only ENUMERATED
            // form; Lighting Output's REAL never reaches this arm.
            164 => Self::LifeSafetyState(LifeSafetyState::from_raw(value)),
            161 => Self::LifeSafetyOperation(LifeSafetyOperation::from_raw(value)), // operation-expected
            163 => Self::SilencedState(SilencedState::from_raw(value)),             // silenced
            // maintenance-required: BACnetMaintenance on life safety
            // point/zone (12.15/12.16) and access door (12.26).
            158 => Self::Maintenance(Maintenance::from_raw(value)),

            // Access door.
            231 => Self::DoorStatus(DoorStatus::from_raw(value)), // door-status
            226 => Self::DoorAlarmState(DoorAlarmState::from_raw(value)), // door-alarm-state
            233 => Self::LockStatus(LockStatus::from_raw(value)), // lock-status
            235 => Self::DoorSecuredStatus(DoorSecuredStatus::from_raw(value)), // secured-status

            // Access control.
            247 | 275 => Self::AccessEvent(AccessEvent::from_raw(value)), // access-event / last-access-event
            261 => Self::AuthorizationMode(AuthorizationMode::from_raw(value)), // authorization-mode
            300 => Self::AccessPassbackMode(AccessPassbackMode::from_raw(value)), // passback-mode
            318 => Self::AccessUserType(AccessUserType::from_raw(value)),       // user-type
            263 => Self::AccessCredentialDisable(AccessCredentialDisable::from_raw(value)), // credential-disable
            303 => {
                Self::AccessCredentialDisableReason(AccessCredentialDisableReason::from_raw(value))
            } // reason-for-disable
            260 => Self::AuthenticationStatus(AuthenticationStatus::from_raw(value)), // authentication-status
            296 => Self::AccessZoneOccupancyState(AccessZoneOccupancyState::from_raw(value)), // occupancy-state
            // authorization-exemptions is BACnetLIST of the exemption type;
            // resolve_value recurses into list elements with the same arm.
            364 => Self::AuthorizationExemption(AuthorizationExemption::from_raw(value)),

            // Network port.
            427 => Self::NetworkType(NetworkType::from_raw(value)), // network-type
            426 => Self::NetworkNumberQuality(NetworkNumberQuality::from_raw(value)), // network-number-quality
            408 | 435 => Self::IPMode(IPMode::from_raw(value)), // bacnet-ip-mode / bacnet-ipv6-mode
            417 => Self::NetworkPortCommand(NetworkPortCommand::from_raw(value)), // command
            482 => Self::ProtocolLevel(ProtocolLevel::from_raw(value)), // protocol-level

            // Structured view (12.29): subordinate-relationships is
            // BACnetARRAY[N] of BACnetRelationship (resolve_value recurses
            // into elements); default-subordinate-relationship is the scalar
            // companion. Represents (491) is BACnetDeviceObjectReference, so
            // it has no enumerating arm here.
            489 | 490 => Self::Relationship(Relationship::from_raw(value)),

            // Lighting / timer / channel.
            378 => Self::LightingInProgress(LightingInProgress::from_raw(value)), // in-progress
            385 => Self::LightingTransition(LightingTransition::from_raw(value)), // transition
            398 => Self::TimerState(TimerState::from_raw(value)),                 // timer-state
            395 => Self::TimerTransition(TimerTransition::from_raw(value)), // last-state-change
            370 => Self::WriteStatus(WriteStatus::from_raw(value)),         // write-status

            // Lift / escalator.
            456 => Self::LiftCarMode(LiftCarMode::from_raw(value)), // car-mode
            467 => Self::LiftGroupMode(LiftGroupMode::from_raw(value)), // group-mode
            462 => Self::EscalatorMode(EscalatorMode::from_raw(value)), // escalator-mode
            477 => Self::EscalatorOperationDirection(EscalatorOperationDirection::from_raw(value)), // operation-direction
            453 => Self::LiftCarDriveStatus(LiftCarDriveStatus::from_raw(value)), // car-drive-status
            448 | 457 => Self::LiftCarDirection(LiftCarDirection::from_raw(value)), // car-assigned/moving-direction
            449 => Self::LiftCarDoorCommand(LiftCarDoorCommand::from_raw(value)), // car-door-command
            // car-door-status is `BACnetARRAY[N] of BACnetDoorStatus` (Lift
            // object, Clause 12); the standard defines no lift-specific door
            // status enumeration.
            450 => Self::DoorStatus(DoorStatus::from_raw(value)),

            // Audit.
            498 => Self::AuditLevel(AuditLevel::from_raw(value)), // audit-level

            _ => Self::Unknown(value),
        }
    }
}

/// Finish the job that `decode_application_value` starts: if `value` is an
/// `Enumerated`, resolve it to its named enumeration for `property`.
///
/// Returns `None` for any non-enumerated value (there is nothing to promote).
/// For a `PropertyValue::List`, resolve each element with
/// [`ResolvedEnum::from_property`] yourself — the element enumeration is the
/// same as the scalar case for that property.
///
/// ```
/// use bacnet_types::enums::{resolve_enum, PropertyIdentifier, ResolvedEnum};
/// use bacnet_types::primitives::PropertyValue;
///
/// // Suppose `decode_application_value` returned this for the `object-type`
/// // property of some object:
/// let decoded = PropertyValue::Enumerated(19);
///
/// match resolve_enum(PropertyIdentifier::OBJECT_TYPE, &decoded) {
///     Some(ResolvedEnum::ObjectType(t)) => {
///         assert_eq!(t, bacnet_types::enums::ObjectType::MULTI_STATE_VALUE);
///         assert_eq!(t.to_string(), "MULTI_STATE_VALUE");
///     }
///     _ => unreachable!(),
/// }
/// ```
pub fn resolve_enum(property: PropertyIdentifier, value: &PropertyValue) -> Option<ResolvedEnum> {
    match value {
        PropertyValue::Enumerated(n) => Some(ResolvedEnum::from_property(property, *n)),
        _ => None,
    }
}

/// A BACnet `BitString` promoted to its named bit-string type, keyed by property.
///
/// The bit-string analogue of [`ResolvedEnum`]: on the wire a `BitString` is
/// just `(unused_bits, data)` bytes with no type identity, so the property
/// decides which `BACnetXxx` BIT STRING it is (ASHRAE 135 Clause 21). A property
/// with no known named bit-string type yields [`ResolvedBits::Unknown`],
/// preserving the raw bytes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ResolvedBits {
    /// `status-flags` / `member-status-flags` → [`StatusFlags`].
    StatusFlags(StatusFlags),
    /// `event-enable` / `acked-transitions` / `ack-required` → [`EventTransitionBits`].
    EventTransitionBits(EventTransitionBits),
    /// `limit-enable` → [`LimitEnable`].
    LimitEnable(LimitEnable),
    /// `protocol-services-supported` → [`ServicesSupported`].
    ServicesSupported(ServicesSupported),
    /// `protocol-object-types-supported` → [`ObjectTypesSupported`].
    ObjectTypesSupported(ObjectTypesSupported),
    /// No known named bit string for this property; raw bytes preserved.
    Unknown {
        /// Number of unused bits in the last byte.
        unused_bits: u8,
        /// The bit data bytes.
        data: Vec<u8>,
    },
}

impl ResolvedBits {
    /// Promote a raw `BitString` payload to its named bit-string type using the
    /// property it was read from.
    ///
    /// `unused_bits` and `data` are the fields of
    /// [`PropertyValue::BitString`]. A property with no known named bit-string
    /// type — vendor-proprietary or object-type-dependent — yields
    /// [`ResolvedBits::Unknown`] with the bytes intact.
    pub fn from_property(property: PropertyIdentifier, unused_bits: u8, data: &[u8]) -> Self {
        match property.to_raw() {
            // status-flags / member-status-flags.
            111 | 347 => Self::StatusFlags(crate::bitstring::status_flags_from_bacnet(data)),

            // event-enable / acked-transitions / ack-required.
            0 | 1 | 35 => Self::EventTransitionBits(EventTransitionBits::from_bacnet(data)),

            // limit-enable.
            52 => Self::LimitEnable(LimitEnable::from_bacnet(data)),

            // protocol-services-supported.
            97 => Self::ServicesSupported(ServicesSupported::from_bacnet(data)),

            // protocol-object-types-supported.
            96 => Self::ObjectTypesSupported(ObjectTypesSupported::from_bacnet(data)),

            _ => Self::Unknown {
                unused_bits,
                data: data.to_vec(),
            },
        }
    }
}

impl core::fmt::Display for ResolvedBits {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::StatusFlags(v) => core::fmt::Display::fmt(v, f),
            Self::EventTransitionBits(v) => core::fmt::Display::fmt(v, f),
            Self::LimitEnable(v) => core::fmt::Display::fmt(v, f),
            Self::ServicesSupported(v) => core::fmt::Display::fmt(v, f),
            Self::ObjectTypesSupported(v) => core::fmt::Display::fmt(v, f),
            Self::Unknown { data, .. } => {
                f.write_str("0x")?;
                for byte in data {
                    write!(f, "{byte:02X}")?;
                }
                Ok(())
            }
        }
    }
}

/// Bit-string analogue of [`resolve_enum`]: if `value` is a `BitString`, resolve
/// it to its named bit-string type for `property`; otherwise return `None`.
pub fn resolve_bits(property: PropertyIdentifier, value: &PropertyValue) -> Option<ResolvedBits> {
    match value {
        PropertyValue::BitString { unused_bits, data } => {
            Some(ResolvedBits::from_property(property, *unused_bits, data))
        }
        _ => None,
    }
}

/// A decoded property value with every `Enumerated` promoted to its named enum.
///
/// This mirrors [`PropertyValue`] one-to-one, except that
/// [`PropertyValue::Enumerated`]'s bare `u32` is replaced by a resolved
/// [`ResolvedEnum`], and [`PropertyValue::BitString`]'s raw bytes by a resolved
/// [`ResolvedBits`]. Every other variant carries exactly the same data as its
/// `PropertyValue` counterpart. Build one with [`resolve_value`].
#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedValue {
    /// Null value.
    Null,
    /// Boolean value.
    Boolean(bool),
    /// Unsigned integer (up to 64-bit for BACnet Unsigned64).
    Unsigned(u64),
    /// Signed integer.
    Signed(i32),
    /// IEEE 754 single-precision float.
    Real(f32),
    /// IEEE 754 double-precision float.
    Double(f64),
    /// Octet string (raw bytes).
    OctetString(Vec<u8>),
    /// Character string (UTF-8).
    CharacterString(String),
    /// Bit string, resolved to its named bit-string type for the property.
    BitString(ResolvedBits),
    /// Enumerated value, resolved to its named enumeration for the property.
    Enumerated(ResolvedEnum),
    /// Date value.
    Date(Date),
    /// Time value.
    Time(Time),
    /// Object identifier.
    ObjectIdentifier(ObjectIdentifier),
    /// A sequence (array) of resolved values.
    List(Vec<ResolvedValue>),
    /// Raw, already-encoded application-layer bytes (see
    /// [`crate::primitives::PropertyValue::ApplicationData`]) — there is no
    /// enum/bit-string naming for opaque context-tagged content, so it
    /// carries through unchanged.
    ApplicationData(Vec<u8>),
}

/// Finish the job `decode_application_value` starts: take its decoded
/// [`PropertyValue`] and return the same value with every `Enumerated` promoted
/// to its named [`ResolvedEnum`] for `property`. All non-enumerated data is
/// carried through unchanged. `List` elements are resolved recursively against
/// the same property.
///
/// ```
/// use bacnet_types::enums::{resolve_value, PropertyIdentifier, ResolvedEnum, ResolvedValue, ObjectType};
/// use bacnet_types::primitives::PropertyValue;
///
/// // What decode_application_value() gave you for the `object-type` property:
/// let decoded = PropertyValue::Enumerated(19);
///
/// let resolved = resolve_value(PropertyIdentifier::OBJECT_TYPE, decoded);
/// assert_eq!(
///     resolved,
///     ResolvedValue::Enumerated(ResolvedEnum::ObjectType(ObjectType::MULTI_STATE_VALUE)),
/// );
///
/// // Non-enum data is untouched:
/// let n = resolve_value(PropertyIdentifier::PRESENT_VALUE, PropertyValue::Real(21.5));
/// assert_eq!(n, ResolvedValue::Real(21.5));
/// ```
pub fn resolve_value(property: PropertyIdentifier, value: PropertyValue) -> ResolvedValue {
    match value {
        PropertyValue::Null => ResolvedValue::Null,
        PropertyValue::Boolean(v) => ResolvedValue::Boolean(v),
        PropertyValue::Unsigned(v) => ResolvedValue::Unsigned(v),
        PropertyValue::Signed(v) => ResolvedValue::Signed(v),
        PropertyValue::Real(v) => ResolvedValue::Real(v),
        PropertyValue::Double(v) => ResolvedValue::Double(v),
        PropertyValue::OctetString(v) => ResolvedValue::OctetString(v),
        PropertyValue::CharacterString(v) => ResolvedValue::CharacterString(v),
        PropertyValue::BitString { unused_bits, data } => {
            ResolvedValue::BitString(ResolvedBits::from_property(property, unused_bits, &data))
        }
        PropertyValue::Enumerated(n) => {
            ResolvedValue::Enumerated(ResolvedEnum::from_property(property, n))
        }
        PropertyValue::Date(v) => ResolvedValue::Date(v),
        PropertyValue::Time(v) => ResolvedValue::Time(v),
        PropertyValue::ObjectIdentifier(v) => ResolvedValue::ObjectIdentifier(v),
        PropertyValue::List(values) => ResolvedValue::List(
            values
                .into_iter()
                .map(|v| resolve_value(property, v))
                .collect(),
        ),
        PropertyValue::ApplicationData(bytes) => ResolvedValue::ApplicationData(bytes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_object_type() {
        let r = ResolvedEnum::from_property(PropertyIdentifier::OBJECT_TYPE, 19);
        assert_eq!(r, ResolvedEnum::ObjectType(ObjectType::MULTI_STATE_VALUE));
        assert_eq!(r.to_string(), "MULTI_STATE_VALUE");
    }

    #[test]
    fn resolves_units_across_aliases() {
        for prop in [
            PropertyIdentifier::UNITS,
            PropertyIdentifier::OUTPUT_UNITS,
            PropertyIdentifier::CAR_LOAD_UNITS,
        ] {
            assert!(matches!(
                ResolvedEnum::from_property(prop, 0),
                ResolvedEnum::EngineeringUnits(_)
            ));
        }
    }

    #[test]
    fn unmapped_property_is_unknown() {
        // present-value is object-type-dependent, so it stays Unknown.
        let r = ResolvedEnum::from_property(PropertyIdentifier::PRESENT_VALUE, 7);
        assert_eq!(r, ResolvedEnum::Unknown(7));
        assert_eq!(r.to_string(), "7");
    }

    #[test]
    fn alarm_value_resolves_to_binary_pv() {
        assert_eq!(
            ResolvedEnum::from_property(PropertyIdentifier::ALARM_VALUE, 1),
            ResolvedEnum::BinaryPV(BinaryPV::ACTIVE),
        );
    }

    #[test]
    fn plural_alarm_and_fault_values_remain_unknown() {
        assert_eq!(
            ResolvedEnum::from_property(PropertyIdentifier::ALARM_VALUES, 1),
            ResolvedEnum::Unknown(1)
        );
        assert_eq!(
            ResolvedEnum::from_property(PropertyIdentifier::FAULT_VALUES, 1),
            ResolvedEnum::Unknown(1)
        );
    }

    #[test]
    fn vendor_object_type_passes_through() {
        // 128 is a vendor-proprietary object type: named enum keeps the number.
        let r = ResolvedEnum::from_property(PropertyIdentifier::OBJECT_TYPE, 128);
        assert_eq!(r, ResolvedEnum::ObjectType(ObjectType::from_raw(128)));
        assert_eq!(r.to_string(), "128");
    }

    #[test]
    fn resolve_value_promotes_enum_and_keeps_rest() {
        // Enum gets a named variant.
        assert_eq!(
            resolve_value(
                PropertyIdentifier::EVENT_STATE,
                PropertyValue::Enumerated(0)
            ),
            ResolvedValue::Enumerated(ResolvedEnum::EventState(EventState::from_raw(0))),
        );
        // Non-enum data passes through byte-for-byte.
        assert_eq!(
            resolve_value(
                PropertyIdentifier::OBJECT_NAME,
                PropertyValue::CharacterString("AI-1".into())
            ),
            ResolvedValue::CharacterString("AI-1".into()),
        );
    }

    #[test]
    fn resolve_value_recurses_into_lists() {
        let list = PropertyValue::List(vec![
            PropertyValue::Enumerated(0),
            PropertyValue::Enumerated(1),
        ]);
        let ResolvedValue::List(items) = resolve_value(PropertyIdentifier::ACCEPTED_MODES, list)
        else {
            panic!("expected list");
        };
        assert!(items.iter().all(|i| matches!(
            i,
            ResolvedValue::Enumerated(ResolvedEnum::LifeSafetyMode(_))
        )));
    }

    #[test]
    fn resolves_status_flags_bitstring() {
        // 4-bit status-flags, 4 unused: wire byte 0x80 (in-alarm set), unused=4.
        let bits = ResolvedBits::from_property(PropertyIdentifier::STATUS_FLAGS, 4, &[0x80]);
        assert_eq!(bits, ResolvedBits::StatusFlags(StatusFlags::IN_ALARM));
        assert_eq!(bits.to_string(), "IN_ALARM");
    }

    #[test]
    fn status_flags_decode_ignores_declared_length() {
        // Clause 20.2.10 fixes bit 0 at the MSB of the first octet, so the
        // flags survive a peer that declares the wrong unused-bit count.
        for unused in [0, 4, 5] {
            assert_eq!(
                ResolvedBits::from_property(PropertyIdentifier::STATUS_FLAGS, unused, &[0xC0]),
                ResolvedBits::StatusFlags(StatusFlags::IN_ALARM | StatusFlags::FAULT),
            );
        }
    }

    #[test]
    fn segmentation_out_of_range_stays_unknown() {
        // Segmentation is u8-backed; a wire value past u8 must not wrap into a
        // valid-looking named variant (259 & 0xFF would be NONE).
        assert_eq!(
            ResolvedEnum::from_property(PropertyIdentifier::SEGMENTATION_SUPPORTED, 259),
            ResolvedEnum::Unknown(259),
        );
        assert_eq!(
            ResolvedEnum::from_property(PropertyIdentifier::SEGMENTATION_SUPPORTED, 3),
            ResolvedEnum::Segmentation(Segmentation::NONE),
        );
    }

    #[test]
    fn car_door_status_resolves_to_door_status() {
        // Car_Door_Status is BACnetARRAY[N] of BACnetDoorStatus (Lift object);
        // 135-2020 has no lift-specific door status enumeration.
        let r = ResolvedEnum::from_property(PropertyIdentifier::CAR_DOOR_STATUS, 3);
        assert_eq!(r, ResolvedEnum::DoorStatus(DoorStatus::DOOR_FAULT));
        assert_eq!(r.to_string(), "DOOR_FAULT");
        assert_eq!(
            ResolvedEnum::from_property(PropertyIdentifier::CAR_DOOR_STATUS, 0),
            ResolvedEnum::DoorStatus(DoorStatus::CLOSED),
        );
    }

    #[test]
    fn backup_and_restore_state_failure_states_resolve_by_name() {
        // Backup_And_Restore_State (338) legitimately reports both failure
        // states during Clause 19.1 procedures; they must not surface as
        // bare numbers.
        let r = ResolvedEnum::from_property(PropertyIdentifier::BACKUP_AND_RESTORE_STATE, 5);
        assert_eq!(
            r,
            ResolvedEnum::BackupAndRestoreState(BackupAndRestoreState::BACKUP_FAILURE)
        );
        assert_eq!(r.to_string(), "BACKUP_FAILURE");
        assert_eq!(
            ResolvedEnum::from_property(PropertyIdentifier::BACKUP_AND_RESTORE_STATE, 6)
                .to_string(),
            "RESTORE_FAILURE"
        );
    }

    #[test]
    fn resolves_limit_enable_and_event_enable() {
        assert!(matches!(
            ResolvedBits::from_property(PropertyIdentifier::LIMIT_ENABLE, 6, &[0xC0]),
            ResolvedBits::LimitEnable(_)
        ));
        assert!(matches!(
            ResolvedBits::from_property(PropertyIdentifier::EVENT_ENABLE, 5, &[0xE0]),
            ResolvedBits::EventTransitionBits(_)
        ));
    }

    #[test]
    fn resolves_object_types_supported() {
        let mut data = vec![0u8; 9];
        data[8] = 0x80; // bit 64 => ColorTemperature
        let bits = ResolvedBits::from_property(
            PropertyIdentifier::PROTOCOL_OBJECT_TYPES_SUPPORTED,
            7,
            &data,
        );
        match bits {
            ResolvedBits::ObjectTypesSupported(ots) => {
                assert!(ots.contains(ObjectType::COLOR_TEMPERATURE));
            }
            _ => panic!("expected ObjectTypesSupported"),
        }
    }

    #[test]
    fn unmapped_bitstring_is_unknown() {
        let bits = ResolvedBits::from_property(PropertyIdentifier::PROPERTY_LIST, 0, &[0xAB]);
        assert_eq!(
            bits,
            ResolvedBits::Unknown {
                unused_bits: 0,
                data: vec![0xAB],
            }
        );
        assert_eq!(bits.to_string(), "0xAB");
    }

    #[test]
    fn resolve_value_promotes_bitstring() {
        let v = resolve_value(
            PropertyIdentifier::STATUS_FLAGS,
            PropertyValue::BitString {
                unused_bits: 4,
                data: vec![0x40], // fault set
            },
        );
        assert_eq!(
            v,
            ResolvedValue::BitString(ResolvedBits::StatusFlags(StatusFlags::FAULT)),
        );
    }

    #[test]
    fn resolve_bits_ignores_non_bitstring() {
        assert!(resolve_bits(
            PropertyIdentifier::STATUS_FLAGS,
            &PropertyValue::Unsigned(1)
        )
        .is_none());
    }

    #[test]
    fn resolve_enum_ignores_non_enumerated() {
        assert!(resolve_enum(
            PropertyIdentifier::OBJECT_TYPE,
            &PropertyValue::Unsigned(19)
        )
        .is_none());
        assert!(matches!(
            resolve_enum(
                PropertyIdentifier::EVENT_STATE,
                &PropertyValue::Enumerated(0)
            ),
            Some(ResolvedEnum::EventState(_))
        ));
    }
}
