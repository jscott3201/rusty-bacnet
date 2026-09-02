//! Structural recognition for the service-16 formal Error body.

use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::Error;

use crate::constructed::decode_object_property_reference;
use crate::{primitives, tags};

pub(super) fn decode_formal_body(data: &[u8]) -> Result<Option<(ErrorClass, ErrorCode)>, Error> {
    if data.is_empty() {
        return Ok(None);
    }
    let (first, content_start) = match tags::decode_tag(data, 0) {
        Ok(decoded) => decoded,
        Err(_) => return Ok(None),
    };
    if !first.is_opening_tag(0) {
        return Ok(None);
    }

    let (error_body, offset) = tags::extract_context_value(data, content_start, 0)?;
    let (error_class, error_code) = decode_error_pair(error_body)?;
    let (reference_opening, reference_start) = tags::decode_tag(data, offset)?;
    if !reference_opening.is_opening_tag(1) {
        return Err(Error::decoding(
            offset,
            "formal WPM Error expected opening tag 1",
        ));
    }
    let (reference_body, end) = tags::extract_context_value(data, reference_start, 1)?;
    if end != data.len() {
        return Err(Error::decoding(
            end,
            "formal WPM Error has trailing content",
        ));
    }
    decode_object_property_reference(reference_body)?;
    Ok(Some((error_class, error_code)))
}

fn decode_error_pair(data: &[u8]) -> Result<(ErrorClass, ErrorCode), Error> {
    let (class, offset) = decode_enumerated(data, 0, "error-class")?;
    let (code, end) = decode_enumerated(data, offset, "error-code")?;
    if end != data.len() {
        return Err(Error::decoding(
            end,
            "formal WPM Error [0] has extra fields",
        ));
    }
    Ok((ErrorClass::from_raw(class), ErrorCode::from_raw(code)))
}

fn decode_enumerated(data: &[u8], offset: usize, field: &str) -> Result<(u16, usize), Error> {
    let (tag, content_start) = tags::decode_tag(data, offset)?;
    if tag.class != tags::TagClass::Application || tag.number != tags::app_tag::ENUMERATED {
        return Err(Error::decoding(
            offset,
            format!("formal WPM Error {field} expected application Enumerated"),
        ));
    }
    let end = content_start
        .checked_add(tag.length as usize)
        .filter(|end| *end <= data.len())
        .ok_or_else(|| Error::decoding(content_start, format!("{field} payload is truncated")))?;
    let value = primitives::decode_unsigned(&data[content_start..end])?;
    let value = u16::try_from(value)
        .map_err(|_| Error::decoding(content_start, format!("{field} exceeds u16")))?;
    Ok((value, end))
}
