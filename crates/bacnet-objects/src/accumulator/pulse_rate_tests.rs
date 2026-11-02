//! Pulse_Rate: an Unsigned count of the pulses in the last whole
//! Limit_Monitoring_Interval window (Clause 12.61.20, Table 12-79; #2251).

use super::AccumulatorObject;
use crate::traits::BACnetObject;
use bacnet_types::enums::{EngineeringUnits, ErrorClass, ErrorCode, PropertyIdentifier as P};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::PropertyValue;

fn accumulator(interval: u32) -> AccumulatorObject {
    let mut object = AccumulatorObject::new(1, "ACC-1", EngineeringUnits::NO_UNITS).unwrap();
    object.set_limit_monitoring_interval(interval);
    object
}

fn rate(object: &AccumulatorObject) -> PropertyValue {
    object.read_property(P::PULSE_RATE, None).unwrap()
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
    object.record_pulses(40);
    object.advance_rate_window(30);
    object.record_pulses(5);
    // Half a window: nothing is published yet.
    assert_eq!(rate(&object), PropertyValue::Unsigned(0));
    object.advance_rate_window(29);
    assert_eq!(rate(&object), PropertyValue::Unsigned(0));
    object.advance_rate_window(1);
    assert_eq!(rate(&object), PropertyValue::Unsigned(45));
    // The next window starts empty, and a quiet window reads zero.
    object.advance_rate_window(60);
    assert_eq!(rate(&object), PropertyValue::Unsigned(0));
}

#[test]
fn a_late_call_closes_one_window_and_carries_the_remainder() {
    let mut object = accumulator(10);
    object.record_pulses(7);
    // 25 seconds: one window closes with every pulse counted, and the 5
    // seconds past the last whole interval start the next one.
    object.advance_rate_window(25);
    assert_eq!(rate(&object), PropertyValue::Unsigned(7));
    object.record_pulses(3);
    object.advance_rate_window(4);
    assert_eq!(rate(&object), PropertyValue::Unsigned(7));
    object.advance_rate_window(1);
    assert_eq!(rate(&object), PropertyValue::Unsigned(3));
    // Counting saturates rather than wrapping, and so does the clock.
    object.record_pulses(u64::MAX);
    object.record_pulses(1);
    object.advance_rate_window(u32::MAX);
    assert_eq!(rate(&object), PropertyValue::Unsigned(u64::MAX));
}

#[test]
fn without_an_interval_no_window_closes() {
    let mut object = accumulator(0);
    object.set_pulse_rate(9);
    object.record_pulses(100);
    object.advance_rate_window(u32::MAX);
    assert_eq!(rate(&object), PropertyValue::Unsigned(9));
}

#[test]
fn writing_the_interval_starts_a_new_window() {
    let mut object = accumulator(60);
    object.record_pulses(50);
    object.advance_rate_window(50);
    object
        .write_property(
            P::LIMIT_MONITORING_INTERVAL,
            None,
            PropertyValue::Unsigned(20),
            None,
        )
        .unwrap();
    // The 50 pulses and 50 seconds belonged to the old window.
    object.advance_rate_window(19);
    assert_eq!(rate(&object), PropertyValue::Unsigned(0));
    object.record_pulses(4);
    object.advance_rate_window(1);
    assert_eq!(rate(&object), PropertyValue::Unsigned(4));
}

#[test]
fn out_of_service_keeps_the_clients_rate() {
    let mut object = accumulator(10);
    set_out_of_service(&mut object, true);
    object
        .write_property(P::PULSE_RATE, None, PropertyValue::Unsigned(500), None)
        .unwrap();
    object.record_pulses(3);
    object.advance_rate_window(10);
    object.set_pulse_rate(8);
    assert_eq!(rate(&object), PropertyValue::Unsigned(500));
    // Back in service, the next window to close sets it again.
    set_out_of_service(&mut object, false);
    assert_eq!(rate(&object), PropertyValue::Unsigned(500));
    object.record_pulses(2);
    object.advance_rate_window(10);
    assert_eq!(rate(&object), PropertyValue::Unsigned(2));
}

#[test]
fn writes_take_an_unsigned_and_refuse_a_real() {
    let mut object = accumulator(0);
    object
        .write_property(P::PULSE_RATE, None, PropertyValue::Unsigned(12), None)
        .unwrap();
    assert_eq!(rate(&object), PropertyValue::Unsigned(12));
    for value in [PropertyValue::Real(12.0), PropertyValue::Double(12.0)] {
        let error = object
            .write_property(P::PULSE_RATE, None, value, None)
            .unwrap_err();
        assert!(
            matches!(error, Error::Remote(RemoteError::Error { class, code })
                if class == ErrorClass::PROPERTY && code == ErrorCode::INVALID_DATA_TYPE),
            "{error:?}"
        );
    }
    assert_eq!(rate(&object), PropertyValue::Unsigned(12));
}
