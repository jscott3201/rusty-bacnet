//! Field readers shared by the notification decoders.

use super::*;

/// Read the contents of the primitive context tag `expected_tag` at `offset`,
/// returning them with the offset just past the field. `field` names the
/// field in errors.
pub(super) fn decode_context<'a>(
    data: &'a [u8],
    offset: usize,
    expected_tag: u8,
    field: &'static str,
) -> Result<(&'a [u8], usize), Error> {
    let (tag, pos) = tags::decode_tag(data, offset)?;
    if !tag.is_context(expected_tag) {
        return Err(misplaced(
            data,
            &tag,
            Some(expected_tag),
            offset,
            "expected context tag {}",
        )
        .within(field)
        .with_value(expected_tag.into())
        .into());
    }
    let end = pos.checked_add(tag.length as usize).ok_or_else(|| {
        DecodeError::new(DecodingKind::InvalidEncoding, pos, "length overflow").within(field)
    })?;
    if end > data.len() {
        return Err(Error::buffer_too_short(end, data.len()));
    }
    Ok((&data[pos..end], end))
}

/// Read a context-tagged Unsigned that fits in 32 bits.
pub(super) fn decode_context_u32(
    data: &[u8],
    offset: usize,
    expected_tag: u8,
    field: &'static str,
) -> Result<(u32, usize), Error> {
    let (content, end) = decode_context(data, offset, expected_tag, field)?;
    let value = primitives::decode_unsigned(content)?;
    let value = u32::try_from(value).map_err(|_| {
        DecodeError::new(DecodingKind::OutOfRange, offset, "exceeds u32").within(field)
    })?;
    Ok((value, end))
}

/// Decode a context-tagged Enumerated into an open enumeration newtype through its `from_raw`
/// constructor, so values outside the named set (including proprietary ones) are kept as-is.
pub(super) fn decode_context_enum<T>(
    data: &[u8],
    offset: usize,
    expected_tag: u8,
    field: &'static str,
    from_raw: fn(u32) -> T,
) -> Result<(T, usize), Error> {
    let (value, end) = decode_context_u32(data, offset, expected_tag, field)?;
    Ok((from_raw(value), end))
}

/// Read a context-tagged BOOLEAN, one content octet holding 0 or 1.
pub(super) fn decode_context_bool(
    data: &[u8],
    offset: usize,
    expected_tag: u8,
    field: &'static str,
) -> Result<(bool, usize), Error> {
    let (content, end) = decode_context(data, offset, expected_tag, field)?;
    let value = match content {
        [0] => false,
        [1] => true,
        _ => {
            return Err(DecodeError::new(
                DecodingKind::InvalidEncoding,
                offset,
                "expected Boolean 0 or 1",
            )
            .within(field)
            .into());
        }
    };
    Ok((value, end))
}

pub(super) fn finish_variant(
    value: NotificationParameters,
    consumed: usize,
    body_end: usize,
    variant_tag: u8,
) -> Result<NotificationParameters, Error> {
    if consumed != body_end {
        return Err(DecodeError::new(
            DecodingKind::Trailing,
            consumed,
            "NotificationParameters variant {} has unexpected fields",
        )
        .with_value(variant_tag.into())
        .into());
    }
    Ok(value)
}

pub(super) fn closing_tag_start(
    data: &[u8],
    end: usize,
    tag_number: u8,
    field: &'static str,
) -> Result<usize, Error> {
    let width = if tag_number > 14 { 2 } else { 1 };
    let start = end.checked_sub(width).ok_or_else(|| {
        DecodeError::new(DecodingKind::Missing, end, "missing closing tag").within(field)
    })?;
    let (tag, next) = tags::decode_tag(data, start)?;
    if !tag.is_closing_tag(tag_number) || next != end {
        return Err(DecodeError::new(
            DecodingKind::InvalidTag,
            start,
            "expected closing tag {}",
        )
        .within(field)
        .with_value(tag_number.into())
        .into());
    }
    Ok(start)
}

pub(super) fn decode_context_value<T>(
    data: &[u8],
    offset: usize,
    expected_tag: u8,
    field: &'static str,
    decode: impl FnOnce(&[u8]) -> Result<T, Error>,
) -> Result<(T, usize), Error> {
    let (content, end) = decode_context(data, offset, expected_tag, field)?;
    Ok((decode(content)?, end))
}

pub(super) fn decode_context_u16(
    data: &[u8],
    offset: usize,
    expected_tag: u8,
    field: &'static str,
) -> Result<(u16, usize), Error> {
    let (content, end) = decode_context(data, offset, expected_tag, field)?;
    let value = primitives::decode_unsigned(content)?;
    let value = u16::try_from(value).map_err(|_| {
        DecodeError::new(DecodingKind::OutOfRange, offset, "exceeds u16").within(field)
    })?;
    Ok((value, end))
}

pub(super) fn decode_context_status_flags(
    data: &[u8],
    offset: usize,
    expected_tag: u8,
    field: &'static str,
) -> Result<(StatusFlags, usize), Error> {
    let (content, end) = decode_context(data, offset, expected_tag, field)?;
    let [4, bits] = content else {
        return Err(DecodeError::new(
            DecodingKind::InvalidEncoding,
            offset,
            "must contain four bits",
        )
        .within(field)
        .into());
    };
    if bits & 0x0f != 0 {
        return Err(DecodeError::new(
            DecodingKind::InvalidEncoding,
            offset,
            "must have zero padding",
        )
        .within(field)
        .into());
    }
    Ok((StatusFlags::from_bits_retain(bits >> 4), end))
}
