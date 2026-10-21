//! The database's pass over due Averaging samples (#1144): the referenced
//! local property is read and recorded, and an attempt that yields no usable
//! value is recorded as a miss.
use super::*;
use crate::analog::AnalogValueObject;
use crate::averaging::AveragingObject;
use crate::command_source::CommandOrigin;
use crate::value_types::LargeAnalogValueObject;
use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::{EngineeringUnits, ObjectType};
use bacnet_types::primitives::PropertyValue;
use std::sync::{Arc, Mutex};

use PropertyIdentifier as P;

fn av1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap()
}

fn avg1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::AVERAGING, 1).unwrap()
}

fn secs(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}

/// A database on a hand-moved monotonic clock holding AV-1 at `value` and a
/// LargeAnalogValue, with AVG-1 sampling `reference` every 2 s (10 s over 5
/// samples). The schedule starts when AVG-1 is added at time zero.
fn fixture(
    value: f32,
    reference: Option<BACnetObjectPropertyReference>,
) -> (ObjectDatabase, Arc<Mutex<Duration>>) {
    let time = Arc::new(Mutex::new(Duration::ZERO));
    let source = Arc::clone(&time);
    let mut db = ObjectDatabase::new();
    db.set_monotonic_clock_internal(Some(Arc::new(move || *source.lock().unwrap())));
    db.add(Box::new(
        AnalogValueObject::new(1, "AV-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(LargeAnalogValueObject::new(1, "LAV-1").unwrap()))
        .unwrap();
    set_av1(&mut db, value);
    let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
    avg.set_window_interval(10).unwrap();
    avg.set_window_samples(5).unwrap();
    avg.set_object_property_reference(reference);
    db.add(Box::new(avg)).unwrap();
    (db, time)
}

/// Command AV-1 at priority 16, as the local device.
fn set_av1(db: &mut ObjectDatabase, value: f32) {
    let origin = CommandOrigin::Local {
        owner_device: ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap(),
        initiating_object: None,
    };
    db.get_mut(&av1())
        .unwrap()
        .write_property_from(
            P::PRESENT_VALUE,
            None,
            PropertyValue::Real(value),
            Some(16),
            &origin,
        )
        .unwrap();
}

fn to(oid: ObjectIdentifier, property: P) -> Option<BACnetObjectPropertyReference> {
    Some(BACnetObjectPropertyReference::new(oid, property.to_raw()))
}

fn read(db: &ObjectDatabase, property: P) -> PropertyValue {
    db.get(&avg1())
        .unwrap()
        .read_property(property, None)
        .unwrap()
}

/// `(Attempted_Samples, Valid_Samples)`.
fn counts(db: &ObjectDatabase) -> (PropertyValue, PropertyValue) {
    (read(db, P::ATTEMPTED_SAMPLES), read(db, P::VALID_SAMPLES))
}

fn at(db: &mut ObjectDatabase, time: &Mutex<Duration>, now: Duration) -> Vec<ObjectIdentifier> {
    *time.lock().unwrap() = now;
    db.sample_due_averaging_objects(now)
}

#[test]
fn averaging_samples_read_the_referenced_property_each_spacing() {
    let (mut db, time) = fixture(10.0, to(av1(), P::PRESENT_VALUE));
    assert!(at(&mut db, &time, secs(1)).is_empty());
    assert_eq!(read(&db, P::ATTEMPTED_SAMPLES), PropertyValue::Unsigned(0));

    assert_eq!(at(&mut db, &time, secs(2)), [avg1()]);
    assert_eq!(read(&db, P::AVERAGE_VALUE), PropertyValue::Real(10.0));
    assert!(at(&mut db, &time, secs(3)).is_empty());

    set_av1(&mut db, 20.0);
    assert_eq!(at(&mut db, &time, secs(4)), [avg1()]);
    assert_eq!(read(&db, P::AVERAGE_VALUE), PropertyValue::Real(15.0));
    assert_eq!(read(&db, P::MAXIMUM_VALUE), PropertyValue::Real(20.0));
    assert_eq!(
        counts(&db),
        (PropertyValue::Unsigned(2), PropertyValue::Unsigned(2))
    );

    // Five samples fill the window at 10 s, the Window_Interval; the sixth
    // pushes the first out.
    for second in [6, 8, 10] {
        at(&mut db, &time, secs(second));
    }
    assert_eq!(read(&db, P::ATTEMPTED_SAMPLES), PropertyValue::Unsigned(5));
    set_av1(&mut db, 30.0);
    at(&mut db, &time, secs(12));
    assert_eq!(read(&db, P::MINIMUM_VALUE), PropertyValue::Real(20.0));
    assert_eq!(read(&db, P::ATTEMPTED_SAMPLES), PropertyValue::Unsigned(5));
}

#[test]
fn averaging_samples_read_an_indexed_reference() {
    let reference = BACnetObjectPropertyReference {
        object_identifier: av1(),
        property_identifier: P::PRIORITY_ARRAY.to_raw(),
        property_array_index: Some(16),
    };
    let (mut db, time) = fixture(7.5, Some(reference));
    assert_eq!(at(&mut db, &time, secs(2)), [avg1()]);
    assert_eq!(read(&db, P::AVERAGE_VALUE), PropertyValue::Real(7.5));
}

#[test]
fn averaging_unreadable_or_unusable_reference_counts_a_missed_attempt() {
    let lav1 = ObjectIdentifier::new(ObjectType::LARGE_ANALOG_VALUE, 1).unwrap();
    let missing = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 9).unwrap();
    let indexed_scalar = BACnetObjectPropertyReference {
        object_identifier: av1(),
        property_identifier: P::PRESENT_VALUE.to_raw(),
        property_array_index: Some(1),
    };
    for (case, reference) in [
        ("no such object", to(missing, P::PRESENT_VALUE)),
        ("no such property", to(av1(), P::WINDOW_INTERVAL)),
        ("index on a scalar", Some(indexed_scalar)),
        ("CharacterString", to(av1(), P::OBJECT_NAME)),
        ("Double", to(lav1, P::PRESENT_VALUE)),
    ] {
        let (mut db, time) = fixture(10.0, reference);
        // The miss is still a change to report.
        assert_eq!(at(&mut db, &time, secs(2)), [avg1()], "{case}");
        assert_eq!(
            counts(&db),
            (PropertyValue::Unsigned(1), PropertyValue::Unsigned(0)),
            "{case}"
        );
        assert!(
            matches!(read(&db, P::AVERAGE_VALUE), PropertyValue::Real(v) if v.is_nan()),
            "{case}"
        );
    }
}

#[test]
fn averaging_without_a_reference_or_a_clock_is_never_sampled() {
    // No reference: the application feeds this object.
    let (mut db, time) = fixture(10.0, None);
    assert!(at(&mut db, &time, secs(3_600)).is_empty());
    assert_eq!(read(&db, P::ATTEMPTED_SAMPLES), PropertyValue::Unsigned(0));

    // No monotonic clock: the database isn't in a running server.
    let (mut db, _) = fixture(10.0, to(av1(), P::PRESENT_VALUE));
    db.set_monotonic_clock_internal(None);
    assert!(db.sample_due_averaging_objects(Duration::MAX).is_empty());
    assert_eq!(read(&db, P::ATTEMPTED_SAMPLES), PropertyValue::Unsigned(0));
}
