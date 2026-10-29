//! `BACnetEventParameter` — structured `Event_Parameters` algorithm choice.
//!
//! See the [`BACnetEventParameter`] type for details.

#[cfg(not(feature = "std"))]
use alloc::{string::String, vec::Vec};

use crate::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetExtendedPropertyState, BACnetPropertyStates,
    BACnetProprietaryPropertyState,
};
use crate::enums::{AccessEvent, LifeSafetyState, TimerState};
use crate::error::{DecodeError, DecodingKind, Error};
use crate::primitives::{PropertyValue, StatusFlags};

// ---------------------------------------------------------------------------
// Algorithm tag constants
// ---------------------------------------------------------------------------

/// Algorithm tag constants for the [`BACnetEventParameter`] CHOICE.
///
/// Each tag is also the number of the event type the algorithm reports
/// (Clause 21, `BACnetEventType`). They are used as the leading element of
/// the flat [`PropertyValue::List`] encoding so that an enrollment's
/// parameters survive a complete property round trip without requiring
/// ASN.1 context-tagged framing.
pub mod event_parameter_tag {
    /// Tag 0: the change-of-bitstring algorithm.
    pub const CHANGE_OF_BITSTRING: u8 = 0;
    /// Tag 1: the change-of-state algorithm.
    pub const CHANGE_OF_STATE: u8 = 1;
    /// Tag 2: the change-of-value algorithm.
    pub const CHANGE_OF_VALUE: u8 = 2;
    /// Tag 3: the command-failure algorithm.
    pub const COMMAND_FAILURE: u8 = 3;
    /// Tag 4: the floating-limit algorithm.
    pub const FLOATING_LIMIT: u8 = 4;
    /// Tag 5: the out-of-range algorithm.
    pub const OUT_OF_RANGE: u8 = 5;
    /// Tag 8: the change-of-life-safety algorithm.
    pub const CHANGE_OF_LIFE_SAFETY: u8 = 8;
    /// Tag 9: a vendor-defined algorithm (the `extended` alternative).
    pub const EXTENDED: u8 = 9;
    /// Tag 10: the buffer-ready algorithm.
    pub const BUFFER_READY: u8 = 10;
    /// Tag 11: the unsigned-range algorithm.
    pub const UNSIGNED_RANGE: u8 = 11;
    /// Tag 13: the access-event algorithm.
    pub const ACCESS_EVENT: u8 = 13;
    /// Tag 14: the double-out-of-range algorithm.
    pub const DOUBLE_OUT_OF_RANGE: u8 = 14;
    /// Tag 15: the signed-out-of-range algorithm.
    pub const SIGNED_OUT_OF_RANGE: u8 = 15;
    /// Tag 16: the unsigned-out-of-range algorithm.
    pub const UNSIGNED_OUT_OF_RANGE: u8 = 16;
    /// Tag 17: the change-of-characterstring algorithm.
    pub const CHANGE_OF_CHARACTERSTRING: u8 = 17;
    /// Tag 18: the change-of-status-flags algorithm.
    pub const CHANGE_OF_STATUS_FLAGS: u8 = 18;
    /// Tag 20: no algorithm (`none`).
    pub const NONE: u8 = 20;
    /// Tag 21: the change-of-discrete-value algorithm.
    pub const CHANGE_OF_DISCRETE_VALUE: u8 = 21;
    /// Tag 22: the change-of-timer algorithm.
    pub const CHANGE_OF_TIMER: u8 = 22;
}

/// The `cov-criteria [1] CHOICE` nested inside `change-of-value`.
///
/// The change-of-value algorithm monitors either a bitmask or
/// a referenced-property increment; this preserves which alternative is in
/// use along with its payload.
#[derive(Debug, Clone, PartialEq)]
pub enum ChangeOfValueCriteria {
    /// `bitmask [0] BIT STRING` — report when the masked bits change.
    Bitmask {
        /// Number of unused bits in the last byte of `data`.
        unused_bits: u8,
        /// The bitmask bytes.
        data: Vec<u8>,
    },
    /// `referenced-property-increment [1] REAL` — report on magnitude change.
    ReferencedPropertyIncrement(f32),
}

