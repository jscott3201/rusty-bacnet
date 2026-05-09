//! BACnet constructed data types per ASHRAE 135-2020.
//!
//! This module provides compound/structured types that are used by higher-level
//! BACnet objects (Calendar, Schedule, TrendLog, NotificationClass, Loop, etc.).
//! All types follow the same `no_std`-compatible pattern used in `primitives.rs`.

#[cfg(not(feature = "std"))]
use alloc::{vec, vec::Vec};

use crate::error::Error;
use crate::primitives::{Date, ObjectIdentifier, Time};
use crate::MacAddr;

// ---------------------------------------------------------------------------
// BACnetDateRange (Clause 21 -- used by CalendarEntry and BACnetSpecialEvent)
// ---------------------------------------------------------------------------

/// BACnet date range: a SEQUENCE of start and end Date values.
///
/// Encoded as 8 bytes: 4 bytes for start_date followed by 4 bytes for end_date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BACnetDateRange {
    /// The start of the date range (inclusive).
    pub start_date: Date,
    /// The end of the date range (inclusive).
    pub end_date: Date,
}

impl BACnetDateRange {
    /// Encode to 8 bytes (start_date || end_date).
    pub fn encode(&self) -> [u8; 8] {
        let mut out = [0u8; 8];
        out[..4].copy_from_slice(&self.start_date.encode());
        out[4..].copy_from_slice(&self.end_date.encode());
        out
    }

    /// Decode from at least 8 bytes.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        if data.len() < 8 {
            return Err(Error::buffer_too_short(8, data.len()));
        }
        Ok(Self {
            start_date: Date::decode(&data[0..4])?,
            end_date: Date::decode(&data[4..8])?,
        })
    }
}

// ---------------------------------------------------------------------------
// BACnetWeekNDay (Clause 21 -- used by CalendarEntry)
// ---------------------------------------------------------------------------

/// BACnet Week-And-Day: OCTET STRING(3) encoding month, week_of_month,
/// and day_of_week.
///
/// Each field may be `0xFF` to mean "any" (wildcard).
///
/// - `month`: 1-12, 13=odd, 14=even, 0xFF=any
/// - `week_of_month`: 1=first, 2=second, ..., 5=last, 6=any-in-first,
///   0xFF=any
/// - `day_of_week`: 1=Monday..7=Sunday, 0xFF=any
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BACnetWeekNDay {
    /// Month (1-14, or 0xFF for any).
    pub month: u8,
    /// Week of month (1-6, or 0xFF for any).
    pub week_of_month: u8,
    /// Day of week (1-7, or 0xFF for any).
    pub day_of_week: u8,
}

impl BACnetWeekNDay {
    /// Wildcard value indicating "any" for any field.
    pub const ANY: u8 = 0xFF;

    /// Encode to 3 bytes.
    pub fn encode(&self) -> [u8; 3] {
        [self.month, self.week_of_month, self.day_of_week]
    }

    /// Decode from at least 3 bytes.
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        if data.len() < 3 {
            return Err(Error::buffer_too_short(3, data.len()));
        }
        Ok(Self {
            month: data[0],
            week_of_month: data[1],
            day_of_week: data[2],
        })
    }
}

// ---------------------------------------------------------------------------
// BACnetCalendarEntry (Clause 12.6.3 -- property list of Calendar object)
// ---------------------------------------------------------------------------

/// BACnet calendar entry: a CHOICE between a specific date, a date range,
/// or a week-and-day pattern.
///
/// Context tags per spec:
/// - `[0]` Date
/// - `[1]` DateRange
/// - `[2]` WeekNDay
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BACnetCalendarEntry {
    /// A single specific date (context tag 0).
    Date(Date),
    /// A contiguous date range (context tag 1).
    DateRange(BACnetDateRange),
    /// A recurring week-and-day pattern (context tag 2).
    WeekNDay(BACnetWeekNDay),
}

// ---------------------------------------------------------------------------
// BACnetTimeValue (Clause 12.17.4 -- used by Schedule weekly_schedule)
// ---------------------------------------------------------------------------

/// BACnet time-value pair: a Time followed by an application-tagged value.
///
/// The `value` field holds raw application-tagged bytes because the value
/// type is polymorphic (Real, Boolean, Unsigned, Null, etc.) and the Schedule
/// object stores them opaquely for later dispatch.
#[derive(Debug, Clone, PartialEq)]
pub struct BACnetTimeValue {
    /// The time at which the value applies.
    pub time: Time,
    /// Raw application-tagged BACnet encoding of the value.
    pub value: Vec<u8>,
}

