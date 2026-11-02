//! What a CreateObject initial value may set on a new Structured View that
//! WriteProperty can't change afterwards (#2208).
//!
//! Every property that gives a view its content (Node_Type, Node_Subtype,
//! Subordinate_List, Subordinate_Annotations) is read-only over the network,
//! and so is Object_Name, so a view a client creates takes them as initial
//! values or stays empty and named by default.
//!
//! Subordinate_List and Subordinate_Annotations keep one size (Clause
//! 12.29.7 and 12.29.8): giving either sets the other's size too. The
//! annotations of new members are empty, and a new member of the list is the
//! uninitialized reference, at the reserved instance, which a client
//! ignores. So with both given at one size, their order doesn't matter;
//! with sizes that differ, the later one sets the size.

use crate::prelude::*;
use bacnet_types::constructed::BACnetDeviceObjectReference;
use bacnet_types::enums::{NodeType, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::StructuredViewObject;
use crate::common;
use crate::device_reference;

/// The properties [`initialize`] sets.
pub(super) const CREATION_ONLY: &[PropertyIdentifier] = &[
    PropertyIdentifier::OBJECT_NAME,
    PropertyIdentifier::NODE_TYPE,
    PropertyIdentifier::NODE_SUBTYPE,
    PropertyIdentifier::SUBORDINATE_LIST,
    PropertyIdentifier::SUBORDINATE_ANNOTATIONS,
];

/// Apply one whole initial value of [`CREATION_ONLY`] to `view`.
///
/// Object_Name takes a non-empty CharacterString. Node_Type takes the
/// values Clause 12.29.5 names (UNKNOWN to ZONE), any other
/// VALUE_OUT_OF_RANGE. Subordinate_List takes the references, each
/// checked as [`add_subordinate`](StructuredViewObject::add_subordinate)
/// checks it. Subordinate_Annotations takes CharacterStrings. Another
/// datatype is INVALID_DATA_TYPE. On a refusal the view is unchanged.
pub(super) fn initialize(
    view: &mut StructuredViewObject,
    property: PropertyIdentifier,
    value: PropertyValue,
) -> Result<(), Error> {
    match property {
        PropertyIdentifier::OBJECT_NAME => common::initialize_object_name(&mut view.name, &value)?,
        PropertyIdentifier::NODE_TYPE => {
            let PropertyValue::Enumerated(raw) = value else {
                return Err(common::invalid_data_type_error());
            };
            if raw > NodeType::ZONE.to_raw() {
                return Err(common::value_out_of_range_error());
            }
            view.node_type = raw;
        }
        PropertyIdentifier::NODE_SUBTYPE => {
            let PropertyValue::CharacterString(subtype) = value else {
                return Err(common::invalid_data_type_error());
            };
            view.node_subtype = subtype;
        }
        PropertyIdentifier::SUBORDINATE_LIST => {
            let references: Vec<BACnetDeviceObjectReference> =
                device_reference::decode_references(&value)?;
            device_reference::check_device_members(&references)?;
            view.subordinate_annotations
                .resize(references.len(), String::new());
            view.subordinate_list = references;
        }
        PropertyIdentifier::SUBORDINATE_ANNOTATIONS => {
            let annotations = annotations(value)?;
            view.subordinate_list
                .resize_with(annotations.len(), uninitialized_member);
            view.subordinate_annotations = annotations;
        }
        _ => return Err(common::write_access_denied_error()),
    }
    Ok(())
}

/// The annotations a whole Subordinate_Annotations value holds: one string
/// decodes alone, several as a list.
fn annotations(value: PropertyValue) -> Result<Vec<String>, Error> {
    let elements = match value {
        PropertyValue::List(elements) => elements,
        single => vec![single],
    };
    elements
        .into_iter()
        .map(|element| match element {
            PropertyValue::CharacterString(text) => Ok(text),
            _ => Err(common::invalid_data_type_error()),
        })
        .collect()
}

/// The Subordinate_List element a grown array gets: no object, at the
/// reserved instance (Clause 12.29.7).
fn uninitialized_member() -> BACnetDeviceObjectReference {
    BACnetDeviceObjectReference {
        device_identifier: None,
        object_identifier: device_reference::unset_identifier(ObjectType::STRUCTURED_VIEW),
    }
}
