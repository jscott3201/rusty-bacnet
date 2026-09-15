use bacnet_objects::{
    analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject},
    audit::AuditReporterObject,
    binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject},
    device::DeviceObject,
    event_enrollment::{AlertEnrollmentObject, EventEnrollmentObject},
    multistate::{MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject},
    staging::{StagingConfig, StagingObject},
    value_types::TimeValueObject,
};
use bacnet_types::constructed::BACnetStageLimitValue;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

use super::*;

mod life_safety;
mod schedule;

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
    db.add(Box::new(DeviceObject::new(Default::default()).unwrap()))
        .unwrap();
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
        ObjectType::DEVICE,
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

#[test]
fn pics_analog_property_metadata_is_exact_for_each_configuration() {
    use bacnet_objects::traits::BACnetObject;
    use PropertyIdentifier as P;

    // Expected (identifier, optional, writable) rows, not generated from metadata.
    let base = [
        (P::OBJECT_IDENTIFIER, false, false),
        (P::OBJECT_NAME, false, true),
        (P::DESCRIPTION, true, true),
        (P::OBJECT_TYPE, false, false),
        (P::PRESENT_VALUE, false, true),
        (P::STATUS_FLAGS, false, false),
        (P::EVENT_STATE, false, false),
        (P::EVENT_DETECTION_ENABLE, true, true),
        (P::OUT_OF_SERVICE, false, true),
        (P::UNITS, false, false),
        (P::COV_INCREMENT, true, true),
        (P::HIGH_LIMIT, true, true),
        (P::LOW_LIMIT, true, true),
        (P::DEADBAND, true, true),
        (P::LIMIT_ENABLE, true, true),
        (P::EVENT_ENABLE, true, true),
        (P::NOTIFY_TYPE, true, true),
        (P::NOTIFICATION_CLASS, true, true),
        (P::TIME_DELAY, true, true),
        (P::TIME_DELAY_NORMAL, true, true),
        (P::RELIABILITY, true, true),
        (P::RELIABILITY_EVALUATION_INHIBIT, true, true),
        (P::ACKED_TRANSITIONS, true, false),
        (P::EVENT_TIME_STAMPS, true, false),
        (P::EVENT_MESSAGE_TEXTS, true, false),
        (P::PROPERTY_LIST, false, false),
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
            let kind = object.object_identifier().object_type();
            let mut expected = base.to_vec();
            if kind != ObjectType::ANALOG_INPUT {
                let optional = kind == ObjectType::ANALOG_VALUE;
                expected.splice(
                    10..10,
                    [
                        (P::PRIORITY_ARRAY, optional, true),
                        (P::RELINQUISH_DEFAULT, optional, true),
                        (P::CURRENT_COMMAND_PRIORITY, optional, false),
                    ],
                );
            }
            if configuration & 1 != 0 && kind != ObjectType::ANALOG_OUTPUT {
                expected.extend([
                    (P::FAULT_HIGH_LIMIT, true, false),
                    (P::FAULT_LOW_LIMIT, true, false),
                ]);
            }
            if configuration & 2 != 0 {
                expected.push((P::MIN_PRES_VALUE, true, false));
            }
            if configuration & 4 != 0 {
                expected.push((P::MAX_PRES_VALUE, true, false));
            }
            let required = object.required_properties();
            let mut db = ObjectDatabase::new();
            db.add(object).unwrap();
            let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
            let support = &pics.supported_object_types[0];
            assert_eq!(support.object_type, kind);
            if kind != ObjectType::ANALOG_INPUT {
                assert_eq!(support.createable, kind == ObjectType::ANALOG_OUTPUT);
            }
            let rows: Vec<_> = support
                .supported_properties
                .iter()
                .map(|row| {
                    assert!(row.access.readable);
                    (row.property_id, row.access.optional, row.access.writable)
                })
                .collect();
            assert_eq!(rows, expected, "configuration {configuration}");
            assert_eq!(
                rows.iter()
                    .filter_map(|&(p, optional, _)| (!optional).then_some(p))
                    .collect::<Vec<_>>(),
                required.as_ref()
            );
        }
    }
}

