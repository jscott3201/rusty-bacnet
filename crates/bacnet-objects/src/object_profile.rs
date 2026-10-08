//! The optional Tags, Profile_Location and Profile_Name rows (#1553).
//!
//! Most Clause 12 object tables list these three as optional rows. An object
//! that carries an [`ObjectProfile`] serves the rows it provisions, before
//! registration, as an [`ObjectAuditPolicy`](crate::audit::ObjectAuditPolicy)
//! provisions the audit rows; an object that provisions none serves none,
//! exactly as before. The Color, Color Temperature, Lighting Output and
//! Binary Lighting Output objects carry one (`set_profile` on each).
//!
//! - Tags is a BACnetARRAY of BACnetNameValue that peers may write: whole, an
//!   element by index, or its size at index 0, which truncates or appends
//!   (Clause 12.1.5.1). An appended tag is a semantic tag with an empty
//!   name, a local choice the clause leaves open. A tag name may not hold a
//!   semicolon (Annex Y.1.4), and the array holds at most [`MAX_TAGS`].
//! - Profile_Location and Profile_Name are the application's: read-only over
//!   the network, as their O conformance code permits, so the PICS lists
//!   them as readable only. Provisioning checks each against its subclause:
//!   a location is empty or its URI scheme is http, https or bacnet, and a
//!   profile name begins with a decimal vendor identifier and a dash.
//!
//! Objects built with `new` keep Tags in memory. Their
//! `with_tags_persistence` constructors attach application-owned storage:
//! successful writes then override configured Tags across reconstruction.
//! Saved Tags never provision an absent row. See [`TagsPersistence`].

use bacnet_encoding::constructed::{decode_name_value, encode_name_value};
use bacnet_encoding::tags::Tag;
use bacnet_types::constructed::{BACnetNameValue, TagValue};
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;
use bytes::BytesMut;

use crate::common;
use crate::property_metadata::{
    PropertyConformance::Optional,
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly},
};

mod persistence;
mod saving;
pub use persistence::{
    FileTagsPersistence, TagsPersistence, TagsSnapshot, MAX_TAGS_SNAPSHOT_BYTES,
};
pub(crate) use saving::ProfileState;

/// Resource cap on Tags elements, the bound Exception_Schedule and a
/// Calendar's Date_List have. A longer array is NO_SPACE_TO_WRITE_PROPERTY.
pub const MAX_TAGS: usize = 1024;

/// The optional Tags, Profile_Location and Profile_Name rows of one object.
///
/// A field left `None` keeps its row out of the object; `Some` serves it,
/// an empty Tags array included. Provision it with the object's
/// `set_profile`, which [`check`](Self::check)s it first.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ObjectProfile {
    /// Tags: the object's tags, writable by peers.
    pub tags: Option<Vec<BACnetNameValue>>,
    /// Profile_Location: the URI of the file defining the profile.
    pub profile_location: Option<String>,
    /// Profile_Name: the profile the object conforms to.
    pub profile_name: Option<String>,
}

impl ObjectProfile {
    /// Check the rows as an object takes them. Too many tags is
    /// NO_SPACE_TO_WRITE_PROPERTY; a tag value that isn't a primitive is
    /// INVALID_DATA_TYPE; a tag name with a semicolon, a non-empty location
    /// with another URI scheme, or a profile name without its vendor prefix
    /// is VALUE_OUT_OF_RANGE.
    pub fn check(&self) -> Result<(), Error> {
        if let Some(tags) = &self.tags {
            check_tags(tags)?;
        }
        if self
            .profile_location
            .as_deref()
            .is_some_and(|location| !location_scheme_allowed(location))
        {
            return Err(common::value_out_of_range_error());
        }
        if self
            .profile_name
            .as_deref()
            .is_some_and(|name| !vendor_prefixed(name))
        {
            return Err(common::value_out_of_range_error());
        }
        Ok(())
    }

    /// The provisioned rows, in the tables' order: Tags, Profile_Location,
    /// Profile_Name.
    pub(crate) fn metadata(&self) -> impl Iterator<Item = PropertyMetadata> {
        [
            (P::TAGS, self.tags.is_some(), Always),
            (
                P::PROFILE_LOCATION,
                self.profile_location.is_some(),
                ReadOnly,
            ),
            (P::PROFILE_NAME, self.profile_name.is_some(), ReadOnly),
        ]
        .into_iter()
        .filter(|&(_, present, _)| present)
        .map(|(property, _, write)| PropertyMetadata::new(property, Optional, None, write))
    }

    /// Read one of the three rows; `None` for any other property.
    pub(crate) fn read(
        &self,
        property: P,
        index: Option<u32>,
    ) -> Option<Result<PropertyValue, Error>> {
        let text = |value: &Option<String>| match value {
            None => Err(common::unknown_property_error()),
            Some(_) if index.is_some() => Err(common::property_is_not_an_array_error()),
            Some(text) => Ok(PropertyValue::CharacterString(text.clone())),
        };
        Some(match property {
            P::TAGS => match &self.tags {
                None => Err(common::unknown_property_error()),
                Some(tags) => read_tags(tags, index),
            },
            P::PROFILE_LOCATION => text(&self.profile_location),
            P::PROFILE_NAME => text(&self.profile_name),
            _ => return None,
        })
    }

