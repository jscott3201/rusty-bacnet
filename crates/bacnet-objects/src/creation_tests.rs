//! The constructor table against the catalogue and the Device's list
//! (#2172).

use super::*;
use crate::catalogue::representatives;
use crate::device::SUPPORTED_OBJECT_TYPES;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

/// The types CreateObject builds, kept by hand so a type that leaves the
/// table fails here as well as in its creation test.
const CREATEABLE: [ObjectType; 27] = [
    ObjectType::ANALOG_INPUT,
    ObjectType::ANALOG_OUTPUT,
    ObjectType::ANALOG_VALUE,
    ObjectType::BINARY_INPUT,
    ObjectType::BINARY_OUTPUT,
    ObjectType::BINARY_VALUE,
    ObjectType::MULTI_STATE_INPUT,
    ObjectType::MULTI_STATE_OUTPUT,
    ObjectType::MULTI_STATE_VALUE,
    ObjectType::INTEGER_VALUE,
    ObjectType::POSITIVE_INTEGER_VALUE,
    ObjectType::LARGE_ANALOG_VALUE,
    ObjectType::CHARACTERSTRING_VALUE,
    ObjectType::OCTETSTRING_VALUE,
    ObjectType::BITSTRING_VALUE,
    ObjectType::DATE_VALUE,
    ObjectType::TIME_VALUE,
    ObjectType::DATETIME_VALUE,
    ObjectType::DATEPATTERN_VALUE,
    ObjectType::TIMEPATTERN_VALUE,
    ObjectType::DATETIMEPATTERN_VALUE,
    ObjectType::CALENDAR,
    ObjectType::SCHEDULE,
    ObjectType::NOTIFICATION_CLASS,
    ObjectType::TREND_LOG,
    ObjectType::EVENT_ENROLLMENT,
    ObjectType::STRUCTURED_VIEW,
];

#[test]
fn the_table_builds_exactly_the_createable_catalogue_types() {
    let catalogue = representatives();
    for object in &catalogue {
        let kind = object.object_identifier().object_type();
        assert_eq!(
            builtin_constructor(kind).is_some(),
            CREATEABLE.contains(&kind),
            "{kind:?}"
        );
    }
    // Every createable type is in the catalogue, so none escapes the check.
    for kind in CREATEABLE {
        assert!(
            catalogue
                .iter()
                .any(|object| object.object_identifier().object_type() == kind),
            "{kind:?}"
        );
    }
}

#[test]
fn the_table_lists_each_type_once_and_only_types_the_device_lists() {
    let types: Vec<ObjectType> = builtin_createable_types().collect();
    assert_eq!(types, CREATEABLE);
    for kind in types {
        assert!(SUPPORTED_OBJECT_TYPES.contains(&kind), "{kind:?}");
    }
    assert!(builtin_constructor(ObjectType::DEVICE).is_none());
    assert!(builtin_constructor(ObjectType::NETWORK_PORT).is_none());
    assert!(builtin_constructor(ObjectType::TREND_LOG_MULTIPLE).is_none());
}

#[test]
fn each_constructor_builds_its_own_type_at_the_instance_and_name_given() {
    for kind in CREATEABLE {
        let constructor = builtin_constructor(kind).expect("in the table");
        let object = constructor(7, "made".into(), &[]).unwrap();
        assert_eq!(
            object.object_identifier(),
            ObjectIdentifier::new(kind, 7).unwrap()
        );
        assert_eq!(object.object_name(), "made");
        // The reserved instance is the handler's to refuse; past it the
        // object constructor refuses.
        assert!(constructor(ObjectIdentifier::MAX_INSTANCE + 1, "x".into(), &[]).is_err());
    }
}

#[test]
fn analog_and_binary_values_serve_only_the_audit_rows_asked_for() {
    for kind in [ObjectType::ANALOG_VALUE, ObjectType::BINARY_VALUE] {
        let constructor = builtin_constructor(kind).unwrap();
        let plain = constructor(1, "plain".into(), &[]).unwrap();
        assert!(
            plain
                .read_property(PropertyIdentifier::AUDIT_LEVEL, None)
                .is_err()
        );
        let audited = constructor(
            1,
            "audited".into(),
            &[
                PropertyIdentifier::DESCRIPTION,
                PropertyIdentifier::AUDIT_LEVEL,
            ],
        )
        .unwrap();
        assert_eq!(
            audited
                .read_property(PropertyIdentifier::AUDIT_LEVEL, None)
                .unwrap(),
            PropertyValue::Enumerated(bacnet_types::enums::AuditLevel::DEFAULT.to_raw()),
            "{kind:?}"
        );
        assert!(
            audited
                .read_property(PropertyIdentifier::AUDITABLE_OPERATIONS, None)
                .is_err()
        );
    }
}