#[test]
fn pics_binary_commandable_property_metadata_is_exact() {
    use bacnet_objects::traits::BACnetObject;
    use bacnet_types::primitives::PropertyValue;
    use PropertyIdentifier as P;

    // Independent (identifier, optional, writable) fixture in legacy order.
    let base = [
        (P::OBJECT_IDENTIFIER, false, false),
        (P::OBJECT_NAME, false, true),
        (P::DESCRIPTION, true, true),
        (P::OBJECT_TYPE, false, false),
        (P::PRESENT_VALUE, false, true),
        (P::STATUS_FLAGS, false, false),
        (P::EVENT_STATE, false, false),
        (P::EVENT_DETECTION_ENABLE, true, true),
        (P::EVENT_ENABLE, true, true),
        (P::TIME_DELAY, true, true),
        (P::TIME_DELAY_NORMAL, true, true),
        (P::NOTIFY_TYPE, true, true),
        (P::NOTIFICATION_CLASS, true, true),
        (P::ACKED_TRANSITIONS, true, false),
        (P::EVENT_TIME_STAMPS, true, false),
        (P::EVENT_MESSAGE_TEXTS, true, false),
        (P::OUT_OF_SERVICE, false, true),
        (P::RELIABILITY, true, true),
        (P::RELIABILITY_EVALUATION_INHIBIT, true, true),
        (P::ACTIVE_TEXT, true, true),
        (P::INACTIVE_TEXT, true, true),
    ];
    for out_of_service in [false, true] {
        for detection_enabled in [false, true] {
            let objects: [Box<dyn BACnetObject>; 2] = [
                Box::new(BinaryValueObject::new(1, "BV-1").unwrap()),
                Box::new(BinaryOutputObject::new(1, "BO-1").unwrap()),
            ];
            for mut object in objects {
                let kind = object.object_identifier().object_type();
                for (p, enabled) in [
                    (P::OUT_OF_SERVICE, out_of_service),
                    (P::EVENT_DETECTION_ENABLE, detection_enabled),
                ] {
                    object
                        .write_property(p, None, PropertyValue::Boolean(enabled), None)
                        .unwrap();
                }
                let optional = kind == ObjectType::BINARY_VALUE;
                let mut expected = base.to_vec();
                expected.splice(
                    17..17,
                    [
                        (P::PRIORITY_ARRAY, optional, true),
                        (P::RELINQUISH_DEFAULT, optional, true),
                        (P::CURRENT_COMMAND_PRIORITY, optional, false),
                    ],
                );
                if optional {
                    expected.push((P::ALARM_VALUE, true, true));
                } else {
                    expected.insert(20, (P::POLARITY, false, false));
                    expected.insert(5, (P::FEEDBACK_VALUE, true, true));
                }
                expected.push((P::PROPERTY_LIST, false, false));
                let required = object.required_properties();
                let mut db = ObjectDatabase::new();
                db.add(object).unwrap();
                let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
                assert_eq!(pics.supported_object_types.len(), 1);
                let support = &pics.supported_object_types[0];
                assert_eq!(support.object_type, kind);
                assert!(support.createable);
                let rows: Vec<_> = support
                    .supported_properties
                    .iter()
                    .map(|row| {
                        assert!(row.access.readable);
                        (row.property_id, row.access.optional, row.access.writable)
                    })
                    .collect();
                assert_eq!(
                    rows, expected,
                    "{kind:?}, OOS={out_of_service}, detection={detection_enabled}"
                );
                assert_eq!(
                    rows.iter()
                        .filter_map(|&(p, optional, _)| (!optional).then_some(p))
                        .collect::<Vec<_>>(),
                    required.as_ref()
                );
            }
        }
    }
}

