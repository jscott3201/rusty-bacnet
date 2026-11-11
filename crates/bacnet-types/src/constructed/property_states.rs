#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use crate::enums::{SCConnectionState, SCHubConnectorState};
use crate::error::Error;

const EXTENDED_VALUE_FACTOR: u32 = 100_000;

/// Property-state value for a choice tag greater than 254.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BACnetExtendedPropertyState {
    tag: u32,
    value: u32,
}

impl BACnetExtendedPropertyState {
    /// Build the tag-63 representation for a choice tag greater than 254.
    pub fn new(tag: u32, value: u32) -> Result<Self, Error> {
        if tag <= 254 {
            return Err(Error::decoding(
                0,
                "extended property-state tag must exceed 254",
            ));
        }
        if value >= EXTENDED_VALUE_FACTOR {
            return Err(Error::decoding(
                0,
                "extended property-state value must be below 100000",
            ));
        }
        tag.checked_mul(EXTENDED_VALUE_FACTOR)
            .and_then(|base| base.checked_add(value))
            .ok_or_else(|| Error::decoding(0, "extended property-state value exceeds u32"))?;
        Ok(Self { tag, value })
    }

    /// Decode the Unsigned32 carried by context tag 63.
    pub fn from_encoded(encoded: u32) -> Result<Self, Error> {
        Self::new(
            encoded / EXTENDED_VALUE_FACTOR,
            encoded % EXTENDED_VALUE_FACTOR,
        )
    }

    /// Return the choice tag represented by tag 63.
    pub const fn tag(self) -> u32 {
        self.tag
    }

    /// Return the vendor enumeration value.
    pub const fn value(self) -> u32 {
        self.value
    }

    /// Return the Unsigned32 encoded under context tag 63.
    pub fn encoded(self) -> u32 {
        self.tag * EXTENDED_VALUE_FACTOR + self.value
    }
}

/// A property-states choice above 254 the stack names: its tag, its name in
/// the standard's kebab case, and the enumeration its value belongs to.
///
/// Both are additions after revision 22 (135-2020) that ASHRAE's revision 24
/// data types name. Naming a choice changes display and text input only: the
/// wire form is the same `tag * 100000 + value` under context tag 63 for any
/// tag, named or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NamedExtendedChoice {
    /// The choice tag, as in `BACnetExtendedPropertyState::tag`.
    pub tag: u32,
    /// The choice's name, `sc-connection-state`.
    pub name: &'static str,
    /// The data type of the value, as the revision 24 data names it without
    /// the `0-` prefix: `BACnetSCConnectionState`.
    pub datatype: &'static str,
}

/// The extended choices the stack names, by tag.
pub const NAMED_EXTENDED_CHOICES: &[NamedExtendedChoice] = &[
    NamedExtendedChoice {
        tag: 258,
        name: "sc-connection-state",
        datatype: "BACnetSCConnectionState",
    },
    NamedExtendedChoice {
        tag: 259,
        name: "sc-hub-connector-state",
        datatype: "BACnetSCHubConnectorState",
    },
];

impl NamedExtendedChoice {
    /// The named choice with this tag, if the stack names it.
    pub fn from_tag(tag: u32) -> Option<&'static Self> {
        NAMED_EXTENDED_CHOICES.iter().find(|c| c.tag == tag)
    }

    /// The named choice with this name, ignoring case and `-`/`_` separators.
    pub fn from_name(name: &str) -> Option<&'static Self> {
        NAMED_EXTENDED_CHOICES
            .iter()
            .find(|c| crate::enums::names_eq(c.name, name))
    }

    /// The identifier of `value` in this choice's enumeration
    /// (`CONNECTED`), or `None` when the table doesn't name it.
    pub fn value_name(&self, value: u32) -> Option<&'static str> {
        match self.tag {
            258 => SCConnectionState::ALL_NAMED
                .iter()
                .find(|(_, v)| v.to_raw() == value)
                .map(|&(name, _)| name),
            259 => SCHubConnectorState::ALL_NAMED
                .iter()
                .find(|(_, v)| v.to_raw() == value)
                .map(|&(name, _)| name),
            _ => None,
        }
    }

    /// The value a name stands for in this choice's enumeration, ignoring
    /// case and `-`/`_` separators.
    pub fn value_from_name(&self, name: &str) -> Option<u32> {
        match self.tag {
            258 => SCConnectionState::ALL_NAMED
                .iter()
                .find(|(n, _)| crate::enums::names_eq(n, name))
                .map(|(_, v)| v.to_raw()),
            259 => SCHubConnectorState::ALL_NAMED
                .iter()
                .find(|(n, _)| crate::enums::names_eq(n, name))
                .map(|(_, v)| v.to_raw()),
            _ => None,
        }
    }
}

