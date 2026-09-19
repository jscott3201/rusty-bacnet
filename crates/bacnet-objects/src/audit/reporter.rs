use bacnet_types::bitstring::{AuditOperationFlags, BACnetPriorityFilter};
use bacnet_types::enums::{
    AuditLevel, AuditOperation, ErrorClass, ErrorCode, EventState, ObjectType, PropertyIdentifier,
    Reliability,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use crate::property_metadata::{PropertyConformance, PropertyWriteCapability};
use crate::traits::BACnetObject;

use super::super::AuditReporterObject;

fn read(reporter: &AuditReporterObject, property: PropertyIdentifier) -> PropertyValue {
    reporter.read_property(property, None).unwrap()
}

fn assert_write_access_denied(error: Error) {
    assert!(matches!(
        error,
        Error::Protocol { class, code }
            if class == ErrorClass::PROPERTY.to_raw() as u32
                && code == ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32
    ));
}

#[test]
fn audit_reporter_constructor_has_exact_inert_required_property_defaults() {
    let reporter = AuditReporterObject::new(42, "AR-42").unwrap();
    let identifier = ObjectIdentifier::new(ObjectType::AUDIT_REPORTER, 42).unwrap();
    let expected = [
        (
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyValue::ObjectIdentifier(identifier),
        ),
        (
            PropertyIdentifier::OBJECT_NAME,
            PropertyValue::CharacterString("AR-42".into()),
        ),
        (
            PropertyIdentifier::OBJECT_TYPE,
            PropertyValue::Enumerated(ObjectType::AUDIT_REPORTER.to_raw()),
        ),
        (
            PropertyIdentifier::DESCRIPTION,
            PropertyValue::CharacterString(String::new()),
        ),
        (
            PropertyIdentifier::STATUS_FLAGS,
            PropertyValue::BitString {
                unused_bits: 4,
                data: vec![0],
            },
        ),
        (
            PropertyIdentifier::RELIABILITY,
            PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw()),
        ),
        (
            PropertyIdentifier::EVENT_STATE,
            PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
        ),
        (
            PropertyIdentifier::AUDIT_LEVEL,
            PropertyValue::Enumerated(AuditLevel::NONE.to_raw()),
        ),
        (
            PropertyIdentifier::AUDIT_SOURCE_REPORTER,
            PropertyValue::Boolean(false),
        ),
        (
            PropertyIdentifier::AUDITABLE_OPERATIONS,
            PropertyValue::BitString {
                unused_bits: 0,
                data: Vec::new(),
            },
        ),
        (
            PropertyIdentifier::AUDIT_PRIORITY_FILTER,
            PropertyValue::BitString {
                unused_bits: 0,
                data: vec![0xff, 0xff],
            },
        ),
        (
            PropertyIdentifier::ISSUE_CONFIRMED_NOTIFICATIONS,
            PropertyValue::Boolean(false),
        ),
    ];

    for (property, value) in expected {
        assert_eq!(read(&reporter, property), value, "property {property}");
    }
}

#[test]
fn audit_reporter_canonical_metadata_and_required_projection_are_complete() {
    use PropertyConformance::{Optional, RequiredRead};
    use PropertyWriteCapability::{Always, ReadOnly};

    let reporter = AuditReporterObject::new(1, "AR-1").unwrap();
    let metadata = reporter.property_metadata();
    assert_eq!(
        metadata
            .iter()
            .map(|row| {
                (
                    row.property_identifier,
                    row.conformance,
                    row.write_capability,
                )
            })
            .collect::<Vec<_>>(),
        vec![
            (
                PropertyIdentifier::OBJECT_IDENTIFIER,
                RequiredRead,
                ReadOnly
            ),
            (PropertyIdentifier::OBJECT_NAME, RequiredRead, ReadOnly),
            (PropertyIdentifier::OBJECT_TYPE, RequiredRead, ReadOnly),
            (PropertyIdentifier::DESCRIPTION, Optional, Always),
            (PropertyIdentifier::STATUS_FLAGS, RequiredRead, ReadOnly),
            (PropertyIdentifier::RELIABILITY, RequiredRead, ReadOnly),
            (PropertyIdentifier::EVENT_STATE, RequiredRead, ReadOnly),
            (PropertyIdentifier::AUDIT_LEVEL, RequiredRead, ReadOnly),
            (
                PropertyIdentifier::AUDIT_SOURCE_REPORTER,
                RequiredRead,
                ReadOnly,
            ),
            (
                PropertyIdentifier::AUDITABLE_OPERATIONS,
                RequiredRead,
                ReadOnly,
            ),
            (
                PropertyIdentifier::AUDIT_PRIORITY_FILTER,
                RequiredRead,
                ReadOnly,
            ),
            (
                PropertyIdentifier::ISSUE_CONFIRMED_NOTIFICATIONS,
                RequiredRead,
                ReadOnly,
            ),
            (PropertyIdentifier::PROPERTY_LIST, RequiredRead, ReadOnly),
        ]
    );
    assert!(metadata.iter().all(|row| row.presence_condition.is_none()));

    let required = reporter.required_properties();
    assert_eq!(
        required.as_ref(),
        [
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
            PropertyIdentifier::PROPERTY_LIST,
        ]
    );
    assert!(!required.contains(&PropertyIdentifier::DESCRIPTION));
}

