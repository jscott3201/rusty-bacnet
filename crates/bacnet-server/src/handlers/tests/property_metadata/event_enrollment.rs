use super::*;

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
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyIdentifier::EVENT_ENABLE,
            PropertyIdentifier::ACKED_TRANSITIONS,
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyIdentifier::EVENT_TIME_STAMPS,
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
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyIdentifier::EVENT_ENABLE,
            PropertyIdentifier::ACKED_TRANSITIONS,
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyIdentifier::EVENT_TIME_STAMPS,
        ]
    );
    assert_eq!(
        rpm_property_ids(&db, oid, PropertyIdentifier::OPTIONAL),
        vec![PropertyIdentifier::DESCRIPTION]
    );
}
