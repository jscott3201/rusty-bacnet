//! Downstream-style integration coverage for the complete Reporter configuration contract.

use bacnet_objects::property_metadata::{PropertyMetadata, property_list};
use std::borrow::Cow;

use bacnet_objects::audit::{AuditReporterAuthority, AuditReporterObject};
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::bitstring::{AuditOperationFlags, BACnetPriorityFilter};
use bacnet_types::constructed::BACnetObjectSelector;
use bacnet_types::enums::{AuditLevel, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

struct CustomReporter {
    inner: AuditReporterObject,
    lends: usize,
}

impl BACnetObject for CustomReporter {
    fn audit_reporter(&self) -> Option<&AuditReporterObject> {
        Some(&self.inner)
    }

    // Configuration goes through the Reporter's own authority, so the custom
    // object sees each one only as a lend.
    fn audit_reporter_mut(&mut self) -> Option<AuditReporterAuthority<'_>> {
        self.lends += 1;
        self.inner.audit_reporter_mut()
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

    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        self.inner.property_metadata()
    }
}

fn read(object: &dyn BACnetObject, property: PropertyIdentifier) -> PropertyValue {
    object.read_property(property, None).unwrap()
}

fn snapshot(object: &dyn BACnetObject) -> Vec<(PropertyIdentifier, PropertyValue)> {
    property_list(&object.property_metadata())
        .map(|p| (p, read(object, p)))
        .collect()
}

#[test]
fn custom_reporter_configuration_replaces_every_setting_through_trait_object() {
    let mut custom = CustomReporter {
        inner: AuditReporterObject::new(1, "Custom Reporter").unwrap(),
        lends: 0,
    };
    let target = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap();
    let operations = AuditOperationFlags::from_bits((1 << 1) | (1 << 63)).unwrap();
    let priorities = BACnetPriorityFilter::from_bits((1 << 7) | (1 << 15));
    let object: &mut dyn BACnetObject = &mut custom;
    object
        .audit_reporter_mut()
        .expect("an Audit Reporter")
        .configure(
            AuditLevel::AUDIT_ALL,
            operations,
            true,
            Some(vec![BACnetObjectSelector::Object(target)]),
            priorities,
            None,
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
    let reporter = object.audit_reporter().unwrap();
    assert!(reporter.monitors_object_internal(target));
    assert!(reporter.reports_write_internal(PropertyIdentifier::PRESENT_VALUE, Some(8)));
    assert!(reporter.reports_write_internal(PropertyIdentifier::PRESENT_VALUE, Some(16)));
    assert!(!reporter.reports_write_internal(PropertyIdentifier::PRESENT_VALUE, Some(1)));

    // Empty selection retains the property and selects no ordinary targets.
    object
        .audit_reporter_mut()
        .expect("an Audit Reporter")
        .configure(
            AuditLevel::AUDIT_CONFIG,
            AuditOperationFlags::empty(),
            false,
            Some(vec![]),
            BACnetPriorityFilter::empty(),
            None,
        )
        .unwrap();
    assert_eq!(
        read(object, PropertyIdentifier::MONITORED_OBJECTS),
        PropertyValue::List(vec![])
    );
    assert!(
        !object
            .audit_reporter()
            .unwrap()
            .monitors_object_internal(target)
    );
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
        .audit_reporter_mut()
        .expect("an Audit Reporter")
        .configure(
            AuditLevel::AUDIT_ALL,
            operations,
            true,
            None,
            BACnetPriorityFilter::all(),
            None,
        )
        .unwrap();
    assert!(!bacnet_objects::property_metadata::has_property(
        &object.property_metadata(),
        PropertyIdentifier::MONITORED_OBJECTS
    ));
    assert!(
        matches!(object.read_property(PropertyIdentifier::MONITORED_OBJECTS, None),
        Err(Error::Remote(RemoteError::Error { class, code }))
            if class == ErrorClass::PROPERTY
                && code == ErrorCode::UNKNOWN_PROPERTY)
    );
    assert!(
        object
            .audit_reporter()
            .unwrap()
            .monitors_object_internal(target)
    );
    assert_eq!(custom.lends, 3);
}

#[test]
fn custom_reporter_invalid_configuration_preserves_all_settings_and_property_presence() {
    let mut custom = CustomReporter {
        inner: AuditReporterObject::new(1, "Custom Reporter").unwrap(),
        lends: 0,
    };
    for selectors in [None, Some(vec![]), Some(vec![BACnetObjectSelector::None])] {
        let object: &mut dyn BACnetObject = &mut custom;
        object
            .audit_reporter_mut()
            .expect("an Audit Reporter")
            .configure(
                AuditLevel::AUDIT_ALL,
                AuditOperationFlags::from_bits(2).unwrap(),
                true,
                selectors.clone(),
                BACnetPriorityFilter::from_bits(1 << 7),
                None,
            )
            .unwrap();
        let before = snapshot(object);
        assert!(matches!(
            object
                .audit_reporter_mut()
                .expect("an Audit Reporter")
                .configure(
                    AuditLevel::DEFAULT,
                    AuditOperationFlags::empty(),
                    false,
                    if selectors.is_none() {
                        Some(vec![])
                    } else {
                        None
                    },
                    BACnetPriorityFilter::all(),
                    None,
                ),
            Err(Error::OutOfRange(_))
        ));
        assert_eq!(snapshot(object), before);
    }
    assert_eq!(
        custom.lends, 6,
        "every configuration, valid or not, borrows the custom object's Reporter"
    );
}

#[test]
fn non_reporter_lends_no_configuration_authority() {
    let mut other = BinaryValueObject::new(1, "Other").unwrap();
    let object: &mut dyn BACnetObject = &mut other;
    assert!(object.audit_reporter().is_none());
    assert!(object.audit_reporter_mut().is_none());
}
