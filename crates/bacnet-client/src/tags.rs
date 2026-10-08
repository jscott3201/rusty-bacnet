//! Typed interpretation of the value bytes returned by a Tags read.
//!
//! Raw ReadProperty and ReadPropertyMultiple results retain their original
//! APIs. Pass a successful result's echoed array index and value bytes here.

use bacnet_encoding::constructed::decode_name_value;
use bacnet_encoding::primitives::decode_application_value;
use bacnet_types::constructed::BACnetNameValue;
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

/// The three forms of a Tags array read (Clause 12.1.5.1).
#[derive(Debug, Clone, PartialEq)]
pub enum TagsRead {
    /// All tags, in wire order; an empty array is valid.
    Whole(Vec<BACnetNameValue>),
    /// One element selected by a one-based index.
    Element(BACnetNameValue),
    /// The array length read at index zero.
    Size(u32),
}

/// Decode the entire payload of a successful Tags RP or RPM result.
///
/// `None` selects the whole array, `Some(0)` its Unsigned size and any
/// positive index one element. Invalid or trailing bytes return an error;
/// no successfully decoded prefix is returned. A semantic tag has no value,
/// while a valued NULL has `Some(PropertyValue::Null)`. Date and Time are
/// separate primitive choices; a combined pair is invalid (2024-04-29
/// errata, item 37). This decoder does not validate application tag naming
/// policies or impose the bundled server's provisioning limits.
///
/// ```
/// use bacnet_client::tags::{decode_tags_read, TagsRead};
/// assert_eq!(decode_tags_read(Some(0), &[0x21, 2]).unwrap(), TagsRead::Size(2));
/// assert_eq!(decode_tags_read(None, &[]).unwrap(), TagsRead::Whole(vec![]));
/// ```
pub fn decode_tags_read(array_index: Option<u32>, octets: &[u8]) -> Result<TagsRead, Error> {
    if array_index == Some(0) {
        let (value, end) = decode_application_value(octets, 0)?;
        if let PropertyValue::Unsigned(size) = value {
            if end == octets.len() {
                return u32::try_from(size)
                    .map(TagsRead::Size)
                    .map_err(|_| Error::decoding(0, "Tags array size exceeds u32"));
            }
        }
        return Err(Error::decoding(0, "Tags array size must be one Unsigned"));
    }
    if array_index.is_some() {
        let (tag, end) = decode_name_value(octets, 0)?;
        if end != octets.len() {
            return Err(Error::decoding(end, "trailing bytes after Tags element"));
        }
        return Ok(TagsRead::Element(tag));
    }
    let mut tags = Vec::new();
    let mut offset = 0;
    while offset < octets.len() {
        let (tag, end) = decode_name_value(octets, offset)?;
        if end <= offset || end > octets.len() {
            return Err(Error::decoding(offset, "invalid Tags element boundary"));
        }
        tags.push(tag);
        offset = end;
    }
    Ok(TagsRead::Whole(tags))
}

#[cfg(test)]
mod tests;
