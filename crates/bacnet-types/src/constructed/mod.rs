//! BACnet constructed data types per ASHRAE 135-2020.
//!
//! This module provides compound/structured types that are used by higher-level
//! BACnet objects (Calendar, Schedule, TrendLog, NotificationClass, Loop, etc.).
//! All types follow the same `no_std`-compatible pattern used in `primitives.rs`.

#[cfg(not(feature = "std"))]
use alloc::{string::String, vec::Vec};

use crate::bitstring::{DaysOfWeek, EventTransitionBits};
use crate::enums::LifeSafetyState;
use crate::error::Error;
use crate::primitives::{Date, ObjectIdentifier, PropertyValue, Time};
use crate::MacAddr;

mod action;
pub use action::{BACnetActionCommand, BACnetActionList};
mod access;
pub use access::{
    BACnetAccessRule, BACnetAssignedAccessRights, BACnetAuthenticationFactor,
    BACnetAuthenticationFactorFormat, BACnetAuthenticationPolicy, BACnetAuthenticationPolicyEntry,
    BACnetCredentialAuthenticationFactor,
};
mod audit;
pub use audit::{
    AuditPropertyReference, BACnetAuditLogDatum, BACnetAuditLogQueryParameters,
    BACnetAuditLogRecord, BACnetAuditLogRecordResult, BACnetAuditNotification,
    BACnetObjectSelector,
};
mod color;
pub use color::{BACnetColorCommand, BACnetXyColor};
mod event_notification;
pub use event_notification::{
    ChangeOfValueChoice, EventNotificationRequest, NotificationParameters,
};
mod lift;
pub use lift::{
    AssignedLandingCall, BACnetAssignedLandingCalls, BACnetLandingCallStatus,
    BACnetLandingDoorStatus, BACnetLiftCarCallList, LandingCallCommand, LandingDoor,
};
mod lighting;
pub use lighting::BACnetLightingCommand;
mod log;
pub use log::{
    BACnetEventLogRecord, BACnetLogMultipleRecord, BACnetLogRecord, EventLogDatum, LogData,
    LogDatum, LogValue,
};
mod network_port;
pub use network_port::{BACnetBDTEntry, BACnetFDTEntry, BACnetHostAddress, BACnetHostNPort};
mod property_access;
pub use property_access::{AccessResult, BACnetPropertyAccessResult};
mod property_value;
pub use property_value::BACnetPropertyValue;
mod read_access;
pub use read_access::{PropertyReference, ReadAccessSpecification};
mod staging;
pub use staging::BACnetStageLimitValue;

// ---------------------------------------------------------------------------
// BACnetDateRange (Clause 21 -- used by CalendarEntry and BACnetSpecialEvent)
// ---------------------------------------------------------------------------

/// BACnet date range: a SEQUENCE of start and end Date values.
///
/// On the wire each Date is application-tagged; the codec is
/// `bacnet_encoding::constructed::{encode_date_range, decode_date_range}`.
/// An unspecified start or end date leaves that side of the range open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BACnetDateRange {
    /// The start of the date range (inclusive).
    pub start_date: Date,
    /// The end of the date range (inclusive).
    pub end_date: Date,
}

// ---------------------------------------------------------------------------
// BACnetWeekNDay (Clause 21 -- used by CalendarEntry)
// ---------------------------------------------------------------------------

