//! Exactly the Audit Log lends `AuditLog`, exactly the Analog Value, Binary
//! Value, Color and Color Temperature lend `AuditPolicy`, and exactly the
//! Audit Reporter lends its Reporter, each through both of its accessors,
//! which lend the same object (#2179). Also the source owner's check on a
//! Reporter's configuration, and the Averaging and Pulse Converter passes,
//! which reach the built-in objects only.

use crate::audit::AuditReporterObject;
use crate::catalogue::representatives;
use crate::database::{AuditOwnership, ObjectDatabase};
use crate::traits::BACnetObject;
use alloc::boxed::Box;
use alloc::collections::BTreeSet;
use alloc::sync::Arc;
use bacnet_types::bitstring::{AuditOperationFlags, BACnetPriorityFilter};
use bacnet_types::constructed::{BACnetObjectPropertyReference, BACnetObjectSelector};
use bacnet_types::enums::{AuditLevel, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use core::time::Duration;

const AUDIT_LOG_LENDERS: [ObjectType; 1] = [ObjectType::AUDIT_LOG];
const AUDIT_POLICY_LENDERS: [ObjectType; 4] = [
    ObjectType::ANALOG_VALUE,
    ObjectType::BINARY_VALUE,
    ObjectType::COLOR,
    ObjectType::COLOR_TEMPERATURE,
];
const AUDIT_REPORTER_LENDERS: [ObjectType; 1] = [ObjectType::AUDIT_REPORTER];

/// Check that across the catalogue exactly `lenders` answer `lent`, whose
/// two answers (shared, exclusive) must agree.
fn lenders_are<T: PartialEq + core::fmt::Debug>(
    capability: &str,
    lenders: &[ObjectType],
    lent: impl Fn(&mut Box<dyn BACnetObject>) -> (Option<T>, Option<T>),
) {
    let mut seen = BTreeSet::new();
    for mut object in representatives() {
        let object_type = object.object_identifier().object_type();
        let (shared, exclusive) = lent(&mut object);
        assert_eq!(
            shared, exclusive,
            "{object_type:?}: the two {capability} accessors must lend the same object"
        );
        let lends = lenders.contains(&object_type);
        assert_eq!(
            shared.is_some(),
            lends,
            "{object_type:?} {} lend {capability}",
            if lends { "must" } else { "must not" }
        );
        seen.insert(object_type.to_raw());
    }
    for object_type in lenders {
        assert!(
            seen.contains(&object_type.to_raw()),
            "{object_type:?} is not in the catalogue"
        );
    }
}

fn address<T: ?Sized>(lent: &T) -> *const () {
    (lent as *const T).cast()
}

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

#[test]
fn exactly_the_audit_log_lends_audit_log_through_both_accessors() {
    lenders_are("AuditLog", &AUDIT_LOG_LENDERS, |object| {
        let shared = object.audit_log().map(address);
        (shared, object.audit_log_mut().map(|log| address(log)))
    });
}

#[test]
fn exactly_the_policy_objects_lend_audit_policy_through_both_accessors() {
    lenders_are("AuditPolicy", &AUDIT_POLICY_LENDERS, |object| {
        let shared = object.audit_policy().map(address);
        (shared, object.audit_policy_mut().map(|p| address(p)))
    });
    // Each lender also lends the sealed built-in assignment authority.
    for mut object in representatives() {
        if let Some(policy) = object.audit_policy_mut() {
            assert!(policy.policy_authority().is_some());
        }
    }
}

#[test]
fn exactly_the_audit_reporter_lends_its_reporter_through_both_accessors() {
    lenders_are("the Audit Reporter", &AUDIT_REPORTER_LENDERS, |object| {
        let shared = object.audit_reporter().map(|r| r.object_identifier());
        let exclusive = object.audit_reporter_mut().map(|r| r.object_identifier());
        (shared, exclusive)
    });
}

fn configure(
    reporter: &mut AuditReporterObject,
    owner: Option<&alloc::sync::Weak<AuditOwnership>>,
    selectors: Option<alloc::vec::Vec<BACnetObjectSelector>>,
) -> Result<(), Error> {
    let mut authority = reporter.audit_reporter_mut().unwrap();
    if let Some(owner) = owner {
        authority = authority.with_source_owner(owner);
    }
    authority.configure(
        AuditLevel::AUDIT_ALL,
        AuditOperationFlags::empty(),
        false,
        selectors,
        BACnetPriorityFilter::all(),
        None,
    )
}

#[test]
fn an_active_source_owner_refuses_monitored_objects_only_while_active() {
    let mut reporter = AuditReporterObject::new(1, "AR").unwrap();
    let owner = AuditOwnership::for_source(
        oid(ObjectType::DEVICE, 1),
        oid(ObjectType::AUDIT_REPORTER, 1),
    );
    let lease = Arc::downgrade(&owner);
    let selected = || Some(alloc::vec![]);
    assert!(matches!(
        configure(&mut reporter, Some(&lease), selected()),
        Err(Error::Encoding(_))
    ));
    assert!(
        reporter
            .configuration_internal()
            .monitored_objects
            .is_none()
    );
    // Without monitored objects the owner has nothing to refuse.
    configure(&mut reporter, Some(&lease), None).unwrap();
    assert_eq!(
        reporter.configuration_internal().audit_level,
        AuditLevel::AUDIT_ALL
    );
    // A sealed owner, or none at all, leaves the Reporter's own checks.
    owner.seal();
    configure(&mut reporter, Some(&lease), selected()).unwrap();
    assert!(
        reporter
            .configuration_internal()
            .monitored_objects
            .is_some()
    );
    configure(&mut reporter, None, None).unwrap();
}

/// A custom object of any type: none of the built-in state.
struct Custom(ObjectIdentifier, &'static str);

impl BACnetObject for Custom {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.0
    }
    fn object_name(&self) -> &str {
        self.1
    }
    fn read_property(
        &self,
        _: PropertyIdentifier,
        _: Option<u32>,
    ) -> Result<bacnet_types::primitives::PropertyValue, Error> {
        Err(crate::common::unknown_property_error())
    }
    fn write_property(
        &mut self,
        _: PropertyIdentifier,
        _: Option<u32>,
        _: bacnet_types::primitives::PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        Err(crate::common::unknown_property_error())
    }
    fn property_metadata(
        &self,
    ) -> alloc::borrow::Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        crate::test_rows::read_only(&[])
    }
}

