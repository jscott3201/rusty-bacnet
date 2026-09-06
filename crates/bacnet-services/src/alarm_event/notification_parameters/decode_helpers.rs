use super::*;
use crate::common::decode_context;

pub(super) fn finish_variant(
    value: NotificationParameters,
    consumed: usize,
    body_end: usize,
    variant_tag: u8,
) -> Result<NotificationParameters, Error> {
    if consumed != body_end {
        return Err(Error::decoding(
            consumed,
            format!("NotificationParameters variant {variant_tag} has unexpected fields"),
        ));
    }
    Ok(value)
}

pub(super) fn closing_tag_start(
    data: &[u8],
    end: usize,
    tag_number: u8,
    field: &str,
) -> Result<usize, Error> {
    let width = if tag_number > 14 { 2 } else { 1 };
    let start = end
        .checked_sub(width)
        .ok_or_else(|| Error::decoding(end, format!("{field} missing closing tag")))?;
    let (tag, next) = tags::decode_tag(data, start)?;
    if !tag.is_closing_tag(tag_number) || next != end {
        return Err(Error::decoding(
            start,
            format!("{field} expected closing tag {tag_number}"),
        ));
    }
    Ok(start)
}

pub(super) fn decode_context_value<T>(
    data: &[u8],
    offset: usize,
    expected_tag: u8,
    field: &str,
    decode: impl FnOnce(&[u8]) -> Result<T, Error>,
) -> Result<(T, usize), Error> {
    let (content, end) = decode_context(data, offset, expected_tag, field)?;
    Ok((decode(content)?, end))
}

pub(super) fn decode_context_u16(
    data: &[u8],
    offset: usize,
    expected_tag: u8,
    field: &str,
) -> Result<(u16, usize), Error> {
    let (content, end) = decode_context(data, offset, expected_tag, field)?;
    let value = primitives::decode_unsigned(content)?;
    let value = u16::try_from(value)
        .map_err(|_| Error::decoding(offset, format!("{field} exceeds u16")))?;
    Ok((value, end))
}

pub(super) fn decode_context_status_flags(
    data: &[u8],
    offset: usize,
    expected_tag: u8,
    field: &str,
) -> Result<(u8, usize), Error> {
    let (content, end) = decode_context(data, offset, expected_tag, field)?;
    let [4, bits] = content else {
        return Err(Error::decoding(
            offset,
            format!("{field} must contain four bits"),
        ));
    };
    if bits & 0x0f != 0 {
        return Err(Error::decoding(
            offset,
            format!("{field} must have zero padding"),
        ));
    }
    Ok((bits >> 4, end))
}
