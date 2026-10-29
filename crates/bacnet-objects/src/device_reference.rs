//! Read values, write decoding and the Device check for every
//! device-qualified reference property (#1182, #1234, #1313).
//!
//! A `BACnetDeviceObjectPropertyReference` or `BACnetDeviceObjectReference`
//! value is served as `PropertyValue::ApplicationData` holding the reference's
//! context-tagged members, encoded by the shared codecs in
//! `bacnet_encoding::constructed`: an array or list of them is one such value
//! per element. A written value is decoded here too, with one set of errors
//! whichever object takes it, and each stored reference passes
//! [`check_device_member`] first.
//!
//! The users: Averaging and Event Enrollment Object_Property_Reference, the
//! Log_DeviceObjectProperty of both trend objects, Schedule and Channel
//! List_Of_Object_Property_References, Global Group Group_Members, Structured
//! View Subordinate_List, Staging Target_References, the Life Safety member
//! lists, Access Door Door_Members, Access Point Access_Doors and
//! Access_Event_Credential, Access Zone Entry_Points and Exit_Points, Access
//! Rights Accompaniment, Access User Credentials, Members and Member_Of, the
//! elevator family's Energy_Meter_Ref and Audit Log Member_Of. A Command's
//! action commands and an Access Rights rule carry a device identifier inside
//! another production, so they share only [`check_device_member`].
//!
//! # Refusal codes for a written value
//!
//! One rule for every writable user, so the same octets get the same answer
//! whichever property they're written to:
//!
//! - A value of another kind than raw octets (or a list of raw-octet chunks),
//!   or octets whose first element can't open the production, is PROPERTY /
//!   INVALID_DATA_TYPE: the value isn't a reference at all.
//! - In a list or array, the same goes for any later element that can't open
//!   the production: each element is a value of its own.
//! - A single-reference value (a property holding one reference, or one array
//!   element written by index) that opens correctly but isn't exactly one
//!   whole reference is PROPERTY / INVALID_DATA_ENCODING, whatever follows
//!   the reference: a second reference, a context tag of no member, an
//!   application tag, or nothing usable at all. Once the value has opened as
//!   a reference, anything other than exactly one is an encoding fault.
//! - A reference that opens correctly but doesn't decode in full is
//!   INVALID_DATA_ENCODING everywhere.
//! - The Device member is judged after the decode, by
//!   [`check_device_member`] (VALUE_OUT_OF_RANGE), and on properties held to
//!   this device then by [`check_local_member`].
//!
//! # Unset references
//!
//! A property holding at most one reference (Averaging and Event Enrollment
//! Object_Property_Reference, Trend Log Log_DeviceObjectProperty) keeps no
//! reference while unset, and reads as [`unset_reference`] then, since the
//! production has no empty encoding (#1417). Writing a reference that
//! [`BACnetDeviceObjectPropertyReference::is_unset`] clears the property, so
//! a value read writes back unchanged. A NULL is no reference: it is refused
//! as INVALID_DATA_TYPE, which the bundled server turns into the success
//! that changes nothing (Clause 15.9.2, `handlers::relinquish`).
//!
//! The single-reference rule lives in `common::decode_single_element`, which
//! the Loop and Pulse Converter references in [`crate::reference`] use too
//! (#1395). Those hold a `BACnetObjectPropertyReference`, which has no Device
//! member, so the same octets get the same code there as well.

use crate::prelude::*;
use bacnet_encoding::constructed::{
    decode_device_object_property_reference, decode_device_object_reference,
    encode_device_object_property_reference, encode_device_object_reference,
};
use bacnet_encoding::tags::Tag;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetDeviceObjectReference, BACnetObjectPropertyReference,
};
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;

use crate::common;

/// One of the two device-qualified reference productions of Clause 21, with
/// its shared codec.
pub(crate) trait DeviceReference: Sized {
    /// Append the reference's context-tagged members to `buf`.
    fn encode(&self, buf: &mut BytesMut);

    /// Decode one reference at `offset`: the reference and the offset past it.
    fn decode(bytes: &[u8], offset: usize) -> Result<(Self, usize), Error>;

    /// Whether `tag` can open the production: the first member it may begin
    /// with.
    fn starts(tag: &Tag) -> bool;

    /// The optional Device member.
    fn device(&self) -> Option<ObjectIdentifier>;
}

impl DeviceReference for BACnetDeviceObjectPropertyReference {
    fn encode(&self, buf: &mut BytesMut) {
        encode_device_object_property_reference(buf, self);
    }

    fn decode(bytes: &[u8], offset: usize) -> Result<(Self, usize), Error> {
        decode_device_object_property_reference(bytes, offset)
    }

    /// The object identifier under context tag 0 always comes first.
    fn starts(tag: &Tag) -> bool {
        tag.is_context(0)
    }

