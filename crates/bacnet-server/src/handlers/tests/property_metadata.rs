use super::*;

mod access_identity;
mod access_topology;
mod accumulator;
mod audit_log;
mod averaging;
mod calendar;
mod color;
mod command;
mod device;
mod elevator;
mod event_enrollment;
mod file;
mod group;
mod life_safety;
mod lighting;
mod load_control;
mod log;
mod network_port;
mod notification_class;
mod schedule;
mod timer;

use bacnet_objects::audit::AuditReporterObject;
use bacnet_objects::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
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
    let alert_source = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    db.add(Box::new(
        AlertEnrollmentObject::new(1, "AE-1", alert_source).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(AuditReporterObject::new(1, "AR-1").unwrap()))
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

pub(super) fn assert_rpm_selector_bytes(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    selector: PropertyIdentifier,
    expected: &[PropertyIdentifier],
) {
    assert_eq!(rpm_property_ids(db, oid, selector), expected);
    let encode_request = |properties: &[PropertyIdentifier]| {
        let request = ReadPropertyMultipleRequest {
            list_of_read_access_specs: vec![ReadAccessSpecification {
                object_identifier: oid,
                list_of_property_references: properties
                    .iter()
                    .map(|&property_identifier| PropertyReference {
                        property_identifier,
                        property_array_index: None,
                    })
                    .collect(),
            }],
        };
        let mut bytes = BytesMut::new();
        request.encode(&mut bytes);
        bytes
    };
    let request_bytes = encode_request(&[selector]);
    let mut legacy = BytesMut::new();
    handle_read_property_multiple(db, &request_bytes, &mut legacy).unwrap();
    // Explicit reads independently check selector expansion and encoded values.
    let mut explicit_ack = BytesMut::new();
    handle_read_property_multiple(db, &encode_request(expected), &mut explicit_ack).unwrap();
    assert_eq!(legacy, explicit_ack);
    let mut bounded = BytesMut::new();
    let budget = crate::server::ReadPropertyMultipleBudget {
        max_result_elements: expected.len(),
        max_service_ack_bytes: legacy.len(),
    };
    super::super::rpm_budget::handle_rpm_budgeted(db, &request_bytes, &mut bounded, budget)
        .unwrap();
    assert_eq!(bounded, legacy, "{oid:?} {selector:?}");
    let mut prefix = BytesMut::from(&b"prefix"[..]);
    assert!(matches!(
        super::super::rpm_budget::handle_rpm_budgeted(
            db,
            &request_bytes,
            &mut prefix,
            crate::server::ReadPropertyMultipleBudget {
                max_result_elements: expected.len() - 1,
                ..budget
            }
        ),
        Err(super::super::rpm_budget::RpmFailure::Work)
    ));
    assert_eq!(&prefix[..], b"prefix");
    assert!(matches!(
        super::super::rpm_budget::handle_rpm_budgeted(
            db,
            &request_bytes,
            &mut prefix,
            crate::server::ReadPropertyMultipleBudget {
                max_service_ack_bytes: legacy.len() - 1,
                ..budget
            }
        ),
        Err(super::super::rpm_budget::RpmFailure::Bytes)
    ));
    assert_eq!(&prefix[..], b"prefix");
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
            PropertyIdentifier::RELIABILITY_EVALUATION_INHIBIT,
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
            PropertyIdentifier::RELIABILITY_EVALUATION_INHIBIT,
            PropertyIdentifier::ACTIVE_TEXT,
            PropertyIdentifier::INACTIVE_TEXT,
            PropertyIdentifier::ALARM_VALUE,
        ]
    );
}

