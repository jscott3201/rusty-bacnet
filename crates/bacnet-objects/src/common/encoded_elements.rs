//! Splitting a written array or list whose elements are constructed values
//! that reach the object as raw octets (`PropertyValue::ApplicationData`).

use crate::prelude::*;
use bacnet_encoding::tags::{self, Tag};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

/// A shared codec for one element: the element and the offset past it.
pub(crate) type ElementDecoder<T> = fn(&[u8], usize) -> Result<(T, usize), Error>;

/// The byte chunks of a written value: the raw payload, or the elements of a
/// value as a read returns it.
pub(crate) fn chunks(value: &PropertyValue) -> Result<Vec<&[u8]>, Error> {
    match value {
        PropertyValue::ApplicationData(bytes) => Ok(vec![bytes]),
        PropertyValue::List(elements) => elements
            .iter()
            .map(|element| match element {
                PropertyValue::ApplicationData(bytes) => Ok(bytes.as_slice()),
                _ => Err(super::invalid_data_type_error()),
            })
            .collect(),
        _ => Err(super::invalid_data_type_error()),
    }
}

/// Decode every element in `value`, back to back within each chunk.
///
/// `starts` says whether a tag can begin an element of the property's
/// datatype; `decode` is the shared codec for one element. An element that
/// starts with any other tag is INVALID_DATA_TYPE, one that doesn't decode
/// INVALID_DATA_ENCODING.
pub(crate) fn decode_elements<T>(
    value: &PropertyValue,
    starts: fn(&Tag) -> bool,
    decode: ElementDecoder<T>,
) -> Result<Vec<T>, Error> {
    let mut elements = Vec::new();
    for bytes in chunks(value)? {
        let mut offset = 0;
        while offset < bytes.len() {
            let (element, end) = decode_element(bytes, offset, starts, decode)?;
            elements.push(element);
            offset = end;
        }
    }
    Ok(elements)
}

/// Decode the one element a single-element value holds: a property holding
/// one constructed value, or one array element written by index.
///
/// The value's chunks are joined first (see [`chunks`]): a value read back
/// is one chunk, and a caller that split the octets at each member's tag
/// hands over the same octets in pieces. On top of [`decode_element`]'s
/// refusals, octets that hold no element at all, or anything after the one
/// element, are INVALID_DATA_ENCODING: once the value has opened as the
/// element, any count other than one is an encoding fault, whatever tag
/// follows. A value whose datatype has an empty encoding checks for it
/// before calling this.
pub(crate) fn decode_single_element<T>(
    value: &PropertyValue,
    starts: fn(&Tag) -> bool,
    decode: ElementDecoder<T>,
) -> Result<T, Error> {
    let bytes = chunks(value)?.concat();
    if bytes.is_empty() {
        return Err(super::invalid_data_encoding_error());
    }
    let (element, end) = decode_element(&bytes, 0, starts, decode)?;
    if end != bytes.len() {
        return Err(super::invalid_data_encoding_error());
    }
    Ok(element)
}

/// Decode the element at `offset`, with the errors [`decode_elements`]
/// describes, returning it and the offset past it.
pub(crate) fn decode_element<T>(
    bytes: &[u8],
    offset: usize,
    starts: fn(&Tag) -> bool,
    decode: ElementDecoder<T>,
) -> Result<(T, usize), Error> {
    match tags::decode_tag(bytes, offset) {
        Ok((tag, _)) if starts(&tag) => {}
        Ok(_) => return Err(super::invalid_data_type_error()),
        Err(_) => return Err(super::invalid_data_encoding_error()),
    }
    decode(bytes, offset).map_err(|_| super::invalid_data_encoding_error())
}