    fn device(&self) -> Option<ObjectIdentifier> {
        self.device_identifier
    }
}

impl DeviceReference for BACnetDeviceObjectReference {
    fn encode(&self, buf: &mut BytesMut) {
        encode_device_object_reference(buf, self);
    }

    fn decode(bytes: &[u8], offset: usize) -> Result<(Self, usize), Error> {
        decode_device_object_reference(bytes, offset)
    }

    /// The optional Device member under context tag 0, or, without one, the
    /// object identifier under context tag 1.
    fn starts(tag: &Tag) -> bool {
        tag.is_context(0) || tag.is_context(1)
    }

    fn device(&self) -> Option<ObjectIdentifier> {
        self.device_identifier
    }
}

// ---------------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------------

/// One reference as a read serves it.
pub(crate) fn reference_value<R: DeviceReference>(reference: &R) -> PropertyValue {
    let mut encoded = BytesMut::new();
    reference.encode(&mut encoded);
    PropertyValue::ApplicationData(encoded.to_vec())
}

/// The elements of a BACnetARRAY of references, one value each, for
/// `common::read_array`.
pub(crate) fn reference_elements<R: DeviceReference>(references: &[R]) -> Vec<PropertyValue> {
    references.iter().map(reference_value).collect()
}

/// A BACnetLIST of references, one value per element.
pub(crate) fn reference_list<R: DeviceReference>(references: &[R]) -> PropertyValue {
    PropertyValue::List(reference_elements(references))
}

/// A BACnetLIST of references as one run of octets, the elements back to
/// back: the form the server's AddListElement and RemoveListElement edit for
/// a Schedule's List_Of_Object_Property_References.
pub(crate) fn reference_run<R: DeviceReference>(references: &[R]) -> PropertyValue {
    let mut encoded = BytesMut::new();
    for reference in references {
        reference.encode(&mut encoded);
    }
    PropertyValue::ApplicationData(encoded.to_vec())
}

/// `object_type` at the reserved instance 4194303, the identifier Clause 12.1
/// lets a property hold to mean uninitialized or unused.
pub(crate) fn unset_identifier(object_type: ObjectType) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, ObjectIdentifier::WILDCARD_INSTANCE)
        .expect("a standard object type at the reserved instance is a valid identifier")
}

/// The standard form of a reference that names nothing (#1417): the
/// Present_Value of `object_type` at the reserved instance, in this device.
///
/// A reference production has no empty encoding, so this is what a reference
/// property serves while unset, what a client writes to clear one, and what
/// an array element gets when an index-0 write grows the array: a Trend Log
/// Multiple's Log_DeviceObjectProperty (Clause 12.30.11) or an Access Rights
/// rule's time range (Clauses 12.34.9.3 and 12.34.10.1). Each property picks
/// the object type its references usually name; the reserved instance alone
/// marks the reference unset
/// ([`BACnetDeviceObjectPropertyReference::is_unset`]).
pub(crate) fn unset_reference(object_type: ObjectType) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference::new_local(
        unset_identifier(object_type),
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )
}

/// A reference property that holds at most one reference, as a read serves
/// it: `reference`, or the unset form naming `unset_type` without one.
pub(crate) fn optional_reference_value(
    reference: Option<&BACnetDeviceObjectPropertyReference>,
    unset_type: ObjectType,
) -> PropertyValue {
    match reference {
        Some(reference) => reference_value(reference),
        None => reference_value(&unset_reference(unset_type)),
    }
}

/// `reference`, or `None` when it is the unset form, which a reference
/// property stores as no reference at all.
pub(crate) fn set_or_unset(
    reference: BACnetDeviceObjectPropertyReference,
) -> Option<BACnetDeviceObjectPropertyReference> {
    (!reference.is_unset()).then_some(reference)
}

/// A reference to a property of an object in this device, in the
/// device-qualified form with no Device member.
pub(crate) fn local_property_reference(
    reference: &BACnetObjectPropertyReference,
) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference {
        object_identifier: reference.object_identifier,
        property_identifier: reference.property_identifier,
        property_array_index: reference.property_array_index,
        device_identifier: None,
    }
}

// ---------------------------------------------------------------------------
// Writes
// ---------------------------------------------------------------------------

/// Every reference in `value` in order, or the refusal with the zero-based
/// position of the element it names (none for a refusal of the whole value).
fn decode_all<R: DeviceReference>(value: &PropertyValue) -> Result<Vec<R>, (Option<usize>, Error)> {
    let mut references = Vec::new();
    for bytes in common::chunks(value).map_err(|error| (None, error))? {
        let mut offset = 0;
        while offset < bytes.len() {
            let (reference, end) = common::decode_element(bytes, offset, R::starts, R::decode)
                .map_err(|error| (Some(references.len()), error))?;
            references.push(reference);
            offset = end;
        }
    }
    Ok(references)
}