/// Structured `Event_Parameters` for an EventEnrollment object.
///
/// Models every algorithm alternative of the Clause 21 `BACnetEventParameter`
/// `CHOICE`. A context tag the 2020 production doesn't assign (a later
/// addendum's or a vendor's) is preserved verbatim via
/// [`BACnetEventParameter::Opaque`], so values written by a remote client
/// are never silently discarded.
///
/// The wire form is the full ASN.1 CHOICE framing of the Clause 21
/// production, produced by
/// `bacnet_encoding::constructed::encode_event_parameter` /
/// `decode_event_parameter` and carried in
/// [`PropertyValue::ApplicationData`]. The [`PropertyValue::List`] form here
/// (leading algorithm tag as [`PropertyValue::Unsigned`]; see
/// [`BACnetEventParameter::encode`] and [`BACnetEventParameter::decode`]) is
/// the LEGACY pre-framing layout, kept for compatibility with values written
/// before the framing migration (#154).
#[derive(Debug, Clone, PartialEq)]
pub enum BACnetEventParameter {
    /// `change-of-bitstring [0]`: report when masked bits match a value.
    ChangeOfBitstring {
        /// `time-delay [0] Unsigned` — debounce seconds.
        time_delay: u32,
        /// `bitmask [1] BIT STRING` — bits of interest.
        bitmask: (u8, Vec<u8>),
        /// `list-of-bitstring-values [2]`, a list of bit strings — alarm values.
        list_of_values: Vec<(u8, Vec<u8>)>,
    },
    /// `change-of-state [1]`: report when the monitored value matches a
    /// configured [`BACnetPropertyStates`] value.
    ChangeOfState {
        /// `time-delay [0] Unsigned` — debounce seconds.
        time_delay: u32,
        /// `list-of-values [1]`, a list of [`BACnetPropertyStates`] — alarm values.
        list_of_values: Vec<BACnetPropertyStates>,
    },
    /// `change-of-value [2]`: report on a bitmask or increment change.
    ChangeOfValue {
        /// `time-delay [0] Unsigned` — debounce seconds.
        time_delay: u32,
        /// `cov-criteria [1] CHOICE`.
        criteria: ChangeOfValueCriteria,
    },
    /// `command-failure [3]`: report when a commanded value and its feedback
    /// disagree.
    CommandFailure {
        /// `time-delay [0] Unsigned` — debounce seconds.
        time_delay: u32,
        /// `feedback-property-reference [1]` — where the feedback is read.
        feedback_property_reference: BACnetDeviceObjectPropertyReference,
    },
    /// `floating-limit [4]`: report when the value leaves a band around a
    /// setpoint reference.
    FloatingLimit {
        /// `time-delay [0] Unsigned` — debounce seconds.
        time_delay: u32,
        /// `setpoint-reference [1] BACnetDeviceObjectPropertyReference` — where
        /// the moving setpoint is read.
        setpoint_reference: BACnetDeviceObjectPropertyReference,
        /// `low-diff-limit [2] REAL` — how far below the setpoint the value may
        /// fall.
        low_diff_limit: f32,
        /// `high-diff-limit [3] REAL` — how far above the setpoint the value may
        /// rise.
        high_diff_limit: f32,
        /// `deadband [4] REAL` — hysteresis applied on the return to normal.
        deadband: f32,
    },
    /// `out-of-range [5]`: report when the value leaves a fixed band.
    OutOfRange {
        /// `time-delay [0] Unsigned` — debounce seconds.
        time_delay: u32,
        /// `low-limit [1] REAL`.
        low_limit: f32,
        /// `high-limit [2] REAL`.
        high_limit: f32,
        /// `deadband [3] REAL`.
        deadband: f32,
    },
    /// `change-of-life-safety [8]`: report when a life safety state enters
    /// one of the listed states, or the mode changes.
    ChangeOfLifeSafety {
        /// `time-delay [0] Unsigned` — debounce seconds.
        time_delay: u32,
        /// `list-of-life-safety-alarm-values [1]` — states that are
        /// life-safety alarms.
        list_of_life_safety_alarm_values: Vec<LifeSafetyState>,
        /// `list-of-alarm-values [2]` — states that are ordinary alarms.
        list_of_alarm_values: Vec<LifeSafetyState>,
        /// `mode-property-reference [3]` — where the mode is read.
        mode_property_reference: BACnetDeviceObjectPropertyReference,
    },
    /// `extended [9]`: vendor-defined algorithm.
    Extended {
        /// `vendor-id [0] Unsigned16`.
        vendor_id: u16,
        /// `extended-event-type [1] Unsigned`.
        extended_event_type: u32,
        /// Encoded `parameters [2]` list items (each a CHOICE), without the
        /// enclosing tag pair.
        parameters: Vec<u8>,
    },
    /// `buffer-ready [10]`: report when a log has gathered enough new
    /// records.
    BufferReady {
        /// `notification-threshold [0] Unsigned` — new records that trigger a
        /// notification.
        notification_threshold: u32,
        /// `previous-notification-count [1] Unsigned32` — the record count at
        /// the last notification.
        previous_notification_count: u32,
    },
    /// `unsigned-range [11]`: report when an Unsigned value leaves a band.
    UnsignedRange {
        /// `time-delay [0] Unsigned` — debounce seconds.
        time_delay: u32,
        /// `low-limit [1] Unsigned`.
        low_limit: u64,
        /// `high-limit [2] Unsigned`.
        high_limit: u64,
    },
    /// `access-event [13]`: report the listed access events.
    AccessEvent {
        /// `list-of-access-events [0]` — the events to report.
        list_of_access_events: Vec<AccessEvent>,
        /// `access-event-time-reference [1]` — where the event's time is read.
        access_event_time_reference: BACnetDeviceObjectPropertyReference,
    },
    /// `double-out-of-range [14]`: [`Self::OutOfRange`] for a Double value.
    DoubleOutOfRange {
        /// `time-delay [0] Unsigned` — debounce seconds.
        time_delay: u32,
        /// `low-limit [1] Double`.
        low_limit: f64,
        /// `high-limit [2] Double`.
        high_limit: f64,
        /// `deadband [3] Double`.
        deadband: f64,
    },
    /// `signed-out-of-range [15]`: [`Self::OutOfRange`] for a signed value.
    SignedOutOfRange {
        /// `time-delay [0] Unsigned` — debounce seconds.
        time_delay: u32,
        /// `low-limit [1] INTEGER`.
        low_limit: i32,
        /// `high-limit [2] INTEGER`.
        high_limit: i32,
        /// `deadband [3] Unsigned`.
        deadband: u64,
    },
    /// `unsigned-out-of-range [16]`: [`Self::OutOfRange`] for an Unsigned
    /// value.
    UnsignedOutOfRange {
        /// `time-delay [0] Unsigned` — debounce seconds.
        time_delay: u32,
        /// `low-limit [1] Unsigned`.
        low_limit: u64,
        /// `high-limit [2] Unsigned`.
        high_limit: u64,
        /// `deadband [3] Unsigned`.
        deadband: u64,
    },
    /// `change-of-characterstring [17]`: report when a string matches one of
    /// the listed values.
    ChangeOfCharacterstring {
        /// `time-delay [0] Unsigned` — debounce seconds.
        time_delay: u32,
        /// `list-of-alarm-values [1]` — the alarm strings.
        list_of_alarm_values: Vec<String>,
    },
    /// `change-of-status-flags [18]`: report when one of the selected status
    /// flags changes.
    ChangeOfStatusFlags {
        /// `time-delay [0] Unsigned` — debounce seconds.
        time_delay: u32,
        /// `selected-flags [1] BACnetStatusFlags` — the flags watched.
        selected_flags: StatusFlags,
    },
    /// `none [20] NULL`: no algorithm; the enrollment only reports faults.
    None,
    /// `change-of-discrete-value [21]`: report any change of a discrete
    /// value.
    ChangeOfDiscreteValue {
        /// `time-delay [0] Unsigned` — debounce seconds.
        time_delay: u32,
    },
    /// `change-of-timer [22]`: report when a Timer enters one of the listed
    /// states.
    ChangeOfTimer {
        /// `time-delay [0] Unsigned` — debounce seconds.
        time_delay: u32,
        /// `alarm-values [1]` — the timer states that are alarms.
        alarm_values: Vec<TimerState>,
        /// `update-time-reference [2]` — where the timer's update time is
        /// read.
        update_time_reference: BACnetDeviceObjectPropertyReference,
    },
    /// Preserves an alternative under a context tag the 2020 production
    /// doesn't assign.
    ///
    /// `data` is the complete SEQUENCE body. The framed encoder rejects the
    /// assigned, omitted, deprecated and reserved tags in this variant; tag
    /// 255 is the internal marker of a legacy raw-octet value.
    Opaque {
        /// The unrecognized algorithm tag.
        tag: u8,
        /// The raw parameter bytes.
        data: Vec<u8>,
    },
}

