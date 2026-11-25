//! The NotificationParameters choices that report a changed value, a
//! command, life safety, a vendor's extension or a buffer, one decoder per
//! choice. [`decode_bounded`](super::parameters_decode::decode_bounded)
//! checks the choice's framing and picks the decoder by its tag.

use super::decode_helpers::{
    decode_context_status_flags, decode_context_u16, decode_context_value, finish_variant,
};
use super::*;
use crate::constructed::tagged::decode_ctx_character_string;

/// `[0]` change of bitstring: the fields up to `variant_body_end`.
pub(super) fn decode_change_of_bitstring(
    data: &[u8],
    inner_start: usize,
    variant_body_end: usize,
) -> Result<NotificationParameters, Error> {
    // [0] referenced-bitstring
    let ((unused, bits), pos) = decode_context_value(
        data,
        inner_start,
        0,
        "ChangeOfBitstring referenced-bitstring",
        primitives::decode_bit_string,
    )?;
    // [1] status-flags
    let (status_flags, pos) =
        decode_context_status_flags(data, pos, 1, "ChangeOfBitstring status-flags")?;
    finish_variant(
        NotificationParameters::ChangeOfBitstring {
            referenced_bitstring: (unused, bits),
            status_flags,
        },
        pos,
        variant_body_end,
        0,
    )
}

/// `[1]` change of state: the fields up to `variant_body_end`.
pub(super) fn decode_change_of_state(
    data: &[u8],
    inner_start: usize,
    variant_body_end: usize,
) -> Result<NotificationParameters, Error> {
    let mut pos = inner_start;
    // [0] new-state: BACnetPropertyStates — wrapped in opening/closing [0]
    let (t, p) = tags::decode_tag(data, pos)?;
    if !t.is_opening || t.number != 0 {
        return Err(misplaced_tag(
            data,
            &t,
            Some(0),
            pos,
            "ChangeOfState: expected opening tag [0] for new-state",
        ));
    }
    pos = p;
    let new_state = decode_property_states(data, &mut pos)?;
    // Skip closing tag [0]
    let (ct, cp) = tags::decode_tag(data, pos)?;
    if !ct.is_closing || ct.number != 0 {
        return Err(Error::decoding_kind(
            unclosed_kind(&ct),
            pos,
            "ChangeOfState: expected closing [0]",
        ));
    }
    pos = cp;
    // [1] status-flags
    let (status_flags, pos) =
        decode_context_status_flags(data, pos, 1, "ChangeOfState status-flags")?;
    finish_variant(
        NotificationParameters::ChangeOfState {
            new_state,
            status_flags,
        },
        pos,
        variant_body_end,
        1,
    )
}

/// `[2]` change of value: the fields up to `variant_body_end`.
pub(super) fn decode_change_of_value(
    data: &[u8],
    inner_start: usize,
    variant_body_end: usize,
) -> Result<NotificationParameters, Error> {
    let mut pos = inner_start;
    // [0] new-value CHOICE — wrapped in opening/closing [0]
    let (t, p) = tags::decode_tag(data, pos)?;
    if !t.is_opening || t.number != 0 {
        return Err(misplaced_tag(
            data,
            &t,
            Some(0),
            pos,
            "ChangeOfValue: expected opening [0] for new-value",
        ));
    }
    pos = p;
    // Peek CHOICE tag
    let (choice_tag, _) = tags::decode_tag(data, pos)?;
    let new_value = match choice_tag.number {
        0 if choice_tag.is_context(0) => {
            let ((unused_bits, data), end) = decode_context_value(
                data,
                pos,
                0,
                "ChangeOfValue changed-bits",
                primitives::decode_bit_string,
            )?;
            pos = end;
            ChangeOfValueChoice::ChangedBits { unused_bits, data }
        }
        1 if choice_tag.is_context(1) => {
            let (value, end) = decode_context_value(
                data,
                pos,
                1,
                "ChangeOfValue changed-value",
                primitives::decode_real,
            )?;
            pos = end;
            ChangeOfValueChoice::ChangedValue(value)
        }
        _ => {
            return Err(misplaced_tag(
                data,
                &choice_tag,
                None,
                pos,
                "ChangeOfValue: expected context choice [0] or [1]",
            ));
        }
    };
    // Closing tag [0]
    let (ct, cp) = tags::decode_tag(data, pos)?;
    if !ct.is_closing || ct.number != 0 {
        return Err(Error::decoding_kind(
            unclosed_kind(&ct),
            pos,
            "ChangeOfValue: expected closing [0]",
        ));
    }
    pos = cp;
    // [1] status-flags
    let (status_flags, pos) =
        decode_context_status_flags(data, pos, 1, "ChangeOfValue status-flags")?;
    finish_variant(
        NotificationParameters::ChangeOfValue {
            new_value,
            status_flags,
        },
        pos,
        variant_body_end,
        2,
    )
}