#[test]
fn rpm_metadata_selectors_are_exact_for_audit_reporter() {
    let db = make_metadata_db();
    let oid = ObjectIdentifier::new(ObjectType::AUDIT_REPORTER, 1).unwrap();

    assert_eq!(
        rpm_property_ids(&db, oid, PropertyIdentifier::ALL),
        vec![
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::AUDIT_LEVEL,
            PropertyIdentifier::AUDIT_SOURCE_REPORTER,
            PropertyIdentifier::AUDITABLE_OPERATIONS,
            PropertyIdentifier::AUDIT_PRIORITY_FILTER,
            PropertyIdentifier::ISSUE_CONFIRMED_NOTIFICATIONS,
        ]
    );
    assert_eq!(
        rpm_property_ids(&db, oid, PropertyIdentifier::REQUIRED),
        vec![
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::AUDIT_LEVEL,
            PropertyIdentifier::AUDIT_SOURCE_REPORTER,
            PropertyIdentifier::AUDITABLE_OPERATIONS,
            PropertyIdentifier::AUDIT_PRIORITY_FILTER,
            PropertyIdentifier::ISSUE_CONFIRMED_NOTIFICATIONS,
        ]
    );
    assert_eq!(
        rpm_property_ids(&db, oid, PropertyIdentifier::OPTIONAL),
        vec![PropertyIdentifier::DESCRIPTION]
    );
}

#[test]
fn rpm_metadata_analog_required_optional_and_budgeted_bytes_agree() {
    use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
    use bacnet_objects::traits::BACnetObject;
    use PropertyIdentifier as P;

    let required = vec![
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::PRESENT_VALUE,
        P::STATUS_FLAGS,
        P::EVENT_STATE,
        P::OUT_OF_SERVICE,
        P::UNITS,
    ];
    let optional = vec![
        P::DESCRIPTION,
        P::EVENT_DETECTION_ENABLE,
        P::COV_INCREMENT,
        P::HIGH_LIMIT,
        P::LOW_LIMIT,
        P::DEADBAND,
        P::LIMIT_ENABLE,
        P::EVENT_ENABLE,
        P::NOTIFY_TYPE,
        P::NOTIFICATION_CLASS,
        P::TIME_DELAY,
        P::TIME_DELAY_NORMAL,
        P::RELIABILITY,
        P::RELIABILITY_EVALUATION_INHIBIT,
        P::ACKED_TRANSITIONS,
        P::EVENT_TIME_STAMPS,
        P::EVENT_MESSAGE_TEXTS,
    ];
    for configuration in 0..8 {
        let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
        let mut av = AnalogValueObject::new(1, "AV-1", 62).unwrap();
        let mut ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
        if configuration & 1 != 0 {
            ai.configure_fault_out_of_range(-10.0, 100.0).unwrap();
            av.configure_fault_out_of_range(-10.0, 100.0).unwrap();
        }
        macro_rules! bounds {
            ($object:ident) => {
                if configuration & 2 != 0 {
                    $object.set_min_pres_value(-20.0);
                }
                if configuration & 4 != 0 {
                    $object.set_max_pres_value(120.0);
                }
            };
        }
        bounds!(ai);
        bounds!(av);
        bounds!(ao);
        let objects: [Box<dyn BACnetObject>; 3] = [Box::new(ai), Box::new(av), Box::new(ao)];
        for object in objects {
            let oid = object.object_identifier();
            let kind = oid.object_type();
            let mut expected_required = required.clone();
            let mut expected_optional = optional.clone();
            let commandable = [
                P::PRIORITY_ARRAY,
                P::RELINQUISH_DEFAULT,
                P::CURRENT_COMMAND_PRIORITY,
            ];
            if kind == ObjectType::ANALOG_OUTPUT {
                expected_required.extend(commandable);
            } else if kind == ObjectType::ANALOG_VALUE {
                expected_optional.splice(2..2, commandable);
            }
            if configuration & 1 != 0 && kind != ObjectType::ANALOG_OUTPUT {
                expected_optional.extend([P::FAULT_HIGH_LIMIT, P::FAULT_LOW_LIMIT]);
            }
            if configuration & 2 != 0 {
                expected_optional.push(P::MIN_PRES_VALUE);
            }
            if configuration & 4 != 0 {
                expected_optional.push(P::MAX_PRES_VALUE);
            }
            let all = object.property_list().into_owned();
            let metadata = object.property_metadata().into_owned();
            let mut db = ObjectDatabase::new();
            db.add(object).unwrap();

            for (selector, expected) in [
                (P::REQUIRED, &expected_required),
                (P::OPTIONAL, &expected_optional),
                (P::ALL, &all),
            ] {
                assert_eq!(rpm_property_ids(&db, oid, selector), *expected);
                let projected: Vec<_> = metadata
                    .iter()
                    .filter_map(|row| {
                        let selected = row.property_identifier != P::PROPERTY_LIST
                            && match selector {
                                P::REQUIRED => row.conformance.is_required(),
                                P::OPTIONAL => !row.conformance.is_required(),
                                _ => true,
                            };
                        selected.then_some(row.property_identifier)
                    })
                    .collect();
                assert_eq!(projected, *expected);

                assert_rpm_selector_bytes(&db, oid, selector, expected);
            }
        }
    }
}

