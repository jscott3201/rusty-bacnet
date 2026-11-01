//! SourceReporter forwards the Averaging application route and its property
//! COV admission (#1083), and the sample schedule the server follows (#1144),
//! instead of inheriting the trait defaults.
use super::*;
use bacnet_objects::averaging::AveragingObject;
use bacnet_types::constructed::BACnetObjectPropertyReference;
use std::sync::Arc;

#[test]
fn averaging_sample_route_and_property_cov_admission_survive_wrapping() {
    let mut object: Box<dyn BACnetObject> = Box::new(AveragingObject::new(1, "AVG-1").unwrap());
    let owner = bacnet_objects::database::AuditOwnership::for_source(
        oid(ObjectType::DEVICE, 123),
        selected(),
    );
    source_reporter::install(&mut object, &owner).unwrap();

    assert!(!object.supports_cov());
    assert!(object.supports_subscribe_cov_property());
    assert!(object.supports_cov_property(PropertyIdentifier::AVERAGE_VALUE));
    for sample in [PropertyValue::Real(3.0), PropertyValue::Unsigned(5)] {
        object.add_averaging_sample_internal(Some(sample)).unwrap();
    }
    // A missed attempt reaches the wrapped object too.
    object.add_averaging_sample_internal(None).unwrap();
    assert_eq!(
        object
            .read_property(PropertyIdentifier::ATTEMPTED_SAMPLES, None)
            .unwrap(),
        PropertyValue::Unsigned(3)
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::AVERAGE_VALUE, None)
            .unwrap(),
        PropertyValue::Real(4.0)
    );
    assert_eq!(
        object
            .read_property(PropertyIdentifier::VALID_SAMPLES, None)
            .unwrap(),
        PropertyValue::Unsigned(2)
    );
}

#[test]
fn averaging_sample_schedule_survives_wrapping() {
    let reference = BACnetObjectPropertyReference::new(
        oid(ObjectType::ANALOG_VALUE, 1),
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    );
    let mut averaging = AveragingObject::new(1, "AVG-1").unwrap();
    averaging.set_object_property_reference(Some(reference.clone()));
    let mut object: Box<dyn BACnetObject> = Box::new(averaging);
    let owner = bacnet_objects::database::AuditOwnership::for_source(
        oid(ObjectType::DEVICE, 123),
        selected(),
    );
    source_reporter::install(&mut object, &owner).unwrap();

    // 900 s over 15 samples: the first is due a minute after the clock binds.
    object.timed().unwrap().bind_clocks(
        &bacnet_objects::traits::Clocks::default()
            .with_monotonic(Some(Arc::new(|| Duration::ZERO))),
    );
    let due = Duration::from_secs(60);
    assert_eq!(object.timed().unwrap().next_monotonic_deadline(), Some(due));
    assert_eq!(
        object.take_due_averaging_sample_internal(due),
        Some(reference)
    );
    assert_eq!(
        object.timed().unwrap().next_monotonic_deadline(),
        Some(due * 2)
    );
}
