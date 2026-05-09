use super::decode_timer::{decode_change_of_discrete_value, decode_change_of_timer};
use super::*;

impl NotificationParameters {
    /// Decode notification parameters from a position just past the opening
    /// tag of the eventValues wrapper.
    pub fn decode(data: &[u8], offset: usize) -> Result<Self, Error> {
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
                let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
                let sf_end = sf_pos + sf_tag.length as usize;
                if sf_end > data.len() {
                    return Err(Error::decoding(sf_pos, "ChangeOfState: truncated flags"));
                }
                let status_flags = decode_status_flags(&data[sf_pos..sf_end]);
                Ok(Self::ChangeOfState {
                    new_state,
                    status_flags,
                })
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
                let (choice_tag, choice_pos) = tags::decode_tag(data, pos)?;
                let new_value = if choice_tag.number == 0 {
                    // [0] changed-bits
                    let end = choice_pos + choice_tag.length as usize;
                    if end > data.len() {
                        return Err(Error::decoding(choice_pos, "ChangeOfValue: truncated bits"));
                    }
                    let (unused, bits) = primitives::decode_bit_string(&data[choice_pos..end])?;
                    pos = end;
                    ChangeOfValueChoice::ChangedBits {
                        unused_bits: unused,
                        data: bits,
                    }
                } else {
                    // [1] changed-value (Real)
                    let end = choice_pos + choice_tag.length as usize;
                    if end > data.len() {
                        return Err(Error::decoding(choice_pos, "ChangeOfValue: truncated real"));
                    }
                    let v = primitives::decode_real(&data[choice_pos..end])?;
                    pos = end;
                    ChangeOfValueChoice::ChangedValue(v)
                };
                // Closing tag [0]
                let (ct, cp) = tags::decode_tag(data, pos)?;
                if !ct.is_closing || ct.number != 0 {
                    return Err(Error::decoding(pos, "ChangeOfValue: expected closing [0]"));
                }
                pos = cp;
                // [1] status-flags
                let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
                let sf_end = sf_pos + sf_tag.length as usize;
                if sf_end > data.len() {
                    return Err(Error::decoding(sf_pos, "ChangeOfValue: truncated flags"));
                }
                let status_flags = decode_status_flags(&data[sf_pos..sf_end]);
                Ok(Self::ChangeOfValue {
                    new_value,
                    status_flags,
                })
            }
            // [5] Out of range
            5 => {
                let mut pos = inner_start;
                // [0] exceeding-value
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(p, "OutOfRange: truncated exceeding_value"));
                }
                let exceeding_value = primitives::decode_real(&data[p..end])?;
                pos = end;
                // [1] status-flags
                let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
                let sf_end = sf_pos + sf_tag.length as usize;
                if sf_end > data.len() {
                    return Err(Error::decoding(sf_pos, "OutOfRange: truncated flags"));
                }
                let status_flags = decode_status_flags(&data[sf_pos..sf_end]);
                pos = sf_end;
                // [2] deadband
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(p, "OutOfRange: truncated deadband"));
                }
                let deadband = primitives::decode_real(&data[p..end])?;
                pos = end;
                // [3] exceeded-limit
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(p, "OutOfRange: truncated exceeded_limit"));
                }
                let exceeded_limit = primitives::decode_real(&data[p..end])?;
                let _ = (pos, end);
                Ok(Self::OutOfRange {
                    exceeding_value,
                    status_flags,
                    deadband,
                    exceeded_limit,
                })
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
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "BufferReady: truncated previous_notification",
                    ));
                }
                let previous_notification = primitives::decode_unsigned(&data[p..end])? as u32;
                pos = end;
                // [2] current-notification
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "BufferReady: truncated current_notification",
                    ));
                }
                let current_notification = primitives::decode_unsigned(&data[p..end])? as u32;
                let _ = (pos, end);
                Ok(Self::BufferReady {
                    buffer_property,
                    previous_notification,
                    current_notification,
                })
            }
            // [11] Unsigned range
            11 => {
                let mut pos = inner_start;
                // [0] exceeding-value
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "UnsignedRange: truncated exceeding_value",
                    ));
                }
                let exceeding_value = primitives::decode_unsigned(&data[p..end])?;
                pos = end;
                // [1] status-flags
                let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
                let sf_end = sf_pos + sf_tag.length as usize;
                if sf_end > data.len() {
                    return Err(Error::decoding(sf_pos, "UnsignedRange: truncated flags"));
                }
                let status_flags = decode_status_flags(&data[sf_pos..sf_end]);
                pos = sf_end;
                // [2] exceeded-limit
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "UnsignedRange: truncated exceeded_limit",
                    ));
                }
                let exceeded_limit = primitives::decode_unsigned(&data[p..end])?;
                let _ = (pos, end);
                Ok(Self::UnsignedRange {
                    exceeding_value,
                    status_flags,
                    exceeded_limit,
                })
            }
            // [0] Change of bitstring
            0 => {
                let mut pos = inner_start;
                // [0] referenced-bitstring
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(p, "ChangeOfBitstring: truncated bitstring"));
                }
                let (unused, bits) = primitives::decode_bit_string(&data[p..end])?;
                pos = end;
                // [1] status-flags
                let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
                let sf_end = sf_pos + sf_tag.length as usize;
                if sf_end > data.len() {
                    return Err(Error::decoding(
                        sf_pos,
                        "ChangeOfBitstring: truncated flags",
                    ));
                }
                let status_flags = decode_status_flags(&data[sf_pos..sf_end]);
                Ok(Self::ChangeOfBitstring {
                    referenced_bitstring: (unused, bits),
                    status_flags,
                })
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
                let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
                let sf_end = sf_pos + sf_tag.length as usize;
                if sf_end > data.len() {
                    return Err(Error::decoding(sf_pos, "CommandFailure: truncated flags"));
                }
                let status_flags = decode_status_flags(&data[sf_pos..sf_end]);
                pos = sf_end;
                // [2] feedback-value — opening/closing, raw
                let (t, p) = tags::decode_tag(data, pos)?;
                if !t.is_opening || t.number != 2 {
                    return Err(Error::decoding(pos, "CommandFailure: expected opening [2]"));
                }
                let (feedback_value, _after) = extract_raw_context(data, p, 2)?;
                Ok(Self::CommandFailure {
                    command_value,
                    status_flags,
                    feedback_value,
                })
            }
            // [4] Floating limit
            4 => {
                let mut pos = inner_start;
                // [0] reference-value
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "FloatingLimit: truncated reference_value",
                    ));
                }
                let reference_value = primitives::decode_real(&data[p..end])?;
                pos = end;
                // [1] status-flags
                let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
                let sf_end = sf_pos + sf_tag.length as usize;
                if sf_end > data.len() {
                    return Err(Error::decoding(sf_pos, "FloatingLimit: truncated flags"));
                }
                let status_flags = decode_status_flags(&data[sf_pos..sf_end]);
                pos = sf_end;
                // [2] setpoint-value
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "FloatingLimit: truncated setpoint_value",
                    ));
                }
                let setpoint_value = primitives::decode_real(&data[p..end])?;
                pos = end;
                // [3] error-limit
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(p, "FloatingLimit: truncated error_limit"));
                }
                let error_limit = primitives::decode_real(&data[p..end])?;
                let _ = (pos, end);
                Ok(Self::FloatingLimit {
                    reference_value,
                    status_flags,
                    setpoint_value,
                    error_limit,
                })
            }
            // [8] Change of life safety
            8 => {
                let mut pos = inner_start;
                // [0] new-state
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "ChangeOfLifeSafety: truncated new_state",
                    ));
                }
                let new_state = primitives::decode_unsigned(&data[p..end])? as u32;
                pos = end;
                // [1] new-mode
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(p, "ChangeOfLifeSafety: truncated new_mode"));
                }
                let new_mode = primitives::decode_unsigned(&data[p..end])? as u32;
                pos = end;
                // [2] status-flags
                let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
                let sf_end = sf_pos + sf_tag.length as usize;
                if sf_end > data.len() {
                    return Err(Error::decoding(
                        sf_pos,
                        "ChangeOfLifeSafety: truncated flags",
                    ));
                }
                let status_flags = decode_status_flags(&data[sf_pos..sf_end]);
                pos = sf_end;
                // [3] operation-expected
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "ChangeOfLifeSafety: truncated operation_expected",
                    ));
                }
                let operation_expected = primitives::decode_unsigned(&data[p..end])? as u32;
                let _ = (pos, end);
                Ok(Self::ChangeOfLifeSafety {
                    new_state,
                    new_mode,
                    status_flags,
                    operation_expected,
                })
            }
            // [9] Extended
            9 => {
                let mut pos = inner_start;
                // [0] vendor-id
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(p, "Extended: truncated vendor_id"));
                }
                let vendor_id = primitives::decode_unsigned(&data[p..end])? as u16;
                pos = end;
                // [1] extended-event-type
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "Extended: truncated extended_event_type",
                    ));
                }
                let extended_event_type = primitives::decode_unsigned(&data[p..end])? as u32;
                pos = end;
                // [2] parameters — opening/closing, raw
                let (t, p) = tags::decode_tag(data, pos)?;
                if !t.is_opening || t.number != 2 {
                    return Err(Error::decoding(pos, "Extended: expected opening [2]"));
                }
                let (parameters, _after) = extract_raw_context(data, p, 2)?;
                Ok(Self::Extended {
                    vendor_id,
                    extended_event_type,
                    parameters,
                })
            }
            // [13] Access event
            13 => {
                let mut pos = inner_start;
                // [0] access-event
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(p, "AccessEvent: truncated access_event"));
                }
                let access_event = primitives::decode_unsigned(&data[p..end])? as u32;
                pos = end;
                // [1] status-flags
                let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
                let sf_end = sf_pos + sf_tag.length as usize;
                if sf_end > data.len() {
                    return Err(Error::decoding(sf_pos, "AccessEvent: truncated flags"));
                }
                let status_flags = decode_status_flags(&data[sf_pos..sf_end]);
                pos = sf_end;
                // [2] access-event-tag
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "AccessEvent: truncated access_event_tag",
                    ));
                }
                let access_event_tag = primitives::decode_unsigned(&data[p..end])? as u32;
                pos = end;
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
                if !t.is_opening || t.number != 5 {
                    return Err(Error::decoding(
                        pos,
                        "AccessEvent: expected opening [5] for authentication-factor",
                    ));
                }
                let (authentication_factor, _after) = extract_raw_context(data, p, 5)?;
                Ok(Self::AccessEvent {
                    access_event,
                    status_flags,
                    access_event_tag,
                    access_event_time,
                    access_credential,
                    authentication_factor,
                })
            }
            // [14] Double out of range
            14 => {
                let mut pos = inner_start;
                // [0] exceeding-value
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "DoubleOutOfRange: truncated exceeding_value",
                    ));
                }
                let exceeding_value = primitives::decode_double(&data[p..end])?;
                pos = end;
                // [1] status-flags
                let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
                let sf_end = sf_pos + sf_tag.length as usize;
                if sf_end > data.len() {
                    return Err(Error::decoding(sf_pos, "DoubleOutOfRange: truncated flags"));
                }
                let status_flags = decode_status_flags(&data[sf_pos..sf_end]);
                pos = sf_end;
                // [2] deadband
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(p, "DoubleOutOfRange: truncated deadband"));
                }
                let deadband = primitives::decode_double(&data[p..end])?;
                pos = end;
                // [3] exceeded-limit
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "DoubleOutOfRange: truncated exceeded_limit",
                    ));
                }
                let exceeded_limit = primitives::decode_double(&data[p..end])?;
                let _ = (pos, end);
                Ok(Self::DoubleOutOfRange {
                    exceeding_value,
                    status_flags,
                    deadband,
                    exceeded_limit,
                })
            }
            // [15] Signed out of range
            15 => {
                let mut pos = inner_start;
                // [0] exceeding-value
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "SignedOutOfRange: truncated exceeding_value",
                    ));
                }
                let exceeding_value = primitives::decode_signed(&data[p..end])?;
                pos = end;
                // [1] status-flags
                let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
                let sf_end = sf_pos + sf_tag.length as usize;
                if sf_end > data.len() {
                    return Err(Error::decoding(sf_pos, "SignedOutOfRange: truncated flags"));
                }
                let status_flags = decode_status_flags(&data[sf_pos..sf_end]);
                pos = sf_end;
                // [2] deadband
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(p, "SignedOutOfRange: truncated deadband"));
                }
                let deadband = primitives::decode_unsigned(&data[p..end])?;
                pos = end;
                // [3] exceeded-limit
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "SignedOutOfRange: truncated exceeded_limit",
                    ));
                }
                let exceeded_limit = primitives::decode_signed(&data[p..end])?;
                let _ = (pos, end);
                Ok(Self::SignedOutOfRange {
                    exceeding_value,
                    status_flags,
                    deadband,
                    exceeded_limit,
                })
            }
            // [16] Unsigned out of range
            16 => {
                let mut pos = inner_start;
                // [0] exceeding-value
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "UnsignedOutOfRange: truncated exceeding_value",
                    ));
                }
                let exceeding_value = primitives::decode_unsigned(&data[p..end])?;
                pos = end;
                // [1] status-flags
                let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
                let sf_end = sf_pos + sf_tag.length as usize;
                if sf_end > data.len() {
                    return Err(Error::decoding(
                        sf_pos,
                        "UnsignedOutOfRange: truncated flags",
                    ));
                }
                let status_flags = decode_status_flags(&data[sf_pos..sf_end]);
                pos = sf_end;
                // [2] deadband
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(p, "UnsignedOutOfRange: truncated deadband"));
                }
                let deadband = primitives::decode_unsigned(&data[p..end])?;
                pos = end;
                // [3] exceeded-limit
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "UnsignedOutOfRange: truncated exceeded_limit",
                    ));
                }
                let exceeded_limit = primitives::decode_unsigned(&data[p..end])?;
                let _ = (pos, end);
                Ok(Self::UnsignedOutOfRange {
                    exceeding_value,
                    status_flags,
                    deadband,
                    exceeded_limit,
                })
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
                let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
                let sf_end = sf_pos + sf_tag.length as usize;
                if sf_end > data.len() {
                    return Err(Error::decoding(
                        sf_pos,
                        "ChangeOfCharacterstring: truncated flags",
                    ));
                }
                let status_flags = decode_status_flags(&data[sf_pos..sf_end]);
                pos = sf_end;
                // [2] alarm-value
                let (opt_data, _new_pos) = tags::decode_optional_context(data, pos, 2)?;
                let alarm_value = match opt_data {
                    Some(content) => primitives::decode_character_string(content)?,
                    None => {
                        return Err(Error::decoding(
                            pos,
                            "ChangeOfCharacterstring: missing alarm_value",
                        ))
                    }
                };
                Ok(Self::ChangeOfCharacterstring {
                    changed_value,
                    status_flags,
                    alarm_value,
                })
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
                let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
                let sf_end = sf_pos + sf_tag.length as usize;
                if sf_end > data.len() {
                    return Err(Error::decoding(
                        sf_pos,
                        "ChangeOfStatusFlags: truncated flags",
                    ));
                }
                let referenced_flags = decode_status_flags(&data[sf_pos..sf_end]);
                Ok(Self::ChangeOfStatusFlags {
                    present_value,
                    referenced_flags,
                })
            }
            // [19] Change of reliability
            19 => {
                let mut pos = inner_start;
                // [0] reliability
                let (t, p) = tags::decode_tag(data, pos)?;
                let end = p + t.length as usize;
                if end > data.len() {
                    return Err(Error::decoding(
                        p,
                        "ChangeOfReliability: truncated reliability",
                    ));
                }
                let reliability = primitives::decode_unsigned(&data[p..end])? as u32;
                pos = end;
                // [1] status-flags
                let (sf_tag, sf_pos) = tags::decode_tag(data, pos)?;
                let sf_end = sf_pos + sf_tag.length as usize;
                if sf_end > data.len() {
                    return Err(Error::decoding(
                        sf_pos,
                        "ChangeOfReliability: truncated flags",
                    ));
                }
                let status_flags = decode_status_flags(&data[sf_pos..sf_end]);
                pos = sf_end;
                // [2] property-values — opening/closing, raw
                let (t, p) = tags::decode_tag(data, pos)?;
                if !t.is_opening || t.number != 2 {
                    return Err(Error::decoding(
                        pos,
                        "ChangeOfReliability: expected opening [2]",
                    ));
                }
                let (property_values, _after) = extract_raw_context(data, p, 2)?;
                Ok(Self::ChangeOfReliability {
                    reliability,
                    status_flags,
                    property_values,
                })
            }
            // [20] None
            20 => Ok(Self::NoneParams),
            // [21] Change of discrete value
            21 => decode_change_of_discrete_value(data, inner_start),
            // [22] Change of timer
            22 => decode_change_of_timer(data, inner_start),
            other => Err(Error::decoding(
                offset,
                format!("NotificationParameters variant [{other}] unknown"),
            )),
        }
    }
}
