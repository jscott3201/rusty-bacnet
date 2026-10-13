//! `BACnetNameValue` framing (Clause 21, corrected by errata 2024-04-29
//! item 37), the element of a Tags array (#1553).
//!
//! A name under primitive context tag 0 is followed by at most one
//! application-tagged primitive. Date and Time are valid separately; their
//! pair is not one NameValue. The next array element starts at context tag 0.

use bacnet_types::constructed::BACnetNameValue;
use bacnet_types::error::Error;
use bytes::BytesMut;

use super::tagged::decode_ctx_character_string;
use crate::primitives;
use crate::tags::{self, TagClass};

const WHAT: &str = "BACnetNameValue";

/// Encode one `BACnetNameValue`, appending to `buf`.
///
/// A nonprimitive value (`List` or `ApplicationData`) or a name too long
/// to encode returns an error before `buf` changes.
pub fn encode_name_value(buf: &mut BytesMut, value: &BACnetNameValue) -> Result<(), Error> {
    if value
        .value
        .as_ref()
        .is_some_and(|value| !value.is_primitive())
    {
        return Err(Error::Encoding(format!(
            "{WHAT}: a value must be a primitive datatype"
        )));
    }
    primitives::encode_ctx_character_string(buf, 0, &value.name)?;
    if let Some(primitive) = &value.value {
        primitives::encode_property_value(buf, primitive)?;
    }
    Ok(())
}

/// Whether another application-tagged value follows; false at end of input.
fn next_is_application(data: &[u8], offset: usize) -> Result<bool, Error> {
    if offset >= data.len() {
        return Ok(false);
    }
    let (tag, _) = tags::decode_tag(data, offset)?;
    Ok(tag.class == TagClass::Application)
}

/// Decode one `BACnetNameValue` at `offset`, returning it and the next offset.
///
/// The next context-tagged element is left to the caller, which must check
/// complete consumption when decoding a whole value. Missing or malformed
/// names, unsupported character sets, unknown application types, truncated
/// values and a second application value (including Date followed by Time)
/// are errors.
pub fn decode_name_value(data: &[u8], offset: usize) -> Result<(BACnetNameValue, usize), Error> {
    let (name, offset) = decode_ctx_character_string(data, offset, 0, WHAT)?;
    if !next_is_application(data, offset)? {
        return Ok((BACnetNameValue { name, value: None }, offset));
    }
    let (value, offset) = primitives::decode_application_value(data, offset)?;
    if next_is_application(data, offset)? {
        return Err(Error::decoding(
            offset,
            format!("{WHAT}: a value must contain only one primitive"),
        ));
    }
    Ok((
        BACnetNameValue {
            name,
            value: Some(value),
        },
        offset,
    ))
}
