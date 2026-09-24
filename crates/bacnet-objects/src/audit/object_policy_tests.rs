use super::*;
use crate::audit::AuditReporterObject;
use crate::{analog::AnalogValueObject, binary::BinaryValueObject, traits::BACnetObject};
use bacnet_types::{enums::PropertyIdentifier as P, primitives::PropertyValue};

fn objects(policy: ObjectAuditPolicy) -> Vec<Box<dyn BACnetObject>> {
    let mut av = AnalogValueObject::new(1, "av", 62).unwrap();
    av.set_audit_policy(policy);
    let mut bv = BinaryValueObject::new(1, "bv").unwrap();
    bv.set_audit_policy(policy);
    vec![Box::new(av), Box::new(bv)]
}

#[test]
fn object_audit_policy_independent_presence_metadata_and_scalar_writes() {
    for selected in [
        P::AUDIT_LEVEL,
        P::AUDITABLE_OPERATIONS,
        P::AUDIT_PRIORITY_FILTER,
    ] {
        let policy = ObjectAuditPolicy {
            level: (selected == P::AUDIT_LEVEL).then_some(AuditLevel::DEFAULT),
            operations: (selected == P::AUDITABLE_OPERATIONS)
                .then_some(AuditOperationFlags::empty()),
            priority_filter: (selected == P::AUDIT_PRIORITY_FILTER)
                .then_some(AuditPriorityPolicy::Inherit),
        };
        for mut object in objects(policy) {
            for property in [
                P::AUDIT_LEVEL,
                P::AUDITABLE_OPERATIONS,
                P::AUDIT_PRIORITY_FILTER,
            ] {
                assert_eq!(
                    object.property_list().contains(&property),
                    property == selected
                );
                assert_eq!(
                    object.read_property(property, None).is_ok(),
                    property == selected
                );
                if property != selected {
                    assert!(object
                        .write_property(property, None, PropertyValue::Null, None)
                        .is_err());
                }
            }
            let row = *object
                .property_metadata()
                .iter()
                .find(|row| row.property_identifier == selected)
                .unwrap();
            assert!(!row.is_required());
            assert!(row.write_capability.is_writable());
            let before = object.read_property(selected, None).unwrap();
            assert!(object.read_property(selected, Some(0)).is_err());
            assert!(object
                .write_property(selected, Some(0), PropertyValue::Null, None)
                .is_err());
            for invalid in [PropertyValue::Boolean(true), PropertyValue::Unsigned(3)] {
                assert!(object
                    .write_property(selected, None, invalid, None)
                    .is_err());
                assert_eq!(object.read_property(selected, None).unwrap(), before);
            }
            object
                .write_property(selected, None, PropertyValue::Null, Some(16))
                .unwrap();
            assert_eq!(object.read_property(selected, None).unwrap(), before);
            assert!(object
                .write_property(selected, None, PropertyValue::Null, Some(0))
                .is_err());
        }
    }
}

#[test]
fn object_audit_policy_vectors_atomic_validation_and_null_priority() {
    let policy = ObjectAuditPolicy {
        level: Some(AuditLevel::DEFAULT),
        operations: Some(AuditOperationFlags::empty()),
        priority_filter: Some(AuditPriorityPolicy::Inherit),
    };
    for mut object in objects(policy) {
        let operations = PropertyValue::BitString {
            unused_bits: 6,
            data: vec![0xc0],
        }; // READ and WRITE
        object
            .write_property(P::AUDITABLE_OPERATIONS, None, operations.clone(), None)
            .unwrap();
        assert_eq!(
            object.read_property(P::AUDITABLE_OPERATIONS, None).unwrap(),
            operations
        );
        for invalid in [
            PropertyValue::BitString {
                unused_bits: 0,
                data: vec![0, 0, 0x80],
            },
            PropertyValue::BitString {
                unused_bits: 3,
                data: vec![1],
            },
        ] {
            assert!(object
                .write_property(P::AUDITABLE_OPERATIONS, None, invalid, None)
                .is_err());
            assert_eq!(
                object.read_property(P::AUDITABLE_OPERATIONS, None).unwrap(),
                operations
            );
        }
        let filter = PropertyValue::BitString {
            unused_bits: 0,
            data: vec![0x80, 1],
        };
        object
            .write_property(P::AUDIT_PRIORITY_FILTER, None, filter.clone(), None)
            .unwrap();
        assert_eq!(
            object
                .read_property(P::AUDIT_PRIORITY_FILTER, None)
                .unwrap(),
            filter
        );
        assert!(object
            .write_property(
                P::AUDIT_PRIORITY_FILTER,
                None,
                PropertyValue::BitString {
                    unused_bits: 0,
                    data: vec![0x80]
                },
                None
            )
            .is_err());
        assert_eq!(
            object
                .read_property(P::AUDIT_PRIORITY_FILTER, None)
                .unwrap(),
            filter
        );
        object
            .write_property(P::AUDIT_PRIORITY_FILTER, None, PropertyValue::Null, None)
            .unwrap();
        assert_eq!(
            object
                .read_property(P::AUDIT_PRIORITY_FILTER, None)
                .unwrap(),
            PropertyValue::Null
        );
        object
            .write_property(P::AUDIT_LEVEL, None, PropertyValue::Enumerated(99), None)
            .unwrap();
        assert_eq!(
            object.read_property(P::AUDIT_LEVEL, None).unwrap(),
            PropertyValue::Enumerated(99)
        );
    }
}

#[test]
fn object_audit_policy_specific_priority_inheritance_and_reporter_master_none() {
    let mut reporter = AuditReporterObject::new(1, "reporter").unwrap();
    reporter.set_audit_level(AuditLevel::AUDIT_ALL).unwrap();
    let mut operations = AuditOperationFlags::empty();
    operations.insert(bacnet_types::enums::AuditOperation::WRITE);
    reporter.set_auditable_operations(operations);
    reporter.set_audit_priority_filter(BACnetPriorityFilter::from_bits(1 << 7));
    for filter in [None, Some(AuditPriorityPolicy::Inherit)] {
        let policy = ObjectAuditPolicy {
            priority_filter: filter,
            ..Default::default()
        }
        .effective_internal(&reporter);
        assert!(policy.reports(
            bacnet_types::enums::AuditOperation::WRITE,
            Some(P::PRESENT_VALUE),
            Some(8)
        ));
        assert!(!policy.reports(
            bacnet_types::enums::AuditOperation::WRITE,
            Some(P::PRESENT_VALUE),
            Some(16)
        ));
        assert!(policy.reports(
            bacnet_types::enums::AuditOperation::WRITE,
            Some(P::DESCRIPTION),
            None
        ));
    }
    reporter.set_audit_level(AuditLevel::NONE).unwrap();
    let policy = ObjectAuditPolicy {
        level: Some(AuditLevel::AUDIT_ALL),
        operations: Some(operations),
        ..Default::default()
    }
    .effective_internal(&reporter);
    assert!(!policy.reports(
        bacnet_types::enums::AuditOperation::WRITE,
        Some(P::DESCRIPTION),
        None
    ));
}
