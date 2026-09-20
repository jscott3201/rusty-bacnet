use bacnet_types::bitstring::{AuditOperationFlags, BACnetPriorityFilter};
use bacnet_types::constructed::BACnetObjectSelector as Selector;
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
fn audit_reporter_configuration_hook_is_opt_in_and_atomic() {
    let operations = AuditOperationFlags::from_bits((1 << 1) | (1 << 63)).unwrap();
    let mut other = crate::binary::BinaryValueObject::new(1, "Other").unwrap();
    assert!(other.audit_reporter_internal().is_none());
    assert!(matches!(
        other.configure_audit_reporter_internal(AuditLevel::AUDIT_ALL, operations, true),
        Err(Error::Protocol { class, code })
            if class == ErrorClass::OBJECT.to_raw() as u32
                && code == ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32
    ));

    let mut object: Box<dyn BACnetObject> = Box::new(AuditReporterObject::new(1, "AR").unwrap());
    object
        .configure_audit_reporter_internal(AuditLevel::AUDIT_ALL, operations, true)
        .unwrap();
    let properties = [
        PropertyIdentifier::AUDIT_LEVEL,
        PropertyIdentifier::AUDITABLE_OPERATIONS,
        PropertyIdentifier::ISSUE_CONFIRMED_NOTIFICATIONS,
        PropertyIdentifier::AUDIT_PRIORITY_FILTER,
        PropertyIdentifier::RELIABILITY,
    ];
    let before: Vec<_> = properties
        .iter()
        .map(|&p| object.read_property(p, None).unwrap())
        .collect();
    assert!(
        object
            .configure_audit_reporter_internal(
                AuditLevel::DEFAULT,
                AuditOperationFlags::empty(),
                false,
            )
            .is_err()
    );
    for (&property, expected) in properties.iter().zip(before) {
        assert_eq!(object.read_property(property, None).unwrap(), expected);
    }
    let reporter = object.audit_reporter_internal().unwrap();
    assert_eq!(reporter.auditable_operations, operations);
    assert!(reporter.confirmed_internal());
    assert!(reporter.monitored_objects.is_none());
    assert_eq!(reporter.audit_priority_filter, BACnetPriorityFilter::all());
    object
        .configure_audit_reporter_internal(AuditLevel::NONE, AuditOperationFlags::empty(), false)
        .unwrap();
    let reporter = object.audit_reporter_internal().unwrap();
    assert_eq!(reporter.audit_level, AuditLevel::NONE);
    assert!(reporter.auditable_operations.is_empty());
    assert!(!reporter.confirmed_internal());
}

#[test]
fn audit_reporter_auditing_failure_filter_invalidates_pending_epoch() {
    let mut reporter = AuditReporterObject::new(1, "AR").unwrap();
    let status = reporter.status_internal();
    let mut flags = AuditOperationFlags::empty();
    flags.insert(AuditOperation::AUDITING_FAILURE);
    reporter.set_auditable_operations(flags);
    assert_eq!(status.auditing_failure_epoch(), None);
    reporter.set_audit_level(AuditLevel::AUDIT_CONFIG).unwrap();
    let first = status.auditing_failure_epoch().unwrap();
    reporter.set_monitored_objects(Some(vec![]));
    reporter.set_audit_priority_filter(BACnetPriorityFilter::empty());
    assert_eq!(status.auditing_failure_epoch(), Some(first));
    reporter.set_audit_level(AuditLevel::NONE).unwrap();
    assert_eq!(status.auditing_failure_epoch(), None);
    reporter.set_audit_level(AuditLevel::AUDIT_ALL).unwrap();
    let second = status.auditing_failure_epoch().unwrap();
    assert_ne!(first, second);
    reporter.set_auditable_operations(AuditOperationFlags::empty());
    assert_eq!(status.auditing_failure_epoch(), None);
    reporter.set_auditable_operations(flags);
    assert_ne!(status.auditing_failure_epoch().unwrap(), second);
}

#[test]
fn audit_reporter_monitored_objects_is_an_optional_array_even_when_absent() {
    let reporter = AuditReporterObject::new(1, "AR").unwrap();
    assert!(reporter.is_array_property(PropertyIdentifier::MONITORED_OBJECTS));
    for index in [None, Some(0), Some(1)] {
        assert!(matches!(
            reporter.read_property(PropertyIdentifier::MONITORED_OBJECTS, index),
            Err(Error::Protocol { class, code })
                if class == ErrorClass::PROPERTY.to_raw() as u32
                    && code == ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32
        ));
    }
    assert!(!reporter
        .property_list()
        .contains(&PropertyIdentifier::MONITORED_OBJECTS));
}

