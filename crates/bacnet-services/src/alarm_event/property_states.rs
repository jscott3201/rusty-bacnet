use super::*;
use crate::common::{decode_context, decode_context_u32};

/// Encode a BACnetPropertyStates value.
pub(super) fn encode_property_states(buf: &mut BytesMut, state: &BACnetPropertyStates) {
    match state {
        BACnetPropertyStates::BooleanValue(v) => {
            primitives::encode_ctx_boolean(buf, 0, *v);
        }
        BACnetPropertyStates::BinaryValue(v) => {
            primitives::encode_ctx_unsigned(buf, 1, *v as u64);
        }
        BACnetPropertyStates::EventType(v) => {
            primitives::encode_ctx_unsigned(buf, 2, *v as u64);
        }
        BACnetPropertyStates::Polarity(v) => {
            primitives::encode_ctx_unsigned(buf, 3, *v as u64);
        }
        BACnetPropertyStates::ProgramChange(v) => {
            primitives::encode_ctx_unsigned(buf, 4, *v as u64);
        }
        BACnetPropertyStates::ProgramState(v) => {
            primitives::encode_ctx_unsigned(buf, 5, *v as u64);
        }
        BACnetPropertyStates::ReasonForHalt(v) => {
            primitives::encode_ctx_unsigned(buf, 6, *v as u64);
        }
        BACnetPropertyStates::Reliability(v) => {
            primitives::encode_ctx_unsigned(buf, 7, *v as u64);
        }
        BACnetPropertyStates::State(v) => {
            primitives::encode_ctx_unsigned(buf, 8, *v as u64);
        }
        BACnetPropertyStates::SystemStatus(v) => {
            primitives::encode_ctx_unsigned(buf, 9, *v as u64);
        }
        BACnetPropertyStates::Units(v) => {
            primitives::encode_ctx_unsigned(buf, 10, *v as u64);
        }
        BACnetPropertyStates::LifeSafetyMode(v) => {
            primitives::encode_ctx_unsigned(buf, 12, *v as u64);
        }
        BACnetPropertyStates::UnsignedValue(v) => {
            primitives::encode_ctx_unsigned(buf, 11, *v as u64);
        }
        BACnetPropertyStates::LifeSafetyState(v) => {
            primitives::encode_ctx_unsigned(buf, 13, *v as u64);
        }
        BACnetPropertyStates::DoorAlarmState(v) => {
            primitives::encode_ctx_unsigned(buf, 14, *v as u64);
        }
        BACnetPropertyStates::Action(v) => {
            primitives::encode_ctx_unsigned(buf, 15, *v as u64);
        }
        BACnetPropertyStates::DoorSecuredStatus(v) => {
            primitives::encode_ctx_unsigned(buf, 16, *v as u64);
        }
        BACnetPropertyStates::DoorStatus(v) => {
            primitives::encode_ctx_unsigned(buf, 17, *v as u64);
        }
        BACnetPropertyStates::DoorValue(v) => {
            primitives::encode_ctx_unsigned(buf, 18, *v as u64);
        }
        BACnetPropertyStates::TimerState(v) => {
            primitives::encode_ctx_unsigned(buf, 38, *v as u64);
        }
        BACnetPropertyStates::TimerTransition(v) => {
            primitives::encode_ctx_unsigned(buf, 39, *v as u64);
        }
        BACnetPropertyStates::LiftCarDirection(v) => {
            primitives::encode_ctx_unsigned(buf, 40, *v as u64);
        }
        BACnetPropertyStates::LiftCarDoorCommand(v) => {
            primitives::encode_ctx_unsigned(buf, 42, *v as u64);
        }
        BACnetPropertyStates::Other { tag, data } => {
            primitives::encode_ctx_octet_string(buf, *tag, data);
        }
    }
}

/// Decode BACnetPropertyStates from the current position. Advances `pos`.
pub(super) fn decode_property_states(
    data: &[u8],
    pos: &mut usize,
) -> Result<BACnetPropertyStates, Error> {
    let (tag, content_start) = tags::decode_tag(data, *pos)?;
    let end = content_start + tag.length as usize;
    if end > data.len() {
        return Err(Error::decoding(
            content_start,
            "BACnetPropertyStates: truncated",
        ));
    }
    let content = &data[content_start..end];
    *pos = end;
    match tag.number {
        0 => Ok(BACnetPropertyStates::BooleanValue(
            !content.is_empty() && content[0] != 0,
        )),
        1 => Ok(BACnetPropertyStates::BinaryValue(
            primitives::decode_unsigned(content)? as u32,
        )),
        2 => Ok(BACnetPropertyStates::EventType(
            primitives::decode_unsigned(content)? as u32,
        )),
        3 => Ok(BACnetPropertyStates::Polarity(
            primitives::decode_unsigned(content)? as u32,
        )),
        4 => Ok(BACnetPropertyStates::ProgramChange(
            primitives::decode_unsigned(content)? as u32,
        )),
        5 => Ok(BACnetPropertyStates::ProgramState(
            primitives::decode_unsigned(content)? as u32,
        )),
        6 => Ok(BACnetPropertyStates::ReasonForHalt(
            primitives::decode_unsigned(content)? as u32,
        )),
        7 => Ok(BACnetPropertyStates::Reliability(
            primitives::decode_unsigned(content)? as u32,
        )),
        8 => Ok(BACnetPropertyStates::State(
            primitives::decode_unsigned(content)? as u32,
        )),
        9 => Ok(BACnetPropertyStates::SystemStatus(
            primitives::decode_unsigned(content)? as u32,
        )),
        10 => Ok(BACnetPropertyStates::Units(
            primitives::decode_unsigned(content)? as u32,
        )),
        11 => Ok(BACnetPropertyStates::UnsignedValue(
            primitives::decode_unsigned(content)? as u32,
        )),
        12 => Ok(BACnetPropertyStates::LifeSafetyMode(
            primitives::decode_unsigned(content)? as u32,
        )),
        13 => Ok(BACnetPropertyStates::LifeSafetyState(
            primitives::decode_unsigned(content)? as u32,
        )),
        14 => Ok(BACnetPropertyStates::DoorAlarmState(
            primitives::decode_unsigned(content)? as u32,
        )),
        15 => Ok(BACnetPropertyStates::Action(
            primitives::decode_unsigned(content)? as u32,
        )),
        16 => Ok(BACnetPropertyStates::DoorSecuredStatus(
            primitives::decode_unsigned(content)? as u32,
        )),
        17 => Ok(BACnetPropertyStates::DoorStatus(
            primitives::decode_unsigned(content)? as u32,
        )),
        18 => Ok(BACnetPropertyStates::DoorValue(
            primitives::decode_unsigned(content)? as u32,
        )),
        38 => Ok(BACnetPropertyStates::TimerState(
            primitives::decode_unsigned(content)? as u32,
        )),
        39 => Ok(BACnetPropertyStates::TimerTransition(
            primitives::decode_unsigned(content)? as u32,
        )),
        40 => Ok(BACnetPropertyStates::LiftCarDirection(
            primitives::decode_unsigned(content)? as u32,
        )),
        42 => Ok(BACnetPropertyStates::LiftCarDoorCommand(
            primitives::decode_unsigned(content)? as u32,
        )),
        n => Ok(BACnetPropertyStates::Other {
            tag: n,
            data: content.to_vec(),
        }),
    }
}

