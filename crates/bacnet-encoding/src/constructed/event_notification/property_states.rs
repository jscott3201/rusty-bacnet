#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use super::*;
use crate::constructed::{decode_property_state, encode_property_state};

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
                return Err(DecodeError::new(
                    DecodingKind::InvalidTag,
                    pos,
                    "context tag nesting depth exceeds maximum ({})",
                )
                .with_value(tags::MAX_CONTEXT_NESTING_DEPTH as u64)
                .into());
            }
            stack[depth] = tag.number;
            depth += 1;
            pos = content_start;
        } else if tag.is_closing {
            if tag.number != stack[depth - 1] {
                return Err(DecodeError::new(
                    DecodingKind::InvalidTag,
                    pos,
                    "closing tag {} does not match opening tag {}",
                )
                .with_values(tag.number.into(), stack[depth - 1].into())
                .into());
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
                return Err(Error::buffer_too_short(pos, data.len()));
            }
        }
    }

    Err(DecodeError::new(
        DecodingKind::Missing,
        start,
        "extract_raw_context: missing closing tag [{}]",
    )
    .with_value(tag_number.into())
    .into())
}

// ---------------------------------------------------------------------------