/// BACnet Week-And-Day: OCTET STRING(3) encoding month, week_of_month,
/// and day_of_week.
///
/// Each field may be `0xFF` to mean "any" (wildcard).
///
/// - `month`: 1-12, 13=odd months, 14=even months, 0xFF=any
/// - `week_of_month`: 1-5 = the days numbered 1-7, 8-14, 15-21, 22-28 and
///   29-31; 6 = the last 7 days of the month; 7, 8 and 9 = the 7 days before
///   the last 7, 14 and 21 days; 0xFF=any
/// - `day_of_week`: 1=Monday..7=Sunday, 0xFF=any
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BACnetWeekNDay {
    /// Month (1-14, or 0xFF for any).
    pub month: u8,
    /// Week of month (1-9, or 0xFF for any).
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
// BACnetCalendarEntry (Clause 21.6 -- Calendar Date_List; Calendar object Clause 12.9)
// ---------------------------------------------------------------------------

/// BACnet calendar entry: a CHOICE between a specific date, a date range,
/// or a week-and-day pattern.
///
/// Context tags per spec:
/// - `[0]` Date
/// - `[1]` DateRange
/// - `[2]` WeekNDay
///
/// The wire codec is `bacnet_encoding::constructed::{encode_calendar_entry,
/// decode_calendar_entry}`.
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
// BACnetTimeValue (Clause 12.24 -- Schedule Weekly_Schedule; Clause 21.6)
// ---------------------------------------------------------------------------

/// BACnet time-value pair: a Time followed by a value of any primitive
/// datatype.
///
/// The value is typed, so a Schedule writes it with its own datatype. Only
/// primitive values ([`PropertyValue::is_primitive`]) belong here; the codec
/// refuses to encode a `List` or `ApplicationData`.
#[derive(Debug, Clone, PartialEq)]
pub struct BACnetTimeValue {
    /// The time at which the value applies.
    pub time: Time,
    /// The value: NULL, BOOLEAN, Unsigned, INTEGER, REAL, Double, OCTET
    /// STRING, CharacterString, BIT STRING, ENUMERATED, Date, Time or
    /// BACnetObjectIdentifier.
    pub value: PropertyValue,
}

// ---------------------------------------------------------------------------
// SpecialEventPeriod (Clause 12.24 -- Schedule Exception_Schedule; Clause 21.6)
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
// BACnetSpecialEvent (Clause 12.24 -- Schedule Exception_Schedule; Clause 21.6)
// ---------------------------------------------------------------------------

/// BACnet special event: an exception schedule entry combining a period
/// definition, a list of time-value pairs, and a priority.
#[derive(Debug, Clone, PartialEq)]
pub struct BACnetSpecialEvent {
    /// The period this special event applies to.
    pub period: SpecialEventPeriod,
    /// Ordered list of time-value pairs to apply during this period.
    pub list_of_time_values: Vec<BACnetTimeValue>,
    /// Priority for conflict resolution, 1 (highest) to 16 (lowest).
    ///
    /// Held as the full Unsigned the wire can carry, so a decoded event keeps
    /// a priority outside that range intact for its consumer to refuse; the
    /// Schedule object answers such a value with VALUE_OUT_OF_RANGE.
    pub event_priority: u64,
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

    /// Whether the reference names nothing: its object is at the reserved
    /// instance 4194303, which Clause 12.1 lets an identifier hold to mean
    /// uninitialized or unused. A property of this type has no empty
    /// encoding, so the stack serves an unset one in this form (#1417).
    pub fn is_unset(&self) -> bool {
        self.object_identifier.instance_number() == ObjectIdentifier::WILDCARD_INSTANCE
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

    /// Whether the device identifier is absent or names a Device object,
    /// the rule [`BACnetDeviceObjectReference::device_identifier_is_device`]
    /// applies to that type's device member.
    pub fn device_identifier_is_device(&self) -> bool {
        device_identifier_is_device(self.device_identifier)
    }

    /// Whether the reference names nothing: its object, or its Device when
    /// it has one, is at the reserved instance 4194303 (Clause 12.1), the
    /// rule Clause 12.30.11 gives an empty Trend Log Multiple element. A
    /// property of this type has no empty encoding, so the stack serves an
    /// unset one in this form (#1417).
    pub fn is_unset(&self) -> bool {
        let reserved =
            |oid: ObjectIdentifier| oid.instance_number() == ObjectIdentifier::WILDCARD_INSTANCE;
        reserved(self.object_identifier) || self.device_identifier.is_some_and(reserved)
    }
}

/// Whether an optional device member is absent or a Device object
/// identifier, the only object type the member of a device-qualified
/// reference can hold (Clause 21). The `device_identifier_is_device` methods
/// of both reference types apply it, and so does every check the objects
/// and the Python bindings run on a device member.
pub fn device_identifier_is_device(device: Option<ObjectIdentifier>) -> bool {
    device.is_none_or(|device| device.object_type() == crate::enums::ObjectType::DEVICE)
}

// ---------------------------------------------------------------------------
// BACnetAddress (Clause 21 -- network address used by BACnetRecipient)
// ---------------------------------------------------------------------------

/// A BACnet network address: network number (0 = local) plus MAC address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BACnetAddress {
    /// Network number (0 = local network, 1-65534 = remote, 65535 = broadcast).
    pub network_number: u16,
    /// MAC-layer address (variable length, at most [`Self::MAX_MAC_LEN`] on the
    /// wire; empty = broadcast).
    pub mac_address: MacAddr,
}

impl BACnetAddress {
    /// The longest `mac_address`, in octets, that this stack encodes or decodes
    /// in any `BACnetAddress` (#1124, #1156), and of any DADR or SADR the
    /// network layer encodes or decodes (#1141).
    ///
    /// Clause 21 puts no length on the OCTET STRING, but a MAC is only useful
    /// if it names a node on some data link. Table 6-2 gives the network-layer
    /// address length of each standard data link, and the longest there is 7
    /// octets (a LonTalk Neuron_ID destination). The data links this stack
    /// serves address a node in at most 18 octets: 1 for MS/TP, 6 for B/IP,
    /// Ethernet and B/SC, and 18 for B/IPv6, whose port names a peer by its
    /// 16-octet IPv6 address and 2-octet UDP port. A longer MAC names no node
    /// on any of them, nor on a standard data link behind a router.
    ///
    /// Every codec for this type in bacnet-encoding holds to the bound: the
    /// recipient, ValueSource and AuditLogQuery decoders refuse a longer MAC
    /// and their encoders refuse to write one. The local setters that store a
    /// configured recipient (a Recipient_List destination or the
    /// Audit_Notification_Recipient) refuse one too, so a stored recipient
    /// always decodes again. The NPDU codec refuses a longer DLEN or SLEN
    /// (`NpduAddress::MAX_MAC_LEN` in bacnet-encoding), and bacnet-network's
    /// `NetworkLayer` and `BACnetRouter` drop a frame whose link-layer source
    /// MAC is longer (#1198), so the source addresses the stack learns off the
    /// network, which COV subscription lists and audit records report, fit the
    /// bound as well. So does the device MAC a You-Are request assigns (#1200).
    pub const MAX_MAC_LEN: usize = 18;

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
// BACnetDestination -- notification recipient, schedule, and delivery options
// ---------------------------------------------------------------------------

/// A single entry in a NotificationClass recipient list.
///
/// Specifies *who* receives the notification, *when* (days/times), and *how*
/// (confirmed vs. unconfirmed, which transition types).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BACnetDestination {
    /// Weekdays when this destination accepts notifications.
    pub valid_days: DaysOfWeek,
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
    /// Event transitions whose notifications this destination receives.
    pub transitions: EventTransitionBits,
}

// ---------------------------------------------------------------------------
// BACnetEventNotificationSubscription (Clause 21)
// ---------------------------------------------------------------------------

/// One entry of a Notification Forwarder's Subscribed_Recipients (Clause
/// 12.51.9): a recipient that asked for forwarded notifications for a limited
/// time.
///
/// The recipient and process identifier identify the entry: the list services
/// match on those two members alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BACnetEventNotificationSubscription {
    /// Device or address that receives the notifications (`[0]`).
    pub recipient: BACnetRecipient,
    /// Process on the recipient that receives them (`[1]`).
    pub process_identifier: u32,
    /// `true` for confirmed notifications, `false` for unconfirmed ones (`[2]`).
    pub issue_confirmed_notifications: bool,
    /// Minutes left before the entry lapses (`[3]`). Unlike COV subscription
    /// lifetimes this counts minutes, not seconds.
    pub time_remaining: u32,
}