#[test]
fn audit_reporter_property_list_preserves_bacnet_array_projection() {
    let reporter = AuditReporterObject::new(1, "AR-1").unwrap();
    let supported = [
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
    ];
    assert_eq!(reporter.property_list().as_ref(), supported);

    let projected = supported
        .into_iter()
        .filter(|property| {
            ![
                PropertyIdentifier::OBJECT_IDENTIFIER,
                PropertyIdentifier::OBJECT_NAME,
                PropertyIdentifier::OBJECT_TYPE,
            ]
            .contains(property)
        })
        .map(|property| PropertyValue::Enumerated(property.to_raw()))
        .collect::<Vec<_>>();
    assert_eq!(
        reporter
            .read_property(PropertyIdentifier::PROPERTY_LIST, None)
            .unwrap(),
        PropertyValue::List(projected.clone())
    );
    assert_eq!(
        reporter
            .read_property(PropertyIdentifier::PROPERTY_LIST, Some(0))
            .unwrap(),
        PropertyValue::Unsigned(projected.len() as u64)
    );
    for (index, value) in projected.into_iter().enumerate() {
        assert_eq!(
            reporter
                .read_property(PropertyIdentifier::PROPERTY_LIST, Some(index as u32 + 1),)
                .unwrap(),
            value
        );
    }

    let error = reporter
        .read_property(PropertyIdentifier::PROPERTY_LIST, Some(10))
        .unwrap_err();
    assert!(matches!(
        error,
        Error::Protocol { class, code }
            if class == ErrorClass::PROPERTY.to_raw() as u32
                && code == ErrorCode::INVALID_ARRAY_INDEX.to_raw() as u32
    ));
}

#[test]
fn audit_reporter_local_setters_round_trip_extensible_values() {
    let mut reporter = AuditReporterObject::new(1, "AR-1").unwrap();
    let proprietary_level = AuditLevel::from_raw(128);
    reporter.set_audit_level(proprietary_level).unwrap();

    let mut operations = AuditOperationFlags::empty();
    assert!(operations.insert(AuditOperation::WRITE));
    assert!(operations.insert(AuditOperation::GENERAL));
    reporter.set_auditable_operations(operations);

    let mut priorities = BACnetPriorityFilter::empty();
    priorities.set(1, true).unwrap();
    priorities.set(16, true).unwrap();
    reporter.set_audit_priority_filter(priorities);
    reporter.set_issue_confirmed_notifications(true);

    assert_eq!(
        read(&reporter, PropertyIdentifier::AUDIT_LEVEL),
        PropertyValue::Enumerated(proprietary_level.to_raw())
    );
    let (unused_bits, data) = operations.to_bacnet();
    assert_eq!(
        read(&reporter, PropertyIdentifier::AUDITABLE_OPERATIONS),
        PropertyValue::BitString { unused_bits, data }
    );
    let (unused_bits, data) = priorities.to_bacnet();
    assert_eq!(
        read(&reporter, PropertyIdentifier::AUDIT_PRIORITY_FILTER),
        PropertyValue::BitString { unused_bits, data }
    );
    assert_eq!(
        read(&reporter, PropertyIdentifier::ISSUE_CONFIRMED_NOTIFICATIONS,),
        PropertyValue::Boolean(true)
    );
}

#[test]
fn audit_reporter_rejects_default_level_before_mutation() {
    let mut reporter = AuditReporterObject::new(1, "AR-1").unwrap();
    reporter.set_audit_level(AuditLevel::AUDIT_CONFIG).unwrap();
    assert!(matches!(
        reporter.set_audit_level(AuditLevel::DEFAULT),
        Err(Error::OutOfRange(_))
    ));
    assert_eq!(
        read(&reporter, PropertyIdentifier::AUDIT_LEVEL),
        PropertyValue::Enumerated(AuditLevel::AUDIT_CONFIG.to_raw())
    );
}

#[test]
fn audit_reporter_enabled_without_destination_exposes_configuration_fault() {
    let mut reporter = AuditReporterObject::new(1, "AR-1").unwrap();
    reporter.set_audit_level(AuditLevel::AUDIT_ALL).unwrap();
    assert_eq!(
        read(&reporter, PropertyIdentifier::RELIABILITY),
        PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw())
    );
    assert_eq!(
        read(&reporter, PropertyIdentifier::STATUS_FLAGS),
        PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0x40]
        }
    );
    reporter.set_audit_level(AuditLevel::NONE).unwrap();
    assert_eq!(
        read(&reporter, PropertyIdentifier::RELIABILITY),
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
    );
}

