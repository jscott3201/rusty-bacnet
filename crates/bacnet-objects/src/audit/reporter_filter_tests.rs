use super::*;
use bacnet_types::error::RemoteError;

#[test]
fn audit_reporter_write_filter_and_delivery_health() {
    let mut reporter = AuditReporterObject::new(1, "AR").unwrap();
    let pv = PropertyIdentifier::PRESENT_VALUE;
    assert!(!reporter.reports_write_internal(pv, None));
    reporter.set_audit_level(AuditLevel::AUDIT_ALL).unwrap();
    assert!(!reporter.reports_write_internal(pv, None));
    let mut operations = AuditOperationFlags::empty();
    operations.insert(AuditOperation::WRITE);
    reporter.set_auditable_operations(operations).unwrap();
    reporter
        .set_audit_priority_filter(BACnetPriorityFilter::from_bits(0x8001))
        .unwrap();
    for priority in 0..=17 {
        assert_eq!(
            reporter.reports_write_internal(pv, Some(priority)),
            matches!(priority, 1 | 16)
        );
    }
    assert!(reporter.reports_write_internal(pv, None));
    reporter.set_audit_level(AuditLevel::AUDIT_CONFIG).unwrap();
    assert!(!reporter.reports_write_internal(pv, None));
    assert!(reporter.reports_write_internal(PropertyIdentifier::DESCRIPTION, None));
    reporter
        .set_auditable_operations(AuditOperationFlags::empty())
        .unwrap();
    assert!(!reporter.reports_write_internal(PropertyIdentifier::DESCRIPTION, None));

    let status = reporter.status_internal();
    status.set_configured(true);
    let earlier = status.begin_delivery();
    status.complete_delivery(earlier, false);
    status.complete_delivery(earlier, true);
    assert_eq!(
        read(&reporter, PropertyIdentifier::RELIABILITY),
        PropertyValue::Enumerated(Reliability::COMMUNICATION_FAILURE.to_raw())
    );
    status.complete_delivery(status.begin_delivery(), true);
    assert_eq!(
        read(&reporter, PropertyIdentifier::RELIABILITY),
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
    );
    status.set_configured(false);
    assert_eq!(
        read(&reporter, PropertyIdentifier::RELIABILITY),
        PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw())
    );
}

#[test]
fn audit_reporter_description_null_is_noop_and_indices_fail_before_value() {
    let mut reporter = AuditReporterObject::new(1, "Reporter").unwrap();
    reporter.set_description("retained").unwrap();
    let before = reporter.configuration_internal();
    let status = reporter.status_internal();
    let generation = status.begin_delivery();
    for index in [0, 1] {
        for value in [
            PropertyValue::CharacterString("replacement".into()),
            PropertyValue::Null,
        ] {
            let error = reporter
                .write_property(PropertyIdentifier::DESCRIPTION, Some(index), value, None)
                .unwrap_err();
            assert!(
                matches!(error, Error::Remote(RemoteError::Error { class, code }) if class == ErrorClass::PROPERTY && code == ErrorCode::PROPERTY_IS_NOT_AN_ARRAY)
            );
            assert_eq!(reporter.configuration_internal(), before);
        }
    }
    reporter
        .write_property(
            PropertyIdentifier::DESCRIPTION,
            None,
            PropertyValue::Null,
            None,
        )
        .unwrap();
    assert_eq!(reporter.configuration_internal(), before);
    assert_eq!(status.begin_delivery(), generation);
    assert!(
        matches!(reporter.write_property(PropertyIdentifier::DESCRIPTION,None,PropertyValue::Boolean(false),None),Err(Error::Remote(RemoteError::Error { code,.. })) if code == ErrorCode::INVALID_DATA_TYPE)
    );
}

#[test]
fn audit_reporter_lifecycle_configuration_filters_do_not_use_priority() {
    let mut reporter = AuditReporterObject::new(1, "AR").unwrap();
    reporter
        .set_audit_priority_filter(BACnetPriorityFilter::from_bits(0))
        .unwrap();
    for level in [
        AuditLevel::NONE,
        AuditLevel::AUDIT_CONFIG,
        AuditLevel::AUDIT_ALL,
        AuditLevel::from_raw(64),
    ] {
        reporter.set_audit_level(level).unwrap();
        for selected in [
            AuditOperation::CREATE,
            AuditOperation::DELETE,
            AuditOperation::WRITE,
        ] {
            let mut flags = AuditOperationFlags::empty();
            flags.insert(selected);
            reporter.set_auditable_operations(flags).unwrap();
            for operation in [AuditOperation::CREATE, AuditOperation::DELETE] {
                assert_eq!(
                    crate::audit::ObjectAuditPolicy::default()
                        .effective_internal(&reporter.configuration_internal())
                        .reports(operation, None, None),
                    level != AuditLevel::NONE && selected == operation
                );
            }
        }
    }
}

#[test]
fn audit_reporter_unassigned_creation_selection_needs_type_or_catch_all() {
    let mut reporter = AuditReporterObject::new(1, "AR").unwrap();
    let kind = ObjectType::BINARY_VALUE;
    for (selectors, expected) in [
        (None, true),
        (Some(vec![]), false),
        (Some(vec![Selector::None]), false),
        (
            Some(vec![Selector::Object(
                ObjectIdentifier::new(kind, 1).unwrap(),
            )]),
            false,
        ),
        (
            Some(vec![Selector::ObjectType(ObjectType::ANALOG_INPUT)]),
            false,
        ),
        (Some(vec![Selector::None, Selector::ObjectType(kind)]), true),
    ] {
        reporter.set_monitored_objects(selectors).unwrap();
        assert_eq!(reporter.monitors_unassigned_create_internal(kind), expected);
    }
}