/// Vendor-defined property state using a context tag from 64 through 254.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BACnetProprietaryPropertyState {
    tag: u8,
    data: Vec<u8>,
    constructed: bool,
}

impl BACnetProprietaryPropertyState {
    fn new(tag: u8, data: Vec<u8>, constructed: bool) -> Result<Self, Error> {
        if !(64..=254).contains(&tag) {
            return Err(Error::decoding(
                0,
                "proprietary property-state tag must be in 64..=254",
            ));
        }
        Ok(Self {
            tag,
            data,
            constructed,
        })
    }

    /// Preserve a primitive vendor-defined alternative.
    pub fn primitive(tag: u8, data: Vec<u8>) -> Result<Self, Error> {
        Self::new(tag, data, false)
    }

    /// Preserve the body of a constructed vendor-defined alternative.
    ///
    /// The wire encoder rejects bodies that are not BACnet TLV sequences.
    pub fn constructed(tag: u8, data: Vec<u8>) -> Result<Self, Error> {
        Self::new(tag, data, true)
    }

    /// Return the proprietary context tag.
    pub const fn tag(&self) -> u8 {
        self.tag
    }

    /// Return the encoded primitive contents or constructed body.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Return whether the value uses opening and closing tags.
    pub const fn is_constructed(&self) -> bool {
        self.constructed
    }
}

