use super::*;
use crate::common::decode_context;

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

pub(super) fn extract_trailing_raw_context(
    data: &[u8],
    start: usize,
    tag_number: u8,
    variant_body_end: Option<usize>,
    field: &str,
) -> Result<(Vec<u8>, usize), Error> {
    let Some(end) = variant_body_end else {
        return extract_raw_context(data, start, tag_number);
    };
    let closing = closing_tag_start(data, end, tag_number, field)?;
    if closing < start {
        return Err(Error::decoding(
            start,
            format!("{field} has invalid bounds"),
        ));
    }
    Ok((data[start..closing].to_vec(), end))
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
