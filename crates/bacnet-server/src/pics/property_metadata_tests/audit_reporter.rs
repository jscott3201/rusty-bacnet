use super::*;
use crate::server::AuditReportersConfig;
use PropertyIdentifier as P;

fn expected_rows(monitored: bool) -> Vec<(PropertyIdentifier, bool, bool, bool)> {
    // Independent (identifier, readable, optional, writable) capability rows.
    let mut rows = vec![
        (P::OBJECT_IDENTIFIER, true, false, false),
        (P::OBJECT_NAME, true, false, false),
        (P::OBJECT_TYPE, true, false, false),
        (P::DESCRIPTION, true, true, true),
        (P::STATUS_FLAGS, true, false, false),
        (P::RELIABILITY, true, false, false),
        (P::EVENT_STATE, true, false, false),
        (P::AUDIT_LEVEL, true, false, false),
        (P::AUDIT_SOURCE_REPORTER, true, false, false),
        (P::AUDITABLE_OPERATIONS, true, false, false),
        (P::AUDIT_PRIORITY_FILTER, true, false, false),
        (P::ISSUE_CONFIRMED_NOTIFICATIONS, true, false, false),
        (P::PROPERTY_LIST, true, false, false),
    ];
    if monitored {
        rows.push((P::MONITORED_OBJECTS, true, true, false));
    }
    rows.sort_by_key(|row| row.0.to_raw());
    rows
}

fn exercise_instances(configured_instances: &[Option<u32>]) {
    let mut expected_documents = None;
    // Fresh databases exercise independent HashMap layouts in addition to both
    // insertion orders. Either instance may expose the optional property, and
    // the server's selected producer must not hide other objects' capabilities.
    for _ in 0..16 {
        for order in [[1, 2], [2, 1]] {
            for &configured in configured_instances {
                let mut db = ObjectDatabase::new();
                for instance in order {
                    let mut reporter =
                        AuditReporterObject::new(instance, format!("ar-{instance}")).unwrap();
                    if configured == Some(instance) {
                        reporter.set_monitored_objects(Some(vec![])).unwrap();
                    }
                    db.add(Box::new(reporter)).unwrap();
                }
                for selected in [1, 2] {
                    let config = ServerConfig {
                        audit_reporters: Some(AuditReportersConfig {
                            reporters: vec![ObjectIdentifier::new(
                                ObjectType::AUDIT_REPORTER,
                                selected,
                            )
                            .unwrap()],
                        }),
                        ..Default::default()
                    };
                    let pics = generate_pics(&db, &config, &PicsConfig::default());
                    assert_eq!(pics.supported_object_types.len(), 1);
                    let support = &pics.supported_object_types[0];
                    assert_eq!(support.object_type, ObjectType::AUDIT_REPORTER);
                    assert!(!support.createable);
                    assert!(support.deleteable);
                    assert_eq!(
                        support
                            .supported_properties
                            .iter()
                            .any(|row| row.property_id == P::MONITORED_OBJECTS),
                        configured.is_some()
                    );
                    let mut rows = support
                        .supported_properties
                        .iter()
                        .map(|row| {
                            (
                                row.property_id,
                                row.access.readable,
                                row.access.optional,
                                row.access.writable,
                            )
                        })
                        .collect::<Vec<_>>();
                    rows.sort_by_key(|row| row.0.to_raw());
                    assert_eq!(rows, expected_rows(configured.is_some()));
                    let documents = (pics.generate_text(), pics.generate_markdown());
                    if let Some(expected) = &expected_documents {
                        assert_eq!(&documents, expected);
                    } else {
                        expected_documents = Some(documents);
                    }
                }
            }
        }
    }
}

#[test]
fn pics_audit_reporter_mixed_instances_have_stable_union_capabilities() {
    exercise_instances(&[Some(1), Some(2)]);
}

#[test]
fn pics_audit_reporter_all_unconfigured_instances_omit_monitored_objects() {
    exercise_instances(&[None]);
}

#[test]
fn pics_audit_reporter_union_is_independent_of_instance_traversal() {
    use bacnet_objects::traits::BACnetObject;

    let first = AuditReporterObject::new(1, "ar-1").unwrap();
    let mut second = AuditReporterObject::new(2, "ar-2").unwrap();
    for configured in [false, true] {
        second
            .set_monitored_objects(configured.then(Vec::new))
            .unwrap();
        // Force both traversals rather than depending on which HashMap order a
        // particular test run happens to produce. Use the production union path.
        for objects in [
            [&first as &dyn BACnetObject, &second as &dyn BACnetObject],
            [&second as &dyn BACnetObject, &first as &dyn BACnetObject],
        ] {
            let rows = PicsGenerator::audit_reporter_property_support(&objects)
                .iter()
                .map(|row| {
                    (
                        row.property_id,
                        row.access.readable,
                        row.access.optional,
                        row.access.writable,
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(rows, expected_rows(configured));
        }
    }
}
