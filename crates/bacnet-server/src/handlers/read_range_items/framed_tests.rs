//! Before/after table test for #2177: the framed-element choice ReadRange
//! made by (object type, property) before the metadata rows carried value
//! types, frozen, against the one the rows give now, for every list row of
//! every catalogue object. A list held framed is one of these rows; a
//! property without a row can't be read, so ReadRange never reaches it.

use super::*;
use bacnet_objects::network_port::NetworkPortObject;

#[test]
fn catalogue_list_rows_frame_their_elements_as_before() {
    let mut objects = bacnet_objects::catalogue::representatives();
    objects.push(Box::new(
        NetworkPortObject::new_bip(2, "NP-BIP", Default::default()).unwrap(),
    ));
    objects.push(Box::new(
        NetworkPortObject::new_bip6(3, "NP-BIP6", Default::default()).unwrap(),
    ));
    let mut differences = Vec::new();
    let mut lists = 0;
    for object in &objects {
        let object_type = object.object_identifier().object_type();
        for row in object.property_metadata().iter() {
            let property = row.property_identifier;
            let Some(value_type) = row.effective_value_type(object_type) else {
                continue;
            };
            if !value_type.is_list() {
                continue;
            }
            lists += 1;
            let new = FramedElement::for_datatype(value_type.datatype);
            let old = old_framed(object_type, property);
            if new != old {
                differences.push(format!("{object_type:?} {property:?}: {new:?} was {old:?}"));
            }
        }
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
    assert!(lists > 30, "only {lists} list rows");
}

/// The framed-element choice before #2177, frozen.
fn old_framed(object_type: ObjectType, property: PropertyIdentifier) -> Option<FramedElement> {
    match property {
        PropertyIdentifier::RECIPIENT_LIST
            if matches!(
                object_type,
                ObjectType::NOTIFICATION_CLASS | ObjectType::NOTIFICATION_FORWARDER
            ) =>
        {
            Some(FramedElement::Destination)
        }
        PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES
            if matches!(object_type, ObjectType::SCHEDULE | ObjectType::TIMER) =>
        {
            Some(FramedElement::DeviceObjectPropertyReference)
        }
        PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS if object_type == ObjectType::DEVICE => {
            Some(FramedElement::CovSubscription)
        }
        PropertyIdentifier::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS
            if object_type == ObjectType::DEVICE =>
        {
            Some(FramedElement::CovMultipleSubscription)
        }
        PropertyIdentifier::SUBSCRIBED_RECIPIENTS
            if object_type == ObjectType::NOTIFICATION_FORWARDER =>
        {
            Some(FramedElement::EventNotificationSubscription)
        }
        _ => None,
    }
}
