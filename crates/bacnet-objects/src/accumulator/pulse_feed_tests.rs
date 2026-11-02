//! Fed pulses step Present_Value through Prescale, modulo
//! Max_Pres_Value + 1 (Clauses 12.61.4, 12.61.13), and the rate window's
//! deadline always moves past the instant it was advanced to (#2259).

use super::AccumulatorObject;
use crate::traits::{BACnetObject, Clocks, LocalInput, LocalInputValue, Timed};
use alloc::sync::Arc;
use bacnet_types::constructed::BACnetPrescale;
use bacnet_types::enums::{EngineeringUnits, PropertyIdentifier as P};
use bacnet_types::primitives::PropertyValue;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use core::time::Duration;

fn accumulator() -> AccumulatorObject {
    AccumulatorObject::new(1, "ACC-1", EngineeringUnits::NO_UNITS).unwrap()
}

fn present_value(object: &AccumulatorObject) -> PropertyValue {
    object.read_property(P::PRESENT_VALUE, None).unwrap()
}

fn feed(object: &mut AccumulatorObject, pulses: u64) {
    object
        .apply_local_input(LocalInputValue::Pulses(pulses))
        .unwrap();
}

#[test]
fn without_prescale_each_pulse_is_one_count() {
    let mut object = accumulator();
    object.set_present_value(100);
    feed(&mut object, 25);
    feed(&mut object, 0);
    assert_eq!(present_value(&object), PropertyValue::Unsigned(125));
}

#[test]
fn prescale_steps_by_its_ratio_and_keeps_the_remainder() {
    // Two counts for every fifteen pulses: the remainder of a feed carries
    // into the next, so no pulse is lost however the feeds are split.
    let mut object = accumulator();
    object.set_prescale(BACnetPrescale {
        multiplier: 2,
        modulo_divide: 15,
    });
    feed(&mut object, 7); // 14 of 15: no step yet
    assert_eq!(present_value(&object), PropertyValue::Unsigned(0));
    feed(&mut object, 1); // 16: one step, 1 left
    assert_eq!(present_value(&object), PropertyValue::Unsigned(1));
    feed(&mut object, 82); // 1 + 164 = 165: eleven steps, none left
    assert_eq!(present_value(&object), PropertyValue::Unsigned(12));
    // The same 90 pulses in one feed land on the same value.
    let mut whole = accumulator();
    whole.set_prescale(BACnetPrescale {
        multiplier: 2,
        modulo_divide: 15,
    });
    feed(&mut whole, 90);
    assert_eq!(present_value(&whole), PropertyValue::Unsigned(12));
}

#[test]
fn a_prescale_without_a_divisor_leaves_present_value_alone() {
    let mut object = accumulator();
    object.set_prescale(BACnetPrescale {
        multiplier: 1,
        modulo_divide: 0,
    });
    feed(&mut object, 10);
    assert_eq!(present_value(&object), PropertyValue::Unsigned(0));
}

#[test]
fn present_value_wraps_past_max_pres_value() {
    let mut object = accumulator();
    object
        .write_property(P::MAX_PRES_VALUE, None, PropertyValue::Unsigned(999), None)
        .unwrap();
    object.set_present_value(995);
    feed(&mut object, 7); // 1002 modulo 1000
    assert_eq!(present_value(&object), PropertyValue::Unsigned(2));
    // A feed of many whole turns lands where the remainder says.
    feed(&mut object, 5_000 + 998);
    assert_eq!(present_value(&object), PropertyValue::Unsigned(0));
    // At the default maximum it wraps at 2^64, with the largest prescale.
    let mut widest = accumulator();
    widest.set_present_value(u64::MAX);
    feed(&mut widest, 2);
    assert_eq!(present_value(&widest), PropertyValue::Unsigned(1));
    widest.set_prescale(BACnetPrescale {
        multiplier: u32::MAX,
        modulo_divide: 1,
    });
    feed(&mut widest, u64::MAX);
    let expected = (1 + u128::from(u64::MAX) * u128::from(u32::MAX)) % (1u128 << 64);
    assert_eq!(
        present_value(&widest),
        PropertyValue::Unsigned(u64::try_from(expected).unwrap())
    );
}

#[test]
fn out_of_service_drops_the_pulses_for_present_value() {
    let mut object = accumulator();
    object.set_prescale(BACnetPrescale {
        multiplier: 1,
        modulo_divide: 2,
    });
    feed(&mut object, 1); // half a step held
    object.set_present_value(40);
    object
        .write_property(P::OUT_OF_SERVICE, None, PropertyValue::Boolean(true), None)
        .unwrap();
    feed(&mut object, 9);
    assert_eq!(present_value(&object), PropertyValue::Unsigned(40));
    // Back in service, counting resumes from the held value and remainder:
    // the out-of-service pulses aren't caught up.
    object
        .write_property(P::OUT_OF_SERVICE, None, PropertyValue::Boolean(false), None)
        .unwrap();
    feed(&mut object, 1);
    assert_eq!(present_value(&object), PropertyValue::Unsigned(41));
}

