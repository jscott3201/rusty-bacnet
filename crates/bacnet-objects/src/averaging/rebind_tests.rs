//! An Averaging object restarts its sample schedule when its monotonic
//! clock changes, but not when a database rebinds its clocks for another
//! clock's sake (#2179).

use super::*;
use crate::clock::{ClockFrame, ClockReader};
use crate::database::ObjectDatabase;
use crate::traits::MonotonicClock;
use alloc::sync::Arc;
use std::sync::Mutex;

struct NoWallClock;

impl ClockReader for NoWallClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        None
    }
}

/// A monotonic clock the test moves by hand.
fn clock() -> (Arc<Mutex<Duration>>, Arc<MonotonicClock>) {
    let time = Arc::new(Mutex::new(Duration::ZERO));
    let source = Arc::clone(&time);
    (time, Arc::new(move || *source.lock().unwrap()))
}

/// AVG-1 sampling every 2 s (10 s over 5 samples), in a database on `clock`.
fn sampled(clock: Arc<MonotonicClock>) -> (ObjectDatabase, ObjectIdentifier) {
    let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
    avg.set_window_interval(10).unwrap();
    avg.set_window_samples(5).unwrap();
    avg.set_object_property_reference(Some(BACnetObjectPropertyReference::new(
        ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap(),
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    let oid = avg.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(avg)).unwrap();
    db.set_monotonic_clock(Some(clock));
    (db, oid)
}

fn next_sample(db: &mut ObjectDatabase, oid: ObjectIdentifier) -> Option<Duration> {
    let object = db.get_mut(&oid).unwrap();
    object.timed().unwrap().next_monotonic_deadline()
}

#[test]
fn binding_the_wall_clock_or_the_waker_keeps_the_sample_schedule() {
    let (time, monotonic) = clock();
    let (mut db, oid) = sampled(monotonic);
    assert_eq!(next_sample(&mut db, oid), Some(Duration::from_secs(2)));
    *time.lock().unwrap() = Duration::from_secs(1);
    db.set_clock_reader(Some(Arc::new(NoWallClock)));
    db.set_deadline_waker(Some(Arc::new(|| {})));
    assert_eq!(next_sample(&mut db, oid), Some(Duration::from_secs(2)));
}

#[test]
fn a_new_monotonic_clock_restarts_the_sample_schedule() {
    let (time, monotonic) = clock();
    let (mut db, oid) = sampled(monotonic);
    *time.lock().unwrap() = Duration::from_secs(1);
    let (other_time, other) = clock();
    *other_time.lock().unwrap() = Duration::from_secs(5);
    db.set_monotonic_clock(Some(other));
    assert_eq!(next_sample(&mut db, oid), Some(Duration::from_secs(7)));
}