#[test]
fn rpm_metadata_binary_required_optional_and_budgeted_bytes_agree() {
    use bacnet_objects::traits::BACnetObject;
    use bacnet_types::primitives::PropertyValue;
    use PropertyIdentifier as P;

    let base = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::DESCRIPTION,
        P::OBJECT_TYPE,
        P::PRESENT_VALUE,
        P::STATUS_FLAGS,
        P::EVENT_STATE,
        P::EVENT_DETECTION_ENABLE,
        P::EVENT_ENABLE,
        P::TIME_DELAY,
        P::TIME_DELAY_NORMAL,
        P::NOTIFY_TYPE,
        P::NOTIFICATION_CLASS,
        P::ACKED_TRANSITIONS,
        P::EVENT_TIME_STAMPS,
        P::EVENT_MESSAGE_TEXTS,
        P::OUT_OF_SERVICE,
        P::PRIORITY_ARRAY,
        P::RELINQUISH_DEFAULT,
        P::CURRENT_COMMAND_PRIORITY,
        P::RELIABILITY,
        P::RELIABILITY_EVALUATION_INHIBIT,
        P::ACTIVE_TEXT,
        P::INACTIVE_TEXT,
    ];
    for out_of_service in [false, true] {
        for detection_enabled in [false, true] {
            let objects: [Box<dyn BACnetObject>; 2] = [
                Box::new(BinaryValueObject::new(1, "BV-1").unwrap()),
                Box::new(BinaryOutputObject::new(1, "BO-1").unwrap()),
            ];
            for mut object in objects {
                let oid = object.object_identifier();
                let mut all = base.to_vec();
                let mut required = vec![
                    P::OBJECT_IDENTIFIER,
                    P::OBJECT_NAME,
                    P::OBJECT_TYPE,
                    P::PRESENT_VALUE,
                    P::STATUS_FLAGS,
                    P::EVENT_STATE,
                    P::OUT_OF_SERVICE,
                ];
                if oid.object_type() == ObjectType::BINARY_OUTPUT {
                    all.insert(20, P::POLARITY);
                    all.insert(5, P::FEEDBACK_VALUE);
                    required.extend([
                        P::PRIORITY_ARRAY,
                        P::RELINQUISH_DEFAULT,
                        P::CURRENT_COMMAND_PRIORITY,
                        P::POLARITY,
                    ]);
                } else {
                    all.push(P::ALARM_VALUE);
                }
                let optional: Vec<_> = all
                    .iter()
                    .copied()
                    .filter(|p| !required.contains(p))
                    .collect();
                for (p, enabled) in [
                    (P::OUT_OF_SERVICE, out_of_service),
                    (P::EVENT_DETECTION_ENABLE, detection_enabled),
                ] {
                    object
                        .write_property(p, None, PropertyValue::Boolean(enabled), None)
                        .unwrap();
                }
                object
                    .write_property(
                        P::PRESENT_VALUE,
                        None,
                        PropertyValue::Enumerated(1),
                        Some(8),
                    )
                    .unwrap();
                let mut db = ObjectDatabase::new();
                db.add(object).unwrap();
                for (selector, expected) in [
                    (P::ALL, all.as_slice()),
                    (P::REQUIRED, required.as_slice()),
                    (P::OPTIONAL, optional.as_slice()),
                    (P::PROPERTY_LIST, &[P::PROPERTY_LIST]),
                ] {
                    assert_rpm_selector_bytes(&db, oid, selector, expected);
                }
            }
        }
    }
}