// ---------------------------------------------------------------------------
// SpecialEventPeriod (Clause 12.17.5 -- used by BACnetSpecialEvent)
// ---------------------------------------------------------------------------

/// The period portion of a BACnetSpecialEvent: either an inline
/// CalendarEntry or a reference to an existing Calendar object.
///
/// Context tags per spec:
/// - `[0]` CalendarEntry (constructed)
/// - `[1]` CalendarReference (ObjectIdentifier)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpecialEventPeriod {
    /// An inline calendar entry (context tag 0).
    CalendarEntry(BACnetCalendarEntry),
    /// A reference to a Calendar object (context tag 1).
    CalendarReference(ObjectIdentifier),
}

// ---------------------------------------------------------------------------
// BACnetSpecialEvent (Clause 12.17.5 -- exception_schedule of Schedule)
// ---------------------------------------------------------------------------

/// BACnet special event: an exception schedule entry combining a period
/// definition, a list of time-value pairs, and a priority.
#[derive(Debug, Clone, PartialEq)]
pub struct BACnetSpecialEvent {
    /// The period this special event applies to.
    pub period: SpecialEventPeriod,
    /// Ordered list of time-value pairs to apply during this period.
    pub list_of_time_values: Vec<BACnetTimeValue>,
    /// Priority for conflict resolution (1=highest..16=lowest).
    pub event_priority: u8,
}

// ---------------------------------------------------------------------------
// BACnetObjectPropertyReference (Clause 21 -- used by Loop and others)
// ---------------------------------------------------------------------------

/// A reference to a specific property (and optionally an array index) on
/// a specific object within the same device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BACnetObjectPropertyReference {
    /// The object being referenced.
    pub object_identifier: ObjectIdentifier,
    /// The property being referenced (PropertyIdentifier raw value).
    pub property_identifier: u32,
    /// Optional array index within the property.
    pub property_array_index: Option<u32>,
}

impl BACnetObjectPropertyReference {
    /// Create a reference without an array index.
    pub fn new(object_identifier: ObjectIdentifier, property_identifier: u32) -> Self {
        Self {
            object_identifier,
            property_identifier,
            property_array_index: None,
        }
    }

    /// Create a reference with an array index.
    pub fn new_indexed(
        object_identifier: ObjectIdentifier,
        property_identifier: u32,
        array_index: u32,
    ) -> Self {
        Self {
            object_identifier,
            property_identifier,
            property_array_index: Some(array_index),
        }
    }
}

// ---------------------------------------------------------------------------
// BACnetDeviceObjectPropertyReference (Clause 21 -- used by several objects)
// ---------------------------------------------------------------------------

/// Like `BACnetObjectPropertyReference` but may also specify a remote device.
///
/// When `device_identifier` is `None`, the reference is to an object in the
/// local device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BACnetDeviceObjectPropertyReference {
    /// The object being referenced.
    pub object_identifier: ObjectIdentifier,
    /// The property being referenced (PropertyIdentifier raw value).
    pub property_identifier: u32,
    /// Optional array index within the property.
    pub property_array_index: Option<u32>,
    /// Optional device identifier (None = local device).
    pub device_identifier: Option<ObjectIdentifier>,
}

impl BACnetDeviceObjectPropertyReference {
    /// Create a local-device reference without an array index.
    pub fn new_local(object_identifier: ObjectIdentifier, property_identifier: u32) -> Self {
        Self {
            object_identifier,
            property_identifier,
            property_array_index: None,
            device_identifier: None,
        }
    }

    /// Create a remote-device reference without an array index.
    pub fn new_remote(
        object_identifier: ObjectIdentifier,
        property_identifier: u32,
        device_identifier: ObjectIdentifier,
    ) -> Self {
        Self {
            object_identifier,
            property_identifier,
            property_array_index: None,
            device_identifier: Some(device_identifier),
        }
    }

    /// Create a reference with an array index (may be local or remote).
    pub fn with_index(mut self, array_index: u32) -> Self {
        self.property_array_index = Some(array_index);
        self
    }
}

// ---------------------------------------------------------------------------
// BACnetAddress (Clause 21 -- network address used by BACnetRecipient)
// ---------------------------------------------------------------------------

/// A BACnet network address: network number (0 = local) plus MAC address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BACnetAddress {
    /// Network number (0 = local network, 1-65534 = remote, 65535 = broadcast).
    pub network_number: u16,
    /// MAC-layer address (variable length; empty = local broadcast).
    pub mac_address: MacAddr,
}

