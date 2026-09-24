use super::*;

#[test]
fn object_audit_policy_snapshot_is_forwarded_from_the_original_instance() {
    use bacnet_objects::{
        analog::AnalogValueObject,
        audit::{AuditPriorityPolicy, ObjectAuditPolicy},
    };
    let policy = ObjectAuditPolicy {
        level: Some(AuditLevel::NONE),
        operations: Some(AuditOperationFlags::empty()),
        priority_filter: Some(AuditPriorityPolicy::Inherit),
    };
    let mut value = AnalogValueObject::new(7, "wrapped AV", 62).unwrap();
    value.set_audit_policy(policy);
    let mut object: Box<dyn BACnetObject> = Box::new(value);
    let owner =
        bacnet_objects::database::AuditOwnership::new(oid(ObjectType::DEVICE, 123), selected());
    source_reporter::install(&mut object, &owner).unwrap();
    assert_eq!(object.audit_object_policy_internal(), policy);
    object
        .write_property(
            PropertyIdentifier::AUDIT_LEVEL,
            None,
            PropertyValue::Enumerated(AuditLevel::AUDIT_ALL.to_raw()),
            None,
        )
        .unwrap();
    assert_eq!(
        object.audit_object_policy_internal().level,
        Some(AuditLevel::AUDIT_ALL)
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::AUDIT_LEVEL, None)
            .unwrap(),
        PropertyValue::Enumerated(AuditLevel::AUDIT_ALL.to_raw())
    );
    owner.seal();
    assert_eq!(
        object.audit_object_policy_internal().level,
        Some(AuditLevel::AUDIT_ALL)
    );
}
