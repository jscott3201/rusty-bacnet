use super::*;
use crate::common::{decode_context, decode_context_u32};
use bacnet_encoding::constructed::{decode_property_state, encode_property_state};

/// Encode a BACnetPropertyStates value.
pub(super) fn encode_property_states(
    buf: &mut BytesMut,
    state: &BACnetPropertyStates,
) -> Result<(), Error> {
    encode_property_state(buf, state)
}

/// Decode BACnetPropertyStates from the current position. Advances `pos`.
pub(super) fn decode_property_states(
    data: &[u8],
    pos: &mut usize,
) -> Result<BACnetPropertyStates, Error> {
    let (state, end) = decode_property_state(data, *pos)?;
    *pos = end;
    Ok(state)
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