#[test]
fn audit_reporter_monitored_objects_local_configuration_and_removal_are_truthful() {
    let mut reporter = AuditReporterObject::new(1, "AR").unwrap();
    let required = reporter.required_properties().into_owned();
    let original = reporter.property_metadata().into_owned();
    let selected = ObjectIdentifier::new(ObjectType::BINARY_VALUE, 42).unwrap();
    let other = ObjectIdentifier::new(ObjectType::BINARY_VALUE, 43).unwrap();
    let input = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let selectors = vec![
        Selector::None,
        Selector::Object(selected),
        Selector::ObjectType(ObjectType::ANALOG_INPUT),
    ];
    let expected = vec![
        PropertyValue::Null,
        PropertyValue::ObjectIdentifier(selected),
        PropertyValue::Enumerated(0),
    ];
    reporter.set_monitored_objects(Some(selectors.clone()));
    assert_eq!(
        read(&reporter, PropertyIdentifier::MONITORED_OBJECTS),
        PropertyValue::List(expected.clone())
    );
    assert_eq!(
        reporter
            .read_property(PropertyIdentifier::MONITORED_OBJECTS, Some(0))
            .unwrap(),
        PropertyValue::Unsigned(3)
    );
    for (i, value) in expected.iter().enumerate() {
        assert_eq!(
            reporter
                .read_property(PropertyIdentifier::MONITORED_OBJECTS, Some(i as u32 + 1))
                .unwrap(),
            *value
        );
    }
    for index in [4, u32::MAX] {
        assert!(
            matches!(reporter.read_property(PropertyIdentifier::MONITORED_OBJECTS, Some(index)),
            Err(Error::Protocol { class, code }) if class == ErrorClass::PROPERTY.to_raw() as u32 && code == ErrorCode::INVALID_ARRAY_INDEX.to_raw() as u32)
        );
    }
    assert_eq!(reporter.required_properties().as_ref(), required);
    assert!(reporter
        .property_metadata()
        .iter()
        .any(
            |row| row.property_identifier == PropertyIdentifier::MONITORED_OBJECTS
                && row.conformance == PropertyConformance::Optional
                && row.write_capability == PropertyWriteCapability::ReadOnly
                && row.presence_condition.is_none()
        ));
    assert!(reporter.monitors_object_internal(selected));
    assert!(!reporter.monitors_object_internal(other));
    assert!(reporter.monitors_object_internal(input));
    // Independent Clause 21 application-tag vector. The existing primitive
    // codec owns wire framing, including the concatenated BACnetARRAY.
    let vector = [0x00, 0xc4, 0x01, 0x40, 0x00, 0x2a, 0x91, 0x00];
    let mut encoded = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(
        &mut encoded,
        &read(&reporter, PropertyIdentifier::MONITORED_OBJECTS),
    )
    .unwrap();
    assert_eq!(&encoded[..], vector);
    let mut offset = 0;
    for selector in selectors {
        let (value, next) =
            bacnet_encoding::primitives::decode_application_value(&vector, offset).unwrap();
        assert!(next > offset);
        assert_eq!(Selector::decode_property_value(&value).unwrap(), selector);
        offset = next;
    }
    assert_eq!(offset, vector.len());
    for selection in [vec![], vec![Selector::None, Selector::None]] {
        reporter.set_monitored_objects(Some(selection));
        assert!(!reporter.monitors_object_internal(selected));
        assert!(!reporter.monitors_object_internal(input));
        assert!(reporter
            .property_list()
            .contains(&PropertyIdentifier::MONITORED_OBJECTS));
        assert!(reporter.monitors_object_internal(reporter.object_identifier()));
    }
    reporter.set_monitored_objects(None);
    assert_eq!(reporter.property_metadata().as_ref(), original);
    assert!(reporter.monitors_object_internal(selected));
    assert!(reporter.monitors_object_internal(other));
    assert!(reporter.monitors_object_internal(input));
    assert!(
        matches!(reporter.read_property(PropertyIdentifier::MONITORED_OBJECTS, None),
        Err(Error::Protocol { code, .. }) if code == ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32)
    );
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

#[test]
fn audit_reporter_lifecycle_configuration_filters_do_not_use_priority() {
    let mut reporter = AuditReporterObject::new(1, "AR").unwrap();
    reporter.set_audit_priority_filter(BACnetPriorityFilter::from_bits(0));
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
            reporter.set_auditable_operations(flags);
            for operation in [AuditOperation::CREATE, AuditOperation::DELETE] {
                assert_eq!(
                    reporter.reports_lifecycle_internal(operation),
                    level != AuditLevel::NONE && selected == operation
                );
            }
            assert!(!reporter.reports_lifecycle_internal(AuditOperation::WRITE));
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
        reporter.set_monitored_objects(selectors);
        assert_eq!(reporter.monitors_unassigned_create_internal(kind), expected);
    }
}