#[test]
fn custom_averaging_and_pulse_converter_objects_are_neither_sampled_nor_judged() {
    let mut db = ObjectDatabase::new();
    let averaging = oid(ObjectType::AVERAGING, 1);
    let converter = oid(ObjectType::PULSE_CONVERTER, 1);
    db.add(Box::new(Custom(averaging, "custom averaging")))
        .unwrap();
    db.add(Box::new(Custom(converter, "custom converter")))
        .unwrap();
    db.set_monotonic_clock(Some(Arc::new(|| Duration::ZERO)));
    assert!(db.sample_due_averaging_objects(Duration::MAX).is_empty());
    assert!(!db.check_input_reference(&converter));
    assert!(db.count_pulse_inputs().is_empty());
}

#[test]
fn a_built_in_averaging_object_is_sampled_once_due() {
    let mut db = ObjectDatabase::new();
    let mut averaging = crate::averaging::AveragingObject::new(1, "AVG").unwrap();
    averaging.set_object_property_reference(Some(BACnetObjectPropertyReference::new(
        oid(ObjectType::ANALOG_VALUE, 9),
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    db.add(Box::new(averaging)).unwrap();
    db.set_monotonic_clock(Some(Arc::new(|| Duration::ZERO)));
    assert_eq!(
        db.sample_due_averaging_objects(Duration::from_secs(60)),
        [oid(ObjectType::AVERAGING, 1)]
    );
}