/// A hand-set monotonic clock and a waker that counts its calls.
fn clocks() -> (Clocks, Arc<AtomicU64>, Arc<AtomicUsize>) {
    let millis = Arc::new(AtomicU64::new(0));
    let wakes = Arc::new(AtomicUsize::new(0));
    let read = Arc::clone(&millis);
    let woken = Arc::clone(&wakes);
    let clocks = Clocks::default()
        .with_monotonic(Some(Arc::new(move || {
            Duration::from_millis(read.load(Ordering::Relaxed))
        })))
        .with_deadline_waker(Some(Arc::new(move || {
            woken.fetch_add(1, Ordering::Relaxed);
        })));
    (clocks, millis, wakes)
}

#[test]
fn a_bound_clock_opens_the_window_and_an_interval_write_wakes_the_host() {
    let (clocks, millis, wakes) = clocks();
    let mut object = accumulator();
    object.set_limit_monitoring_interval(10);
    millis.store(3_500, Ordering::Relaxed);
    object.bind_clocks(&clocks);
    assert_eq!(
        object.next_monotonic_deadline(),
        Some(Duration::from_millis(13_500))
    );
    // Binding the same clock again leaves the window where it is.
    millis.store(5_000, Ordering::Relaxed);
    object.bind_clocks(&clocks);
    assert_eq!(
        object.next_monotonic_deadline(),
        Some(Duration::from_millis(13_500))
    );
    object
        .write_property(
            P::LIMIT_MONITORING_INTERVAL,
            None,
            PropertyValue::Unsigned(4),
            None,
        )
        .unwrap();
    assert_eq!(
        object.next_monotonic_deadline(),
        Some(Duration::from_millis(9_000))
    );
    assert_eq!(wakes.load(Ordering::Relaxed), 1);
}

#[test]
fn a_new_clock_reopens_the_window_on_its_time_base_keeping_the_pulses() {
    let (first, millis, _wakes) = clocks();
    let mut object = accumulator();
    object.set_limit_monitoring_interval(10);
    millis.store(2_000, Ordering::Relaxed);
    object.bind_clocks(&first);
    assert_eq!(
        object.next_monotonic_deadline(),
        Some(Duration::from_secs(12))
    );
    feed(&mut object, 5);
    let (second, other, _) = clocks();
    other.store(100_000, Ordering::Relaxed);
    object.bind_clocks(&second);
    assert_eq!(
        object.next_monotonic_deadline(),
        Some(Duration::from_secs(110))
    );
    assert!(!object.advance_monotonic_time(Duration::from_secs(109)));
    assert!(object.advance_monotonic_time(Duration::from_secs(110)));
    assert_eq!(
        object.read_property(P::PULSE_RATE, None).unwrap(),
        PropertyValue::Unsigned(5)
    );
}

#[test]
fn the_deadline_always_moves_past_the_advance() {
    let (clocks, _millis, _wakes) = clocks();
    let mut object = accumulator();
    object.set_limit_monitoring_interval(3);
    object.bind_clocks(&clocks);
    let mut deadline = object.next_monotonic_deadline().unwrap();
    assert_eq!(deadline, Duration::from_secs(3));
    // Advance exactly at each deadline, a millisecond before one, and well
    // after one: the next deadline is always later than the instant passed.
    for now in [
        Duration::from_secs(3),
        Duration::from_secs(3),
        Duration::from_millis(5_999),
        Duration::from_secs(6),
        Duration::from_millis(16_250),
        Duration::from_secs(18),
    ] {
        object.advance_monotonic_time(now);
        let next = object.next_monotonic_deadline().unwrap();
        assert!(next > now, "deadline {next:?} not after {now:?}");
        assert!(next >= deadline, "deadline went back to {next:?}");
        deadline = next;
    }
    assert_eq!(deadline, Duration::from_secs(21));
}

#[test]
fn a_snapshot_reads_as_the_object_without_its_clocks() {
    let (clocks, _millis, _wakes) = clocks();
    let mut object = accumulator();
    object.bind_clocks(&clocks);
    feed(&mut object, 6);
    let snapshot = object.cov_snapshot().unwrap();
    assert_eq!(
        snapshot.read_property(P::PRESENT_VALUE, None).unwrap(),
        PropertyValue::Unsigned(6)
    );
}