impl BACnetAddress {
    /// Create a local-broadcast address.
    pub fn local_broadcast() -> Self {
        Self {
            network_number: 0,
            mac_address: MacAddr::new(),
        }
    }

    /// Create a BACnet/IP address from a 6-byte octet-string (4-byte IPv4 + 2-byte port).
    pub fn from_ip(ip_port_bytes: [u8; 6]) -> Self {
        Self {
            network_number: 0,
            mac_address: MacAddr::from_slice(&ip_port_bytes),
        }
    }
}

// ---------------------------------------------------------------------------
// BACnetRecipient (Clause 21 -- used by BACnetDestination / NotificationClass)
// ---------------------------------------------------------------------------

/// A BACnet notification recipient: either a Device object reference or a
/// network address.
///
/// Context tags per spec:
/// - `[0]` Device (ObjectIdentifier)
/// - `[1]` Address (BACnetAddress)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BACnetRecipient {
    /// A device identified by its Object Identifier (context tag 0).
    Device(ObjectIdentifier),
    /// A device identified by its network address (context tag 1).
    Address(BACnetAddress),
}

// ---------------------------------------------------------------------------
// BACnetDestination (Clause 12.15.5 -- recipient_list of NotificationClass)
// ---------------------------------------------------------------------------

/// A single entry in a NotificationClass recipient list.
///
/// Specifies *who* receives the notification, *when* (days/times), and *how*
/// (confirmed vs. unconfirmed, which transition types).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BACnetDestination {
    /// Bitmask of valid days (bit 0 = Monday ... bit 6 = Sunday), 7 bits used.
    pub valid_days: u8,
    /// Start of the daily time window during which this destination is active.
    pub from_time: Time,
    /// End of the daily time window.
    pub to_time: Time,
    /// The notification recipient.
    pub recipient: BACnetRecipient,
    /// Process identifier on the receiving device.
    pub process_identifier: u32,
    /// If true, use ConfirmedEventNotification; otherwise unconfirmed.
    pub issue_confirmed_notifications: bool,
    /// Bitmask of event transitions to send (bit 0=ToOffNormal, bit 1=ToFault,
    /// bit 2=ToNormal), 3 bits used.
    pub transitions: u8,
}

// ---------------------------------------------------------------------------
// LogDatum (Clause 12.20.5 -- log_buffer element datum of TrendLog)
// ---------------------------------------------------------------------------

/// The datum field of a BACnetLogRecord: a CHOICE covering all possible
/// logged value types.
///
/// Context tags per spec:
/// - `[0]` log-status (BACnetLogStatus, 8-bit flags)
/// - `[1]` boolean-value
/// - `[2]` real-value
/// - `[3]` enum-value (unsigned)
/// - `[4]` unsigned-value
/// - `[5]` signed-value
/// - `[6]` bitstring-value
/// - `[7]` null-value
/// - `[8]` failure (BACnetError)
/// - `[9]` time-change (REAL, clock-adjustment seconds)
/// - `[10]` any-value (raw application-tagged bytes)
#[derive(Debug, Clone, PartialEq)]
pub enum LogDatum {
    /// Log-status flags (context tag 0).  Bit 0=log-disabled, bit 1=buffer-purged,
    /// bit 2=log-interrupted.
    LogStatus(u8),
    /// Boolean value (context tag 1).
    BooleanValue(bool),
    /// Real (f32) value (context tag 2).
    RealValue(f32),
    /// Enumerated value (context tag 3).
    EnumValue(u32),
    /// Unsigned integer value (context tag 4).
    UnsignedValue(u64),
    /// Signed integer value (context tag 5).
    SignedValue(i64),
    /// Bit-string value (context tag 6).
    BitstringValue {
        /// Number of unused bits in the last byte.
        unused_bits: u8,
        /// The bit data.
        data: Vec<u8>,
    },
    /// Null value (context tag 7).
    NullValue,
    /// Error (context tag 8): error class + error code.
    Failure {
        /// Raw BACnet error class value.
        error_class: u32,
        /// Raw BACnet error code value.
        error_code: u32,
    },
    /// Time-change: clock-adjustment amount in seconds (context tag 9).
    TimeChange(f32),
    /// Any-value: raw application-tagged bytes for types not enumerated above
    /// (context tag 10).
    AnyValue(Vec<u8>),
}

// ---------------------------------------------------------------------------
// BACnetLogRecord (Clause 12.20.5 -- log_buffer of TrendLog)
// ---------------------------------------------------------------------------