/// `[3]` command failure: the fields up to `variant_body_end`.
pub(super) fn decode_command_failure(
    data: &[u8],
    inner_start: usize,
    variant_body_end: usize,
) -> Result<NotificationParameters, Error> {
    let mut pos = inner_start;
    // [0] command-value — opening/closing, raw
    let (t, p) = tags::decode_tag(data, pos)?;
    if !t.is_opening || t.number != 0 {
        return Err(misplaced_tag(
            data,
            &t,
            Some(0),
            pos,
            "CommandFailure: expected opening [0]",
        ));
    }
    let (command_value, after) = extract_raw_context(data, p, 0)?;
    pos = after;
    // [1] status-flags
    let (status_flags, pos) =
        decode_context_status_flags(data, pos, 1, "CommandFailure status-flags")?;
    // [2] feedback-value — opening/closing, raw
    let (t, p) = tags::decode_tag(data, pos)?;
    if !t.is_opening || t.number != 2 {
        return Err(misplaced_tag(
            data,
            &t,
            Some(2),
            pos,
            "CommandFailure: expected opening [2]",
        ));
    }
    let (feedback_value, after) = extract_raw_context(data, p, 2)?;
    finish_variant(
        NotificationParameters::CommandFailure {
            command_value,
            status_flags,
            feedback_value,
        },
        after,
        variant_body_end,
        3,
    )
}

/// `[8]` change of life safety: the fields up to `variant_body_end`.
pub(super) fn decode_change_of_life_safety(
    data: &[u8],
    inner_start: usize,
    variant_body_end: usize,
) -> Result<NotificationParameters, Error> {
    let (new_state, pos) = decode_context_enum(
        data,
        inner_start,
        0,
        "ChangeOfLifeSafety new-state",
        LifeSafetyState::from_raw,
    )?;
    let (new_mode, pos) = decode_context_enum(
        data,
        pos,
        1,
        "ChangeOfLifeSafety new-mode",
        LifeSafetyMode::from_raw,
    )?;
    let (status_flags, pos) =
        decode_context_status_flags(data, pos, 2, "ChangeOfLifeSafety status-flags")?;
    let (operation_expected, pos) = decode_context_enum(
        data,
        pos,
        3,
        "ChangeOfLifeSafety operation-expected",
        LifeSafetyOperation::from_raw,
    )?;
    finish_variant(
        NotificationParameters::ChangeOfLifeSafety {
            new_state,
            new_mode,
            status_flags,
            operation_expected,
        },
        pos,
        variant_body_end,
        8,
    )
}

/// `[9]` extended: the fields up to `variant_body_end`.
pub(super) fn decode_extended(
    data: &[u8],
    inner_start: usize,
    variant_body_end: usize,
) -> Result<NotificationParameters, Error> {
    // [0] vendor-id
    let (vendor_id, pos) = decode_context_u16(data, inner_start, 0, "Extended vendor-id")?;
    // [1] extended-event-type
    let (extended_event_type, pos) =
        decode_context_u32(data, pos, 1, "Extended extended-event-type")?;
    // [2] parameters — opening/closing, raw
    let (t, p) = tags::decode_tag(data, pos)?;
    if !t.is_opening_tag(2) {
        return Err(misplaced_tag(
            data,
            &t,
            Some(2),
            pos,
            "Extended: expected opening [2]",
        ));
    }
    let (parameters, after) = extract_raw_context(data, p, 2)?;
    finish_variant(
        NotificationParameters::Extended {
            vendor_id,
            extended_event_type,
            parameters,
        },
        after,
        variant_body_end,
        9,
    )
}