// ---------------------------------------------------------------------------
// BACnetPortPermission (Clause 21)
// ---------------------------------------------------------------------------

/// One element of a Notification Forwarder's Port_Filter (Clause 12.51.11):
/// whether notifications that arrive through one network port are forwarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BACnetPortPermission {
    /// The port, by its Clause 6 port ID; 0 on a node that does not route
    /// (`[0]`).
    pub port_id: u8,
    /// `true` when notifications received through the port are forwarded
    /// (`[1]`).
    pub enabled: bool,
}

// ---------------------------------------------------------------------------
// BACnetScale (Clause 21)
// ---------------------------------------------------------------------------

/// BACnet Scale: CHOICE { float-scale \[0\] Real, integer-scale \[1\] Integer }.
#[derive(Debug, Clone, PartialEq)]
pub enum BACnetScale {
    /// Present_Value is multiplied by this factor to get engineering units.
    FloatScale(f32),
    /// Present_Value is multiplied by ten raised to this power to get engineering units.
    IntegerScale(i32),
}

// ---------------------------------------------------------------------------
// BACnetPrescale (Clause 21)
// ---------------------------------------------------------------------------

/// BACnet Prescale: SEQUENCE { multiplier Unsigned, modulo-divide Unsigned }.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BACnetPrescale {
    /// Numerator of the pulse-to-value conversion ratio, added to the accumulator per input pulse.
    pub multiplier: u32,
    /// Denominator of the conversion ratio; each time the accumulator reaches it, the value steps
    /// by one.
    pub modulo_divide: u32,
}