/// A single record stored in a TrendLog object's log buffer.
///
/// Contains a timestamp (date + time), the logged datum, and optional
/// status flags that were in effect at logging time.
#[derive(Debug, Clone, PartialEq)]
pub struct BACnetLogRecord {
    /// The date at which this record was logged.
    pub date: Date,
    /// The time at which this record was logged.
    pub time: Time,
    /// The logged datum.
    pub log_datum: LogDatum,
    /// Optional status flags at time of logging (4-bit BACnet StatusFlags).
    pub status_flags: Option<u8>,
}

// ---------------------------------------------------------------------------
// BACnetScale (Clause 21)
// ---------------------------------------------------------------------------

/// BACnet Scale: CHOICE { float-scale [0] Real, integer-scale [1] Integer }.
#[derive(Debug, Clone, PartialEq)]
pub enum BACnetScale {
    FloatScale(f32),
    IntegerScale(i32),
}

// ---------------------------------------------------------------------------
// BACnetPrescale (Clause 21)
// ---------------------------------------------------------------------------

/// BACnet Prescale: SEQUENCE { multiplier Unsigned, modulo-divide Unsigned }.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BACnetPrescale {
    pub multiplier: u32,
    pub modulo_divide: u32,
}

// ---------------------------------------------------------------------------
// BACnetPropertyStates (Clause 21)
// ---------------------------------------------------------------------------

/// BACnet Property States — CHOICE type with 40+ variants.
/// We represent common variants typed, uncommon as raw bytes.
#[derive(Debug, Clone, PartialEq)]
pub enum BACnetPropertyStates {
    BooleanValue(bool),      // [0]
    BinaryValue(u32),        // [1] BACnetBinaryPV
    EventType(u32),          // [2]
    Polarity(u32),           // [3]
    ProgramChange(u32),      // [4]
    ProgramState(u32),       // [5]
    ReasonForHalt(u32),      // [6]
    Reliability(u32),        // [7]
    State(u32),              // [8] BACnetEventState
    SystemStatus(u32),       // [9]
    Units(u32),              // [10]
    UnsignedValue(u32),      // [11]
    LifeSafetyMode(u32),     // [12]
    LifeSafetyState(u32),    // [13]
    DoorAlarmState(u32),     // [14]
    Action(u32),             // [15]
    DoorSecuredStatus(u32),  // [16]
    DoorStatus(u32),         // [17]
    DoorValue(u32),          // [18]
    LiftCarDirection(u32),   // [40]
    LiftCarDoorCommand(u32), // [42]
    TimerState(u32),         // [38]
    TimerTransition(u32),    // [39]
    /// Catch-all for uncommon variants.
    Other {
        tag: u8,
        data: Vec<u8>,
    },
}

// ---------------------------------------------------------------------------
// BACnetShedLevel (Clause 12 — used by LoadControl)
// ---------------------------------------------------------------------------

/// BACnet ShedLevel — CHOICE for LoadControl.
#[derive(Debug, Clone, PartialEq)]
pub enum BACnetShedLevel {
    /// Shed level as a percentage (0–100).
    Percent(u32),
    /// Shed level as an abstract level value.
    Level(u32),
    /// Shed level as a floating-point amount.
    Amount(f32),
}

// ---------------------------------------------------------------------------
// BACnetLightingCommand (Clause 21 -- used by LightingOutput)
// ---------------------------------------------------------------------------

/// BACnet Lighting Command -- controls lighting operations.
///
/// Per ASHRAE 135-2020 Clause 21, this type is used by the LightingOutput
/// object's LIGHTING_COMMAND property to specify a lighting operation
/// (e.g., fade, ramp, step) with optional parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct BACnetLightingCommand {
    /// The lighting operation (LightingOperation enum raw value).
    pub operation: u32,
    /// Optional target brightness level (0.0 to 100.0 percent).
    pub target_level: Option<f32>,
    /// Optional ramp rate (percent per second).
    pub ramp_rate: Option<f32>,
    /// Optional step increment (percent).
    pub step_increment: Option<f32>,
    /// Optional fade time (milliseconds).
    pub fade_time: Option<u32>,
    /// Optional priority (1-16).
    pub priority: Option<u32>,
}

// ---------------------------------------------------------------------------
// BACnetDeviceObjectReference (Clause 21 -- used by Access Control objects)
// ---------------------------------------------------------------------------

/// BACnet Device Object Reference (simplified).
///
/// References an object, optionally on a specific device. Used by access
/// control objects (e.g., BACnetAccessRule location).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BACnetDeviceObjectReference {
    /// Optional device identifier (None = local device).
    pub device_identifier: Option<ObjectIdentifier>,
    /// The object being referenced.
    pub object_identifier: ObjectIdentifier,
}