/// Discrete or enumerated property value used by event and fault parameters.
///
/// Each variant is one alternative of the `BACnetPropertyStates` choice from
/// Standard 135-2020 Clause 21. The standard alternatives are primitive values
/// under their own context tag, listed below. Tags 26, 29, 35, 61 and 62 have
/// no alternative and the decoder rejects them. Decoders use `Other` for
/// proprietary context tags 64 through 254.
///
/// | Tag | Alternative | Type |
/// |---|---|---|
/// | `[0]` | `boolean-value` | BOOLEAN |
/// | `[1]` | `binary-value` | `BACnetBinaryPV` |
/// | `[2]` | `event-type` | `BACnetEventType` |
/// | `[3]` | `polarity` | `BACnetPolarity` |
/// | `[4]` | `program-change` | `BACnetProgramRequest` |
/// | `[5]` | `program-state` | `BACnetProgramState` |
/// | `[6]` | `reason-for-halt` | `BACnetProgramError` |
/// | `[7]` | `reliability` | `BACnetReliability` |
/// | `[8]` | `state` | `BACnetEventState` |
/// | `[9]` | `system-status` | `BACnetDeviceStatus` |
/// | `[10]` | `units` | `BACnetEngineeringUnits` |
/// | `[11]` | `unsigned-value` | Unsigned |
/// | `[12]` | `life-safety-mode` | `BACnetLifeSafetyMode` |
/// | `[13]` | `life-safety-state` | `BACnetLifeSafetyState` |
/// | `[14]` | `restart-reason` | `BACnetRestartReason` |
/// | `[15]` | `door-alarm-state` | `BACnetDoorAlarmState` |
/// | `[16]` | `action` | `BACnetAction` |
/// | `[17]` | `door-secured-status` | `BACnetDoorSecuredStatus` |
/// | `[18]` | `door-status` | `BACnetDoorStatus` |
/// | `[19]` | `door-value` | `BACnetDoorValue` |
/// | `[20]` | `file-access-method` | `BACnetFileAccessMethod` |
/// | `[21]` | `lock-status` | `BACnetLockStatus` |
/// | `[22]` | `life-safety-operation` | `BACnetLifeSafetyOperation` |
/// | `[23]` | `maintenance` | `BACnetMaintenance` |
/// | `[24]` | `node-type` | `BACnetNodeType` |
/// | `[25]` | `notify-type` | `BACnetNotifyType` |
/// | `[27]` | `shed-state` | `BACnetShedState` |
/// | `[28]` | `silenced-state` | `BACnetSilencedState` |
/// | `[30]` | `access-event` | `BACnetAccessEvent` |
/// | `[31]` | `zone-occupancy-state` | `BACnetAccessZoneOccupancyState` |
/// | `[32]` | `access-credential-disable-reason` | `BACnetAccessCredentialDisableReason` |
/// | `[33]` | `access-credential-disable` | `BACnetAccessCredentialDisable` |
/// | `[34]` | `authentication-status` | `BACnetAuthenticationStatus` |
/// | `[36]` | `backup-state` | `BACnetBackupState` |
/// | `[37]` | `write-status` | `BACnetWriteStatus` |
/// | `[38]` | `lighting-in-progress` | `BACnetLightingInProgress` |
/// | `[39]` | `lighting-operation` | `BACnetLightingOperation` |
/// | `[40]` | `lighting-transition` | `BACnetLightingTransition` |
/// | `[41]` | `integer-value` | INTEGER (signed) |
/// | `[42]` | `binary-lighting-value` | `BACnetBinaryLightingPV` |
/// | `[43]` | `timer-state` | `BACnetTimerState` |
/// | `[44]` | `timer-transition` | `BACnetTimerTransition` |
/// | `[45]` | `bacnet-ip-mode` | `BACnetIPMode` |
/// | `[46]` | `network-port-command` | `BACnetNetworkPortCommand` |
/// | `[47]` | `network-type` | `BACnetNetworkType` |
/// | `[48]` | `network-number-quality` | `BACnetNetworkNumberQuality` |
/// | `[49]` | `escalator-operation-direction` | `BACnetEscalatorOperationDirection` |
/// | `[50]` | `escalator-fault` | `BACnetEscalatorFault` |
/// | `[51]` | `escalator-mode` | `BACnetEscalatorMode` |
/// | `[52]` | `lift-car-direction` | `BACnetLiftCarDirection` |
/// | `[53]` | `lift-car-door-command` | `BACnetLiftCarDoorCommand` |
/// | `[54]` | `lift-car-drive-status` | `BACnetLiftCarDriveStatus` |
/// | `[55]` | `lift-car-mode` | `BACnetLiftCarMode` |
/// | `[56]` | `lift-group-mode` | `BACnetLiftGroupMode` |
/// | `[57]` | `lift-fault` | `BACnetLiftFault` |
/// | `[58]` | `protocol-level` | `BACnetProtocolLevel` |
/// | `[59]` | `audit-level` | `BACnetAuditLevel` |
/// | `[60]` | `audit-operation` | `BACnetAuditOperation` |
/// | `[63]` | `extended-value` | Unsigned32, packed as described below |
/// | `[64]` to `[254]` | vendor-defined | primitive or constructed, kept as raw contents |
///
/// `extended-value` carries a choice tag above 254 packed into one Unsigned32
/// as tag × 100000 + value.
#[derive(Debug, Clone, PartialEq)]
pub enum BACnetPropertyStates {
    /// Tag 0: a BOOLEAN.
    BooleanValue(bool),
    /// Tag 1: `BACnetBinaryPV`.
    BinaryValue(u32),
    /// Tag 2: `BACnetEventType`.
    EventType(u32),
    /// Tag 3: `BACnetPolarity`.
    Polarity(u32),
    /// Tag 4: `BACnetProgramRequest`.
    ProgramChange(u32),
    /// Tag 5: `BACnetProgramState`.
    ProgramState(u32),
    /// Tag 6: `BACnetProgramError`.
    ReasonForHalt(u32),
    /// Tag 7: `BACnetReliability`.
    Reliability(u32),
    /// Tag 8: `BACnetEventState`.
    State(u32),
    /// Tag 9: `BACnetDeviceStatus`.
    SystemStatus(u32),
    /// Tag 10: `BACnetEngineeringUnits`.
    Units(u32),
    /// Tag 11: an Unsigned.
    UnsignedValue(u32),
    /// Tag 12: `BACnetLifeSafetyMode`.
    LifeSafetyMode(u32),
    /// Tag 13: `BACnetLifeSafetyState`.
    LifeSafetyState(u32),
    /// Tag 14: `BACnetRestartReason`.
    RestartReason(u32),
    /// Tag 15: `BACnetDoorAlarmState`.
    DoorAlarmState(u32),
    /// Tag 16: `BACnetAction`.
    Action(u32),
    /// Tag 17: `BACnetDoorSecuredStatus`.
    DoorSecuredStatus(u32),
    /// Tag 18: `BACnetDoorStatus`.
    DoorStatus(u32),
    /// Tag 19: `BACnetDoorValue`.
    DoorValue(u32),
    /// Tag 20: `BACnetFileAccessMethod`.
    FileAccessMethod(u32),
    /// Tag 21: `BACnetLockStatus`.
    LockStatus(u32),
    /// Tag 22: `BACnetLifeSafetyOperation`.
    LifeSafetyOperation(u32),
    /// Tag 23: `BACnetMaintenance`.
    Maintenance(u32),
    /// Tag 24: `BACnetNodeType`.
    NodeType(u32),
    /// Tag 25: `BACnetNotifyType`.
    NotifyType(u32),
    /// Tag 27: `BACnetShedState`.
    ShedState(u32),
    /// Tag 28: `BACnetSilencedState`.
    SilencedState(u32),
    /// Tag 30: `BACnetAccessEvent`.
    AccessEvent(u32),
    /// Tag 31: `BACnetAccessZoneOccupancyState`.
    ZoneOccupancyState(u32),
    /// Tag 32: `BACnetAccessCredentialDisableReason`.
    AccessCredentialDisableReason(u32),
    /// Tag 33: `BACnetAccessCredentialDisable`.
    AccessCredentialDisable(u32),
    /// Tag 34: `BACnetAuthenticationStatus`.
    AuthenticationStatus(u32),
    /// Tag 36: `BACnetBackupState`.
    BackupState(u32),
    /// Tag 37: `BACnetWriteStatus`.
    WriteStatus(u32),
    /// Tag 38: `BACnetLightingInProgress`.
    LightingInProgress(u32),
    /// Tag 39: `BACnetLightingOperation`.
    LightingOperation(u32),
    /// Tag 40: `BACnetLightingTransition`.
    LightingTransition(u32),
    /// Tag 41: a signed INTEGER.
    IntegerValue(i32),
    /// Tag 42: `BACnetBinaryLightingPV`.
    BinaryLightingValue(u32),
    /// Tag 43: `BACnetTimerState`.
    TimerState(u32),
    /// Tag 44: `BACnetTimerTransition`.
    TimerTransition(u32),
    /// Tag 45: `BACnetIPMode`.
    BacnetIpMode(u32),
    /// Tag 46: `BACnetNetworkPortCommand`.
    NetworkPortCommand(u32),
    /// Tag 47: `BACnetNetworkType`.
    NetworkType(u32),
    /// Tag 48: `BACnetNetworkNumberQuality`.
    NetworkNumberQuality(u32),
    /// Tag 49: `BACnetEscalatorOperationDirection`.
    EscalatorOperationDirection(u32),
    /// Tag 50: `BACnetEscalatorFault`.
    EscalatorFault(u32),
    /// Tag 51: `BACnetEscalatorMode`.
    EscalatorMode(u32),
    /// Tag 52: `BACnetLiftCarDirection`.
    LiftCarDirection(u32),
    /// Tag 53: `BACnetLiftCarDoorCommand`.
    LiftCarDoorCommand(u32),
    /// Tag 54: `BACnetLiftCarDriveStatus`.
    LiftCarDriveStatus(u32),
    /// Tag 55: `BACnetLiftCarMode`.
    LiftCarMode(u32),
    /// Tag 56: `BACnetLiftGroupMode`.
    LiftGroupMode(u32),
    /// Tag 57: `BACnetLiftFault`.
    LiftFault(u32),
    /// Tag 58: `BACnetProtocolLevel`.
    ProtocolLevel(u32),
    /// Tag 59: `BACnetAuditLevel`.
    AuditLevel(u32),
    /// Tag 60: `BACnetAuditOperation`.
    AuditOperation(u32),
    /// Tag 63: a choice tag above 254 and its value, unpacked from one Unsigned32.
    ExtendedValue(BACnetExtendedPropertyState),
    /// Vendor-defined context tag 64 through 254.
    Other(BACnetProprietaryPropertyState),
}