#[test]
fn pics_multistate_property_metadata_is_exact() {
    use bacnet_objects::traits::BACnetObject;
    use bacnet_types::primitives::PropertyValue;
    use PropertyIdentifier as P;

    let base = [
        (P::OBJECT_IDENTIFIER, false, false),
        (P::OBJECT_NAME, false, true),
        (P::DESCRIPTION, true, true),
        (P::OBJECT_TYPE, false, false),
        (P::PRESENT_VALUE, false, true),
        (P::STATUS_FLAGS, false, false),
        (P::EVENT_STATE, false, false),
        (P::EVENT_DETECTION_ENABLE, true, true),
        (P::EVENT_ENABLE, true, true),
        (P::TIME_DELAY, true, true),
        (P::TIME_DELAY_NORMAL, true, true),
        (P::NOTIFY_TYPE, true, true),
        (P::NOTIFICATION_CLASS, true, true),
        (P::ACKED_TRANSITIONS, true, false),
        (P::EVENT_TIME_STAMPS, true, false),
        (P::EVENT_MESSAGE_TEXTS, true, false),
        (P::OUT_OF_SERVICE, false, true),
        (P::NUMBER_OF_STATES, false, false),
        (P::RELIABILITY, true, true),
        (P::RELIABILITY_EVALUATION_INHIBIT, true, true),
        (P::STATE_TEXT, true, true),
    ];
    for out_of_service in [false, true] {
        for detection_enabled in [false, true] {
            let objects: [Box<dyn BACnetObject>; 3] = [
                Box::new(MultiStateInputObject::new(1, "MSI-1", 3).unwrap()),
                Box::new(MultiStateValueObject::new(1, "MSV-1", 3).unwrap()),
                Box::new(MultiStateOutputObject::new(1, "MSO-1", 3).unwrap()),
            ];
            for mut object in objects {
                let kind = object.object_identifier().object_type();
                for (p, enabled) in [
                    (P::OUT_OF_SERVICE, out_of_service),
                    (P::EVENT_DETECTION_ENABLE, detection_enabled),
                ] {
                    object
                        .write_property(p, None, PropertyValue::Boolean(enabled), None)
                        .unwrap();
                }
                let mut expected = base.to_vec();
                if kind != ObjectType::MULTI_STATE_INPUT {
                    let optional = kind == ObjectType::MULTI_STATE_VALUE;
                    expected.splice(
                        18..18,
                        [
                            (P::PRIORITY_ARRAY, optional, true),
                            (P::RELINQUISH_DEFAULT, optional, true),
                            (P::CURRENT_COMMAND_PRIORITY, optional, false),
                        ],
                    );
                }
                if kind == ObjectType::MULTI_STATE_OUTPUT {
                    expected.insert(5, (P::FEEDBACK_VALUE, true, true));
                } else {
                    expected.push((P::ALARM_VALUES, true, true));
                }
                if kind != ObjectType::MULTI_STATE_INPUT {
                    expected.extend([
                        (P::VALUE_SOURCE, true, false),
                        (P::LAST_COMMAND_TIME, true, false),
                    ]);
                }
                expected.push((P::PROPERTY_LIST, false, false));
                let required = object.required_properties();
                let mut db = ObjectDatabase::new();
                db.add(object).unwrap();
                let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
                assert_eq!(pics.supported_object_types.len(), 1);
                let support = &pics.supported_object_types[0];
                assert_eq!(support.object_type, kind);
                assert!(support.createable);
                let rows: Vec<_> = support
                    .supported_properties
                    .iter()
                    .map(|row| {
                        assert!(row.access.readable);
                        (row.property_id, row.access.optional, row.access.writable)
                    })
                    .collect();
                assert_eq!(
                    rows, expected,
                    "{kind:?}, OOS={out_of_service}, detection={detection_enabled}"
                );
                assert_eq!(
                    rows.iter()
                        .filter_map(|&(p, optional, _)| (!optional).then_some(p))
                        .collect::<Vec<_>>(),
                    required.as_ref()
                );
            }
        }
    }
}

