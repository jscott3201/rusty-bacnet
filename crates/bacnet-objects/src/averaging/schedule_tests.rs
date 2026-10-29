//! The schedule an Averaging object keeps for the server's samples (Clauses
//! 12.5.14 and 12.5.15, #1144): one sample every Window_Interval /
//! Window_Samples, restarted by everything that empties the window.
use super::*;
use std::sync::Mutex;

use PropertyIdentifier as P;

/// A monotonic clock the test moves by hand.
fn clock() -> (Arc<Mutex<Duration>>, Arc<MonotonicClock>) {
    let time = Arc::new(Mutex::new(Duration::ZERO));
    let source = Arc::clone(&time);
    (time, Arc::new(move || *source.lock().unwrap()))
}

fn reference() -> BACnetObjectPropertyReference {
    BACnetObjectPropertyReference::new(
        ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap(),
        P::PRESENT_VALUE.to_raw(),
    )
}

/// AVG-1 averaging AV-1's Present_Value over 10 s in 5 samples: one every 2 s.
fn sampled(time: &Arc<Mutex<Duration>>, clock: Arc<MonotonicClock>) -> AveragingObject {
    *time.lock().unwrap() = Duration::ZERO;
    let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
    avg.set_window_interval(10).unwrap();
    avg.set_window_samples(5).unwrap();
    avg.set_object_property_reference(Some(reference()));
    avg.bind_monotonic_clock_internal(Some(clock));
    avg
}

fn secs(seconds: f64) -> Duration {
    Duration::from_secs_f64(seconds)
}

#[test]
fn averaging_schedule_spaces_samples_by_window_interval_over_window_samples() {
    let (time, clock) = clock();
    let mut avg = sampled(&time, clock);
    // The first sample is one spacing after the clock is bound.
    assert_eq!(avg.next_monotonic_deadline_internal(), Some(secs(2.0)));
    assert_eq!(avg.take_due_averaging_sample_internal(secs(1.999)), None);
    assert_eq!(
        avg.take_due_averaging_sample_internal(secs(2.0)),
        Some(reference())
    );
    // Claimed once: the same instant has nothing more.
    assert_eq!(avg.take_due_averaging_sample_internal(secs(2.0)), None);
    assert_eq!(avg.next_monotonic_deadline_internal(), Some(secs(4.0)));

    // A pass under a period late keeps the cadence.
    assert!(avg.take_due_averaging_sample_internal(secs(4.5)).is_some());
    assert_eq!(avg.next_monotonic_deadline_internal(), Some(secs(6.0)));
    // A pass a period or more late takes one sample, not a burst, and starts
    // the cadence over from then.
    assert!(avg.take_due_averaging_sample_internal(secs(11.0)).is_some());
    assert_eq!(avg.next_monotonic_deadline_internal(), Some(secs(13.0)));
    assert_eq!(avg.take_due_averaging_sample_internal(secs(12.9)), None);

    // Claiming a sample doesn't record one: that is the database's job.
    assert_eq!(
        avg.read_property(P::ATTEMPTED_SAMPLES, None).unwrap(),
        PropertyValue::Unsigned(0)
    );
}