#[test]
fn audit_reporter_network_writes_to_new_properties_are_denied_without_mutation() {
    let mut reporter = AuditReporterObject::new(1, "AR-1").unwrap();
    reporter.set_audit_level(AuditLevel::AUDIT_ALL).unwrap();
    reporter.set_issue_confirmed_notifications(true);

    let writes = [
        (
            PropertyIdentifier::RELIABILITY,
            PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw()),
        ),
        (
            PropertyIdentifier::AUDIT_LEVEL,
            PropertyValue::Enumerated(AuditLevel::NONE.to_raw()),
        ),
        (
            PropertyIdentifier::AUDIT_SOURCE_REPORTER,
            PropertyValue::Boolean(true),
        ),
        (
            PropertyIdentifier::AUDITABLE_OPERATIONS,
            PropertyValue::BitString {
                unused_bits: 7,
                data: vec![0x80],
            },
        ),
        (
            PropertyIdentifier::AUDIT_PRIORITY_FILTER,
            PropertyValue::BitString {
                unused_bits: 0,
                data: vec![0, 0],
            },
        ),
        (
            PropertyIdentifier::ISSUE_CONFIRMED_NOTIFICATIONS,
            PropertyValue::Boolean(false),
        ),
    ];

    for (property, value) in writes {
        let before = read(&reporter, property);
        let error = reporter
            .write_property(property, None, value, None)
            .unwrap_err();
        assert_write_access_denied(error);
        assert_eq!(read(&reporter, property), before);
        assert!(!reporter.is_writable_property(property));
    }
}

#[test]
fn audit_reporter_write_filter_and_delivery_health() {
    let mut reporter = AuditReporterObject::new(1, "AR").unwrap();
    let pv = PropertyIdentifier::PRESENT_VALUE;
    assert!(!reporter.reports_write_internal(pv, None, false));
    reporter.set_audit_level(AuditLevel::AUDIT_ALL).unwrap();
    assert!(!reporter.reports_write_internal(pv, None, false));
    let mut operations = AuditOperationFlags::empty();
    operations.insert(AuditOperation::WRITE);
    reporter.set_auditable_operations(operations);
    reporter.set_audit_priority_filter(BACnetPriorityFilter::from_bits(0x8001));
    for priority in 0..=17 {
        assert_eq!(
            reporter.reports_write_internal(pv, Some(priority), false),
            matches!(priority, 1 | 16)
        );
    }
    assert!(reporter.reports_write_internal(pv, None, false));
    reporter.set_audit_level(AuditLevel::AUDIT_CONFIG).unwrap();
    assert!(!reporter.reports_write_internal(pv, None, false));
    assert!(reporter.reports_write_internal(PropertyIdentifier::DESCRIPTION, None, false));
    reporter.set_auditable_operations(AuditOperationFlags::empty());
    assert!(reporter.reports_write_internal(PropertyIdentifier::DESCRIPTION, None, true));

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
fn audit_reporter_description_write_and_metadata_remain_compatible() {
    let mut reporter = AuditReporterObject::new(1, "AR-1").unwrap();
    assert!(reporter.is_writable_property(PropertyIdentifier::DESCRIPTION));
    for property in reporter.property_list().iter().copied() {
        assert_eq!(
            reporter.is_writable_property(property),
            property == PropertyIdentifier::DESCRIPTION
        );
    }
    assert!(!reporter.is_writable_property(PropertyIdentifier::PROPERTY_LIST));

    reporter
        .write_property(
            PropertyIdentifier::DESCRIPTION,
            None,
            PropertyValue::CharacterString("network description".into()),
            None,
        )
        .unwrap();
    assert_eq!(
        read(&reporter, PropertyIdentifier::DESCRIPTION),
        PropertyValue::CharacterString("network description".into())
    );

    let error = reporter
        .write_property(
            PropertyIdentifier::DESCRIPTION,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap_err();
    assert!(matches!(
        error,
        Error::Protocol { class, code }
            if class == ErrorClass::PROPERTY.to_raw() as u32
                && code == ErrorCode::INVALID_DATA_TYPE.to_raw() as u32
    ));
    assert_eq!(
        read(&reporter, PropertyIdentifier::DESCRIPTION),
        PropertyValue::CharacterString("network description".into())
    );

    reporter.set_description("local description");
    assert_eq!(
        read(&reporter, PropertyIdentifier::DESCRIPTION),
        PropertyValue::CharacterString("local description".into())
    );
}
