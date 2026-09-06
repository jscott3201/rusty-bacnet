#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

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
/// The variants follow the `BACnetPropertyStates` CHOICE in Standard 135-2020
/// Clause 21. Decoders use `Other` for proprietary context tags 64 through 254.
#[derive(Debug, Clone, PartialEq)]
pub enum BACnetPropertyStates {
    /// `boolean-value [0] BOOLEAN`.
    BooleanValue(bool),
    /// `binary-value [1] BACnetBinaryPV`.
    BinaryValue(u32),
    /// `event-type [2] BACnetEventType`.
    EventType(u32),
    /// `polarity [3] BACnetPolarity`.
    Polarity(u32),
    /// `program-change [4] BACnetProgramRequest`.
    ProgramChange(u32),
    /// `program-state [5] BACnetProgramState`.
    ProgramState(u32),
    /// `reason-for-halt [6] BACnetProgramError`.
    ReasonForHalt(u32),
    /// `reliability [7] BACnetReliability`.
    Reliability(u32),
    /// `state [8] BACnetEventState`.
    State(u32),
    /// `system-status [9] BACnetDeviceStatus`.
    SystemStatus(u32),
    /// `units [10] BACnetEngineeringUnits`.
    Units(u32),
    /// `unsigned-value [11] Unsigned`.
    UnsignedValue(u32),
    /// `life-safety-mode [12] BACnetLifeSafetyMode`.
    LifeSafetyMode(u32),
    /// `life-safety-state [13] BACnetLifeSafetyState`.
    LifeSafetyState(u32),
    /// `restart-reason [14] BACnetRestartReason`.
    RestartReason(u32),
    /// `door-alarm-state [15] BACnetDoorAlarmState`.
    DoorAlarmState(u32),
    /// `action [16] BACnetAction`.
    Action(u32),
    /// `door-secured-status [17] BACnetDoorSecuredStatus`.
    DoorSecuredStatus(u32),
    /// `door-status [18] BACnetDoorStatus`.
    DoorStatus(u32),
    /// `door-value [19] BACnetDoorValue`.
    DoorValue(u32),
    /// `file-access-method [20] BACnetFileAccessMethod`.
    FileAccessMethod(u32),
    /// `lock-status [21] BACnetLockStatus`.
    LockStatus(u32),
    /// `life-safety-operation [22] BACnetLifeSafetyOperation`.
    LifeSafetyOperation(u32),
    /// `maintenance [23] BACnetMaintenance`.
    Maintenance(u32),
    /// `node-type [24] BACnetNodeType`.
    NodeType(u32),
    /// `notify-type [25] BACnetNotifyType`.
    NotifyType(u32),
    /// `shed-state [27] BACnetShedState`.
    ShedState(u32),
    /// `silenced-state [28] BACnetSilencedState`.
    SilencedState(u32),
    /// `access-event [30] BACnetAccessEvent`.
    AccessEvent(u32),
    /// `zone-occupancy-state [31] BACnetAccessZoneOccupancyState`.
    ZoneOccupancyState(u32),
    /// `access-credential-disable-reason [32] BACnetAccessCredentialDisableReason`.
    AccessCredentialDisableReason(u32),
    /// `access-credential-disable [33] BACnetAccessCredentialDisable`.
    AccessCredentialDisable(u32),
    /// `authentication-status [34] BACnetAuthenticationStatus`.
    AuthenticationStatus(u32),
    /// `backup-state [36] BACnetBackupState`.
    BackupState(u32),
    /// `write-status [37] BACnetWriteStatus`.
    WriteStatus(u32),
    /// `lighting-in-progress [38] BACnetLightingInProgress`.
    LightingInProgress(u32),
    /// `lighting-operation [39] BACnetLightingOperation`.
    LightingOperation(u32),
    /// `lighting-transition [40] BACnetLightingTransition`.
    LightingTransition(u32),
    /// `integer-value [41] INTEGER`.
    IntegerValue(i32),
    /// `binary-lighting-value [42] BACnetBinaryLightingPV`.
    BinaryLightingValue(u32),
    /// `timer-state [43] BACnetTimerState`.
    TimerState(u32),
    /// `timer-transition [44] BACnetTimerTransition`.
    TimerTransition(u32),
    /// `bacnet-ip-mode [45] BACnetIPMode`.
    BacnetIpMode(u32),
    /// `network-port-command [46] BACnetNetworkPortCommand`.
    NetworkPortCommand(u32),
    /// `network-type [47] BACnetNetworkType`.
    NetworkType(u32),
    /// `network-number-quality [48] BACnetNetworkNumberQuality`.
    NetworkNumberQuality(u32),
    /// `escalator-operation-direction [49] BACnetEscalatorOperationDirection`.
    EscalatorOperationDirection(u32),
    /// `escalator-fault [50] BACnetEscalatorFault`.
    EscalatorFault(u32),
    /// `escalator-mode [51] BACnetEscalatorMode`.
    EscalatorMode(u32),
    /// `lift-car-direction [52] BACnetLiftCarDirection`.
    LiftCarDirection(u32),
    /// `lift-car-door-command [53] BACnetLiftCarDoorCommand`.
    LiftCarDoorCommand(u32),
    /// `lift-car-drive-status [54] BACnetLiftCarDriveStatus`.
    LiftCarDriveStatus(u32),
    /// `lift-car-mode [55] BACnetLiftCarMode`.
    LiftCarMode(u32),
    /// `lift-group-mode [56] BACnetLiftGroupMode`.
    LiftGroupMode(u32),
    /// `lift-fault [57] BACnetLiftFault`.
    LiftFault(u32),
    /// `protocol-level [58] BACnetProtocolLevel`.
    ProtocolLevel(u32),
    /// `audit-level [59] BACnetAuditLevel`.
    AuditLevel(u32),
    /// `audit-operation [60] BACnetAuditOperation`.
    AuditOperation(u32),
    /// `extended-value [63] Unsigned32` with its unpacked tag and value.
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
