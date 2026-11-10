//! A peer-written Description, kept in the storage the object already has
//! (#2364): the write-configuration storage of a Timer, a Channel and an
//! intrinsic reporter, the snapshot formats of the three logs, a
//! Notification Class, a Notification Forwarder and an Access Rights
//! object, the Tags storage of a Device, a Lighting Output and a Binary
//! Lighting Output, and the output storage of a Color and a Color
//! Temperature object.
//!
//! Clause 12 says nothing about restarts, so keeping Description is this
//! stack's choice, as for the rest of that storage: a head end that labelled
//! an object once expects the label to stay. Every type with storage keeps
//! it; an object built without storage, and a type that has none, serves a
//! written Description until a restart.
//!
//! A write of Description is kept as the object's other kept writes are:
//! saved before it is answered, refused with DEVICE / OPERATIONAL_PROBLEM
//! when the save fails, and staged by the server with the request's other
//! kept writes, one save for them all. A NULL succeeds and changes nothing
//! (Clause 15.9), so it is neither saved nor staged. A text past
//! [`MAX_OCTETS`] is refused with RESOURCES / NO_SPACE_TO_WRITE_PROPERTY
//! before anything is saved, so every backend gives that answer.

use crate::prelude::*;

use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use crate::common;

/// The most octets of UTF-8 a kept Description may hold: the cap a kept
/// Event_Message_Texts_Config string has (#2331).
pub(crate) const MAX_OCTETS: usize = crate::event::options::MAX_KEPT_TEXT_OCTETS;

/// Whether a write of `value` to `property` is a NULL to Description, which
/// succeeds and changes nothing, so storage keeps nothing for it.
pub(crate) fn null_write(property: PropertyIdentifier, value: &PropertyValue) -> bool {
    property == PropertyIdentifier::DESCRIPTION && *value == PropertyValue::Null
}

/// The text a kept write of `value` to Description leaves, or the object's
/// refusal of it: INVALID_DATA_TYPE for anything but a CharacterString, and
/// RESOURCES / NO_SPACE_TO_WRITE_PROPERTY for one past [`MAX_OCTETS`].
pub(crate) fn written(value: &PropertyValue) -> Result<String, Error> {
    let PropertyValue::CharacterString(text) = value else {
        return Err(common::invalid_data_type_error());
    };
    if text.len() > MAX_OCTETS {
        return Err(common::protocol_error(
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
        ));
    }
    Ok(text.clone())
}

/// Refuse a saved Description no write leaves: one past [`MAX_OCTETS`].
pub(crate) fn check_saved(description: Option<&str>) -> Result<(), Error> {
    match description {
        Some(text) if text.len() > MAX_OCTETS => {
            Err(Error::Encoding("saved Description is past its cap".into()))
        }
        _ => Ok(()),
    }
}
