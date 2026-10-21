//! `BACnetPropertyReference` and `ReadAccessSpecification` (ASHRAE 135-2020
//! Clause 21), shared by the ReadPropertyMultiple request, the COV and Audit
//! services and a Group object's List_Of_Group_Members (Clause 12.14.5).
//!
//! A property reference is the property identifier in context tag `[0]`,
//! then an optional Unsigned array index in `[1]`. A specification is the
//! object identifier in `[0]`, then its references back to back inside an
//! opening/closing `[1]` pair.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::{DecodeError, DecodingKind, Error};
use bytes::BytesMut;

pub use super::reference_sinks::encode_property_reference_to_sink;
use super::tagged::{
    decode_ctx_object_id, decode_ctx_unsigned, decode_optional_ctx, expect_opening,
};
use super::MAX_FRAMED_ITEMS;
use crate::{primitives, tags};

/// Encode one `BACnetPropertyReference`, appending to `buf`.
pub fn encode_property_reference(buf: &mut BytesMut, reference: &PropertyReference) {
    encode_property_reference_to_sink(buf, reference).expect("BytesMut accepts each reference");
}

/// Decode one `BACnetPropertyReference` at `offset`; returns it and the
/// offset just past it. The array index is read only when the next tag is a
/// primitive context tag `[1]`, so a reference may end the input or be
/// followed by anything else.
///
/// A member whose contents run past the end of `data` fails with
/// [`Error::BufferTooShort`]; any other malformed input with
/// [`Error::Decoding`].
pub fn decode_property_reference(
    data: &[u8],
    offset: usize,
) -> Result<(PropertyReference, usize), Error> {
    const PROPERTY: &str = "PropertyReference property-id";
    const INDEX: &str = "PropertyReference array-index";
    let (property, next) = decode_ctx_unsigned::<u32>(data, offset, 0, PROPERTY)?;
    let (property_array_index, offset) =
        decode_optional_ctx(data, next, 1, INDEX, decode_ctx_unsigned::<u32>)?;
    Ok((
        PropertyReference {
            property_identifier: PropertyIdentifier::from_raw(property),
            property_array_index,
        },
        offset,
    ))
}

/// Encode one `ReadAccessSpecification`, appending to `buf`. An empty
/// reference list encodes as the bare `[1]` pair; the ReadPropertyMultiple
/// request encoder is what refuses one.
pub fn encode_read_access_specification(buf: &mut BytesMut, spec: &ReadAccessSpecification) {
    primitives::encode_ctx_object_id(buf, 0, &spec.object_identifier);
    tags::encode_opening_tag(buf, 1);
    for reference in &spec.list_of_property_references {
        encode_property_reference(buf, reference);
    }
    tags::encode_closing_tag(buf, 1);
}

/// Decode one `ReadAccessSpecification` at `offset`; returns it and the
/// offset just past its closing `[1]` tag, so a sequence of them is walked
/// by calling this at each element's start. Errors are as for
/// [`decode_property_reference`].
pub fn decode_read_access_specification(
    data: &[u8],
    offset: usize,
) -> Result<(ReadAccessSpecification, usize), Error> {
    const WHAT: &str = "ReadAccessSpecification";
    let (object_identifier, end) = decode_ctx_object_id(data, offset, 0, WHAT)?;
    let mut offset = expect_opening(data, end, 1, WHAT)?;
    let mut list_of_property_references = Vec::new();
    loop {
        if offset >= data.len() {
            return Err(DecodeError::new(
                DecodingKind::Missing,
                offset,
                "missing closing tag 1",
            )
            .within(WHAT)
            .into());
        }
        if list_of_property_references.len() >= MAX_FRAMED_ITEMS {
            return Err(DecodeError::new(
                DecodingKind::Overflow,
                offset,
                "property references exceed the item limit",
            )
            .within(WHAT)
            .into());
        }
        let (tag, tag_end) = tags::decode_tag(data, offset)?;
        if tag.is_closing_tag(1) {
            offset = tag_end;
            break;
        }
        let (reference, next) = decode_property_reference(data, offset)?;
        list_of_property_references.push(reference);
        offset = next;
    }
    Ok((
        ReadAccessSpecification {
            object_identifier,
            list_of_property_references,
        },
        offset,
    ))
}
