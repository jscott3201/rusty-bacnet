//! What a CreateObject initial value may set on a new Event Enrollment that
//! WriteProperty can't change afterwards (#2208).
//!
//! Object_Property_Reference is read-only over the network, but an
//! enrollment monitors nothing without it, so a client that creates one
//! gives it as an initial value. Event_Parameters, Notification_Class and
//! the other configuration properties are writable and go the write route.

use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::EventEnrollmentObject;
use crate::common;

/// The properties [`initialize`] sets.
pub(super) const CREATION_ONLY: &[PropertyIdentifier] =
    &[PropertyIdentifier::OBJECT_PROPERTY_REFERENCE];

/// Apply an Object_Property_Reference initial value to `enrollment`: one
/// BACnetDeviceObjectPropertyReference, decoded and checked as
/// [`set_object_property_reference`](EventEnrollmentObject::set_object_property_reference)
/// checks it (a Device member that isn't a Device is VALUE_OUT_OF_RANGE),
/// the unset form leaving the enrollment without one. A NULL or another
/// datatype is INVALID_DATA_TYPE, and octets that aren't one whole reference
/// INVALID_DATA_ENCODING. On a refusal the enrollment is unchanged.
pub(super) fn initialize(
    enrollment: &mut EventEnrollmentObject,
    property: PropertyIdentifier,
    value: PropertyValue,
) -> Result<(), Error> {
    if property != PropertyIdentifier::OBJECT_PROPERTY_REFERENCE {
        return Err(common::write_access_denied_error());
    }
    let reference: BACnetDeviceObjectPropertyReference =
        crate::device_reference::decode_reference(&value)?;
    enrollment.set_object_property_reference(Some(reference))
}