    /// Write one of the three rows; `None` for any other property. A
    /// refused write changes nothing.
    pub(crate) fn write(
        &mut self,
        property: P,
        index: Option<u32>,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        let read_only = |present: bool| match index {
            _ if !present => Err(common::unknown_property_error()),
            Some(_) => Err(common::property_is_not_an_array_error()),
            None => Err(common::write_access_denied_error()),
        };
        Some(match property {
            P::TAGS => match &mut self.tags {
                None => Err(common::unknown_property_error()),
                Some(tags) => write_tags(tags, index, value),
            },
            P::PROFILE_LOCATION => read_only(self.profile_location.is_some()),
            P::PROFILE_NAME => read_only(self.profile_name.is_some()),
            _ => return None,
        })
    }
}

/// One tag as the array element a read returns.
fn encoded(tag: &BACnetNameValue) -> Result<PropertyValue, Error> {
    let mut bytes = BytesMut::new();
    encode_name_value(&mut bytes, tag)?;
    Ok(PropertyValue::ApplicationData(bytes.to_vec()))
}

fn read_tags(tags: &[BACnetNameValue], index: Option<u32>) -> Result<PropertyValue, Error> {
    match index {
        None => Ok(PropertyValue::List(
            tags.iter().map(encoded).collect::<Result<_, _>>()?,
        )),
        Some(0) => Ok(PropertyValue::Unsigned(tags.len() as u64)),
        Some(index) => encoded(&tags[slot(index, tags.len())?]),
    }
}

/// The zero-based slot of a one-based array index below `len`, or
/// INVALID_ARRAY_INDEX.
fn slot(index: u32, len: usize) -> Result<usize, Error> {
    usize::try_from(index - 1)
        .ok()
        .filter(|slot| *slot < len)
        .ok_or_else(common::invalid_array_index_error)
}

/// A BACnetNameValue opens with its name under context tag 0.
fn starts_name_value(tag: &Tag) -> bool {
    tag.is_context(0)
}

fn no_space_error() -> Error {
    common::protocol_error(ErrorClass::RESOURCES, ErrorCode::NO_SPACE_TO_WRITE_PROPERTY)
}

/// WriteProperty of Tags: the whole array, its size at index 0, or one
/// element. The index is checked before the value (Clause 12.1.5.1: an index
/// past the end doesn't grow the array).
fn write_tags(
    tags: &mut Vec<BACnetNameValue>,
    index: Option<u32>,
    value: &PropertyValue,
) -> Result<(), Error> {
    match index {
        None => {
            let written = common::decode_elements(value, starts_name_value, decode_name_value)?;
            check_tags(&written)?;
            *tags = written;
        }
        Some(0) => {
            let PropertyValue::Unsigned(size) = value else {
                return Err(common::invalid_data_type_error());
            };
            let size = usize::try_from(*size)
                .ok()
                .filter(|size| *size <= MAX_TAGS)
                .ok_or_else(no_space_error)?;
            tags.resize_with(size, || BACnetNameValue::semantic(""));
        }
        Some(index) => {
            let slot = slot(index, tags.len())?;
            let tag = common::decode_single_element(value, starts_name_value, decode_name_value)?;
            check_tag(&tag)?;
            tags[slot] = tag;
        }
    }
    Ok(())
}

fn check_tags(tags: &[BACnetNameValue]) -> Result<(), Error> {
    if tags.len() > MAX_TAGS {
        return Err(no_space_error());
    }
    tags.iter().try_for_each(check_tag)
}

/// Annex Y.1.4 keeps semicolons out of tag names, so names can be joined.
fn check_tag(tag: &BACnetNameValue) -> Result<(), Error> {
    if let Some(TagValue::Primitive(value)) = &tag.value {
        if !value.is_primitive() {
            return Err(common::invalid_data_type_error());
        }
    }
    if tag.name.contains(';') {
        return Err(common::value_out_of_range_error());
    }
    Ok(())
}

/// Whether a Profile_Location is empty, which the Profile_Name subclause
/// allows for, or a URI using one of the schemes its own subclause allows.
fn location_scheme_allowed(location: &str) -> bool {
    location.is_empty()
        || location.split_once(':').is_some_and(|(scheme, _)| {
            ["http", "https", "bacnet"]
                .iter()
                .any(|allowed| scheme.eq_ignore_ascii_case(allowed))
        })
}

/// Whether a Profile_Name begins with a vendor identifier in decimal and a
/// dash.
fn vendor_prefixed(name: &str) -> bool {
    name.split_once('-').is_some_and(|(vendor, _)| {
        !vendor.is_empty()
            && vendor.bytes().all(|b| b.is_ascii_digit())
            && vendor.parse::<u16>().is_ok()
    })
}

#[cfg(test)]
#[path = "object_profile_tests.rs"]
mod tests;

#[cfg(test)]
mod persistence_tests;
#[cfg(test)]
mod saving_tests;
#[cfg(test)]
mod test_support;