impl BACnetEventParameter {
    /// Return the algorithm tag for this alternative.
    pub fn tag(&self) -> u8 {
        use event_parameter_tag as T;
        match self {
            Self::ChangeOfBitstring { .. } => T::CHANGE_OF_BITSTRING,
            Self::ChangeOfState { .. } => T::CHANGE_OF_STATE,
            Self::ChangeOfValue { .. } => T::CHANGE_OF_VALUE,
            Self::CommandFailure { .. } => T::COMMAND_FAILURE,
            Self::FloatingLimit { .. } => T::FLOATING_LIMIT,
            Self::OutOfRange { .. } => T::OUT_OF_RANGE,
            Self::ChangeOfLifeSafety { .. } => T::CHANGE_OF_LIFE_SAFETY,
            Self::Extended { .. } => T::EXTENDED,
            Self::BufferReady { .. } => T::BUFFER_READY,
            Self::UnsignedRange { .. } => T::UNSIGNED_RANGE,
            Self::AccessEvent { .. } => T::ACCESS_EVENT,
            Self::DoubleOutOfRange { .. } => T::DOUBLE_OUT_OF_RANGE,
            Self::SignedOutOfRange { .. } => T::SIGNED_OUT_OF_RANGE,
            Self::UnsignedOutOfRange { .. } => T::UNSIGNED_OUT_OF_RANGE,
            Self::ChangeOfCharacterstring { .. } => T::CHANGE_OF_CHARACTERSTRING,
            Self::ChangeOfStatusFlags { .. } => T::CHANGE_OF_STATUS_FLAGS,
            Self::None => T::NONE,
            Self::ChangeOfDiscreteValue { .. } => T::CHANGE_OF_DISCRETE_VALUE,
            Self::ChangeOfTimer { .. } => T::CHANGE_OF_TIMER,
            Self::Opaque { tag, .. } => *tag,
        }
    }
}

mod codec;
mod fault;
mod legacy;
use codec::*;

#[cfg(test)]
mod tests;