#[test]
fn pics_loop_program_notification_class_property_metadata_is_exact() {
    use bacnet_objects::notification_class::NotificationClass;
    use bacnet_objects::{loop_obj::LoopObject, program::ProgramObject, traits::BACnetObject};
    use bacnet_types::primitives::PropertyValue;
    use PropertyIdentifier as P;

    let loop_rows = [
        (P::OBJECT_IDENTIFIER, false, false),
        (P::OBJECT_NAME, false, false),
        (P::DESCRIPTION, true, true),
        (P::OBJECT_TYPE, false, false),
        (P::PRESENT_VALUE, false, false),
        (P::SETPOINT, false, true),
        (P::PROPORTIONAL_CONSTANT, true, true),
        (P::INTEGRAL_CONSTANT, true, true),
        (P::DERIVATIVE_CONSTANT, true, true),
        (P::OUTPUT_UNITS, false, false),
        (P::UPDATE_INTERVAL, true, true),
        (P::STATUS_FLAGS, false, false),
        (P::EVENT_STATE, false, false),
        (P::RELIABILITY, true, true),
        (P::OUT_OF_SERVICE, false, true),
        (P::CONTROLLED_VARIABLE_REFERENCE, false, true),
        (P::MANIPULATED_VARIABLE_REFERENCE, false, true),
        (P::SETPOINT_REFERENCE, false, true),
        (P::PROPERTY_LIST, false, false),
    ];
    let program_rows = [
        (P::OBJECT_IDENTIFIER, false, false),
        (P::OBJECT_NAME, false, false),
        (P::DESCRIPTION, true, true),
        (P::OBJECT_TYPE, false, false),
        (P::PROGRAM_STATE, false, true),
        (P::PROGRAM_CHANGE, false, true),
        (P::REASON_FOR_HALT, true, false),
        (P::STATUS_FLAGS, false, false),
        (P::OUT_OF_SERVICE, false, true),
        (P::RELIABILITY, true, false),
        (P::PROPERTY_LIST, false, false),
    ];
    let notification_class_rows = [
        (P::OBJECT_IDENTIFIER, false, false),
        (P::OBJECT_NAME, false, false),
        (P::DESCRIPTION, true, true),
        (P::OBJECT_TYPE, false, false),
        (P::STATUS_FLAGS, true, false),
        (P::EVENT_STATE, true, false),
        (P::OUT_OF_SERVICE, true, true),
        (P::RELIABILITY, true, false),
        (P::NOTIFICATION_CLASS, false, true),
        (P::PRIORITY, false, false),
        (P::ACK_REQUIRED, false, false),
        (P::RECIPIENT_LIST, false, true),
        (P::PROPERTY_LIST, false, false),
    ];
    for out_of_service in [false, true] {
        let objects: [Box<dyn BACnetObject>; 3] = [
            Box::new(LoopObject::new(1, "LOOP-1", 62).unwrap()),
            Box::new(ProgramObject::new(1, "PRG-1").unwrap()),
            Box::new(NotificationClass::new(1, "NC-1").unwrap()),
        ];
        for mut object in objects {
            let kind = object.object_identifier().object_type();
            object
                .write_property(
                    P::OUT_OF_SERVICE,
                    None,
                    PropertyValue::Boolean(out_of_service),
                    None,
                )
                .unwrap();
            let required = object.required_properties();
            let mut db = ObjectDatabase::new();
            db.add(object).unwrap();
            let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
            assert_eq!(pics.supported_object_types.len(), 1);
            let support = &pics.supported_object_types[0];
            assert_eq!(support.object_type, kind);
            assert!(!support.createable);
            assert!(support.deleteable);
            let rows: Vec<_> = support
                .supported_properties
                .iter()
                .map(|row| {
                    assert!(row.access.readable);
                    (row.property_id, row.access.optional, row.access.writable)
                })
                .collect();
            let expected = if kind == ObjectType::LOOP {
                loop_rows.as_slice()
            } else if kind == ObjectType::PROGRAM {
                program_rows.as_slice()
            } else {
                notification_class_rows.as_slice()
            };
            assert_eq!(rows, expected, "{kind:?}, OOS={out_of_service}");
            assert_eq!(
                rows.iter()
                    .filter_map(|&(p, optional, _)| (!optional).then_some(p))
                    .collect::<Vec<_>>(),
                required.as_ref()
            );
        }
    }
}
