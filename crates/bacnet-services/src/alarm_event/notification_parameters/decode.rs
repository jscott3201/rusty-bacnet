use super::decode_helpers::{
    closing_tag_start, decode_context_status_flags, decode_context_u16, decode_context_value,
    finish_variant as check_variant_end,
};
use super::decode_timer::{decode_change_of_discrete_value, decode_change_of_timer};
use super::*;
use crate::common::{decode_context, decode_context_u32};

impl NotificationParameters {
    /// Decode one notification-parameter choice, with an optional enclosing `[12]` close.
    pub fn decode(data: &[u8], offset: usize) -> Result<Self, Error> {
        let end = closing_tag_start(data, data.len(), 12, "NotificationParameters event-values")
            .unwrap_or(data.len());
        Self::decode_impl(data, offset, end)
    }

    pub(crate) fn decode_bounded(data: &[u8], offset: usize, end: usize) -> Result<Self, Error> {
        Self::decode_impl(data, offset, end)
    }

    fn decode_impl(data: &[u8], offset: usize, end: usize) -> Result<Self, Error> {
        let data = data
            .get(..end)
            .ok_or_else(|| Error::decoding(end, "NotificationParameters boundary exceeds input"))?;
        // Peek the inner opening tag to determine the variant
        if offset >= data.len() {
            return Err(Error::decoding(
                offset,
                "NotificationParameters: empty payload",
            ));
        }
        let (inner_tag, inner_start) = tags::decode_tag(data, offset)?;
        if !inner_tag.is_opening {
            return Err(Error::decoding(
                offset,
                "NotificationParameters: expected opening tag for variant",
            ));
        }
        let variant_tag = inner_tag.number;
        let variant_body_end = closing_tag_start(
            data,
            data.len(),
            variant_tag,
            "NotificationParameters variant",
        )?;
        let finish_variant =
            |value, consumed| check_variant_end(value, consumed, variant_body_end, variant_tag);

        match variant_tag {
            // [1] Change of state
            1 => {
                let mut pos = inner_start;
                // [0] new-state: BACnetPropertyStates — wrapped in opening/closing [0]
                let (t, p) = tags::decode_tag(data, pos)?;
                if !t.is_opening || t.number != 0 {
                    return Err(Error::decoding(
                        pos,
                        "ChangeOfState: expected opening tag [0] for new-state",
                    ));
                }
                pos = p;
                let new_state = decode_property_states(data, &mut pos)?;
                // Skip closing tag [0]
                let (ct, cp) = tags::decode_tag(data, pos)?;
                if !ct.is_closing || ct.number != 0 {
                    return Err(Error::decoding(pos, "ChangeOfState: expected closing [0]"));
                }
                pos = cp;
                // [1] status-flags
                let (status_flags, pos) =
                    decode_context_status_flags(data, pos, 1, "ChangeOfState status-flags")?;
                finish_variant(
                    Self::ChangeOfState {
                        new_state,
                        status_flags,
                    },
                    pos,
                )
            }
            // [2] Change of value
            2 => {
                let mut pos = inner_start;
                // [0] new-value CHOICE — wrapped in opening/closing [0]
                let (t, p) = tags::decode_tag(data, pos)?;
                if !t.is_opening || t.number != 0 {
                    return Err(Error::decoding(
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
                        return Err(Error::decoding(
                            pos,
                            "ChangeOfValue: expected context choice [0] or [1]",
                        ));
                    }
                };
                // Closing tag [0]
                let (ct, cp) = tags::decode_tag(data, pos)?;
                if !ct.is_closing || ct.number != 0 {
                    return Err(Error::decoding(pos, "ChangeOfValue: expected closing [0]"));
                }
                pos = cp;
                // [1] status-flags
                let (status_flags, pos) =
                    decode_context_status_flags(data, pos, 1, "ChangeOfValue status-flags")?;
                finish_variant(
                    Self::ChangeOfValue {
                        new_value,
                        status_flags,
                    },
                    pos,
                )
            }
            // [5] Out of range
            5 => {
                // [0] exceeding-value
                let (exceeding_value, pos) = decode_context_value(
                    data,
                    inner_start,
                    0,
                    "OutOfRange exceeding-value",
                    primitives::decode_real,
                )?;
                // [1] status-flags
                let (status_flags, pos) =
                    decode_context_status_flags(data, pos, 1, "OutOfRange status-flags")?;
                // [2] deadband
                let (deadband, pos) = decode_context_value(
                    data,
                    pos,
                    2,
                    "OutOfRange deadband",
                    primitives::decode_real,
                )?;
                // [3] exceeded-limit
                let (exceeded_limit, pos) = decode_context_value(
                    data,
                    pos,
                    3,
                    "OutOfRange exceeded-limit",
                    primitives::decode_real,
                )?;
                finish_variant(
                    Self::OutOfRange {
                        exceeding_value,
                        status_flags,
                        deadband,
                        exceeded_limit,
                    },
                    pos,
                )
            }
            // [10] Buffer ready
            10 => {
                let mut pos = inner_start;
                // [0] buffer-property: BACnetDeviceObjectPropertyReference
                let (t, p) = tags::decode_tag(data, pos)?;
                if !t.is_opening || t.number != 0 {
                    return Err(Error::decoding(
                        pos,
                        "BufferReady: expected opening [0] for buffer-property",
                    ));
                }
                pos = p;
                let buffer_property = decode_device_obj_prop_ref(data, &mut pos)?;
                // Closing tag [0]
                let (ct, cp) = tags::decode_tag(data, pos)?;
                if !ct.is_closing || ct.number != 0 {
                    return Err(Error::decoding(pos, "BufferReady: expected closing [0]"));
                }
                pos = cp;
                // [1] previous-notification
                let (previous_notification, pos) =
                    decode_context_u32(data, pos, 1, "BufferReady previous-notification")?;
                // [2] current-notification
                let (current_notification, end) =
                    decode_context_u32(data, pos, 2, "BufferReady current-notification")?;
                finish_variant(
                    Self::BufferReady {
                        buffer_property,
                        previous_notification,
                        current_notification,
                    },
                    end,
                )
            }
            // [11] Unsigned range
            11 => {
                // [0] exceeding-value
                let (exceeding_value, pos) = decode_context_value(
                    data,
                    inner_start,
                    0,
                    "UnsignedRange exceeding-value",
                    primitives::decode_unsigned,
                )?;
                // [1] status-flags
                let (status_flags, pos) =
                    decode_context_status_flags(data, pos, 1, "UnsignedRange status-flags")?;
                // [2] exceeded-limit
                let (exceeded_limit, pos) = decode_context_value(
                    data,
                    pos,
                    2,
                    "UnsignedRange exceeded-limit",
                    primitives::decode_unsigned,
                )?;
                finish_variant(
                    Self::UnsignedRange {
                        exceeding_value,
                        status_flags,
                        exceeded_limit,
                    },
                    pos,
                )
            }
            // [0] Change of bitstring
            0 => {
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
                    Self::ChangeOfBitstring {
                        referenced_bitstring: (unused, bits),
                        status_flags,
                    },
                    pos,
                )
            }
            // [3] Command failure
            3 => {
                let mut pos = inner_start;
                // [0] command-value — opening/closing, raw
                let (t, p) = tags::decode_tag(data, pos)?;
                if !t.is_opening || t.number != 0 {
                    return Err(Error::decoding(pos, "CommandFailure: expected opening [0]"));
                }
                let (command_value, after) = extract_raw_context(data, p, 0)?;
                pos = after;
                // [1] status-flags
                let (status_flags, pos) =
                    decode_context_status_flags(data, pos, 1, "CommandFailure status-flags")?;
                // [2] feedback-value — opening/closing, raw
                let (t, p) = tags::decode_tag(data, pos)?;
                if !t.is_opening || t.number != 2 {
                    return Err(Error::decoding(pos, "CommandFailure: expected opening [2]"));
                }
                let (feedback_value, after) = extract_raw_context(data, p, 2)?;
                finish_variant(
                    Self::CommandFailure {
                        command_value,
                        status_flags,
                        feedback_value,
                    },
                    after,
                )
            }
            // [4] Floating limit
            4 => {
                // [0] reference-value
                let (reference_value, pos) = decode_context_value(
                    data,
                    inner_start,
                    0,
                    "FloatingLimit reference-value",
                    primitives::decode_real,
                )?;
                // [1] status-flags
                let (status_flags, pos) =
                    decode_context_status_flags(data, pos, 1, "FloatingLimit status-flags")?;
                // [2] setpoint-value
                let (setpoint_value, pos) = decode_context_value(
                    data,
                    pos,
                    2,
                    "FloatingLimit setpoint-value",
                    primitives::decode_real,
                )?;
                // [3] error-limit
                let (error_limit, pos) = decode_context_value(
                    data,
                    pos,
                    3,
                    "FloatingLimit error-limit",
                    primitives::decode_real,
                )?;
                finish_variant(
                    Self::FloatingLimit {
                        reference_value,
                        status_flags,
                        setpoint_value,
                        error_limit,
                    },
                    pos,
                )
            }
            // [8] Change of life safety
            8 => {
                let (new_state, pos) =
                    decode_context_u32(data, inner_start, 0, "ChangeOfLifeSafety new-state")?;
                let (new_mode, pos) =
                    decode_context_u32(data, pos, 1, "ChangeOfLifeSafety new-mode")?;
                let flags_offset = pos;
                let (flags, pos) = decode_context(data, pos, 2, "ChangeOfLifeSafety status-flags")?;
                let [4, bits] = flags else {
                    return Err(Error::decoding(
                        flags_offset,
                        "ChangeOfLifeSafety status-flags must contain four bits",
                    ));
                };
                if bits & 0x0f != 0 {
                    return Err(Error::decoding(
                        flags_offset,
                        "ChangeOfLifeSafety status-flags must have zero padding",
                    ));
                }
                let status_flags = bits >> 4;
                let (operation_expected, pos) =
                    decode_context_u32(data, pos, 3, "ChangeOfLifeSafety operation-expected")?;
                finish_variant(
                    Self::ChangeOfLifeSafety {
                        new_state,
                        new_mode,
                        status_flags,
                        operation_expected,
                    },
                    pos,
                )
            }
            // [9] Extended
            9 => {
                // [0] vendor-id
                let (vendor_id, pos) =
                    decode_context_u16(data, inner_start, 0, "Extended vendor-id")?;
                // [1] extended-event-type
                let (extended_event_type, pos) =
                    decode_context_u32(data, pos, 1, "Extended extended-event-type")?;
                // [2] parameters — opening/closing, raw
                let (t, p) = tags::decode_tag(data, pos)?;
                if !t.is_opening_tag(2) {
                    return Err(Error::decoding(pos, "Extended: expected opening [2]"));
                }
                let (parameters, after) = extract_raw_context(data, p, 2)?;
                finish_variant(
                    Self::Extended {
                        vendor_id,
                        extended_event_type,
                        parameters,
                    },
                    after,
                )
            }
            // [13] Access event
            13 => {
                // [0] access-event
                let (access_event, pos) =
                    decode_context_u32(data, inner_start, 0, "AccessEvent access-event")?;
                // [1] status-flags
                let (status_flags, pos) =
                    decode_context_status_flags(data, pos, 1, "AccessEvent status-flags")?;
                // [2] access-event-tag
                let (access_event_tag, mut pos) =
                    decode_context_u32(data, pos, 2, "AccessEvent access-event-tag")?;
                // [3] access-event-time: BACnetTimeStamp (DateTime)
                let (ts, new_pos) = primitives::decode_timestamp(data, pos, 3)?;
                pos = new_pos;
                let access_event_time = match ts {
                    BACnetTimeStamp::DateTime { date, time } => (date, time),
                    _ => {
                        return Err(Error::decoding(
                            pos,
                            "AccessEvent: expected DateTime timestamp",
                        ))
                    }
                };
                // [4] access-credential: BACnetDeviceObjectPropertyReference
                let (t, p) = tags::decode_tag(data, pos)?;
                if !t.is_opening || t.number != 4 {
                    return Err(Error::decoding(
                        pos,
                        "AccessEvent: expected opening [4] for access-credential",
                    ));
                }
                pos = p;
                let access_credential = decode_device_obj_prop_ref(data, &mut pos)?;
                let (ct, cp) = tags::decode_tag(data, pos)?;
                if !ct.is_closing || ct.number != 4 {
                    return Err(Error::decoding(pos, "AccessEvent: expected closing [4]"));
                }
                pos = cp;
                // [5] authentication-factor — opening/closing, raw
                let (t, p) = tags::decode_tag(data, pos)?;
                if !t.is_opening_tag(5) {
                    return Err(Error::decoding(
                        pos,
                        "AccessEvent: expected opening [5] for authentication-factor",
                    ));
                }
                let (authentication_factor, after) = extract_raw_context(data, p, 5)?;
                finish_variant(
                    Self::AccessEvent {
                        access_event,
                        status_flags,
                        access_event_tag,
                        access_event_time,
                        access_credential,
                        authentication_factor,
                    },
                    after,
                )
            }
            // [14] Double out of range
            14 => {
                // [0] exceeding-value
                let (exceeding_value, pos) = decode_context_value(
                    data,
                    inner_start,
                    0,
                    "DoubleOutOfRange exceeding-value",
                    primitives::decode_double,
                )?;
                // [1] status-flags
                let (status_flags, pos) =
                    decode_context_status_flags(data, pos, 1, "DoubleOutOfRange status-flags")?;
                // [2] deadband
                let (deadband, pos) = decode_context_value(
                    data,
                    pos,
                    2,
                    "DoubleOutOfRange deadband",
                    primitives::decode_double,
                )?;
                // [3] exceeded-limit
                let (exceeded_limit, pos) = decode_context_value(
                    data,
                    pos,
                    3,
                    "DoubleOutOfRange exceeded-limit",
                    primitives::decode_double,
                )?;
                finish_variant(
                    Self::DoubleOutOfRange {
                        exceeding_value,
                        status_flags,
                        deadband,
                        exceeded_limit,
                    },
                    pos,
                )
            }
            // [15] Signed out of range
            15 => {
                // [0] exceeding-value
                let (exceeding_value, pos) = decode_context_value(
                    data,
                    inner_start,
                    0,
                    "SignedOutOfRange exceeding-value",
                    primitives::decode_signed,
                )?;
                // [1] status-flags
                let (status_flags, pos) =
                    decode_context_status_flags(data, pos, 1, "SignedOutOfRange status-flags")?;
                // [2] deadband
                let (deadband, pos) = decode_context_value(
                    data,
                    pos,
                    2,
                    "SignedOutOfRange deadband",
                    primitives::decode_unsigned,
                )?;
                // [3] exceeded-limit
                let (exceeded_limit, pos) = decode_context_value(
                    data,
                    pos,
                    3,
                    "SignedOutOfRange exceeded-limit",
                    primitives::decode_signed,
                )?;
                finish_variant(
                    Self::SignedOutOfRange {
                        exceeding_value,
                        status_flags,
                        deadband,
                        exceeded_limit,
                    },
                    pos,
                )
            }
            // [16] Unsigned out of range
            16 => {
                // [0] exceeding-value
                let (exceeding_value, pos) = decode_context_value(
                    data,
                    inner_start,
                    0,
                    "UnsignedOutOfRange exceeding-value",
                    primitives::decode_unsigned,
                )?;
                // [1] status-flags
                let (status_flags, pos) =
                    decode_context_status_flags(data, pos, 1, "UnsignedOutOfRange status-flags")?;
                // [2] deadband
                let (deadband, pos) = decode_context_value(
                    data,
                    pos,
                    2,
                    "UnsignedOutOfRange deadband",
                    primitives::decode_unsigned,
                )?;
                // [3] exceeded-limit
                let (exceeded_limit, pos) = decode_context_value(
                    data,
                    pos,
                    3,
                    "UnsignedOutOfRange exceeded-limit",
                    primitives::decode_unsigned,
                )?;
                finish_variant(
                    Self::UnsignedOutOfRange {
                        exceeding_value,
                        status_flags,
                        deadband,
                        exceeded_limit,
                    },
                    pos,
                )
            }
            // [17] Change of characterstring
            17 => {
                let mut pos = inner_start;
                // [0] changed-value
                let (opt_data, new_pos) = tags::decode_optional_context(data, pos, 0)?;
                let changed_value = match opt_data {
                    Some(content) => primitives::decode_character_string(content)?,
                    None => {
                        return Err(Error::decoding(
                            pos,
                            "ChangeOfCharacterstring: missing changed_value",
                        ))
                    }
                };
                pos = new_pos;
                // [1] status-flags
                let (status_flags, pos) = decode_context_status_flags(
                    data,
                    pos,
                    1,
                    "ChangeOfCharacterstring status-flags",
                )?;
                // [2] alarm-value
                let (opt_data, new_pos) = tags::decode_optional_context(data, pos, 2)?;
                let alarm_value = match opt_data {
                    Some(content) => primitives::decode_character_string(content)?,
                    None => {
                        return Err(Error::decoding(
                            pos,
                            "ChangeOfCharacterstring: missing alarm_value",
                        ))
                    }
                };
                finish_variant(
                    Self::ChangeOfCharacterstring {
                        changed_value,
                        status_flags,
                        alarm_value,
                    },
                    new_pos,
                )
            }
            // [18] Change of status flags
            18 => {
                let mut pos = inner_start;
                // [0] present-value — opening/closing, raw
                let (t, p) = tags::decode_tag(data, pos)?;
                if !t.is_opening || t.number != 0 {
                    return Err(Error::decoding(
                        pos,
                        "ChangeOfStatusFlags: expected opening [0]",
                    ));
                }
                let (present_value, after) = extract_raw_context(data, p, 0)?;
                pos = after;
                // [1] referenced-flags
                let (referenced_flags, pos) = decode_context_status_flags(
                    data,
                    pos,
                    1,
                    "ChangeOfStatusFlags referenced-flags",
                )?;
                finish_variant(
                    Self::ChangeOfStatusFlags {
                        present_value,
                        referenced_flags,
                    },
                    pos,
                )
            }
            // [19] Change of reliability
            19 => {
                // [0] reliability
                let (reliability, pos) =
                    decode_context_u32(data, inner_start, 0, "ChangeOfReliability reliability")?;
                // [1] status-flags
                let (status_flags, pos) =
                    decode_context_status_flags(data, pos, 1, "ChangeOfReliability status-flags")?;
                // [2] property-values — opening/closing, raw
                let (t, p) = tags::decode_tag(data, pos)?;
                if !t.is_opening_tag(2) {
                    return Err(Error::decoding(
                        pos,
                        "ChangeOfReliability: expected opening [2]",
                    ));
                }
                let (property_values, after) = extract_raw_context(data, p, 2)?;
                finish_variant(
                    Self::ChangeOfReliability {
                        reliability,
                        status_flags,
                        property_values,
                    },
                    after,
                )
            }
            // [20] None
            20 => finish_variant(Self::NoneParams, inner_start),
            // [21] Change of discrete value
            21 => decode_change_of_discrete_value(data, inner_start, variant_body_end),
            // [22] Change of timer
            22 => decode_change_of_timer(data, inner_start, variant_body_end),
            other => Err(Error::decoding(
                offset,
                format!("NotificationParameters variant [{other}] unknown"),
            )),
        }
    }
}
