use super::*;

use bacnet_objects::binary::BinaryInputObject;
use bacnet_objects::event_enrollment::{AlertEnrollmentObject, EventEnrollmentObject};
use bacnet_objects::value_types::TimeValueObject;
use bacnet_services::common::PropertyReference;
use bacnet_services::rpm::{ReadAccessSpecification, ReadPropertyMultipleACK};

fn make_metadata_db() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(TimeValueObject::new(1, "TV-1").unwrap()))
        .unwrap();
    db.add(Box::new(BinaryInputObject::new(1, "BI-1").unwrap()))
        .unwrap();
    db.add(Box::new(EventEnrollmentObject::new(1, "EE-1", 0).unwrap()))
        .unwrap();
    db.add(Box::new(AlertEnrollmentObject::new(1, "AE-1").unwrap()))
        .unwrap();
    db
}

fn rpm_property_ids(
    db: &ObjectDatabase,
    object_identifier: ObjectIdentifier,
    selector: PropertyIdentifier,
) -> Vec<PropertyIdentifier> {
    let request = ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier,
            list_of_property_references: vec![PropertyReference {
                property_identifier: selector,
                property_array_index: None,
            }],
        }],
    };
    let mut request_bytes = BytesMut::new();
    request.encode(&mut request_bytes);

    let mut response_bytes = BytesMut::new();
    handle_read_property_multiple(db, &request_bytes, &mut response_bytes).unwrap();
    let ack = ReadPropertyMultipleACK::decode(&response_bytes).unwrap();
    let results = &ack.list_of_read_access_results[0].list_of_results;
    for result in results {
        assert!(
            result.error.is_none(),
            "RPM {selector:?} returned an inline error for {:?}: {:?}",
            result.property_identifier,
            result.error
        );
        assert!(
            result.property_value.is_some(),
            "RPM {selector:?} omitted encoded value for {:?}",
            result.property_identifier
        );
    }
    results
        .iter()
        .map(|result| result.property_identifier)
        .collect()
}

#[test]
fn rpm_metadata_selectors_are_exact_for_time_value() {
    let db = make_metadata_db();
    let oid = ObjectIdentifier::new(ObjectType::TIME_VALUE, 1).unwrap();

    assert_eq!(
        rpm_property_ids(&db, oid, PropertyIdentifier::ALL),
        vec![
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::PRIORITY_ARRAY,
            PropertyIdentifier::RELINQUISH_DEFAULT,
        ]
    );
    assert_eq!(
        rpm_property_ids(&db, oid, PropertyIdentifier::REQUIRED),
        vec![
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::STATUS_FLAGS,
        ]
    );
    assert_eq!(
        rpm_property_ids(&db, oid, PropertyIdentifier::OPTIONAL),
        vec![
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::PRIORITY_ARRAY,
            PropertyIdentifier::RELINQUISH_DEFAULT,
        ]
    );
}

#[test]
fn rpm_metadata_selectors_are_exact_for_binary_input() {
    let db = make_metadata_db();
    let oid = ObjectIdentifier::new(ObjectType::BINARY_INPUT, 1).unwrap();

    assert_eq!(
        rpm_property_ids(&db, oid, PropertyIdentifier::ALL),
        vec![
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            PropertyIdentifier::EVENT_ENABLE,
            PropertyIdentifier::TIME_DELAY,
            PropertyIdentifier::TIME_DELAY_NORMAL,
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyIdentifier::ACKED_TRANSITIONS,
            PropertyIdentifier::EVENT_TIME_STAMPS,
            PropertyIdentifier::EVENT_MESSAGE_TEXTS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::POLARITY,
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::ACTIVE_TEXT,
            PropertyIdentifier::INACTIVE_TEXT,
            PropertyIdentifier::ALARM_VALUE,
        ]
    );
    assert_eq!(
        rpm_property_ids(&db, oid, PropertyIdentifier::REQUIRED),
        vec![
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::POLARITY,
        ]
    );
    assert_eq!(
        rpm_property_ids(&db, oid, PropertyIdentifier::OPTIONAL),
        vec![
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            PropertyIdentifier::EVENT_ENABLE,
            PropertyIdentifier::TIME_DELAY,
            PropertyIdentifier::TIME_DELAY_NORMAL,
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyIdentifier::ACKED_TRANSITIONS,
            PropertyIdentifier::EVENT_TIME_STAMPS,
            PropertyIdentifier::EVENT_MESSAGE_TEXTS,
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::ACTIVE_TEXT,
            PropertyIdentifier::INACTIVE_TEXT,
            PropertyIdentifier::ALARM_VALUE,
        ]
    );
}

#[test]
fn rpm_metadata_selectors_are_exact_for_event_enrollment() {
    let db = make_metadata_db();
    let oid = ObjectIdentifier::new(ObjectType::EVENT_ENROLLMENT, 1).unwrap();

    assert_eq!(
        rpm_property_ids(&db, oid, PropertyIdentifier::ALL),
        vec![
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::EVENT_TYPE,
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyIdentifier::EVENT_PARAMETERS,
            PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::EVENT_ENABLE,
            PropertyIdentifier::ACKED_TRANSITIONS,
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyIdentifier::EVENT_TIME_STAMPS,
            PropertyIdentifier::FAULT_TYPE,
            PropertyIdentifier::FAULT_PARAMETERS,
            PropertyIdentifier::TIME_DELAY_NORMAL,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ]
    );
    assert_eq!(
        rpm_property_ids(&db, oid, PropertyIdentifier::REQUIRED),
        vec![
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::EVENT_TYPE,
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyIdentifier::EVENT_PARAMETERS,
            PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::EVENT_ENABLE,
            PropertyIdentifier::ACKED_TRANSITIONS,
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyIdentifier::EVENT_TIME_STAMPS,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::RELIABILITY,
        ]
    );
    assert_eq!(
        rpm_property_ids(&db, oid, PropertyIdentifier::OPTIONAL),
        vec![
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::FAULT_TYPE,
            PropertyIdentifier::FAULT_PARAMETERS,
            PropertyIdentifier::TIME_DELAY_NORMAL,
            PropertyIdentifier::OUT_OF_SERVICE,
        ]
    );
}

#[test]
fn rpm_metadata_selectors_are_exact_for_alert_enrollment() {
    let db = make_metadata_db();
    let oid = ObjectIdentifier::new(ObjectType::ALERT_ENROLLMENT, 1).unwrap();

    assert_eq!(
        rpm_property_ids(&db, oid, PropertyIdentifier::ALL),
        vec![
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            PropertyIdentifier::EVENT_ENABLE,
            PropertyIdentifier::ACKED_TRANSITIONS,
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyIdentifier::EVENT_TIME_STAMPS,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ]
    );
    assert_eq!(
        rpm_property_ids(&db, oid, PropertyIdentifier::REQUIRED),
        vec![
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            PropertyIdentifier::EVENT_ENABLE,
            PropertyIdentifier::ACKED_TRANSITIONS,
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyIdentifier::EVENT_TIME_STAMPS,
        ]
    );
    assert_eq!(
        rpm_property_ids(&db, oid, PropertyIdentifier::OPTIONAL),
        vec![
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ]
    );

    for selector in [
        PropertyIdentifier::ALL,
        PropertyIdentifier::REQUIRED,
        PropertyIdentifier::OPTIONAL,
    ] {
        assert!(
            !rpm_property_ids(&db, oid, selector).contains(&PropertyIdentifier::NOTIFY_TYPE),
            "Alert Enrollment must not advertise its unimplemented Notify_Type"
        );
    }
}
