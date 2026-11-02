//! Pulse_Rate: an Unsigned count of the pulses in the last whole
//! Limit_Monitoring_Interval window (Clause 12.61.20, Table 12-79; #2251).

use super::AccumulatorObject;
use crate::traits::{BACnetObject, LocalInput, LocalInputValue, Timed};
use bacnet_types::enums::{EngineeringUnits, ErrorClass, ErrorCode, PropertyIdentifier as P};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::PropertyValue;
use core::time::Duration;

/// An Accumulator with this interval whose window opened at second 0.
fn accumulator(interval: u32) -> AccumulatorObject {
    let mut object = AccumulatorObject::new(1, "ACC-1", EngineeringUnits::NO_UNITS).unwrap();
    object.set_limit_monitoring_interval(interval);
    at(&mut object, 0);
    object
}

fn rate(object: &AccumulatorObject) -> PropertyValue {
    object.read_property(P::PULSE_RATE, None).unwrap()
}

fn feed(object: &mut AccumulatorObject, pulses: u64) {
    object
        .apply_local_input(LocalInputValue::Pulses(pulses))
        .unwrap();
}

/// Advance the object's monotonic time to `seconds`.
fn at(object: &mut AccumulatorObject, seconds: u64) -> bool {
    object.advance_monotonic_time(Duration::from_secs(seconds))
}

fn set_out_of_service(object: &mut AccumulatorObject, value: bool) {
    object
        .write_property(P::OUT_OF_SERVICE, None, PropertyValue::Boolean(value), None)
        .unwrap();
}

#[test]
fn pulse_rate_is_served_as_an_unsigned() {
    let mut object = accumulator(0);
    assert_eq!(rate(&object), PropertyValue::Unsigned(0));
    object.set_pulse_rate(u64::MAX);
    assert_eq!(rate(&object), PropertyValue::Unsigned(u64::MAX));
}

#[test]
fn a_rate_is_the_count_of_one_whole_window() {
    let mut object = accumulator(60);
    feed(&mut object, 40);
    at(&mut object, 30);
    feed(&mut object, 5);
    // Half a window: nothing is published yet.
    assert_eq!(rate(&object), PropertyValue::Unsigned(0));
    assert!(!at(&mut object, 59));
    assert_eq!(rate(&object), PropertyValue::Unsigned(0));
    assert!(at(&mut object, 60));
    assert_eq!(rate(&object), PropertyValue::Unsigned(45));
    // The next window starts empty, and a quiet window reads zero.
    assert!(at(&mut object, 120));
    assert_eq!(rate(&object), PropertyValue::Unsigned(0));
}

#[test]
fn a_late_call_closes_one_window_and_carries_the_remainder() {
    let mut object = accumulator(10);
    feed(&mut object, 7);
    // At 25 seconds one window closes with every pulse counted, and the 5
    // seconds past the last whole interval belong to the next one.
    at(&mut object, 25);
    assert_eq!(rate(&object), PropertyValue::Unsigned(7));
    feed(&mut object, 3);
    at(&mut object, 29);
    assert_eq!(rate(&object), PropertyValue::Unsigned(7));
    at(&mut object, 30);
    assert_eq!(rate(&object), PropertyValue::Unsigned(3));
    // Counting saturates rather than wrapping.
    feed(&mut object, u64::MAX);
    feed(&mut object, 1);
    at(&mut object, 40);
    assert_eq!(rate(&object), PropertyValue::Unsigned(u64::MAX));
}

#[test]
fn without_an_interval_no_window_closes() {
    let mut object = accumulator(0);
    object.set_pulse_rate(9);
    feed(&mut object, 100);
    at(&mut object, u64::MAX);
    assert_eq!(rate(&object), PropertyValue::Unsigned(9));
    assert_eq!(object.next_monotonic_deadline(), None);
}

#[test]
fn writing_the_interval_starts_a_new_window() {
    let mut object = accumulator(60);
    feed(&mut object, 50);
    at(&mut object, 50);
    object
        .write_property(
            P::LIMIT_MONITORING_INTERVAL,
            None,
            PropertyValue::Unsigned(20),
            None,
        )
        .unwrap();
    // The 50 pulses belonged to the old window; with no clock bound the
    // new one opens at the next advance.
    at(&mut object, 50);
    at(&mut object, 69);
    assert_eq!(rate(&object), PropertyValue::Unsigned(0));
    feed(&mut object, 4);
    at(&mut object, 70);
    assert_eq!(rate(&object), PropertyValue::Unsigned(4));
}

#[test]
fn out_of_service_keeps_the_clients_rate() {
    let mut object = accumulator(10);
    set_out_of_service(&mut object, true);
    object
        .write_property(P::PULSE_RATE, None, PropertyValue::Unsigned(500), None)
        .unwrap();
    feed(&mut object, 3);
    assert!(!at(&mut object, 10));
    object.set_pulse_rate(8);
    assert_eq!(rate(&object), PropertyValue::Unsigned(500));
    // Back in service, the next window to close sets it again.
    set_out_of_service(&mut object, false);
    assert_eq!(rate(&object), PropertyValue::Unsigned(500));
    feed(&mut object, 2);
    assert!(at(&mut object, 20));
    assert_eq!(rate(&object), PropertyValue::Unsigned(2));
}

fn assert_property_error(error: Error, expected: ErrorCode) {
    assert!(
        matches!(error, Error::Remote(RemoteError::Error { class, code })
            if class == ErrorClass::PROPERTY && code == expected),
        "{error:?}"
    );
}

#[test]
fn in_service_writes_are_denied() {
    // Writable only while Out_Of_Service is TRUE (Clause 12.61.20), whatever
    // the value's datatype.
    let mut object = accumulator(0);
    object.set_pulse_rate(3);
    for value in [PropertyValue::Unsigned(12), PropertyValue::Real(12.0)] {
        assert_property_error(
            object
                .write_property(P::PULSE_RATE, None, value, None)
                .unwrap_err(),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
    }
    assert_eq!(rate(&object), PropertyValue::Unsigned(3));
    let row = object
        .property_metadata()
        .iter()
        .find(|row| row.property_identifier == P::PULSE_RATE)
        .copied()
        .unwrap();
    assert_eq!(
        row.write_capability,
        crate::property_metadata::PropertyWriteCapability::WhenOutOfService
    );
}

#[test]
fn writes_take_an_unsigned_and_refuse_a_real() {
    let mut object = accumulator(0);
    set_out_of_service(&mut object, true);
    object
        .write_property(P::PULSE_RATE, None, PropertyValue::Unsigned(12), None)
        .unwrap();
    assert_eq!(rate(&object), PropertyValue::Unsigned(12));
    for value in [PropertyValue::Real(12.0), PropertyValue::Double(12.0)] {
        assert_property_error(
            object
                .write_property(P::PULSE_RATE, None, value, None)
                .unwrap_err(),
            ErrorCode::INVALID_DATA_TYPE,
        );
    }
    assert_eq!(rate(&object), PropertyValue::Unsigned(12));
}
