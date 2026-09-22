//! Downstream-style compatibility coverage for the public Reporter override hooks.

use std::borrow::Cow;

use bacnet_objects::audit::AuditReporterObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::bitstring::{AuditOperationFlags, BACnetPriorityFilter};
use bacnet_types::constructed::BACnetObjectSelector;
use bacnet_types::enums::{AuditLevel, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

// Intentionally overrides only the original three-argument configuration hook.
// This external test crate must keep compiling without implementing the new hook.
struct LegacyReporter {
    inner: AuditReporterObject,
    calls: Vec<(AuditLevel, AuditOperationFlags, bool)>,
}

impl BACnetObject for LegacyReporter {
    fn audit_reporter_internal(&self) -> Option<&AuditReporterObject> {
        Some(&self.inner)
    }

    fn configure_audit_reporter_internal(
        &mut self,
        level: AuditLevel,
        operations: AuditOperationFlags,
        confirmed: bool,
    ) -> Result<(), Error> {
        self.inner
            .configure_audit_reporter_internal(level, operations, confirmed)?;
        self.calls.push((level, operations, confirmed));
        Ok(())
    }

    fn object_identifier(&self) -> ObjectIdentifier {
        self.inner.object_identifier()
    }

    fn object_name(&self) -> &str {
        self.inner.object_name()
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        self.inner.read_property(property, array_index)
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.inner
            .write_property(property, array_index, value, priority)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        self.inner.property_list()
    }
}

fn snapshot(object: &dyn BACnetObject) -> Vec<PropertyValue> {
    [
        PropertyIdentifier::AUDIT_LEVEL,
        PropertyIdentifier::AUDITABLE_OPERATIONS,
        PropertyIdentifier::ISSUE_CONFIRMED_NOTIFICATIONS,
        PropertyIdentifier::MONITORED_OBJECTS,
        PropertyIdentifier::AUDIT_PRIORITY_FILTER,
    ]
    .into_iter()
    .map(|property| object.read_property(property, None).unwrap())
    .collect()
}

#[test]
fn audit_reporter_legacy_override_delegates_defaults_and_rejects_filters_before_mutation() {
    let mut legacy = LegacyReporter {
        inner: AuditReporterObject::new(1, "Legacy Reporter").unwrap(),
        calls: Vec::new(),
    };
    legacy.inner.set_monitored_objects(Some(vec![]));
    let operations = AuditOperationFlags::from_bits(2).unwrap();
    // Old source remains valid, including direct calls through a trait object.
    let object: &mut dyn BACnetObject = &mut legacy;
    object
        .configure_audit_reporter_internal(AuditLevel::AUDIT_ALL, operations, true)
        .unwrap();
    assert_eq!(
        legacy.calls,
        vec![(AuditLevel::AUDIT_ALL, operations, true)]
    );

    let object: &mut dyn BACnetObject = &mut legacy;
    object
        .configure_audit_reporter_with_filters_internal(
            AuditLevel::AUDIT_CONFIG,
            AuditOperationFlags::empty(),
            false,
            None,
            BACnetPriorityFilter::all(),
        )
        .unwrap();
    assert_eq!(
        legacy.calls,
        vec![
            (AuditLevel::AUDIT_ALL, operations, true),
            (
                AuditLevel::AUDIT_CONFIG,
                AuditOperationFlags::empty(),
                false
            ),
        ]
    );
    let before = snapshot(&legacy);
    assert_eq!(
        before[0],
        PropertyValue::Enumerated(AuditLevel::AUDIT_CONFIG.to_raw())
    );
    assert_eq!(before[2], PropertyValue::Boolean(false));
    // Delegation retains legacy behavior, rather than silently clearing selectors.
    assert_eq!(before[3], PropertyValue::List(vec![]));

    let target = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap();
    for (selectors, priorities) in [
        (Some(vec![]), BACnetPriorityFilter::all()),
        (
            Some(vec![BACnetObjectSelector::None]),
            BACnetPriorityFilter::all(),
        ),
        (
            Some(vec![BACnetObjectSelector::Object(target)]),
            BACnetPriorityFilter::all(),
        ),
        (
            Some(vec![BACnetObjectSelector::ObjectType(
                ObjectType::ANALOG_VALUE,
            )]),
            BACnetPriorityFilter::all(),
        ),
        (None, BACnetPriorityFilter::empty()),
        (None, BACnetPriorityFilter::from_bits(1 << 7)),
        (Some(vec![]), BACnetPriorityFilter::empty()),
    ] {
        let object: &mut dyn BACnetObject = &mut legacy;
        assert!(matches!(
            object.configure_audit_reporter_with_filters_internal(
                AuditLevel::AUDIT_ALL, operations, true, selectors, priorities,
            ),
            Err(Error::Protocol { class, code })
                if class == ErrorClass::OBJECT.to_raw() as u32
                    && code == ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32
        ));
        assert_eq!(
            legacy.calls.len(),
            2,
            "unsupported filters must not invoke the legacy override"
        );
        assert_eq!(snapshot(&legacy), before);
    }
}

#[test]
fn audit_reporter_builtin_legacy_hook_preserves_filters_and_error_atomicity() {
    let mut reporter = AuditReporterObject::new(1, "Reporter").unwrap();
    reporter.set_monitored_objects(Some(vec![BACnetObjectSelector::ObjectType(
        ObjectType::ANALOG_VALUE,
    )]));
    reporter.set_audit_priority_filter(BACnetPriorityFilter::from_bits(1 << 7));
    let filters = snapshot(&reporter)[3..].to_vec();
    let object: &mut dyn BACnetObject = &mut reporter;
    object
        .configure_audit_reporter_internal(
            AuditLevel::AUDIT_ALL,
            AuditOperationFlags::from_bits(2).unwrap(),
            true,
        )
        .unwrap();
    let before = snapshot(object);
    assert_eq!(&before[3..], filters);
    assert_eq!(
        before[0],
        PropertyValue::Enumerated(AuditLevel::AUDIT_ALL.to_raw())
    );
    assert_eq!(before[2], PropertyValue::Boolean(true));
    assert!(
        object
            .configure_audit_reporter_internal(
                AuditLevel::DEFAULT,
                AuditOperationFlags::empty(),
                false,
            )
            .is_err()
    );
    assert_eq!(snapshot(object), before);
}