impl BACnetPropertyStates {
    /// Return the semantic scalar used for enum-like event comparisons.
    pub fn as_u32(&self) -> Option<u32> {
        use BACnetPropertyStates as S;

        match self {
            S::BooleanValue(value) => Some(u32::from(*value)),
            S::BinaryValue(value)
            | S::EventType(value)
            | S::Polarity(value)
            | S::ProgramChange(value)
            | S::ProgramState(value)
            | S::ReasonForHalt(value)
            | S::Reliability(value)
            | S::State(value)
            | S::SystemStatus(value)
            | S::Units(value)
            | S::UnsignedValue(value)
            | S::LifeSafetyMode(value)
            | S::LifeSafetyState(value)
            | S::RestartReason(value)
            | S::DoorAlarmState(value)
            | S::Action(value)
            | S::DoorSecuredStatus(value)
            | S::DoorStatus(value)
            | S::DoorValue(value)
            | S::FileAccessMethod(value)
            | S::LockStatus(value)
            | S::LifeSafetyOperation(value)
            | S::Maintenance(value)
            | S::NodeType(value)
            | S::NotifyType(value)
            | S::ShedState(value)
            | S::SilencedState(value)
            | S::AccessEvent(value)
            | S::ZoneOccupancyState(value)
            | S::AccessCredentialDisableReason(value)
            | S::AccessCredentialDisable(value)
            | S::AuthenticationStatus(value)
            | S::BackupState(value)
            | S::WriteStatus(value)
            | S::LightingInProgress(value)
            | S::LightingOperation(value)
            | S::LightingTransition(value)
            | S::BinaryLightingValue(value)
            | S::TimerState(value)
            | S::TimerTransition(value)
            | S::BacnetIpMode(value)
            | S::NetworkPortCommand(value)
            | S::NetworkType(value)
            | S::NetworkNumberQuality(value)
            | S::EscalatorOperationDirection(value)
            | S::EscalatorFault(value)
            | S::EscalatorMode(value)
            | S::LiftCarDirection(value)
            | S::LiftCarDoorCommand(value)
            | S::LiftCarDriveStatus(value)
            | S::LiftCarMode(value)
            | S::LiftGroupMode(value)
            | S::LiftFault(value)
            | S::ProtocolLevel(value)
            | S::AuditLevel(value)
            | S::AuditOperation(value) => Some(*value),
            S::ExtendedValue(value) => Some(value.value()),
            S::IntegerValue(value) => u32::try_from(*value).ok(),
            S::Other(value) if !value.is_constructed() && !value.data().is_empty() => {
                value.data().iter().try_fold(0u32, |acc, byte| {
                    acc.checked_mul(256)?.checked_add(*byte as u32)
                })
            }
            S::Other(_) => None,
        }
    }
}
