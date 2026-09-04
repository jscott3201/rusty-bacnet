use bacnet_objects::{
    audit::AuditReporterObject,
    binary::BinaryInputObject,
    event_enrollment::{AlertEnrollmentObject, EventEnrollmentObject},
    staging::{StagingConfig, StagingObject},
    value_types::TimeValueObject,
};
use bacnet_types::constructed::BACnetStageLimitValue;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

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
    let alert_source = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    db.add(Box::new(
        AlertEnrollmentObject::new(1, "ae-1", alert_source).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        StagingObject::new(
            1,
            "stg-1",
            StagingConfig {
                present_value: 0.0,
                min_present_value: -1.0,
                units: 62,
                priority_for_writing: 8,
                stages: vec![
                    BACnetStageLimitValue {
                        limit: 1.0,
                        values: vec![],
                        deadband: 0.0,
                    },
                    BACnetStageLimitValue {
                        limit: 2.0,
                        values: vec![],
                        deadband: 0.0,
                    },
                ],
                target_references: vec![],
                stage_names: Some(vec!["Low".into(), "High".into()]),
            },
        )
        .unwrap(),
    ))
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
        (
            ObjectType::STAGING,
            PropertyIdentifier::PRESENT_VALUE,
            false,
            true,
        ),
        (
            ObjectType::STAGING,
            PropertyIdentifier::PRESENT_STAGE,
            false,
            false,
        ),
        (ObjectType::STAGING, PropertyIdentifier::STAGES, false, true),
        (
            ObjectType::STAGING,
            PropertyIdentifier::STAGE_NAMES,
            true,
            true,
        ),
        (
            ObjectType::STAGING,
            PropertyIdentifier::TARGET_REFERENCES,
            false,
            true,
        ),
        (
            ObjectType::STAGING,
            PropertyIdentifier::RELIABILITY,
            false,
            true,
        ),
        (
            ObjectType::STAGING,
            PropertyIdentifier::PRIORITY_FOR_WRITING,
            false,
            true,
        ),
        (
            ObjectType::STAGING,
            PropertyIdentifier::MIN_PRES_VALUE,
            false,
            true,
        ),
        (
            ObjectType::STAGING,
            PropertyIdentifier::MAX_PRES_VALUE,
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
        ObjectType::STAGING,
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
    let alert_rows: Vec<_> = alert
        .supported_properties
        .iter()
        .map(|property| {
            (
                property.property_id,
                property.access.optional,
                property.access.writable,
            )
        })
        .collect();
    assert_eq!(
        alert_rows,
        vec![
            (PropertyIdentifier::OBJECT_IDENTIFIER, false, false),
            (PropertyIdentifier::OBJECT_NAME, false, false),
            (PropertyIdentifier::DESCRIPTION, true, true),
            (PropertyIdentifier::OBJECT_TYPE, false, false),
            (PropertyIdentifier::PRESENT_VALUE, false, false),
            (PropertyIdentifier::EVENT_STATE, false, false),
            (PropertyIdentifier::EVENT_DETECTION_ENABLE, false, true,),
            (PropertyIdentifier::NOTIFICATION_CLASS, false, true),
            (PropertyIdentifier::EVENT_ENABLE, false, true),
            (PropertyIdentifier::ACKED_TRANSITIONS, false, false),
            (PropertyIdentifier::NOTIFY_TYPE, false, true),
            (PropertyIdentifier::EVENT_TIME_STAMPS, false, false),
            (PropertyIdentifier::PROPERTY_LIST, false, false),
        ]
    );
}

#[test]
fn pics_audit_reporter_metadata_is_complete_and_exact() {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(AuditReporterObject::new(1, "ar-1").unwrap()))
        .unwrap();
    let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
    let reporter = pics
        .supported_object_types
        .iter()
        .find(|support| support.object_type == ObjectType::AUDIT_REPORTER)
        .expect("Audit Reporter support");
    let rows = reporter
        .supported_properties
        .iter()
        .map(|property| {
            (
                property.property_id,
                property.access.readable,
                property.access.optional,
                property.access.writable,
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        rows,
        vec![
            (PropertyIdentifier::OBJECT_IDENTIFIER, true, false, false),
            (PropertyIdentifier::OBJECT_NAME, true, false, false),
            (PropertyIdentifier::OBJECT_TYPE, true, false, false),
            (PropertyIdentifier::DESCRIPTION, true, true, true),
            (PropertyIdentifier::STATUS_FLAGS, true, false, false),
            (PropertyIdentifier::RELIABILITY, true, false, false),
            (PropertyIdentifier::EVENT_STATE, true, false, false),
            (PropertyIdentifier::AUDIT_LEVEL, true, false, false),
            (
                PropertyIdentifier::AUDIT_SOURCE_REPORTER,
                true,
                false,
                false,
            ),
            (PropertyIdentifier::AUDITABLE_OPERATIONS, true, false, false,),
            (
                PropertyIdentifier::AUDIT_PRIORITY_FILTER,
                true,
                false,
                false,
            ),
            (
                PropertyIdentifier::ISSUE_CONFIRMED_NOTIFICATIONS,
                true,
                false,
                false,
            ),
            (PropertyIdentifier::PROPERTY_LIST, true, false, false),
        ]
    );
}