/// Every reference in a written list or array value, in order.
///
/// The value is raw member octets (what WriteProperty carried), or a list of
/// such chunks (the shape a read returns); each chunk holds whole references
/// back to back. A chunk whose next element can't open the production (see
/// [`DeviceReference::starts`]), or a value of any other kind, is PROPERTY /
/// INVALID_DATA_TYPE. One that opens right but doesn't decode in full is
/// PROPERTY / INVALID_DATA_ENCODING. The Device member isn't checked here:
/// see [`check_device_members`]. A single-reference value goes through
/// [`decode_reference`] instead.
pub(crate) fn decode_references<R: DeviceReference>(
    value: &PropertyValue,
) -> Result<Vec<R>, Error> {
    decode_all(value).map_err(|(_, error)| error)
}

/// [`decode_references`] for a list-valued write whose refusal names the
/// element it refuses: an element that doesn't decode carries its position
/// (`common::at_list_element`), so AddListElement can report the request
/// element behind it.
pub(crate) fn decode_references_at<R: DeviceReference>(
    value: &PropertyValue,
) -> Result<Vec<R>, Error> {
    decode_all(value).map_err(|(element, error)| match element {
        Some(index) => common::at_list_element(error, index),
        None => error,
    })
}

/// The one reference a written single-reference value holds, decoded by
/// `common::decode_single_element`.
///
/// A list of chunks is joined first: a value read back is one chunk, and a
/// caller that split the octets at each member's tag hands over the same
/// octets in pieces. A value of another kind, or octets that don't open the
/// production, is PROPERTY / INVALID_DATA_TYPE. Octets that open it but
/// aren't exactly one whole reference (none, one cut short, or anything at
/// all after it) are PROPERTY / INVALID_DATA_ENCODING; see the module
/// documentation.
pub(crate) fn decode_reference<R: DeviceReference>(value: &PropertyValue) -> Result<R, Error> {
    common::decode_single_element(value, R::starts, R::decode)
}

/// Refuse a Device member that isn't a Device object identifier with
/// PROPERTY / VALUE_OUT_OF_RANGE: the member names the device holding the
/// object, so any other object type can't be honoured. Instance 4194303 is
/// no exception: a Device at that instance passes, and another object type
/// there is refused like any other.
///
/// The rule is bacnet-types' `device_identifier_is_device`, which the Python
/// bindings apply too. Every setter and network write path that stores one
/// of the references this module lists runs it before storing anything, so a
/// refused value leaves the property as it was; so do Command action lists
/// and the Access Rights rules.
pub(crate) fn check_device_member(device: Option<ObjectIdentifier>) -> Result<(), Error> {
    if bacnet_types::constructed::device_identifier_is_device(device) {
        Ok(())
    } else {
        Err(common::value_out_of_range_error())
    }
}

/// [`check_device_member`] on each reference, in order.
pub(crate) fn check_device_members<R: DeviceReference>(references: &[R]) -> Result<(), Error> {
    references
        .iter()
        .try_for_each(|reference| check_device_member(reference.device()))
}

/// Refuse a Device member on a property this stack holds to its own device:
/// [`check_device_member`]'s VALUE_OUT_OF_RANGE first, then PROPERTY /
/// OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED for any Device member at all.
///
/// The Averaging, Trend Log, Trend Log Multiple, Schedule and Staging clauses
/// let such a property stay within its device (12.5.13, 12.25.8, 12.30.11,
/// 12.24.10 and 12.62.14). None of these objects can tell which Device holds
/// it, so the bundled server rewrites a member naming its own Device in local
/// form before the object sees it (`local_references.rs` in bacnet-server).
pub(crate) fn check_local_member(device: Option<ObjectIdentifier>) -> Result<(), Error> {
    check_device_member(device)?;
    match device {
        None => Ok(()),
        Some(_) => Err(common::protocol_error(
            ErrorClass::PROPERTY,
            ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
        )),
    }
}

/// The local reference a written member names, after
/// [`check_local_member`].
pub(crate) fn into_local_property_reference(
    reference: BACnetDeviceObjectPropertyReference,
) -> Result<BACnetObjectPropertyReference, Error> {
    check_local_member(reference.device_identifier)?;
    Ok(BACnetObjectPropertyReference {
        object_identifier: reference.object_identifier,
        property_identifier: reference.property_identifier,
        property_array_index: reference.property_array_index,
    })
}

#[cfg(test)]
#[path = "device_reference_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "device_reference_users_tests.rs"]
mod users_tests;

#[cfg(test)]
#[path = "device_reference_setter_tests.rs"]
mod setter_tests;