#[test]
fn averaging_schedule_restarts_on_every_route_that_empties_the_window() {
    let (time, clock) = clock();
    type Reset = fn(&mut AveragingObject);
    let resets: [(&str, Reset); 7] = [
        ("Window_Interval write", |avg| {
            avg.write_property(P::WINDOW_INTERVAL, None, PropertyValue::Unsigned(10), None)
                .unwrap()
        }),
        ("Window_Samples write", |avg| {
            avg.write_property(P::WINDOW_SAMPLES, None, PropertyValue::Unsigned(5), None)
                .unwrap()
        }),
        ("Attempted_Samples write of zero", |avg| {
            avg.write_property(P::ATTEMPTED_SAMPLES, None, PropertyValue::Unsigned(0), None)
                .unwrap()
        }),
        ("Object_Property_Reference write", |avg| {
            let value = avg
                .read_property(P::OBJECT_PROPERTY_REFERENCE, None)
                .unwrap();
            avg.write_property(P::OBJECT_PROPERTY_REFERENCE, None, value, None)
                .unwrap()
        }),
        ("set_window_interval", |avg| {
            avg.set_window_interval(10).unwrap()
        }),
        ("set_window_samples", |avg| {
            avg.set_window_samples(5).unwrap()
        }),
        ("set_object_property_reference", |avg| {
            avg.set_object_property_reference(Some(reference()))
        }),
    ];
    for (route, reset) in resets {
        let mut avg = sampled(&time, Arc::clone(&clock));
        avg.add_sample(1.0).unwrap();
        // Three quarters of the way to the first sample, the same values are
        // written back: the window empties and the schedule starts over.
        *time.lock().unwrap() = secs(1.5);
        reset(&mut avg);
        assert_eq!(
            avg.read_property(P::ATTEMPTED_SAMPLES, None).unwrap(),
            PropertyValue::Unsigned(0),
            "{route}"
        );
        assert_eq!(
            avg.next_monotonic_deadline_internal(),
            Some(secs(3.5)),
            "{route}"
        );
        assert_eq!(
            avg.take_due_averaging_sample_internal(secs(2.0)),
            None,
            "{route}"
        );
        assert!(
            avg.take_due_averaging_sample_internal(secs(3.5)).is_some(),
            "{route}"
        );
    }

    // A refused write changes neither the window nor the schedule.
    let mut avg = sampled(&time, clock);
    *time.lock().unwrap() = secs(1.5);
    assert!(
        avg.write_property(P::WINDOW_SAMPLES, None, PropertyValue::Unsigned(0), None)
            .is_err()
    );
    assert_eq!(avg.next_monotonic_deadline_internal(), Some(secs(2.0)));
}

#[test]
fn averaging_schedule_follows_new_window_spacing_and_clamps_it() {
    let (time, clock) = clock();
    let mut avg = sampled(&time, clock);
    *time.lock().unwrap() = secs(1.0);
    // 60 s over 4 samples, from the write at 1 s.
    avg.set_window_interval(60).unwrap();
    avg.set_window_samples(4).unwrap();
    assert_eq!(avg.next_monotonic_deadline_internal(), Some(secs(16.0)));

    // One second over 1,440 samples would be under a millisecond apart; the
    // spacing is stretched to the floor.
    avg.set_window_interval(1).unwrap();
    avg.set_window_samples(MAX_WINDOW_SAMPLES).unwrap();
    assert_eq!(
        avg.next_monotonic_deadline_internal(),
        Some(secs(1.0) + MIN_SAMPLE_PERIOD)
    );
}

#[test]
fn averaging_schedule_needs_a_reference_and_a_clock() {
    let (time, clock) = clock();
    // No clock bound: never due, whatever the reference.
    let mut avg = AveragingObject::new(1, "AVG-1").unwrap();
    avg.set_object_property_reference(Some(reference()));
    assert_eq!(avg.next_monotonic_deadline_internal(), None);
    assert_eq!(avg.take_due_averaging_sample_internal(Duration::MAX), None);

    // Bound but without a reference: the application feeds this object.
    let mut avg = sampled(&time, Arc::clone(&clock));
    avg.set_object_property_reference(None);
    assert_eq!(avg.next_monotonic_deadline_internal(), None);
    assert_eq!(avg.take_due_averaging_sample_internal(Duration::MAX), None);

    // Unbinding the clock stops the schedule.
    let mut avg = sampled(&time, clock);
    avg.bind_monotonic_clock_internal(None);
    assert_eq!(avg.next_monotonic_deadline_internal(), None);
    assert_eq!(avg.take_due_averaging_sample_internal(Duration::MAX), None);
}

#[test]
fn averaging_application_samples_leave_the_schedule_alone() {
    let (time, clock) = clock();
    let mut avg = sampled(&time, clock);
    *time.lock().unwrap() = secs(1.0);
    avg.add_averaging_sample_internal(Some(PropertyValue::Real(4.0)))
        .unwrap();
    avg.add_averaging_sample_internal(None).unwrap();
    assert_eq!(avg.next_monotonic_deadline_internal(), Some(secs(2.0)));
    assert_eq!(
        avg.read_property(P::ATTEMPTED_SAMPLES, None).unwrap(),
        PropertyValue::Unsigned(2)
    );
}