/// Decode BACnetDeviceObjectPropertyReference from context-tagged fields.
/// Expects to be positioned at the first inner field. Advances `pos` past the last field.
pub(super) fn decode_device_obj_prop_ref(
    data: &[u8],
    pos: &mut usize,
) -> Result<BACnetDeviceObjectPropertyReference, Error> {
    // [0] objectIdentifier
    let (content, end) = decode_context(data, *pos, 0, "DeviceObjectPropertyRef objectIdentifier")?;
    let object_identifier = ObjectIdentifier::decode(content)?;
    *pos = end;

    // [1] propertyIdentifier
    let (property_identifier, end) =
        decode_context_u32(data, *pos, 1, "DeviceObjectPropertyRef propertyIdentifier")?;
    *pos = end;

    // [2] propertyArrayIndex — optional
    let mut property_array_index = None;
    if *pos < data.len() {
        let (peek, _) = tags::decode_tag(data, *pos)?;
        if peek.is_context(2) {
            let (value, end) =
                decode_context_u32(data, *pos, 2, "DeviceObjectPropertyRef propertyArrayIndex")?;
            property_array_index = Some(value);
            *pos = end;
        }
    }

    // [3] deviceIdentifier — optional
    let mut device_identifier = None;
    if *pos < data.len() {
        let (peek, _) = tags::decode_tag(data, *pos)?;
        if peek.is_context(3) {
            let (content, end) =
                decode_context(data, *pos, 3, "DeviceObjectPropertyRef deviceIdentifier")?;
            device_identifier = Some(ObjectIdentifier::decode(content)?);
            *pos = end;
        }
    }

    Ok(BACnetDeviceObjectPropertyReference {
        object_identifier,
        property_identifier,
        property_array_index,
        device_identifier,
    })
}

/// Extract an encoded BACnet value sequence from a constructed context field.
pub(super) fn extract_raw_context(
    data: &[u8],
    start: usize,
    tag_number: u8,
) -> Result<(Vec<u8>, usize), Error> {
    let mut stack = [0; tags::MAX_CONTEXT_NESTING_DEPTH];
    stack[0] = tag_number;
    let mut depth = 1;
    let mut pos = start;

    while pos < data.len() {
        let tag_start = pos;
        let (tag, content_start) = tags::decode_tag(data, pos)?;
        if tag.is_opening {
            if depth == stack.len() {
                return Err(Error::decoding(
                    pos,
                    format!(
                        "context tag nesting depth exceeds maximum ({})",
                        tags::MAX_CONTEXT_NESTING_DEPTH
                    ),
                ));
            }
            stack[depth] = tag.number;
            depth += 1;
            pos = content_start;
        } else if tag.is_closing {
            if tag.number != stack[depth - 1] {
                return Err(Error::decoding(
                    pos,
                    format!(
                        "closing tag {} does not match opening tag {}",
                        tag.number,
                        stack[depth - 1]
                    ),
                ));
            }
            depth -= 1;
            if depth == 0 {
                return Ok((data[start..tag_start].to_vec(), content_start));
            }
            pos = content_start;
        } else if tag.class == tags::TagClass::Application && tag.number == tags::app_tag::BOOLEAN {
            pos = content_start;
        } else {
            pos = content_start
                .checked_add(tag.length as usize)
                .ok_or_else(|| Error::decoding(content_start, "tag length overflow"))?;
            if pos > data.len() {
                return Err(Error::decoding(
                    content_start,
                    format!("tag data overflows buffer: need {} bytes", tag.length),
                ));
            }
        }
    }

    Err(Error::decoding(
        start,
        format!("extract_raw_context: missing closing tag [{tag_number}]"),
    ))
}

/// Decode status flags from a bit-string content slice.
/// Returns the 4-bit status flags value.
pub(super) fn decode_status_flags(data: &[u8]) -> u8 {
    // Bit string format: first byte = unused bits count, rest = data
    if data.len() >= 2 {
        let unused = data[0];
        data[1] >> (unused.min(7))
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