/// `[10]` buffer ready: the fields up to `variant_body_end`.
pub(super) fn decode_buffer_ready(
    data: &[u8],
    inner_start: usize,
    variant_body_end: usize,
) -> Result<NotificationParameters, Error> {
    let mut pos = inner_start;
    // [0] buffer-property: BACnetDeviceObjectPropertyReference
    let (t, p) = tags::decode_tag(data, pos)?;
    if !t.is_opening || t.number != 0 {
        return Err(misplaced_tag(
            data,
            &t,
            Some(0),
            pos,
            "BufferReady: expected opening [0] for buffer-property",
        ));
    }
    pos = p;
    let buffer_property = decode_device_obj_prop_ref(data, &mut pos)?;
    // Closing tag [0]
    let (ct, cp) = tags::decode_tag(data, pos)?;
    if !ct.is_closing || ct.number != 0 {
        return Err(Error::decoding_kind(
            unclosed_kind(&ct),
            pos,
            "BufferReady: expected closing [0]",
        ));
    }
    pos = cp;
    // [1] previous-notification
    let (previous_notification, pos) =
        decode_context_u32(data, pos, 1, "BufferReady previous-notification")?;
    // [2] current-notification
    let (current_notification, end) =
        decode_context_u32(data, pos, 2, "BufferReady current-notification")?;
    finish_variant(
        NotificationParameters::BufferReady {
            buffer_property,
            previous_notification,
            current_notification,
        },
        end,
        variant_body_end,
        10,
    )
}

/// `[17]` change of characterstring: the fields up to `variant_body_end`.
pub(super) fn decode_change_of_characterstring(
    data: &[u8],
    inner_start: usize,
    variant_body_end: usize,
) -> Result<NotificationParameters, Error> {
    // [0] changed-value
    let (changed_value, pos) = decode_ctx_character_string(
        data,
        inner_start,
        0,
        "ChangeOfCharacterstring changed-value",
    )?;
    // [1] status-flags
    let (status_flags, pos) =
        decode_context_status_flags(data, pos, 1, "ChangeOfCharacterstring status-flags")?;
    // [2] alarm-value
    let (alarm_value, new_pos) =
        decode_ctx_character_string(data, pos, 2, "ChangeOfCharacterstring alarm-value")?;
    finish_variant(
        NotificationParameters::ChangeOfCharacterstring {
            changed_value,
            status_flags,
            alarm_value,
        },
        new_pos,
        variant_body_end,
        17,
    )
}

/// `[18]` change of status flags: the fields up to `variant_body_end`.
pub(super) fn decode_change_of_status_flags(
    data: &[u8],
    inner_start: usize,
    variant_body_end: usize,
) -> Result<NotificationParameters, Error> {
    let mut pos = inner_start;
    // [0] present-value — opening/closing, raw, optional
    let (t, p) = tags::decode_tag(data, pos)?;
    let present_value = if t.is_opening_tag(0) {
        let (value, after) = extract_raw_context(data, p, 0)?;
        pos = after;
        Some(value)
    } else {
        None
    };
    // [1] referenced-flags
    let (referenced_flags, pos) =
        decode_context_status_flags(data, pos, 1, "ChangeOfStatusFlags referenced-flags")?;
    finish_variant(
        NotificationParameters::ChangeOfStatusFlags {
            present_value,
            referenced_flags,
        },
        pos,
        variant_body_end,
        18,
    )
}

/// `[19]` change of reliability: the fields up to `variant_body_end`.
pub(super) fn decode_change_of_reliability(
    data: &[u8],
    inner_start: usize,
    variant_body_end: usize,
) -> Result<NotificationParameters, Error> {
    // [0] reliability
    let (reliability, pos) = decode_context_enum(
        data,
        inner_start,
        0,
        "ChangeOfReliability reliability",
        Reliability::from_raw,
    )?;
    // [1] status-flags
    let (status_flags, pos) =
        decode_context_status_flags(data, pos, 1, "ChangeOfReliability status-flags")?;
    // [2] property-values — opening/closing, raw
    let (t, p) = tags::decode_tag(data, pos)?;
    if !t.is_opening_tag(2) {
        return Err(misplaced_tag(
            data,
            &t,
            Some(2),
            pos,
            "ChangeOfReliability: expected opening [2]",
        ));
    }
    let (property_values, after) = extract_raw_context(data, p, 2)?;
    finish_variant(
        NotificationParameters::ChangeOfReliability {
            reliability,
            status_flags,
            property_values,
        },
        after,
        variant_body_end,
        19,
    )
}
