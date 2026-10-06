//! `BACnetNameValue` framing (Clause 21), the element of a Tags array
//! (#1553).
//!
//! The SEQUENCE has no frame of its own. The name is a CharacterString under
//! primitive context tag 0. A value, when there is one, follows as the
//! application-tagged encoding of its own datatype: one primitive, or a
//! BACnetDateTime as an application Date then an application Time. An element
//! of an array always opens with context tag 0, so an application Time that
//! follows a Date can only be the time of a BACnetDateTime.

use bacnet_types::constructed::{BACnetNameValue, TagValue};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;
use bytes::BytesMut;

use super::tagged::decode_ctx_character_string;
use crate::primitives;
use crate::tags::{self, app_tag, TagClass};

/// The production name decode errors carry.
const WHAT: &str = "BACnetNameValue";

/// Encode one `BACnetNameValue`, appending to `buf`.
///
/// A [`TagValue::Primitive`] that isn't of a primitive datatype (a `List`
/// or `ApplicationData`) returns an error before `buf` changes, as does a
/// name too long to encode.
pub fn encode_name_value(buf: &mut BytesMut, value: &BACnetNameValue) -> Result<(), Error> {
    if let Some(TagValue::Primitive(primitive)) = &value.value {
        if !primitive.is_primitive() {
            return Err(Error::Encoding(format!(
                "{WHAT}: a value must be of a primitive datatype or a BACnetDateTime"
            )));
        }
    }
    primitives::encode_ctx_character_string(buf, 0, &value.name)?;
    match &value.value {
        None => {}
        Some(TagValue::Primitive(primitive)) => primitives::encode_property_value(buf, primitive)?,
        Some(TagValue::DateTime { date, time }) => {
            primitives::encode_app_date(buf, date);
            primitives::encode_app_time(buf, time);
        }
    }
    Ok(())
}

/// Whether the tag at `offset` is application-tagged; `false` at the end of
/// `data`. A tag that doesn't decode is an error.
fn next_is_application(data: &[u8], offset: usize, tag_number: Option<u8>) -> Result<bool, Error> {
    if offset >= data.len() {
        return Ok(false);
    }
    let (tag, _) = tags::decode_tag(data, offset)?;
    Ok(tag.class == TagClass::Application && tag_number.is_none_or(|n| tag.number == n))
}

/// Decode one `BACnetNameValue` at `offset`, returning it and the offset past
/// it.
///
/// Bytes after the element are left for the caller, as the next element of
/// an array begins with context tag 0; a caller decoding a whole value must
/// check the returned offset. A missing or constructed name, a name that
/// isn't a CharacterString of a supported character set, a value under an
/// unknown application tag, and contents that run past `data` are errors.
pub fn decode_name_value(data: &[u8], offset: usize) -> Result<(BACnetNameValue, usize), Error> {
    let (name, offset) = decode_ctx_character_string(data, offset, 0, WHAT)?;
    if !next_is_application(data, offset, None)? {
        return Ok((BACnetNameValue { name, value: None }, offset));
    }
    let (primitive, offset) = primitives::decode_application_value(data, offset)?;
    let (value, offset) = match primitive {
        PropertyValue::Date(date) if next_is_application(data, offset, Some(app_tag::TIME))? => {
            match primitives::decode_application_value(data, offset)? {
                (PropertyValue::Time(time), end) => (TagValue::DateTime { date, time }, end),
                _ => return Err(Error::decoding(offset, format!("{WHAT}: expected a Time"))),
            }
        }
        primitive => (TagValue::Primitive(primitive), offset),
    };
    Ok((
        BACnetNameValue {
            name,
            value: Some(value),
        },
        offset,
    ))
}