// ---------------------------------------------------------------------------
// BACnetAccessRule (Clause 12 -- used by AccessRights object)
// ---------------------------------------------------------------------------

/// BACnet Access Rule for access control objects.
///
/// Specifies a time range and location with an enable/disable flag,
/// used in positive and negative access rules lists.
#[derive(Debug, Clone, PartialEq)]
pub struct BACnetAccessRule {
    /// Time range specifier: 0 = specified, 1 = always.
    pub time_range_specifier: u32,
    /// Optional time range (start date, start time, end date, end time).
    /// Present only when `time_range_specifier` is 0 (specified).
    pub time_range: Option<(Date, Time, Date, Time)>,
    /// Location specifier: 0 = specified, 1 = all.
    pub location_specifier: u32,
    /// Optional location reference. Present only when `location_specifier` is 0 (specified).
    pub location: Option<BACnetDeviceObjectReference>,
    /// Whether access is enabled or disabled by this rule.
    pub enable: bool,
}

// ---------------------------------------------------------------------------
// BACnetAssignedAccessRights (Clause 12 -- used by AccessCredential/AccessUser)
// ---------------------------------------------------------------------------

/// BACnet Assigned Access Rights.
///
/// Associates a reference to an AccessRights object with an enable flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BACnetAssignedAccessRights {
    /// Reference to an AccessRights object.
    pub assigned_access_rights: ObjectIdentifier,
    /// Whether these access rights are currently enabled.
    pub enable: bool,
}

// ---------------------------------------------------------------------------
// BACnetAssignedLandingCalls (Clause 12 -- used by ElevatorGroup)
// ---------------------------------------------------------------------------

/// BACnet Assigned Landing Calls for elevator group.
#[derive(Debug, Clone, PartialEq)]
pub struct BACnetAssignedLandingCalls {
    /// The floor number for this landing call.
    pub floor_number: u8,
    /// Direction: 0=up, 1=down, 2=unknown.
    pub direction: u32,
}

// ---------------------------------------------------------------------------
// FaultParameters (Clause 12.12.50)
// ---------------------------------------------------------------------------

/// Fault parameter variants for configuring fault detection algorithms.
#[derive(Debug, Clone, PartialEq)]
pub enum FaultParameters {
    /// No fault detection.
    FaultNone,
    /// Fault on characterstring match.
    FaultCharacterString { fault_values: Vec<String> },
    /// Vendor-defined fault algorithm.
    FaultExtended {
        vendor_id: u16,
        extended_fault_type: u32,
        parameters: Vec<u8>,
    },
    /// Fault on life safety state match.
    FaultLifeSafety {
        fault_values: Vec<u32>,
        mode_for_reference: BACnetDeviceObjectPropertyReference,
    },
    /// Fault on property state match.
    FaultState {
        fault_values: Vec<BACnetPropertyStates>,
    },
    /// Fault on status flags change.
    FaultStatusFlags {
        reference: BACnetDeviceObjectPropertyReference,
    },
    /// Fault when value exceeds range.
    FaultOutOfRange { min_normal: f64, max_normal: f64 },
    /// Fault from listed reference.
    FaultListed {
        reference: BACnetDeviceObjectPropertyReference,
    },
}

// ---------------------------------------------------------------------------
// BACnetRecipientProcess (Clause 21)
// ---------------------------------------------------------------------------

/// BACnet Recipient Process — a recipient with an associated process identifier.
#[derive(Debug, Clone, PartialEq)]
pub struct BACnetRecipientProcess {
    pub recipient: BACnetRecipient,
    pub process_identifier: u32,
}

// ---------------------------------------------------------------------------
// BACnetCOVSubscription (Clause 21)
// ---------------------------------------------------------------------------

/// BACnet COV Subscription — represents an active COV subscription.
///
/// The `monitored_property_reference` is a `BACnetObjectPropertyReference`
/// (object + property + optional index).
#[derive(Debug, Clone, PartialEq)]
pub struct BACnetCOVSubscription {
    pub recipient: BACnetRecipientProcess,
    pub monitored_property_reference: BACnetObjectPropertyReference,
    pub issue_confirmed_notifications: bool,
    pub time_remaining: u32,
    pub cov_increment: Option<f32>,
}

// ---------------------------------------------------------------------------
// BACnetValueSource (Clause 21)
// ---------------------------------------------------------------------------

/// BACnet Value Source — identifies the source of a property value write.
#[derive(Debug, Clone, PartialEq)]
pub enum BACnetValueSource {
    None,
    Object(ObjectIdentifier),
    Address(BACnetAddress),
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
