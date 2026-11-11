//! The parameters of an event notification (Clauses 13.8 and 13.9) and its
//! event values: what a notification service request carries and what an
//! Event Log record holds (Clause 12.27.13). The encoding crate frames them on
//! the wire.

#[cfg(not(feature = "std"))]
use alloc::{string::String, vec::Vec};

use super::{
    BACnetDeviceObjectPropertyReference, BACnetDeviceObjectReference, BACnetPropertyStates,
    BACnetPropertyValue,
};
use crate::enums::{
    AccessEvent, EventState, EventType, LifeSafetyMode, LifeSafetyOperation, LifeSafetyState,
    NotifyType, Reliability, TimerState, TimerTransition,
};
use crate::primitives::{BACnetTimeStamp, Date, ObjectIdentifier, StatusFlags, Time};

// ---------------------------------------------------------------------------
// EventNotificationRequest
// ---------------------------------------------------------------------------

/// ConfirmedEventNotification / UnconfirmedEventNotification request parameters.
///
/// The same members, from the process identifier through the event values,
/// make up the notification an Event Log record holds.
#[derive(Debug, Clone, PartialEq)]
pub struct EventNotificationRequest {
    /// Process identifier of the notification recipient.
    pub process_identifier: u32,
    /// Device that generated the event.
    pub initiating_device_identifier: ObjectIdentifier,
    /// Object that triggered the event.
    pub event_object_identifier: ObjectIdentifier,
    /// Timestamp of the event transition.
    pub timestamp: BACnetTimeStamp,
    /// Notification class for routing.
    pub notification_class: u32,
    /// Priority (0-255).
    pub priority: u8,
    /// Event algorithm that produced the notification.
    pub event_type: EventType,
    /// Optional message text (\[7\]).
    pub message_text: Option<String>,
    /// Whether this is an alarm, an event, or an acknowledgment notification.
    pub notify_type: NotifyType,
    /// Whether the recipient must acknowledge. Not encoded for an
    /// ACK_NOTIFICATION, which decodes with `false` here.
    pub ack_required: bool,
    /// Event state before this transition. Not encoded for an
    /// ACK_NOTIFICATION, which decodes with NORMAL here.
    pub from_state: EventState,
    /// Event state after this transition.
    pub to_state: EventState,
    /// Optional event values (tag \[12\]).
    pub event_values: Option<NotificationParameters>,
}

// ---------------------------------------------------------------------------
// NotificationParameters
// ---------------------------------------------------------------------------

