//! `BACnetPropertyValue` (Clause 21) and the property-value field that the
//! property access services carry.
//!
//! A property value is four context-tagged members: the property identifier
//! `[0]`, an optional array index `[1]`, the value inside an opening/closing
//! `[2]` pair, and an optional priority `[3]`. The value stays encoded; its
//! datatype depends on the property, which the application layer knows.
//!
//! Decoding the value needs to know where it ends. Each caller names the tags
//! that may follow a value in its own production ([`PropertyValueBoundary`]),
//! which also lets a value encoded in the pre-framing EventParameters form be
//! told apart from the frame around it.

#[cfg(feature = "alloc")]
use bacnet_types::constructed::BACnetPropertyValue;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
#[cfg(feature = "alloc")]
use bytes::{BufMut, BytesMut};

#[cfg(feature = "alloc")]
use crate::primitives;
use crate::tags;

#[cfg(feature = "alloc")]
mod decode;

#[cfg(feature = "alloc")]
pub use decode::{
    decode_bacnet_property_value, decode_bacnet_property_value_in_list,
    decode_bacnet_property_value_in_list_detailed, PropertyValueDecodeError,
    PropertyValueDecodeFailure, PropertyValueDecodeStage,
};

/// Append the encoding of one property value to `buf`.
///
/// The value bytes are copied inside the `[2]` pair as they are; callers that
/// accept values from outside check them first. Only with `alloc`.
#[cfg(feature = "alloc")]
pub fn encode_bacnet_property_value(value: &BACnetPropertyValue, buf: &mut BytesMut) {
    // [0] propertyIdentifier
    primitives::encode_ctx_unsigned(buf, 0, value.property_identifier.to_raw() as u64);
    // [1] propertyArrayIndex (optional)
    if let Some(idx) = value.property_array_index {
        primitives::encode_ctx_unsigned(buf, 1, idx as u64);
    }
    // [2] value (opening/closing)
    tags::encode_opening_tag(buf, 2);
    buf.put_slice(&value.value);
    tags::encode_closing_tag(buf, 2);
    // [3] priority (optional)
    if let Some(prio) = value.priority {
        primitives::encode_ctx_unsigned(buf, 3, prio as u64);
    }
}

/// A tag that may follow an encoded property value in the enclosing
/// production, used to find where the value ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyValueBoundary {
    /// The end of the input.
    End,
    /// A context tag with this number, primitive or opening.
    Context(u8),
    /// A primitive context tag with this number that runs to the end of the
    /// input.
    ContextToEnd(u8),
    /// A closing tag with this number.
    Closing(u8),
}

/// Whether `offset` in `data` is at `boundary`.
fn matches_property_boundary(data: &[u8], offset: usize, boundary: PropertyValueBoundary) -> bool {
    match boundary {
        PropertyValueBoundary::End => offset == data.len(),
        PropertyValueBoundary::Context(number) => {
            tags::decode_tag(data, offset).is_ok_and(|(tag, _)| tag.is_context(number))
        }
        PropertyValueBoundary::ContextToEnd(number) => {
            tags::decode_tag(data, offset).is_ok_and(|(tag, content_start)| {
                tag.is_context(number)
                    && content_start
                        .checked_add(tag.length as usize)
                        .is_some_and(|end| end == data.len())
            })
        }
        PropertyValueBoundary::Closing(number) => {
            tags::decode_tag(data, offset).is_ok_and(|(tag, _)| tag.is_closing_tag(number))
        }
    }
}

/// Extract the encoded value of `property` that starts at `offset`, just
/// inside the opening tag `closing_tag`, returning the value bytes and the
/// offset past the matching closing tag.
///
/// `boundaries` lists what may follow the closing tag in the enclosing
/// production. It is consulted only for an EVENT_PARAMETERS value in the
/// pre-framing form, whose payload can hold octets that look like the closing
/// tag.
pub fn extract_property_value<'a>(
    data: &'a [u8],
    offset: usize,
    closing_tag: u8,
    property: PropertyIdentifier,
    boundaries: &[PropertyValueBoundary],
) -> Result<(&'a [u8], usize), Error> {
    if property == PropertyIdentifier::EVENT_PARAMETERS
        && data.get(offset..offset.saturating_add(2)) == Some(&[0xfe, 0xff])
    {
        // Before EventParameter framing, tag 255 wrapped arbitrary octets. Use
        // the enclosing service grammar to distinguish a payload marker from
        // the wrapper terminator without consuming a sibling property. Reject
        // multiple valid boundaries because the old format cannot disambiguate them.
        let outer_close = (closing_tag << 4) | 0x0f;
        let mut candidate = None;
        for pos in offset.saturating_add(4)..data.len() {
            let end = pos + 1;
            if data[pos] == outer_close
                && data.get(pos - 2..pos) == Some(&[0xff, 0xff])
                && boundaries
                    .iter()
                    .any(|boundary| matches_property_boundary(data, end, *boundary))
            {
                if candidate.is_some() {
                    return Err(Error::invalid_tag(
                        offset,
                        "legacy EventParameters value has ambiguous closing tags",
                    ));
                }
                candidate = Some((pos, end));
            }
        }
        if let Some((pos, end)) = candidate {
            return Ok((&data[offset..pos], end));
        }
        return Err(Error::missing(
            offset,
            "legacy EventParameters value is missing its closing tags",
        ));
    }

    tags::extract_context_value(data, offset, closing_tag)
}
