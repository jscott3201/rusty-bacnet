//! One decoder for the value octets a property read carries (#1296): a
//! ReadProperty ACK, a ReadPropertyMultiple result, a COV notification value,
//! and the local `BACnetServer.read_property`, which encodes the served value
//! first so it comes back exactly as a network read would.
//!
//! The octets are first split into tagged elements by their framing alone,
//! with the shared tag reader (`bacnet_encoding::tags::tree::TagReader`):
//! tag headers, lengths and matched opening and closing tags, nested at
//! most 32 deep. Only broken framing is an error. Then:
//!
//! - A whole Network Port BDT or FDT property keeps its original octets.
//!   If the native table decoder accepts them, the binding projects its
//!   ordered rows. A well-framed value it rejects keeps its tag tree or
//!   raw application data.
//! - When the property is one of the constructed collections the binding
//!   also writes as typed values (a Recipient_List, a Group's members and
//!   results, a Command's Action and others; see
//!   [`bacnet_encoding::constructed::property_element()`]), and the
//!   octets decode as that collection's elements, the value is a typed read
//!   (#1310): each element keeps its octets, and `.value` gives the shape
//!   the typed write takes. A value that doesn't decode that way goes on to
//!   the rules below.
//! - When any element carries a context tag, the value is a constructed
//!   production (a timestamp CHOICE, a vendor's structure and so on) the
//!   binding has no typed form for. Nothing in the octets says where one
//!   list element ends and the next starts, but every tag in them reads
//!   (Clause 20.2.1.6), so the whole value is one value tagged `"tagged"`
//!   (#2618): it holds the octets as received, so writing it back sends the
//!   same octets, and its `.value` is the tree of its tags (see `tag_tree`).
//!   A Priority_Array is the exception: each slot is one tagged element
//!   even when it is the date-time choice under context tag 1 (a DateTime
//!   Value's set slot, #2098), so it still splits into slots. That slot
//!   keeps its octets as read; its `.value` is the date and time. A
//!   Present_Value write takes the bare pair, not them.
//! - When an application element is well framed but [`PropertyValue`] has no
//!   form for its content (a CharacterString in UCS-4, DBCS or JIS X 0208,
//!   UTF-8 that doesn't decode, an ENUMERATED past 32 bits), or the tree
//!   holds more elements than the decoder's bound, the whole value comes
//!   back as [`PropertyValue::ApplicationData`] holding its octets.
//! - Otherwise each element decodes to its typed value. A whole read (no
//!   array index) of a property the stack's value-type table (see
//!   [`standard_value_type`]) marks as a BACnetARRAY or BACnetLIST on that
//!   object type is a [`PropertyValue::List`] at every length, zero and one
//!   included, so its shape doesn't follow its length. Any other read is the
//!   bare value when it holds exactly one element, and a list in wire order
//!   when it holds none or several (a BACnetDateTime is a Date and then a
//!   Time, for example).

use bacnet_encoding::primitives::{decode_application_value, decode_priority_date_time};
use bacnet_encoding::tags::tree::TagReader;
use bacnet_encoding::tags::{self, TagClass};
use bacnet_services::read_property::ReadPropertyACK;
use bacnet_types::datatype::standard_value_type;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::PyPropertyValue;
use super::{constructed_read, tag_tree};

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
    let mut starts = application_elements(octets)?;
    if let Some(table) =
        super::network_port_value::NetworkPortTable::of(object_type, property, array_index)
    {
        let value = match table.read(octets) {
            Ok(value) => value,
            Err(_) if starts.is_none() && tag_tree::is_tree(octets) => {
                PyPropertyValue::tree(octets.to_vec())
            }
            Err(_) => PyPropertyValue::from_rust(PropertyValue::ApplicationData(octets.to_vec())),
        };
        return Ok(value.read_from(property));
    }
    if let Some(value) = constructed_read::decode(object_type, property, array_index, octets) {
        return Ok(value.read_from(property));
    }
    if starts.is_none() && property == PropertyIdentifier::PRIORITY_ARRAY {
        // Each BACnetPriorityValue choice is one tagged element, the
        // date-time choice under context tag 1 included (#2098), so a
        // Priority_Array splits into its slots by framing alone.
        starts = priority_slots(octets);
    }
    let value = match starts {
        Some(starts) => {
            PyPropertyValue::from_rust(generic(object_type, property, array_index, octets, starts))
        }
        None if tag_tree::is_tree(octets) => PyPropertyValue::tree(octets.to_vec()),
        None => PyPropertyValue::from_rust(PropertyValue::ApplicationData(octets.to_vec())),
    };
    Ok(value.read_from(property))
}

/// The value of application elements `octets` by the rules after the typed
/// read; `starts` are where [`application_elements`] found them.
fn generic(
    object_type: ObjectType,
    property: PropertyIdentifier,
    array_index: Option<u32>,
    octets: &[u8],
    starts: Vec<usize>,
) -> PropertyValue {
    let mut elements = Vec::with_capacity(starts.len());
    for start in starts {
        match decode_application_value(octets, start) {
            Ok((element, _)) => elements.push(element),
            Err(_) => return PropertyValue::ApplicationData(octets.to_vec()),
        }
    }
    let collection = array_index.is_none()
        && standard_value_type(object_type, property)
            .is_some_and(|value_type| value_type.is_array() || value_type.is_list());
    match <[PropertyValue; 1]>::try_from(elements) {
        Ok([element]) if !collection => element,
        Ok([element]) => PropertyValue::List(vec![element]),
        Err(elements) => PropertyValue::List(elements),
    }
}

/// Where each slot of a Priority_Array starts, when every context-tagged
/// slot is the date-time choice; `None` for any other context tag.
fn priority_slots(octets: &[u8]) -> Option<Vec<usize>> {
    let mut starts = Vec::new();
    let mut offset = 0;
    while offset < octets.len() {
        let (tag, _) = tags::decode_tag(octets, offset).ok()?;
        let end = if tag.class == TagClass::Context {
            decode_priority_date_time(octets, offset).ok()?.1
        } else {
            decode_application_value(octets, offset).ok()?.1
        };
        starts.push(offset);
        offset = end;
    }
    Some(starts)
}

/// Check the framing of `octets` with the shared tag reader and return
/// where each top-level element starts, or `None` when any of them carries
/// a context tag. Contents aren't typed here: that is the same walk the
/// typed path always made, and the tree is built only for a context tag.
fn application_elements(octets: &[u8]) -> Result<Option<Vec<usize>>, Error> {
    let mut starts = Vec::new();
    let mut context = false;
    for item in TagReader::new(octets) {
        let item = item?;
        if item.depth == 0 && !item.tag.is_closing {
            context |= item.tag.class == TagClass::Context;
            starts.push(item.offset);
        }
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
