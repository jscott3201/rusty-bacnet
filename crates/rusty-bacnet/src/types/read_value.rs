//! One decoder for the value octets a property read carries (#1296): a
//! ReadProperty ACK, a ReadPropertyMultiple result, a COV notification value,
//! and the local `BACnetServer.read_property`, which encodes the served value
//! first so it comes back exactly as a network read would.
//!
//! The octets are first split into tagged elements by their framing alone:
//! tag headers, lengths and matched opening and closing tags. Only broken
//! framing is an error. Then:
//!
//! - When the property is one of the constructed collections the binding
//!   also writes as typed values (a Recipient_List, a Group's members and
//!   results, a Command's Action and others; see
//!   [`bacnet_encoding::constructed::property_element()`]), and the
//!   octets decode as that collection's elements, the value is a typed read
//!   (#1310): each element keeps its octets, and `.value` gives the shape
//!   the typed write takes. A value that doesn't decode that way goes on to
//!   the rules below.
//! - When any element carries a context tag, the value is a constructed
//!   production (a timestamp CHOICE, a list of COV subscriptions and so on).
//!   Nothing in the octets says where one list element ends and the next
//!   starts, so the whole value comes back as
//!   [`PropertyValue::ApplicationData`] holding the octets as received. Local
//!   reads and writes already carry such values that way, and writing one
//!   back sends the same octets.
//! - When an application element is well framed but [`PropertyValue`] has no
//!   form for its content (a CharacterString in UCS-4, DBCS or JIS X 0208,
//!   UTF-8 that doesn't decode, an ENUMERATED past 32 bits), the whole value
//!   comes back the same way, as its octets.
//! - Otherwise each element decodes to its typed value. A whole read (no
//!   array index) of a property the stack's classification table (see
//!   [`standard_array_property`]) marks as a BACnetARRAY or BACnetLIST on that
//!   object type is a [`PropertyValue::List`] at every length, zero and one
//!   included, so its shape doesn't follow its length. Any other read is the
//!   bare value when it holds exactly one element, and a list in wire order
//!   when it holds none or several (a BACnetDateTime is a Date and then a
//!   Time, for example).

use bacnet_encoding::primitives::decode_application_value;
use bacnet_encoding::tags::{self, app_tag, TagClass};
use bacnet_objects::traits::{standard_array_property, standard_list_property};
use bacnet_services::read_property::ReadPropertyACK;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::constructed_read;
use super::PyPropertyValue;

/// Decode the value octets read from `property` of an object of
/// `object_type`, with the read's `array_index`, by the rules in the module
/// documentation. Broken framing is an error; a caller that reports raw
/// octets instead keeps that fallback.
pub(crate) fn decode_read_value(
    object_type: ObjectType,
    property: PropertyIdentifier,
    array_index: Option<u32>,
    octets: &[u8],
) -> Result<PyPropertyValue, Error> {
    let starts = application_elements(octets)?;
    if let Some(value) = constructed_read::decode(object_type, property, array_index, octets) {
        return Ok(value);
    }
    Ok(PyPropertyValue::from_rust(generic(
        object_type,
        property,
        array_index,
        octets,
        starts,
    )))
}

/// The value of `octets` by the rules after the typed read. `starts` are the
/// top-level elements [`application_elements`] found, or `None` when one of
/// them carries a context tag.
fn generic(
    object_type: ObjectType,
    property: PropertyIdentifier,
    array_index: Option<u32>,
    octets: &[u8],
    starts: Option<Vec<usize>>,
) -> PropertyValue {
    let Some(starts) = starts else {
        return PropertyValue::ApplicationData(octets.to_vec());
    };
    let mut elements = Vec::with_capacity(starts.len());
    for start in starts {
        match decode_application_value(octets, start) {
            Ok((element, _)) => elements.push(element),
            Err(_) => return PropertyValue::ApplicationData(octets.to_vec()),
        }
    }
    let collection = array_index.is_none()
        && (standard_array_property(object_type, property)
            || standard_list_property(object_type, property));
    match <[PropertyValue; 1]>::try_from(elements) {
        Ok([element]) if !collection => element,
        Ok([element]) => PropertyValue::List(vec![element]),
        Err(elements) => PropertyValue::List(elements),
    }
}

/// Check the framing of `octets` and return where each top-level element
/// starts, or `None` when any of them carries a context tag. Contents aren't
/// interpreted: a primitive element is skipped by its length (an application
/// BOOLEAN has none), and a constructed one through its matching closing tag.
fn application_elements(octets: &[u8]) -> Result<Option<Vec<usize>>, Error> {
    let mut starts = Vec::new();
    let mut context = false;
    let mut offset = 0;
    while offset < octets.len() {
        let (tag, after) = tags::decode_tag(octets, offset)?;
        if tag.is_closing || (tag.is_opening && tag.class != TagClass::Context) {
            return Err(Error::decoding(offset, "unexpected opening or closing tag"));
        }
        let end = if tag.is_opening {
            tags::extract_context_value(octets, after, tag.number)?.1
        } else if tag.class == TagClass::Application && tag.number == app_tag::BOOLEAN {
            after
        } else {
            let end = after
                .checked_add(tag.length as usize)
                .ok_or_else(|| Error::decoding(after, "length overflow"))?;
            if end > octets.len() {
                return Err(Error::buffer_too_short(end, octets.len()));
            }
            end
        };
        context |= tag.class == TagClass::Context;
        starts.push(offset);
        offset = end;
    }
    Ok((!context).then_some(starts))
}

/// Decode a ReadProperty ACK's value, shaped by the object, property and
/// array index the ACK names.
pub(crate) fn decode_read_ack(ack: &ReadPropertyACK) -> Result<PyPropertyValue, Error> {
    decode_read_value(
        ack.object_identifier.object_type(),
        ack.property_identifier,
        ack.property_array_index,
        &ack.property_value,
    )
}

#[cfg(test)]
#[path = "read_value_tests.rs"]
mod tests;