/// Notification parameter variants for eventValues.
///
/// Each variant but the last is one alternative of the Clause 21
/// `BACnetNotificationParameters` CHOICE, under the context tag its doc
/// gives. An alternative under a tag the 2020 production gives none (7 and
/// 12, which it deprecates and reserves, or a later revision's, 23 and up)
/// decodes as [`Self::Opaque`], so one choice this stack doesn't model
/// doesn't cost the rest of the notification.
#[derive(Debug, Clone, PartialEq)]
pub enum NotificationParameters {
    /// \[0\] Change of bitstring.
    ChangeOfBitstring {
        /// Monitored bitstring value as `(unused_bits, data)`.
        referenced_bitstring: (u8, Vec<u8>),
        /// Status flags of the monitored object.
        status_flags: StatusFlags,
    },
    /// \[1\] Change of state.
    ChangeOfState {
        /// New BACnetPropertyStates value that triggered the notification.
        new_state: BACnetPropertyStates,
        /// Status flags of the monitored object.
        status_flags: StatusFlags,
    },
    /// \[2\] Change of value.
    ChangeOfValue {
        /// New value (changed bits or a REAL) that triggered the notification.
        new_value: ChangeOfValueChoice,
        /// Status flags of the monitored object.
        status_flags: StatusFlags,
    },
    /// \[3\] Command failure. Value byte vectors contain encoded BACnet TLVs.
    CommandFailure {
        /// Commanded value as encoded BACnet bytes.
        command_value: Vec<u8>,
        /// Status flags of the monitored object.
        status_flags: StatusFlags,
        /// Feedback value that disagreed with the command, as encoded BACnet bytes.
        feedback_value: Vec<u8>,
    },
    /// \[4\] Floating limit.
    FloatingLimit {
        /// Current value of the monitored property (not the setpoint reference).
        reference_value: f32,
        /// Status flags of the monitored object.
        status_flags: StatusFlags,
        /// Setpoint the reference value is tracked against.
        setpoint_value: f32,
        /// Differential limit (distance from the setpoint) that was exceeded.
        error_limit: f32,
    },
    /// \[5\] Out of range.
    OutOfRange {
        /// Monitored value that crossed a limit.
        exceeding_value: f32,
        /// Status flags of the monitored object.
        status_flags: StatusFlags,
        /// Deadband applied to the limit comparison, in the monitored value's units.
        deadband: f32,
        /// The limit that was exceeded, in the monitored value's units.
        exceeded_limit: f32,
    },
    /// \[6\] Complex event type.
    ComplexEventType {
        /// Reported property values; each holds encoded BACnet bytes and an optional priority.
        property_values: Vec<BACnetPropertyValue>,
    },
    /// \[8\] Change of life safety.
    ChangeOfLifeSafety {
        /// Life safety state the object entered.
        new_state: LifeSafetyState,
        /// Life safety mode in effect.
        new_mode: LifeSafetyMode,
        /// Status flags of the monitored object.
        status_flags: StatusFlags,
        /// Operation the object expects from an operator.
        operation_expected: LifeSafetyOperation,
    },
    /// \[9\] Extended (vendor-defined). `parameters` contains encoded BACnet TLVs.
    Extended {
        /// Vendor identifier that defines the extended event type.
        vendor_id: u16,
        /// Vendor-specific extended event type number.
        extended_event_type: u32,
        /// Vendor-defined parameters as encoded BACnet bytes.
        parameters: Vec<u8>,
    },
    /// \[10\] Buffer ready.
    BufferReady {
        /// Reference to the log buffer property that has records ready.
        buffer_property: BACnetDeviceObjectPropertyReference,
        /// Total record count at the previous notification for this buffer.
        previous_notification: u32,
        /// Total record count now.
        current_notification: u32,
    },
    /// \[11\] Unsigned range.
    UnsignedRange {
        /// Monitored value that crossed a limit.
        exceeding_value: u64,
        /// Status flags of the monitored object.
        status_flags: StatusFlags,
        /// The limit that was exceeded.
        exceeded_limit: u64,
    },
    /// \[13\] Access event. Authentication-factor bytes contain its three inner context fields.
    AccessEvent {
        /// Access event describing what happened.
        access_event: AccessEvent,
        /// Status flags of the monitored object.
        status_flags: StatusFlags,
        /// Counter distinguishing access events, carried as the `access-event-tag` field.
        access_event_tag: u32,
        /// Time of the access event; a BACnetTimeStamp on the wire, of which only the date-time
        /// choice is accepted.
        access_event_time: (Date, Time),
        /// Credential object (with optional device) that was presented.
        access_credential: BACnetDeviceObjectReference,
        /// Optional authentication factor as encoded BACnet bytes; `None` when absent.
        authentication_factor: Option<Vec<u8>>,
    },
    /// \[14\] Double out of range.
    DoubleOutOfRange {
        /// Monitored value that crossed a limit.
        exceeding_value: f64,
        /// Status flags of the monitored object.
        status_flags: StatusFlags,
        /// Deadband applied to the limit comparison, in the monitored value's units.
        deadband: f64,
        /// The limit that was exceeded, in the monitored value's units.
        exceeded_limit: f64,
    },
    /// \[15\] Signed out of range.
    SignedOutOfRange {
        /// Monitored value that crossed a limit.
        exceeding_value: i32,
        /// Status flags of the monitored object.
        status_flags: StatusFlags,
        /// Deadband applied to the limit comparison (unsigned).
        deadband: u64,
        /// The limit that was exceeded.
        exceeded_limit: i32,
    },
    /// \[16\] Unsigned out of range.
    UnsignedOutOfRange {
        /// Monitored value that crossed a limit.
        exceeding_value: u64,
        /// Status flags of the monitored object.
        status_flags: StatusFlags,
        /// Deadband applied to the limit comparison.
        deadband: u64,
        /// The limit that was exceeded.
        exceeded_limit: u64,
    },
    /// \[17\] Change of characterstring.
    ChangeOfCharacterstring {
        /// New value of the monitored character string.
        changed_value: String,
        /// Status flags of the monitored object.
        status_flags: StatusFlags,
        /// The configured alarm string that the new value matched.
        alarm_value: String,
    },
    /// \[18\] Change of status flags. `present_value` preserves absent and present-empty context \[0\].
    ChangeOfStatusFlags {
        /// Present value as encoded BACnet bytes; `None` when the field is absent.
        present_value: Option<Vec<u8>>,
        /// Status flags of the referenced object.
        referenced_flags: StatusFlags,
    },
    /// \[19\] Change of reliability. `property_values` contains encoded BACnet TLVs.
    ChangeOfReliability {
        /// Reliability value that triggered the notification.
        reliability: Reliability,
        /// Status flags of the monitored object.
        status_flags: StatusFlags,
        /// Properties relevant to the reliability change, as encoded BACnet bytes.
        property_values: Vec<u8>,
    },
    /// \[21\] Change of discrete value. `new_value` contains encoded BACnet TLVs.
    ChangeOfDiscreteValue {
        /// New discrete value as encoded BACnet bytes.
        new_value: Vec<u8>,
        /// Status flags of the monitored object.
        status_flags: StatusFlags,
    },
    /// \[22\] Change of timer.
    ChangeOfTimer {
        /// State the timer entered.
        new_state: TimerState,
        /// Status flags of the monitored object.
        status_flags: StatusFlags,
        /// Date and time of the timer update.
        update_time: (Date, Time),
        /// Transition of the last state change; `None` when absent.
        last_state_change: Option<TimerTransition>,
        /// Initial timeout configured for the timer; `None` when absent.
        initial_timeout: Option<u32>,
        /// Date and time when the timer expires; `None` when absent.
        expiration_time: Option<(Date, Time)>,
    },
    /// An alternative under a context tag the 2020 production doesn't
    /// assign, kept as received, as `BACnetEventParameter::Opaque` keeps one
    /// in Event_Parameters.
    ///
    /// Every alternative of this CHOICE is a SEQUENCE, so `data` is the
    /// complete body between the opening and closing `tag`. The encoder
    /// writes it back unchanged. It refuses a tag that has a variant here,
    /// 20 (left out, matching the event type that reports no values) and
    /// 255, and a body that isn't complete tagged values.
    Opaque {
        /// The context tag: 7, 12, or 23 to 254.
        tag: u8,
        /// The encoded SEQUENCE body, without its enclosing tag pair.
        data: Vec<u8>,
    },
}

/// CHOICE within ChangeOfValue notification parameters.
#[derive(Debug, Clone, PartialEq)]
pub enum ChangeOfValueChoice {
    /// Bitstring value that changed.
    ChangedBits {
        /// Number of unused trailing bits in the last byte of `data`.
        unused_bits: u8,
        /// Bitstring content bytes.
        data: Vec<u8>,
    },
    /// REAL value that changed.
    ChangedValue(f32),
}
