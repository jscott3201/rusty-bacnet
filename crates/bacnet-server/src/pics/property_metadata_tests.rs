use bacnet_objects::{
    binary::BinaryInputObject,
    event_enrollment::{AlertEnrollmentObject, EventEnrollmentObject},
    value_types::TimeValueObject,
};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use super::*;

fn property_support(
    pics: &Pics,
    object_type: ObjectType,
    property_id: PropertyIdentifier,
) -> &PropertySupport {
    pics.supported_object_types
        .iter()
        .find(|support| support.object_type == object_type)
        .and_then(|support| {
            support
                .supported_properties
                .iter()
                .find(|property| property.property_id == property_id)
        })
        .expect("property should be in the PICS list")
}

#[test]
fn pics_projects_migrated_property_metadata() {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(TimeValueObject::new(1, "tv-1").unwrap()))
        .unwrap();
    db.add(Box::new(BinaryInputObject::new(1, "bi-1").unwrap()))
        .unwrap();
    db.add(Box::new(EventEnrollmentObject::new(1, "ee-1", 0).unwrap()))
        .unwrap();
    db.add(Box::new(AlertEnrollmentObject::new(1, "ae-1").unwrap()))
        .unwrap();
    let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());

    for (object_type, property_id, optional, writable) in [
        (
            ObjectType::TIME_VALUE,
            PropertyIdentifier::PRESENT_VALUE,
            false,
            true,
        ),
        (
            ObjectType::TIME_VALUE,
            PropertyIdentifier::PRIORITY_ARRAY,
            true,
            true,
        ),
        (
            ObjectType::TIME_VALUE,
            PropertyIdentifier::STATUS_FLAGS,
            false,
            false,
        ),
        (
            ObjectType::BINARY_INPUT,
            PropertyIdentifier::EVENT_STATE,
            false,
            false,
        ),
        (
            ObjectType::BINARY_INPUT,
            PropertyIdentifier::PRESENT_VALUE,
            false,
            true,
        ),
        (
            ObjectType::BINARY_INPUT,
            PropertyIdentifier::RELIABILITY,
            true,
            true,
        ),
        (
            ObjectType::BINARY_INPUT,
            PropertyIdentifier::ACKED_TRANSITIONS,
            true,
            false,
        ),
        (
            ObjectType::EVENT_ENROLLMENT,
            PropertyIdentifier::EVENT_TIME_STAMPS,
            false,
            false,
        ),
        (
            ObjectType::ALERT_ENROLLMENT,
            PropertyIdentifier::EVENT_TIME_STAMPS,
            false,
            false,
        ),
    ] {
        let support = property_support(&pics, object_type, property_id);
        assert_eq!(
            support.access.optional, optional,
            "{object_type:?} {property_id:?}"
        );
        assert_eq!(
            support.access.writable, writable,
            "{object_type:?} {property_id:?}"
        );
    }

    for object_type in [
        ObjectType::TIME_VALUE,
        ObjectType::BINARY_INPUT,
        ObjectType::EVENT_ENROLLMENT,
        ObjectType::ALERT_ENROLLMENT,
    ] {
        let property_list = property_support(&pics, object_type, PropertyIdentifier::PROPERTY_LIST);
        assert!(!property_list.access.optional);
        assert!(!property_list.access.writable);
    }

    let alert = pics
        .supported_object_types
        .iter()
        .find(|support| support.object_type == ObjectType::ALERT_ENROLLMENT)
        .expect("Alert Enrollment support");
    assert!(alert
        .supported_properties
        .iter()
        .all(|property| property.property_id != PropertyIdentifier::NOTIFY_TYPE));
}
