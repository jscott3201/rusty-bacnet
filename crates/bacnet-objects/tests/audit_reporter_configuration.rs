//! Downstream-style integration coverage for the complete Reporter configuration contract.

use std::borrow::Cow;

use bacnet_objects::audit::AuditReporterObject;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::bitstring::{AuditOperationFlags, BACnetPriorityFilter};
use bacnet_types::constructed::BACnetObjectSelector;
use bacnet_types::enums::{AuditLevel, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

struct CustomReporter {
    inner: AuditReporterObject,
    configurations: usize,
}

impl BACnetObject for CustomReporter {
    fn audit_reporter_internal(&self) -> Option<&AuditReporterObject> {
        Some(&self.inner)
    }

    fn configure_audit_reporter_internal(
        &mut self,
        level: AuditLevel,
        operations: AuditOperationFlags,
        confirmed: bool,
        selectors: Option<Vec<BACnetObjectSelector>>,
        priorities: BACnetPriorityFilter,
    ) -> Result<(), Error> {
        self.inner.configure_audit_reporter_internal(
            level, operations, confirmed, selectors, priorities,
        )?;
        self.configurations += 1;
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

fn read(object: &dyn BACnetObject, property: PropertyIdentifier) -> PropertyValue {
    object.read_property(property, None).unwrap()
}

fn snapshot(object: &dyn BACnetObject) -> Vec<(PropertyIdentifier, PropertyValue)> {
    object
        .property_list()
        .iter()
        .map(|&p| (p, read(object, p)))
        .collect()
}

#[test]
fn custom_reporter_configuration_replaces_every_setting_through_trait_object() {
    let mut custom = CustomReporter {
        inner: AuditReporterObject::new(1, "Custom Reporter").unwrap(),
        configurations: 0,
    };
    let target = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap();
    let operations = AuditOperationFlags::from_bits((1 << 1) | (1 << 63)).unwrap();
    let priorities = BACnetPriorityFilter::from_bits((1 << 7) | (1 << 15));
    let object: &mut dyn BACnetObject = &mut custom;
    object
        .configure_audit_reporter_internal(
            AuditLevel::AUDIT_ALL,
            operations,
            true,
            Some(vec![BACnetObjectSelector::Object(target)]),
            priorities,
        )
        .unwrap();
    assert_eq!(
        read(object, PropertyIdentifier::AUDIT_LEVEL),
        PropertyValue::Enumerated(AuditLevel::AUDIT_ALL.to_raw())
    );
    assert_eq!(
        read(object, PropertyIdentifier::ISSUE_CONFIRMED_NOTIFICATIONS),
        PropertyValue::Boolean(true)
    );
    let (unused_bits, data) = operations.to_bacnet();
    assert_eq!(
        read(object, PropertyIdentifier::AUDITABLE_OPERATIONS),
        PropertyValue::BitString { unused_bits, data }
    );
    let (unused_bits, data) = priorities.to_bacnet();
    assert_eq!(
        read(object, PropertyIdentifier::AUDIT_PRIORITY_FILTER),
        PropertyValue::BitString { unused_bits, data }
    );
    assert_eq!(
        read(object, PropertyIdentifier::MONITORED_OBJECTS),
        PropertyValue::List(vec![
            BACnetObjectSelector::Object(target).encode_property_value()
        ])
    );
    let reporter = object.audit_reporter_internal().unwrap();
    assert!(reporter.monitors_object_internal(target));
    assert!(reporter.reports_write_internal(PropertyIdentifier::PRESENT_VALUE, Some(8)));
    assert!(reporter.reports_write_internal(PropertyIdentifier::PRESENT_VALUE, Some(16)));
    assert!(!reporter.reports_write_internal(PropertyIdentifier::PRESENT_VALUE, Some(1)));

    // Empty selection retains the property and selects no ordinary targets.
    object
        .configure_audit_reporter_internal(
            AuditLevel::AUDIT_CONFIG,
            AuditOperationFlags::empty(),
            false,
            Some(vec![]),
            BACnetPriorityFilter::empty(),
        )
        .unwrap();
    assert_eq!(
        read(object, PropertyIdentifier::MONITORED_OBJECTS),
        PropertyValue::List(vec![])
    );
    assert!(!object
        .audit_reporter_internal()
        .unwrap()
        .monitors_object_internal(target));
    assert_eq!(
        read(object, PropertyIdentifier::AUDIT_LEVEL),
        PropertyValue::Enumerated(AuditLevel::AUDIT_CONFIG.to_raw())
    );
    assert_eq!(
        read(object, PropertyIdentifier::ISSUE_CONFIRMED_NOTIFICATIONS),
        PropertyValue::Boolean(false)
    );
    let (unused_bits, data) = AuditOperationFlags::empty().to_bacnet();
    assert_eq!(
        read(object, PropertyIdentifier::AUDITABLE_OPERATIONS),
        PropertyValue::BitString { unused_bits, data }
    );
    let (unused_bits, data) = BACnetPriorityFilter::empty().to_bacnet();
    assert_eq!(
        read(object, PropertyIdentifier::AUDIT_PRIORITY_FILTER),
        PropertyValue::BitString { unused_bits, data }
    );

    // None removes the property and restores catch-all selection.
    object
        .configure_audit_reporter_internal(
            AuditLevel::AUDIT_ALL,
            operations,
            true,
            None,
            BACnetPriorityFilter::all(),
        )
        .unwrap();
    assert!(!object
        .property_list()
        .contains(&PropertyIdentifier::MONITORED_OBJECTS));
    assert!(
        matches!(object.read_property(PropertyIdentifier::MONITORED_OBJECTS, None),
        Err(Error::Protocol { class, code })
            if class == ErrorClass::PROPERTY.to_raw() as u32
                && code == ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32)
    );
    assert!(object
        .audit_reporter_internal()
        .unwrap()
        .monitors_object_internal(target));
    assert_eq!(custom.configurations, 3);
}

#[test]
fn custom_reporter_invalid_configuration_preserves_all_settings_and_property_presence() {
    let mut custom = CustomReporter {
        inner: AuditReporterObject::new(1, "Custom Reporter").unwrap(),
        configurations: 0,
    };
    for selectors in [None, Some(vec![]), Some(vec![BACnetObjectSelector::None])] {
        let object: &mut dyn BACnetObject = &mut custom;
        object
            .configure_audit_reporter_internal(
                AuditLevel::AUDIT_ALL,
                AuditOperationFlags::from_bits(2).unwrap(),
                true,
                selectors.clone(),
                BACnetPriorityFilter::from_bits(1 << 7),
            )
            .unwrap();
        let before = snapshot(object);
        assert!(matches!(
            object.configure_audit_reporter_internal(
                AuditLevel::DEFAULT,
                AuditOperationFlags::empty(),
                false,
                if selectors.is_none() {
                    Some(vec![])
                } else {
                    None
                },
                BACnetPriorityFilter::all(),
            ),
            Err(Error::OutOfRange(_))
        ));
        assert_eq!(snapshot(object), before);
    }
    assert_eq!(
        custom.configurations, 3,
        "invalid calls cannot commit custom state"
    );
}

#[test]
fn non_reporter_rejects_complete_configuration_without_mutation() {
    let mut other = BinaryValueObject::new(1, "Other").unwrap();
    let object: &mut dyn BACnetObject = &mut other;
    let before = snapshot(object);
    for (selectors, priorities) in [
        (None, BACnetPriorityFilter::all()),
        (Some(vec![]), BACnetPriorityFilter::empty()),
    ] {
        assert!(matches!(object.configure_audit_reporter_internal(
            AuditLevel::AUDIT_ALL, AuditOperationFlags::from_bits(2).unwrap(), true,
            selectors, priorities,
        ), Err(Error::Protocol { class, code })
            if class == ErrorClass::OBJECT.to_raw() as u32
                && code == ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32));
        assert_eq!(snapshot(object), before);
    }
}
