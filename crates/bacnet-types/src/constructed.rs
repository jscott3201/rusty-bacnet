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
    BooleanValue(bool),   // [0]
    BinaryValue(u32),     // [1] BACnetBinaryPV
    EventType(u32),       // [2]
    Polarity(u32),        // [3]
    ProgramChange(u32),   // [4]
    ProgramState(u32),    // [5]
    ReasonForHalt(u32),   // [6]
    Reliability(u32),     // [7]
    State(u32),           // [8] BACnetEventState
    SystemStatus(u32),    // [9]
    Units(u32),           // [10]
    LifeSafetyMode(u32),  // [12]
    LifeSafetyState(u32), // [13]
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
mod tests {
    use super::*;
    use crate::enums::{ObjectType, PropertyIdentifier};

    // --- BACnetDateRange ---

    #[test]
    fn date_range_encode_decode_round_trip() {
        let range = BACnetDateRange {
            start_date: Date {
                year: 124,
                month: 1,
                day: 1,
                day_of_week: 1,
            },
            end_date: Date {
                year: 124,
                month: 12,
                day: 31,
                day_of_week: 2,
            },
        };
        let encoded = range.encode();
        assert_eq!(encoded.len(), 8);
        let decoded = BACnetDateRange::decode(&encoded).unwrap();
        assert_eq!(range, decoded);
    }

    #[test]
    fn date_range_encode_decode_all_unspecified() {
        let range = BACnetDateRange {
            start_date: Date {
                year: Date::UNSPECIFIED,
                month: Date::UNSPECIFIED,
                day: Date::UNSPECIFIED,
                day_of_week: Date::UNSPECIFIED,
            },
            end_date: Date {
                year: Date::UNSPECIFIED,
                month: Date::UNSPECIFIED,
                day: Date::UNSPECIFIED,
                day_of_week: Date::UNSPECIFIED,
            },
        };
        let encoded = range.encode();
        let decoded = BACnetDateRange::decode(&encoded).unwrap();
        assert_eq!(range, decoded);
    }

