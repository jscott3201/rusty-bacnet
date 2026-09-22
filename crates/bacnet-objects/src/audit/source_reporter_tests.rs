use super::*;
use crate::database::ObjectDatabase;

fn reporter(instance: u32) -> AuditReporterObject {
    AuditReporterObject::new(instance, format!("Reporter-{instance}")).unwrap()
}

fn source(object: &dyn BACnetObject) -> PropertyValue {
    object
        .read_property(PropertyIdentifier::AUDIT_SOURCE_REPORTER, None)
        .unwrap()
}

#[test]
fn default_reporter_is_a_deleteable_non_createable_target() {
    let mut reporter = reporter(1);
    assert_eq!(source(&reporter), PropertyValue::Boolean(false));
    assert!(reporter.is_deleteable());
    assert!(!reporter.is_createable());
    assert!(matches!(reporter.write_property(
        PropertyIdentifier::AUDIT_SOURCE_REPORTER, None, PropertyValue::Boolean(true), None,
    ), Err(Error::Protocol { class, code })
        if class == ErrorClass::PROPERTY.to_raw() as u32
            && code == ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32));
    assert_eq!(source(&reporter), PropertyValue::Boolean(false));
}

#[test]
fn source_binding_preserves_configuration_and_is_idempotent_but_not_reassignable() {
    let mut selected = reporter(1);
    selected.set_audit_level(AuditLevel::AUDIT_ALL).unwrap();
    selected.set_auditable_operations(AuditOperationFlags::from_bits(1 << 1).unwrap());
    selected.set_issue_confirmed_notifications(true);
    selected.set_monitored_objects(Some(vec![]));
    selected.set_audit_priority_filter(BACnetPriorityFilter::empty());
    let oid = selected.object_identifier();
    let before: Vec<_> = selected
        .property_list()
        .iter()
        .filter(|&&p| p != PropertyIdentifier::AUDIT_SOURCE_REPORTER)
        .map(|&p| (p, selected.read_property(p, None).unwrap()))
        .collect();
    let target = reporter(2);
    let target_oid = target.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(selected)).unwrap();
    db.add(Box::new(target)).unwrap();
    for _ in 0..2 {
        AuditReporterObject::designate_source_internal(&mut db, oid).unwrap();
    }
    let selected = db.get(&oid).unwrap();
    assert_eq!(source(selected), PropertyValue::Boolean(true));
    assert!(!selected.is_deleteable());
    assert!(!selected.is_createable());
    for (property, expected) in before {
        assert_eq!(selected.read_property(property, None).unwrap(), expected);
    }
    assert!(AuditReporterObject::designate_source_internal(&mut db, target_oid).is_err());
    assert_eq!(source(db.get(&oid).unwrap()), PropertyValue::Boolean(true));
    assert_eq!(
        source(db.get(&target_oid).unwrap()),
        PropertyValue::Boolean(false)
    );
    assert!(db.get(&target_oid).unwrap().is_deleteable());
    assert!(matches!(db.get_mut(&oid).unwrap().write_property(
        PropertyIdentifier::AUDIT_SOURCE_REPORTER, None, PropertyValue::Boolean(false), None,
    ), Err(Error::Protocol { class, code })
        if class == ErrorClass::PROPERTY.to_raw() as u32
            && code == ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32));
    assert_eq!(source(db.get(&oid).unwrap()), PropertyValue::Boolean(true));
}

// An existing downstream implementation with only the old read capability.
// It intentionally does not implement the new binding hook.
struct LegacyReporter(AuditReporterObject);

impl BACnetObject for LegacyReporter {
    fn audit_reporter_internal(&self) -> Option<&AuditReporterObject> {
        Some(&self.0)
    }
    fn object_identifier(&self) -> ObjectIdentifier {
        self.0.object_identifier()
    }
    fn object_name(&self) -> &str {
        self.0.object_name()
    }
    fn read_property(&self, p: PropertyIdentifier, i: Option<u32>) -> Result<PropertyValue, Error> {
        self.0.read_property(p, i)
    }
    fn write_property(
        &mut self,
        p: PropertyIdentifier,
        i: Option<u32>,
        v: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.0.write_property(p, i, v, priority)
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        self.0.property_list()
    }
}

#[test]
fn legacy_capability_defaults_to_atomic_source_opt_out() {
    let legacy = LegacyReporter(reporter(1));
    let oid = legacy.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(legacy)).unwrap();
    assert!(
        matches!(AuditReporterObject::designate_source_internal(&mut db, oid),
        Err(Error::Encoding(message)) if message.contains("does not support source ownership"))
    );
    assert_eq!(source(db.get(&oid).unwrap()), PropertyValue::Boolean(false));
    assert!(db.get(&oid).unwrap().is_deleteable());
}
