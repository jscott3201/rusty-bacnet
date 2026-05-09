use super::property_states::{
    decode_device_obj_prop_ref, decode_property_states, decode_status_flags,
    encode_property_states, extract_raw_context,
};
use super::*;

mod decode;
mod decode_timer;
mod encode;

// ---------------------------------------------------------------------------
// NotificationParameters
// ---------------------------------------------------------------------------

/// Notification parameter variants for eventValues.
#[derive(Debug, Clone, PartialEq)]
pub enum NotificationParameters {
    /// [0] Change of bitstring.
    ChangeOfBitstring {
        referenced_bitstring: (u8, Vec<u8>),
        status_flags: u8,
    },
    /// [1] Change of state.
    ChangeOfState {
        new_state: BACnetPropertyStates,
        status_flags: u8,
    },
    /// [2] Change of value.
    ChangeOfValue {
        new_value: ChangeOfValueChoice,
        status_flags: u8,
    },
    /// [3] Command failure.
    CommandFailure {
        command_value: Vec<u8>,
        status_flags: u8,
        feedback_value: Vec<u8>,
    },
    /// [4] Floating limit.
    FloatingLimit {
        reference_value: f32,
        status_flags: u8,
        setpoint_value: f32,
        error_limit: f32,
    },
    /// [5] Out of range.
    OutOfRange {
        exceeding_value: f32,
        status_flags: u8,
        deadband: f32,
        exceeded_limit: f32,
    },
    /// [8] Change of life safety.
    ChangeOfLifeSafety {
        new_state: u32,
        new_mode: u32,
        status_flags: u8,
        operation_expected: u32,
    },
    /// [9] Extended (vendor-defined).
    Extended {
        vendor_id: u16,
        extended_event_type: u32,
        parameters: Vec<u8>,
    },
    /// [10] Buffer ready.
    BufferReady {
        buffer_property: BACnetDeviceObjectPropertyReference,
        previous_notification: u32,
        current_notification: u32,
    },
    /// [11] Unsigned range.
    UnsignedRange {
        exceeding_value: u64,
        status_flags: u8,
        exceeded_limit: u64,
    },
    /// [13] Access event.
    AccessEvent {
        access_event: u32,
        status_flags: u8,
        access_event_tag: u32,
        access_event_time: (Date, Time),
        access_credential: BACnetDeviceObjectPropertyReference,
        authentication_factor: Vec<u8>,
    },
    /// [14] Double out of range.
    DoubleOutOfRange {
        exceeding_value: f64,
        status_flags: u8,
        deadband: f64,
        exceeded_limit: f64,
    },
    /// [15] Signed out of range.
    SignedOutOfRange {
        exceeding_value: i32,
        status_flags: u8,
        deadband: u64,
        exceeded_limit: i32,
    },
    /// [16] Unsigned out of range.
    UnsignedOutOfRange {
        exceeding_value: u64,
        status_flags: u8,
        deadband: u64,
        exceeded_limit: u64,
    },
    /// [17] Change of characterstring.
    ChangeOfCharacterstring {
        changed_value: String,
        status_flags: u8,
        alarm_value: String,
    },
    /// [18] Change of status flags.
    ChangeOfStatusFlags {
        present_value: Vec<u8>,
        referenced_flags: u8,
    },
    /// [19] Change of reliability.
    ChangeOfReliability {
        reliability: u32,
        status_flags: u8,
        property_values: Vec<u8>,
    },
    /// [20] None.
    NoneParams,
    /// [21] Change of discrete value.
    ChangeOfDiscreteValue {
        new_value: Vec<u8>,
        status_flags: u8,
    },
    /// [22] Change of timer.
    ChangeOfTimer {
        new_state: u32,
        status_flags: u8,
        update_time: (Date, Time),
        last_state_change: u32,
        initial_timeout: u32,
        expiration_time: (Date, Time),
    },
}

/// CHOICE within ChangeOfValue notification parameters.
#[derive(Debug, Clone, PartialEq)]
pub enum ChangeOfValueChoice {
    ChangedBits { unused_bits: u8, data: Vec<u8> },
    ChangedValue(f32),
}
