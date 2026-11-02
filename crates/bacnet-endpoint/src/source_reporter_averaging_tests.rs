//! SourceReporter forwards the Averaging application route and its property
//! COV admission (#1083), and the sample schedule the server follows (#1144),
//! instead of inheriting the trait defaults. A wrapped Pulse Converter is
//! skipped, as documented (#2179).
use super::*;
use bacnet_objects::averaging::AveragingObject;
use bacnet_objects::database::ObjectDatabase;
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

fn wrap(db: &mut ObjectDatabase, object: ObjectIdentifier) {
    let owner = bacnet_objects::database::AuditOwnership::for_source(
        oid(ObjectType::DEVICE, 123),
        selected(),
    );
    db.with_object_adapter(&object, |slot| {
        source_reporter::install(slot, &owner).unwrap()
    })
    .unwrap()
    .unwrap();
}

/// The sample schedule survives wrapping: the database asks for the due
/// sample through the forwarded `timed`, so a wrapped Averaging object is
/// sampled and its deadline moves on instead of staying due (which would
/// wake the monotonic task every millisecond).
#[test]
fn a_wrapped_averaging_object_is_sampled_and_its_deadline_moves_on() {
    let mut db = ObjectDatabase::new();
    let source = bacnet_objects::analog::AnalogValueObject::new(
        1,
        "AV-1",
        bacnet_types::enums::EngineeringUnits::NO_UNITS,
    )
    .unwrap();
    db.add(Box::new(source)).unwrap();
    let mut averaging = AveragingObject::new(1, "AVG-1").unwrap();
    averaging.set_object_property_reference(Some(BACnetObjectPropertyReference::new(
        oid(ObjectType::ANALOG_VALUE, 1),
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    let averaging_oid = averaging.object_identifier();
    db.add(Box::new(averaging)).unwrap();
    wrap(&mut db, averaging_oid);

    // 900 s over 15 samples: the first is due a minute after the clock binds.
    db.set_monotonic_clock(Some(Arc::new(|| Duration::ZERO)));
    let due = Duration::from_secs(60);
    let deadline = |db: &mut ObjectDatabase| {
        let object = db.get_mut(&averaging_oid).unwrap();
        object.timed().unwrap().next_monotonic_deadline()
    };
    let read = |db: &ObjectDatabase, property| {
        db.get(&averaging_oid)
            .unwrap()
            .read_property(property, None)
            .unwrap()
    };
    assert_eq!(deadline(&mut db), Some(due));
    assert!(
        db.sample_due_averaging_objects(due - Duration::from_millis(1))
            .is_empty()
    );
    assert_eq!(db.sample_due_averaging_objects(due), [averaging_oid]);
    assert_eq!(deadline(&mut db), Some(due * 2));
    // A second pass at the same instant has nothing due.
    assert!(db.sample_due_averaging_objects(due).is_empty());
    assert_eq!(
        read(&db, PropertyIdentifier::ATTEMPTED_SAMPLES),
        PropertyValue::Unsigned(1)
    );
    assert_eq!(
        read(&db, PropertyIdentifier::VALID_SAMPLES),
        PropertyValue::Unsigned(1)
    );
}

/// A Pulse Converter is judged and counted as the built-in object only
/// (#2179), so a wrapped one keeps its Reliability whatever its
/// Input_Reference names, and counts nothing.
#[test]
fn a_wrapped_pulse_converter_is_neither_judged_nor_counted() {
    let mut db = ObjectDatabase::new();
    let accumulator = bacnet_objects::accumulator::AccumulatorObject::new(
        9,
        "ACC-9",
        bacnet_types::enums::EngineeringUnits::NO_UNITS,
    )
    .unwrap();
    let accumulator_oid = accumulator.object_identifier();
    db.add(Box::new(accumulator)).unwrap();
    let mut converter = bacnet_objects::accumulator::PulseConverterObject::new(
        1,
        "PC-1",
        bacnet_types::enums::EngineeringUnits::NO_UNITS,
    )
    .unwrap();
    converter.set_input_reference(BACnetObjectPropertyReference::new(
        accumulator_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    ));
    let converter_oid = converter.object_identifier();
    db.add(Box::new(converter)).unwrap();
    wrap(&mut db, converter_oid);
    // The reference now names an object the database doesn't hold: the
    // built-in converter would report CONFIGURATION_ERROR.
    db.remove(&accumulator_oid).unwrap();

    assert!(!db.check_input_reference(&converter_oid));
    assert!(db.count_pulse_inputs().is_empty());
    let reliability = db
        .get(&converter_oid)
        .unwrap()
        .read_property(PropertyIdentifier::RELIABILITY, None)
        .unwrap();
    assert_eq!(
        reliability,
        PropertyValue::Enumerated(bacnet_types::enums::Reliability::NO_FAULT_DETECTED.to_raw())
    );
}
