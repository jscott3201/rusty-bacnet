//! Adding an object queues a retry for each Schedule holding a refused
//! reference to it (#1440), found by asking the Schedules rather than
//! evaluating them; removing one, or adding one nothing refused, queues
//! nothing.

use super::*;
use crate::analog::AnalogValueObject;
use crate::schedule::{ScheduleObject, ScheduleTargetOutcome};
use crate::traits::BACnetObject;
use bacnet_types::calendar::SpecificDate;
use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::{EngineeringUnits, PropertyIdentifier};
use bacnet_types::primitives::{PropertyValue, Time};
use std::sync::atomic::{AtomicUsize, Ordering};

fn av(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, instance).unwrap()
}

fn sch1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::SCHEDULE, 1).unwrap()
}

/// SCH-1 commanding AV-1 and AV-9, after a pass in which AV-9 refused the
/// reference and AV-1 took the value.
fn refusing_schedule() -> Box<ScheduleObject> {
    let reference = |object| {
        BACnetObjectPropertyReference::new(object, PropertyIdentifier::PRESENT_VALUE.to_raw())
    };
    let mut schedule = ScheduleObject::new(1, "SCH-1", PropertyValue::Real(5.0)).unwrap();
    schedule
        .set_object_property_references(vec![reference(av(1)), reference(av(9))])
        .unwrap();
    let noon = Time {
        hour: 12,
        minute: 0,
        second: 0,
        hundredths: 0,
    };
    let monday = SpecificDate::new(2026, 10, 5).unwrap();
    let write = schedule
        .tick_schedule(monday, noon, &|_| false)
        .expect("the first pass writes");
    schedule.complete_schedule_write(
        &write,
        &[
            ScheduleTargetOutcome::Accepted,
            ScheduleTargetOutcome::ReferenceRefused,
        ],
    );
    Box::new(schedule)
}

#[test]
fn adding_a_refused_object_queues_its_retry_and_wakes_the_server() {
    let mut db = ObjectDatabase::new();
    let wakes = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&wakes);
    db.set_membership_waker_internal(Some(Arc::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    })));
    db.add(refusing_schedule()).unwrap();
    // AV-1 took the value and AV-2 is named nowhere: nothing to retry.
    for instance in [1, 2] {
        db.add(Box::new(
            AnalogValueObject::new(
                instance,
                format!("AV-{instance}"),
                EngineeringUnits::DEGREES_CELSIUS,
            )
            .unwrap(),
        ))
        .unwrap();
    }
    assert!(db.take_membership_work_internal().is_empty());
    assert_eq!(wakes.load(Ordering::SeqCst), 0);

    db.add(Box::new(
        AnalogValueObject::new(9, "AV-9", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
    let work = db.take_membership_work_internal();
    assert_eq!(work.schedule_retries, [(sch1(), av(9))]);
    assert!(work.changed.is_empty());

    // The database doesn't write: until the server retries, the refusal
    // stands, so a removal queues nothing and a new add queues it again.
    db.remove(&av(9)).unwrap();
    assert!(db.take_membership_work_internal().is_empty());
    db.add(Box::new(
        AnalogValueObject::new(9, "AV-9", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        AnalogValueObject::new(9, "AV-9", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    assert_eq!(
        db.take_membership_work_internal().schedule_retries,
        [(sch1(), av(9))],
        "queued once however often it is added"
    );
}
