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
//! told apart from the frame around it. Every encoder that writes a value
//! between its tags checks it first with [`check_property_value`], so the
//! decoder's first match is always the value's real end (#2376).

#[cfg(feature = "alloc")]
use bacnet_types::constructed::BACnetPropertyValue;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::{EncodeError, Error};
#[cfg(feature = "alloc")]
use bytes::{BufMut, BytesMut};

#[cfg(feature = "alloc")]
use crate::primitives;
use crate::tags;

#[cfg(feature = "alloc")]
mod decode;

#[cfg(feature = "alloc")]
pub use decode::{
    PropertyValueDecodeError, PropertyValueDecodeFailure, PropertyValueDecodeStage,
    decode_bacnet_property_value, decode_bacnet_property_value_in_list,
    decode_bacnet_property_value_in_list_detailed,
};

/// Append the encoding of one property value to `buf`.
///
/// The value bytes are copied inside the `[2]` pair as they are; callers that
/// accept values from outside check them first. Fails, leaving `buf`
/// unchanged, on a value [`check_property_value`] refuses. Only with `alloc`.
#[cfg(feature = "alloc")]
pub fn encode_bacnet_property_value(
    value: &BACnetPropertyValue,
    buf: &mut BytesMut,
) -> Result<(), Error> {
    check_property_value(value.property_identifier, &value.value, 2)
        .map_err(|refused| refused.within("BACnetPropertyValue"))?;
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
    Ok(())
}

/// Whether `value` opens with the `[255]` wrapper of the pre-framing
/// EventParameters form, which [`extract_property_value`] reads by scanning
/// for its closing pair.
fn is_legacy_event_parameters(property: PropertyIdentifier, value: &[u8]) -> bool {
    property == PropertyIdentifier::EVENT_PARAMETERS && value.starts_with(&LEGACY_OPEN)
}

/// The `[255]` opening tag that starts a pre-framing EventParameters value.
const LEGACY_OPEN: [u8; 2] = [0xfe, 0xff];
/// The `[255]` closing tag that ends one.
const LEGACY_CLOSE: [u8; 2] = [0xff, 0xff];

/// The closing tag octet of the `[closing_tag]` pair around a property value.
const fn outer_closing_octet(closing_tag: u8) -> u8 {
    (closing_tag << 4) | 0x0f
}

/// Check that the encoded value of `property`, written inside the opening and
/// closing tags `closing_tag`, decodes back to itself in every production
/// that carries it.
///
/// Only a value in the pre-framing EventParameters form can fail: the
/// decoder ends it at the first `[255]` closing pair followed by the outer
/// closing tag and a valid next field, so the value must end with that pair,
/// be at least four octets long, and hold no such pair followed by the outer
/// closing tag octet before its end. Every encoder that writes a property
/// value calls this before writing anything.
pub fn check_property_value(
    property: PropertyIdentifier,
    value: &[u8],
    closing_tag: u8,
) -> Result<(), EncodeError> {
    if !is_legacy_event_parameters(property, value) {
        return Ok(());
    }
    if value.len() < 4 || !value.ends_with(&LEGACY_CLOSE) {
        return Err(EncodeError::new(
            "legacy EventParameters value is missing its closing tags",
        ));
    }
    // The decoder looks for the outer closing tag from the value's fifth
    // octet on, so only a window from the third octet on can end it early.
    let outer_close = outer_closing_octet(closing_tag);
    if value
        .windows(3)
        .skip(2)
        .any(|window| window[..2] == LEGACY_CLOSE && window[2] == outer_close)
    {
        return Err(EncodeError::new(
            "legacy EventParameters value holds its outer closing tag before its end",
        ));
    }
    Ok(())
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
    /// A primitive context tag with this number holding one octet, followed
    /// by the end of the input or a context tag `[0]`: a priority as the
    /// encoder writes it, then the end or the next property value.
    OneOctetContext(u8),
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
        PropertyValueBoundary::OneOctetContext(number) => {
            tags::decode_tag(data, offset).is_ok_and(|(tag, content_start)| {
                tag.is_context(number)
                    && tag.length == 1
                    && (matches_property_boundary(
                        data,
                        content_start + 1,
                        PropertyValueBoundary::End,
                    ) || matches_property_boundary(
                        data,
                        content_start + 1,
                        PropertyValueBoundary::Context(0),
                    ))
            })
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
        && data.get(offset..offset.saturating_add(2)) == Some(&LEGACY_OPEN)
    {
        // Before EventParameter framing, tag 255 wrapped arbitrary octets.
        // The value ends at the first `[255]` closing pair followed by the
        // outer closing tag and a valid next field of the enclosing
        // production; a pair followed by anything else is payload. The
        // encoders refuse a value holding such a pair before its end
        // (`check_property_value`), so the first match is the real end of
        // anything they write. From an older peer, a value with more than
        // one possible end is read up to the first (#2376).
        let outer_close = outer_closing_octet(closing_tag);
        for pos in offset.saturating_add(4)..data.len() {
            let end = pos + 1;
            if data[pos] == outer_close
                && data.get(pos - 2..pos) == Some(&LEGACY_CLOSE)
                && boundaries
                    .iter()
                    .any(|boundary| matches_property_boundary(data, end, *boundary))
            {
                return Ok((&data[offset..pos], end));
            }
        }
        return Err(Error::missing(
            offset,
            "legacy EventParameters value is missing its closing tags",
        ));
    }

    tags::extract_context_value(data, offset, closing_tag)
}
