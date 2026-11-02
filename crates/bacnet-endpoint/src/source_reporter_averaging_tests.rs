//! SourceReporter forwards the Averaging application route and its property
//! COV admission (#1083), and the sample schedule's deadline (#1144),
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
        object
            .local_input()
            .unwrap()
            .apply_local_input(bacnet_objects::traits::LocalInputValue::AveragingSample(
                Some(sample),
            ))
            .unwrap();
    }
    // A missed attempt reaches the wrapped object too.
    object
        .local_input()
        .unwrap()
        .apply_local_input(bacnet_objects::traits::LocalInputValue::AveragingSample(
            None,
        ))
        .unwrap();
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

/// The sample schedule's deadline survives wrapping, but the samples don't:
/// the database takes them from the built-in object only (#2179), so a
/// wrapped Averaging object is never sampled and its deadline stays due.
#[test]
fn averaging_sample_deadline_survives_wrapping_but_sampling_does_not() {
    let reference = BACnetObjectPropertyReference::new(
        oid(ObjectType::ANALOG_VALUE, 1),
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    );
    let mut averaging = AveragingObject::new(1, "AVG-1").unwrap();
    averaging.set_object_property_reference(Some(reference));
    let averaging_oid = averaging.object_identifier();
    let mut db = bacnet_objects::database::ObjectDatabase::new();
    db.add(Box::new(averaging)).unwrap();
    let owner = bacnet_objects::database::AuditOwnership::for_source(
        oid(ObjectType::DEVICE, 123),
        selected(),
    );
    db.with_object_adapter(&averaging_oid, |slot| {
        source_reporter::install(slot, &owner).unwrap()
    })
    .unwrap()
    .unwrap();

    // 900 s over 15 samples: the first is due a minute after the clock binds.
    db.set_monotonic_clock(Some(Arc::new(|| Duration::ZERO)));
    let due = Duration::from_secs(60);
    let deadline = |db: &mut bacnet_objects::database::ObjectDatabase| {
        let object = db.get_mut(&averaging_oid).unwrap();
        object.timed().unwrap().next_monotonic_deadline()
    };
    assert_eq!(deadline(&mut db), Some(due));
    assert!(db.sample_due_averaging_objects(due).is_empty());
    assert_eq!(deadline(&mut db), Some(due));
    assert_eq!(
        db.get(&averaging_oid)
            .unwrap()
            .read_property(PropertyIdentifier::ATTEMPTED_SAMPLES, None)
            .unwrap(),
        PropertyValue::Unsigned(0)
    );
}
