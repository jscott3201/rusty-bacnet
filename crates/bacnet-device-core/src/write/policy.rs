//! How the server treats a write to a property beyond what its datatype
//! says (#2177).
//!
//! A property's datatype and shape are on its metadata row
//! ([`bacnet_objects::property_metadata::ValueType`]). Two properties of one
//! datatype can still be written differently: a Schedule's
//! List_Of_Object_Property_References takes AddListElement members as
//! references and keeps them inside the device, while a Timer's list of the
//! same datatype does neither. That is this server's policy for the
//! standard object types, kept here by object type and property rather
//! than on the rows; no object overrides it.

use bacnet_types::enums::{ObjectType as T, PropertyIdentifier as P};

/// What the server does with writes to one property.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct WritePolicy {
    /// AddListElement and RemoveListElement read each element of this list
    /// as an encoding of its datatype (a BACnetDestination, for example),
    /// not as one application-tagged value.
    pub typed_list_elements: bool,
    /// The references this property holds stay inside the device, so a
    /// written member naming this device takes its local form, without the
    /// Device identifier (#1122, #1264).
    pub device_local_references: bool,
}

/// The server's write policy for `property` on an object of `object_type`.
pub const fn write_policy(object_type: T, property: P) -> WritePolicy {
    let typed_list_elements = matches!(
        (object_type, property),
        (
            T::NOTIFICATION_CLASS | T::NOTIFICATION_FORWARDER,
            P::RECIPIENT_LIST
        ) | (T::NOTIFICATION_FORWARDER, P::SUBSCRIBED_RECIPIENTS)
            | (T::CALENDAR, P::DATE_LIST)
            | (T::SCHEDULE, P::LIST_OF_OBJECT_PROPERTY_REFERENCES)
    );
    let device_local_references = matches!(
        (object_type, property),
        (
            T::SCHEDULE | T::CHANNEL,
            P::LIST_OF_OBJECT_PROPERTY_REFERENCES
        ) | (T::STAGING, P::TARGET_REFERENCES)
            | (T::AVERAGING, P::OBJECT_PROPERTY_REFERENCE)
            | (
                T::TREND_LOG | T::TREND_LOG_MULTIPLE,
                P::LOG_DEVICE_OBJECT_PROPERTY
            )
    );
    WritePolicy {
        typed_list_elements,
        device_local_references,
    }
}