// ---------------------------------------------------------------------------
// BACnetShedLevel (Clause 21 — used by LoadControl)
// ---------------------------------------------------------------------------

/// A Load Control shed level (`BACnetShedLevel`, Clause 21): the datatype of
/// Requested_Shed_Level, Expected_Shed_Level and Actual_Shed_Level
/// (Clause 12.28).
///
/// On the wire each alternative is one primitive context tag: percent `[0]`,
/// level `[1]`, amount `[2]`. The codec is
/// `bacnet_encoding::constructed::{encode_shed_level, decode_shed_level}`.
#[derive(Debug, Clone, PartialEq)]
pub enum BACnetShedLevel {
    /// The load to run at, as a percentage of the baseline (Unsigned).
    Percent(u64),
    /// A preconfigured shed level (Unsigned). Level 0 means no shed.
    Level(u64),
    /// Kilowatts to take off the baseline (REAL).
    Amount(f32),
}

// ---------------------------------------------------------------------------
// BACnetDeviceObjectReference (Clause 21 -- used by Access Control objects)
// ---------------------------------------------------------------------------

/// BACnet Device Object Reference.
///
/// Clause 21 encodes the optional device identifier as context `[0]` followed
/// by the required object identifier as context `[1]`. `None` identifies a
/// local object, including Staging target references.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BACnetDeviceObjectReference {
    /// Optional device identifier (None = local device).
    pub device_identifier: Option<ObjectIdentifier>,
    /// The object being referenced.
    pub object_identifier: ObjectIdentifier,
}

impl BACnetDeviceObjectReference {
    /// Whether the device identifier is absent or names a Device object.
    ///
    /// The Clause 21 production gives the first member to the Device that
    /// holds the object, so an identifier of any other object type can't make
    /// a valid reference. Setters and write paths that take these references
    /// refuse one that fails this check.
    pub fn device_identifier_is_device(&self) -> bool {
        device_identifier_is_device(self.device_identifier)
    }
}

impl From<ObjectIdentifier> for BACnetDeviceObjectReference {
    /// A reference to an object in this device: no device identifier.
    fn from(object_identifier: ObjectIdentifier) -> Self {
        Self {
            device_identifier: None,
            object_identifier,
        }
    }
}

// ---------------------------------------------------------------------------
// FaultParameters (Clause 12.12 -- Fault_Parameters of Event Enrollment)
// ---------------------------------------------------------------------------

/// Fault parameter variants for configuring fault detection algorithms.
#[derive(Debug, Clone, PartialEq)]
pub enum FaultParameters {
    /// No fault detection.
    FaultNone,
    /// Fault on characterstring match.
    FaultCharacterString {
        /// Strings that, when the monitored value matches one, indicate a fault.
        fault_values: Vec<String>,
    },
    /// Vendor-defined fault algorithm.
    FaultExtended {
        /// Vendor identifier that owns the extended algorithm.
        vendor_id: u16,
        /// Vendor-defined identifier of the fault algorithm.
        extended_fault_type: u32,
        /// Pre-encoded, vendor-specific parameter bytes carried opaquely.
        parameters: Vec<u8>,
    },
    /// Fault on life safety state match.
    FaultLifeSafety {
        /// Life safety states that indicate a fault.
        fault_values: Vec<LifeSafetyState>,
        /// Reference to the mode property consulted when evaluating these states.
        mode_property_reference: BACnetDeviceObjectPropertyReference,
    },
    /// Fault on property state match.
    FaultState {
        /// Property states that indicate a fault when the monitored property takes one of them.
        fault_values: Vec<BACnetPropertyStates>,
    },
    /// Fault on status flags change.
    FaultStatusFlags {
        /// Reference to the StatusFlags property whose FAULT bit is monitored.
        reference: BACnetDeviceObjectPropertyReference,
    },
    /// Fault when value exceeds range.
    FaultOutOfRange {
        /// Lower bound of the normal range; values below it are a fault.
        min_normal: f64,
        /// Upper bound of the normal range; values above it are a fault.
        max_normal: f64,
    },
    /// Fault from listed reference.
    FaultListed {
        /// BACnetLIST property whose entries carry the fault indications to watch.
        reference: BACnetDeviceObjectPropertyReference,
    },
}