pub(super) fn multistate_objects() -> [Box<dyn bacnet_objects::traits::BACnetObject>; 3] {
    use bacnet_objects::multistate::{
        MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject,
    };
    [
        Box::new(MultiStateInputObject::new(1, "MSI-1", 3).unwrap()),
        Box::new(MultiStateValueObject::new(1, "MSV-1", 3).unwrap()),
        Box::new(MultiStateOutputObject::new(1, "MSO-1", 3).unwrap()),
    ]
}

#[test]
fn rpm_metadata_multistate_required_optional_and_budgeted_bytes_agree() {
    use bacnet_types::primitives::PropertyValue;
    use PropertyIdentifier as P;
    let base = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::DESCRIPTION,
        P::OBJECT_TYPE,
        P::PRESENT_VALUE,
        P::STATUS_FLAGS,
        P::EVENT_STATE,
        P::EVENT_DETECTION_ENABLE,
        P::EVENT_ENABLE,
        P::TIME_DELAY,
        P::TIME_DELAY_NORMAL,
        P::NOTIFY_TYPE,
        P::NOTIFICATION_CLASS,
        P::ACKED_TRANSITIONS,
        P::EVENT_TIME_STAMPS,
        P::EVENT_MESSAGE_TEXTS,
        P::OUT_OF_SERVICE,
        P::NUMBER_OF_STATES,
        P::RELIABILITY,
        P::RELIABILITY_EVALUATION_INHIBIT,
        P::STATE_TEXT,
    ];
    for enabled in [false, true] {
        for mut object in multistate_objects() {
            let oid = object.object_identifier();
            let kind = oid.object_type();
            let mut all = base.to_vec();
            let mut required = vec![
                P::OBJECT_IDENTIFIER,
                P::OBJECT_NAME,
                P::OBJECT_TYPE,
                P::PRESENT_VALUE,
                P::STATUS_FLAGS,
                P::EVENT_STATE,
                P::OUT_OF_SERVICE,
                P::NUMBER_OF_STATES,
            ];
            let commands = [
                P::PRIORITY_ARRAY,
                P::RELINQUISH_DEFAULT,
                P::CURRENT_COMMAND_PRIORITY,
            ];
            if kind != ObjectType::MULTI_STATE_INPUT {
                all.splice(18..18, commands);
            }
            if kind == ObjectType::MULTI_STATE_OUTPUT {
                all.insert(5, P::FEEDBACK_VALUE);
                required.extend(commands);
            } else {
                all.push(P::ALARM_VALUES);
                object
                    .write_property(
                        P::ALARM_VALUES,
                        None,
                        PropertyValue::List(vec![PropertyValue::Unsigned(2); 1024]),
                        None,
                    )
                    .unwrap();
            }
            if kind != ObjectType::MULTI_STATE_INPUT {
                all.extend([P::VALUE_SOURCE, P::LAST_COMMAND_TIME]);
            }
            let optional: Vec<_> = all
                .iter()
                .copied()
                .filter(|p| !required.contains(p))
                .collect();
            object
                .write_property(
                    P::EVENT_DETECTION_ENABLE,
                    None,
                    PropertyValue::Boolean(enabled),
                    None,
                )
                .unwrap();
            object
                .write_property(
                    P::STATE_TEXT,
                    Some(2),
                    PropertyValue::CharacterString("long label".repeat(100)),
                    None,
                )
                .unwrap();
            let mut db = ObjectDatabase::new();
            db.add(object).unwrap();
            for (selector, expected) in [
                (P::ALL, all.as_slice()),
                (P::REQUIRED, required.as_slice()),
                (P::OPTIONAL, optional.as_slice()),
                (P::PROPERTY_LIST, &[P::PROPERTY_LIST]),
            ] {
                assert_rpm_selector_bytes(&db, oid, selector, expected);
            }
        }
    }
}
