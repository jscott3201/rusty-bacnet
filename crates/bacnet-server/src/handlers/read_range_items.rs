//! The items of a BACnetLIST, as ReadRange counts and addresses them (#1025).
//!
//! Most objects hold a list as `PropertyValue::List`, one value per item;
//! Calendar's Date_List and Elevator Group's Landing_Calls hold one encoded
//! element per item. A few hold the whole list framed in one
//! `PropertyValue::ApplicationData`, the bytes ReadProperty returns. Those
//! split back into one encoded item per element, with the element codec the
//! property's datatype names, so By Position addresses elements and not the
//! frame. The Device's COV subscription lists arrive the same way, from the
//! running server's live projection (#1046).

use super::*;
use bacnet_encoding::constructed::{
    decode_cov_multiple_subscription, decode_cov_subscription, decode_destination,
    decode_device_object_property_reference, decode_event_notification_subscription,
};

/// The Clause 21 element of a list held framed, which marks where each
/// element ends.
#[derive(Clone, Copy)]
enum FramedElement {
    /// BACnetDestination: the Recipient_List of Notification Class and
    /// Notification Forwarder (Tables 12-24 and 12-58).
    Destination,
    /// BACnetDeviceObjectPropertyReference: the
    /// List_Of_Object_Property_References of Schedule and Timer (Tables 12-28
    /// and 12-75).
    DeviceObjectPropertyReference,
    /// BACnetCOVSubscription: the Device's Active_COV_Subscriptions (Table
    /// 12-13).
    CovSubscription,
    /// BACnetCOVMultipleSubscription: the Device's
    /// Active_COV_Multiple_Subscriptions (Table 12-13).
    CovMultipleSubscription,
    /// BACnetEventNotificationSubscription: the Notification Forwarder's
    /// Subscribed_Recipients (Table 12-58).
    EventNotificationSubscription,
}

impl FramedElement {
    fn for_datatype(object_type: ObjectType, property: PropertyIdentifier) -> Option<Self> {
        match property {
            PropertyIdentifier::RECIPIENT_LIST
                if matches!(
                    object_type,
                    ObjectType::NOTIFICATION_CLASS | ObjectType::NOTIFICATION_FORWARDER
                ) =>
            {
                Some(Self::Destination)
            }
            PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES
                if matches!(object_type, ObjectType::SCHEDULE | ObjectType::TIMER) =>
            {
                Some(Self::DeviceObjectPropertyReference)
            }
            PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS if object_type == ObjectType::DEVICE => {
                Some(Self::CovSubscription)
            }
            PropertyIdentifier::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS
                if object_type == ObjectType::DEVICE =>
            {
                Some(Self::CovMultipleSubscription)
            }
            PropertyIdentifier::SUBSCRIBED_RECIPIENTS
                if object_type == ObjectType::NOTIFICATION_FORWARDER =>
            {
                Some(Self::EventNotificationSubscription)
            }
            _ => None,
        }
    }

    /// The offset just past the element that starts at `offset`.
    fn end(self, data: &[u8], offset: usize) -> Result<usize, Error> {
        match self {
            Self::Destination => decode_destination(data, offset).map(|(_, end)| end),
            Self::DeviceObjectPropertyReference => {
                decode_device_object_property_reference(data, offset).map(|(_, end)| end)
            }
            Self::CovSubscription => decode_cov_subscription(data, offset).map(|(_, end)| end),
            Self::CovMultipleSubscription => {
                decode_cov_multiple_subscription(data, offset).map(|(_, end)| end)
            }
            Self::EventNotificationSubscription => {
                decode_event_notification_subscription(data, offset).map(|(_, end)| end)
            }
        }
    }
}

fn unsplittable() -> Error {
    Error::Protocol {
        class: ErrorClass::SERVICES,
        code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    }
}

/// The items of a property already known to be a BACnetLIST.
///
/// A list this server cannot split into elements is refused with SERVICES /
/// OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED: positional access into it is
/// functionality it lacks, while ReadProperty still returns the list whole.
/// The Clause 15.8 procedure lets a responder refuse a target it cannot serve
/// for a reason its error table does not name. That covers a framed vendor
/// list with no element codec here, a stored frame that does not decode, and
/// a value of any other shape.
pub(super) fn list_items(
    object_type: ObjectType,
    property: PropertyIdentifier,
    value: PropertyValue,
) -> Result<Vec<PropertyValue>, Error> {
    let bytes = match value {
        PropertyValue::List(items) => return Ok(items),
        PropertyValue::ApplicationData(bytes) => bytes,
        _ => return Err(unsplittable()),
    };
    let element = FramedElement::for_datatype(object_type, property).ok_or_else(unsplittable)?;
    let mut items = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let end = element.end(&bytes, offset).map_err(|_| unsplittable())?;
        items.push(PropertyValue::ApplicationData(bytes[offset..end].to_vec()));
        offset = end;
    }
    Ok(items)
}