    #[test]
    fn date_range_buffer_too_short() {
        // 7 bytes — one short
        let result = BACnetDateRange::decode(&[0; 7]);
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::BufferTooShort { need, have } => {
                assert_eq!(need, 8);
                assert_eq!(have, 7);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn date_range_buffer_empty() {
        let result = BACnetDateRange::decode(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn date_range_extra_bytes_ignored() {
        let range = BACnetDateRange {
            start_date: Date {
                year: 100,
                month: 6,
                day: 15,
                day_of_week: 5,
            },
            end_date: Date {
                year: 100,
                month: 6,
                day: 30,
                day_of_week: 6,
            },
        };
        let encoded = range.encode();
        let mut extended = encoded.to_vec();
        extended.extend_from_slice(&[0xFF, 0xFF]); // extra bytes
        let decoded = BACnetDateRange::decode(&extended).unwrap();
        assert_eq!(range, decoded);
    }

    // --- BACnetWeekNDay ---

    #[test]
    fn week_n_day_encode_decode_round_trip() {
        let wnd = BACnetWeekNDay {
            month: 3,
            week_of_month: 2,
            day_of_week: 5, // Friday
        };
        let encoded = wnd.encode();
        assert_eq!(encoded.len(), 3);
        let decoded = BACnetWeekNDay::decode(&encoded).unwrap();
        assert_eq!(wnd, decoded);
    }

    #[test]
    fn week_n_day_encode_decode_all_any() {
        let wnd = BACnetWeekNDay {
            month: BACnetWeekNDay::ANY,
            week_of_month: BACnetWeekNDay::ANY,
            day_of_week: BACnetWeekNDay::ANY,
        };
        let encoded = wnd.encode();
        assert_eq!(encoded, [0xFF, 0xFF, 0xFF]);
        let decoded = BACnetWeekNDay::decode(&encoded).unwrap();
        assert_eq!(wnd, decoded);
    }

    #[test]
    fn week_n_day_buffer_too_short() {
        // 2 bytes — one short
        let result = BACnetWeekNDay::decode(&[0x03, 0x02]);
        assert!(result.is_err());
        match result.unwrap_err() {
            Error::BufferTooShort { need, have } => {
                assert_eq!(need, 3);
                assert_eq!(have, 2);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn week_n_day_buffer_empty() {
        let result = BACnetWeekNDay::decode(&[]);
        assert!(result.is_err());
    }

    // --- BACnetObjectPropertyReference ---

    #[test]
    fn object_property_reference_basic_construction() {
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        let opr = BACnetObjectPropertyReference::new(oid, 85); // prop 85 = present-value
        assert_eq!(opr.object_identifier, oid);
        assert_eq!(opr.property_identifier, 85);
        assert_eq!(opr.property_array_index, None);
    }

    #[test]
    fn object_property_reference_with_index() {
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        let opr = BACnetObjectPropertyReference::new_indexed(oid, 85, 3);
        assert_eq!(opr.property_array_index, Some(3));
    }

    #[test]
    fn object_property_reference_equality() {
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 5).unwrap();
        let a = BACnetObjectPropertyReference::new(oid, 85);
        let b = BACnetObjectPropertyReference::new(oid, 85);
        assert_eq!(a, b);

        let c = BACnetObjectPropertyReference::new(oid, 77); // different property
        assert_ne!(a, c);
    }

    // --- BACnetDeviceObjectPropertyReference ---

    #[test]
    fn device_object_property_reference_local() {
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 10).unwrap();
        let dopr = BACnetDeviceObjectPropertyReference::new_local(oid, 85);
        assert_eq!(dopr.object_identifier, oid);
        assert_eq!(dopr.property_identifier, 85);
        assert_eq!(dopr.property_array_index, None);
        assert_eq!(dopr.device_identifier, None);
    }

    #[test]
    fn device_object_property_reference_remote() {
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 10).unwrap();
        let dev_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap();
        let dopr = BACnetDeviceObjectPropertyReference::new_remote(oid, 85, dev_oid);
        assert_eq!(dopr.device_identifier, Some(dev_oid));
    }

    #[test]
    fn device_object_property_reference_with_index() {
        let oid = ObjectIdentifier::new(ObjectType::MULTI_STATE_INPUT, 3).unwrap();
        let dopr = BACnetDeviceObjectPropertyReference::new_local(oid, 74).with_index(2); // prop 74 = state-text
        assert_eq!(dopr.property_array_index, Some(2));
        assert_eq!(dopr.device_identifier, None);
    }

    // --- BACnetAddress ---

    #[test]
    fn bacnet_address_local_broadcast() {
        let addr = BACnetAddress::local_broadcast();
        assert_eq!(addr.network_number, 0);
        assert!(addr.mac_address.is_empty());
    }

    #[test]
    fn bacnet_address_from_ip() {
        let ip_port: [u8; 6] = [192, 168, 1, 100, 0xBA, 0xC0]; // 192.168.1.100:47808
        let addr = BACnetAddress::from_ip(ip_port);
        assert_eq!(addr.network_number, 0);
        assert_eq!(addr.mac_address.as_slice(), &ip_port);
    }

    // --- BACnetRecipient ---

    #[test]
    fn bacnet_recipient_device_variant() {
        let dev_oid = ObjectIdentifier::new(ObjectType::DEVICE, 42).unwrap();
        let recipient = BACnetRecipient::Device(dev_oid);
        match recipient {
            BACnetRecipient::Device(oid) => assert_eq!(oid.instance_number(), 42),
            BACnetRecipient::Address(_) => panic!("wrong variant"),
        }
    }

    #[test]
    fn bacnet_recipient_address_variant() {
        let addr = BACnetAddress {
            network_number: 100,
            mac_address: MacAddr::from_slice(&[0x01, 0x02, 0x03]),
        };
        let recipient = BACnetRecipient::Address(addr.clone());
        match recipient {
            BACnetRecipient::Device(_) => panic!("wrong variant"),
            BACnetRecipient::Address(a) => assert_eq!(a, addr),
        }
    }

    // --- BACnetDestination ---

    #[test]
    fn bacnet_destination_construction() {
        let dev_oid = ObjectIdentifier::new(ObjectType::DEVICE, 99).unwrap();
        let dest = BACnetDestination {
            valid_days: 0b0111_1111, // all days
            from_time: Time {
                hour: 0,
                minute: 0,
                second: 0,
                hundredths: 0,
            },
            to_time: Time {
                hour: 23,
                minute: 59,
                second: 59,
                hundredths: 99,
            },
            recipient: BACnetRecipient::Device(dev_oid),
            process_identifier: 1,
            issue_confirmed_notifications: true,
            transitions: 0b0000_0111, // all transitions
        };
        assert_eq!(dest.valid_days & 0x7F, 0x7F);
        assert!(dest.issue_confirmed_notifications);
        assert_eq!(dest.transitions & 0x07, 0x07);
    }

    // --- LogDatum ---

    #[test]
    fn log_datum_variants_clone_eq() {
        let real = LogDatum::RealValue(72.5_f32);
        assert_eq!(real.clone(), LogDatum::RealValue(72.5_f32));

        let bits = LogDatum::BitstringValue {
            unused_bits: 3,
            data: vec![0b1010_0000],
        };
        assert_eq!(bits.clone(), bits);

        let fail = LogDatum::Failure {
            error_class: 2,
            error_code: 31,
        };
        assert_eq!(fail.clone(), fail);

        assert_eq!(LogDatum::NullValue, LogDatum::NullValue);
        assert_ne!(LogDatum::BooleanValue(true), LogDatum::BooleanValue(false));
    }

    // --- BACnetLogRecord ---

    #[test]
    fn log_record_construction() {
        let record = BACnetLogRecord {
            date: Date {
                year: 124,
                month: 3,
                day: 15,
                day_of_week: 5,
            },
            time: Time {
                hour: 10,
                minute: 30,
                second: 0,
                hundredths: 0,
            },
            log_datum: LogDatum::RealValue(23.4_f32),
            status_flags: None,
        };
        assert_eq!(record.date.year, 124);
        assert_eq!(record.status_flags, None);
    }

    #[test]
    fn log_record_with_status_flags() {
        let record = BACnetLogRecord {
            date: Date {
                year: 124,
                month: 1,
                day: 1,
                day_of_week: 1,
            },
            time: Time {
                hour: 0,
                minute: 0,
                second: 0,
                hundredths: 0,
            },
            log_datum: LogDatum::LogStatus(0b010), // buffer-purged
            status_flags: Some(0b0100),            // FAULT set
        };
        assert_eq!(record.status_flags, Some(0b0100));
        match record.log_datum {
            LogDatum::LogStatus(s) => assert_eq!(s, 0b010),
            _ => panic!("wrong datum variant"),
        }
    }

    // --- BACnetCalendarEntry ---

    #[test]
    fn calendar_entry_variants() {
        let date_entry = BACnetCalendarEntry::Date(Date {
            year: 124,
            month: 6,
            day: 15,
            day_of_week: 6,
        });
        let range_entry = BACnetCalendarEntry::DateRange(BACnetDateRange {
            start_date: Date {
                year: 124,
                month: 1,
                day: 1,
                day_of_week: 1,
            },
            end_date: Date {
                year: 124,
                month: 12,
                day: 31,
                day_of_week: 2,
            },
        });
        let wnd_entry = BACnetCalendarEntry::WeekNDay(BACnetWeekNDay {
            month: BACnetWeekNDay::ANY,
            week_of_month: 1,
            day_of_week: 1, // first Monday of every month
        });
        // Just verify they can be constructed and cloned
        let _a = date_entry.clone();
        let _b = range_entry.clone();
        let _c = wnd_entry.clone();
    }

    // --- BACnetSpecialEvent ---

    #[test]
    fn special_event_inline_calendar_entry() {
        let event = BACnetSpecialEvent {
            period: SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::WeekNDay(
                BACnetWeekNDay {
                    month: 12,
                    week_of_month: BACnetWeekNDay::ANY,
                    day_of_week: BACnetWeekNDay::ANY,
                },
            )),
            list_of_time_values: vec![BACnetTimeValue {
                time: Time {
                    hour: 8,
                    minute: 0,
                    second: 0,
                    hundredths: 0,
                },
                value: vec![0x10, 0x00], // raw-tagged Null
            }],
            event_priority: 16, // lowest priority
        };
        assert_eq!(event.event_priority, 16);
        assert_eq!(event.list_of_time_values.len(), 1);
    }

    #[test]
    fn special_event_calendar_reference() {
        let cal_oid = ObjectIdentifier::new(ObjectType::CALENDAR, 0).unwrap();
        let event = BACnetSpecialEvent {
            period: SpecialEventPeriod::CalendarReference(cal_oid),
            list_of_time_values: vec![],
            event_priority: 1, // highest priority
        };
        match &event.period {
            SpecialEventPeriod::CalendarReference(oid) => {
                assert_eq!(oid.instance_number(), 0);
            }
            SpecialEventPeriod::CalendarEntry(_) => panic!("wrong period variant"),
        }
    }

    // --- BACnetRecipientProcess ---

    #[test]
    fn recipient_process_construction() {
        let dev_oid = ObjectIdentifier::new(ObjectType::DEVICE, 100).unwrap();
        let rp = BACnetRecipientProcess {
            recipient: BACnetRecipient::Device(dev_oid),
            process_identifier: 42,
        };
        assert_eq!(rp.process_identifier, 42);
        match &rp.recipient {
            BACnetRecipient::Device(oid) => assert_eq!(oid.instance_number(), 100),
            BACnetRecipient::Address(_) => panic!("wrong variant"),
        }
    }

    // --- BACnetCOVSubscription ---

    #[test]
    fn cov_subscription_creation() {
        let dev_oid = ObjectIdentifier::new(ObjectType::DEVICE, 200).unwrap();
        let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        let sub = BACnetCOVSubscription {
            recipient: BACnetRecipientProcess {
                recipient: BACnetRecipient::Device(dev_oid),
                process_identifier: 7,
            },
            monitored_property_reference: BACnetObjectPropertyReference::new(
                ai_oid,
                PropertyIdentifier::PRESENT_VALUE.to_raw(),
            ),
            issue_confirmed_notifications: true,
            time_remaining: 300,
            cov_increment: Some(0.5),
        };
        assert_eq!(sub.recipient.process_identifier, 7);
        assert_eq!(
            sub.monitored_property_reference
                .object_identifier
                .instance_number(),
            1
        );
        assert!(sub.issue_confirmed_notifications);
        assert_eq!(sub.time_remaining, 300);
        assert_eq!(sub.cov_increment, Some(0.5));
    }

    #[test]
    fn cov_subscription_without_increment() {
        let dev_oid = ObjectIdentifier::new(ObjectType::DEVICE, 50).unwrap();
        let bv_oid = ObjectIdentifier::new(ObjectType::BINARY_VALUE, 3).unwrap();
        let sub = BACnetCOVSubscription {
            recipient: BACnetRecipientProcess {
                recipient: BACnetRecipient::Device(dev_oid),
                process_identifier: 1,
            },
            monitored_property_reference: BACnetObjectPropertyReference::new(
                bv_oid,
                PropertyIdentifier::PRESENT_VALUE.to_raw(),
            ),
            issue_confirmed_notifications: false,
            time_remaining: 0,
            cov_increment: None,
        };
        assert!(!sub.issue_confirmed_notifications);
        assert_eq!(sub.cov_increment, None);
    }

    // --- BACnetValueSource ---

    #[test]
    fn value_source_none_variant() {
        let vs = BACnetValueSource::None;
        assert_eq!(vs, BACnetValueSource::None);
    }

    #[test]
    fn value_source_object_variant() {
        let dev_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
        let vs = BACnetValueSource::Object(dev_oid);
        match vs {
            BACnetValueSource::Object(oid) => assert_eq!(oid.instance_number(), 1),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn value_source_address_variant() {
        let addr = BACnetAddress::from_ip([192, 168, 1, 10, 0xBA, 0xC0]);
        let vs = BACnetValueSource::Address(addr.clone());
        match vs {
            BACnetValueSource::Address(a) => assert_eq!(a, addr),
            _ => panic!("wrong variant"),
        }
    }
}