// ---------------------------------------------------------------------------
// BACnetRecipientProcess (Clause 21)
// ---------------------------------------------------------------------------

/// BACnet Recipient Process — a recipient with an associated process identifier.
#[derive(Debug, Clone, PartialEq)]
pub struct BACnetRecipientProcess {
    /// Device or address that receives the notifications.
    pub recipient: BACnetRecipient,
    /// Process on the recipient that asked for the notifications; echoed back in each one.
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
    /// Subscriber (device or address plus process identifier) that receives the notifications.
    pub recipient: BACnetRecipientProcess,
    /// Object and property being watched.
    pub monitored_property_reference: BACnetObjectPropertyReference,
    /// `true` for confirmed notifications, `false` for unconfirmed ones.
    pub issue_confirmed_notifications: bool,
    /// Seconds left before the subscription lapses; 0 means it never lapses.
    pub time_remaining: u32,
    /// COV increment in use for a numeric monitored property: the requested one, else the
    /// object's COV_Increment. `None` when the monitored property isn't numeric.
    pub cov_increment: Option<f32>,
}

// ---------------------------------------------------------------------------
// BACnetCOVMultipleSubscription (Clause 21)
// ---------------------------------------------------------------------------

/// One COV-multiple context of Device `Active_COV_Multiple_Subscriptions`:
/// a recipient and notification form with one remaining lifetime and maximum
/// notification delay shared by every nested COV reference.
#[derive(Debug, Clone, PartialEq)]
pub struct BACnetCOVMultipleSubscription {
    /// COV-client address and subscriber process identifier (`[0]`).
    pub recipient: BACnetRecipientProcess,
    /// Notification form of this context (`[1]`).
    pub issue_confirmed_notifications: bool,
    /// Remaining context lifetime in seconds (`[2]`).
    pub time_remaining: u32,
    /// Maximum notification delay in seconds (`[3]`).
    pub max_notification_delay: u32,
    /// Monitored objects and their COV references (`[4]`).
    pub list_of_cov_subscription_specifications: Vec<BACnetCOVSubscriptionSpecification>,
}

/// The COV references a COV-multiple context holds for one monitored object.
#[derive(Debug, Clone, PartialEq)]
pub struct BACnetCOVSubscriptionSpecification {
    /// Monitored object (`[0]`).
    pub monitored_object_identifier: ObjectIdentifier,
    /// COV references on that object (`[1]`).
    pub list_of_cov_references: Vec<BACnetCOVReference>,
}

/// One monitored property (a `BACnetPropertyReference`) of a COV-multiple
/// subscription specification.
#[derive(Debug, Clone, PartialEq)]
pub struct BACnetCOVReference {
    /// Monitored property (`[0]` property identifier).
    pub property_identifier: crate::enums::PropertyIdentifier,
    /// Optional array index; absence and zero are distinct.
    pub property_array_index: Option<u32>,
    /// COV increment in use (`[1]`), present for numeric monitored values.
    pub cov_increment: Option<f32>,
    /// Whether notifications carry the time of change (`[2]`).
    pub timestamped: bool,
}

// ---------------------------------------------------------------------------
// BACnetValueSource (Clause 21)
// ---------------------------------------------------------------------------

/// BACnet Value Source — identifies the source of a property value write.
#[derive(Debug, Clone, PartialEq)]
pub enum BACnetValueSource {
    /// No identified source: context \[0\] NULL.
    None,
    /// Source object, optionally qualified by a device: constructed context \[1\].
    Object(BACnetDeviceObjectReference),
    /// Source network/MAC address: constructed context \[2\].
    Address(BACnetAddress),
}

// ---------------------------------------------------------------------------
// BACnetEventParameter -- Event_Parameters algorithm alternatives
// ---------------------------------------------------------------------------

mod event_parameter;
pub use event_parameter::{event_parameter_tag, BACnetEventParameter, ChangeOfValueCriteria};

mod property_states;
pub use property_states::{
    BACnetExtendedPropertyState, BACnetPropertyStates, BACnetProprietaryPropertyState,
};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
