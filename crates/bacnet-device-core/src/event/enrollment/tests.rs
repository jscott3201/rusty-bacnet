//! The moved algorithms on their own (#2319): each event algorithm both
//! ways, the fault algorithms, the parameter encoders and the reference's
//! fingerprint, under `alloc` with no server.

use alloc::vec;

use bacnet_types::constructed::{BACnetPropertyStates, ChangeOfValueCriteria};
use bacnet_types::enums::{ErrorClass, ErrorCode, EventState, EventType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::algorithms::{self, PropertyStateValue};
use super::fault::{FaultAlgorithmEvaluation, SupportedFaultAlgorithm, evaluate_fault_algorithm};
use super::reference::MonitoredReference;
use super::{LocalConfigurationReadError, classify_required_property_read_error};

fn target(evaluation: algorithms::ArmEvaluation) -> Option<EventState> {
    evaluation.indication.map(|indication| indication.target)
}

#[test]
fn out_of_range_goes_over_and_back_with_its_deadband() {
    let eval = |value, current| {
        target(algorithms::eval_out_of_range_struct(
            0.0, 80.0, 1.0, value, current,
        ))
    };
    assert_eq!(eval(90.0, EventState::NORMAL), Some(EventState::HIGH_LIMIT));
    assert_eq!(eval(-5.0, EventState::NORMAL), Some(EventState::LOW_LIMIT));
    assert_eq!(eval(50.0, EventState::NORMAL), None);
    // Back only past the deadband.
    assert_eq!(eval(79.5, EventState::HIGH_LIMIT), None);
    assert_eq!(eval(78.0, EventState::HIGH_LIMIT), Some(EventState::NORMAL));
}

#[test]
fn floating_limit_follows_its_setpoint() {
    let eval = |value, current| {
        target(algorithms::eval_floating_limit_struct(
            20.0, 5.0, 5.0, 1.0, value, current,
        ))
    };
    assert_eq!(eval(26.0, EventState::NORMAL), Some(EventState::HIGH_LIMIT));
    assert_eq!(eval(14.0, EventState::NORMAL), Some(EventState::LOW_LIMIT));
    assert_eq!(eval(20.0, EventState::NORMAL), None);
    assert_eq!(eval(23.0, EventState::HIGH_LIMIT), Some(EventState::NORMAL));
}

#[test]
fn change_of_state_matches_an_alarm_value_and_returns() {
    let alarms = [BACnetPropertyStates::BooleanValue(true)];
    let eval = |value, current| {
        let value = PropertyStateValue::Boolean(value);
        target(algorithms::eval_change_of_state_struct(
            &alarms, value, current, None,
        ))
    };
    assert_eq!(eval(true, EventState::NORMAL), Some(EventState::OFFNORMAL));
    assert_eq!(eval(false, EventState::OFFNORMAL), Some(EventState::NORMAL));
    assert_eq!(eval(false, EventState::NORMAL), None);
}

#[test]
fn change_of_bitstring_matches_under_its_mask() {
    let mask = (4, vec![0xF0]);
    let alarms = [(4, vec![0xA0])];
    let eval = |bits: &[u8], current| {
        target(algorithms::eval_change_of_bitstring_struct(
            &mask, &alarms, bits, current,
        ))
    };
    assert_eq!(
        eval(&[0xA5], EventState::NORMAL),
        Some(EventState::OFFNORMAL)
    );
    assert_eq!(
        eval(&[0x55], EventState::OFFNORMAL),
        Some(EventState::NORMAL)
    );
    assert_eq!(eval(&[0x50], EventState::NORMAL), None);
}

#[test]
fn change_of_value_takes_a_baseline_then_indicates_past_its_increment() {
    let criteria = ChangeOfValueCriteria::ReferencedPropertyIncrement(2.0);
    let first = PropertyValue::Real(10.0);
    let evaluation =
        algorithms::eval_change_of_value_struct(&criteria, &first, None, EventState::NORMAL)
            .expect("a REAL is evaluable");
    assert!(
        evaluation.indication.is_none(),
        "the first sample is the baseline"
    );
    assert_eq!(evaluation.establish_baseline, Some(first.clone()));
    let small = PropertyValue::Real(11.0);
    let evaluation = algorithms::eval_change_of_value_struct(
        &criteria,
        &small,
        Some(&first),
        EventState::NORMAL,
    )
    .unwrap();
    assert!(evaluation.indication.is_none());
    let large = PropertyValue::Real(13.0);
    let evaluation = algorithms::eval_change_of_value_struct(
        &criteria,
        &large,
        Some(&first),
        EventState::NORMAL,
    )
    .unwrap();
    assert_eq!(
        target(evaluation),
        Some(EventState::NORMAL),
        "a same-state indication"
    );
}

#[test]
fn the_none_algorithm_never_indicates_and_values_extract_by_type() {
    assert_eq!(target(algorithms::eval_none_arm(EventState::NORMAL)), None);
    assert_eq!(
        algorithms::extract_real(&PropertyValue::Real(1.5)),
        Some(1.5)
    );
    assert_eq!(
        algorithms::extract_enumerated(&PropertyValue::Enumerated(3)),
        Some(3)
    );
    assert_eq!(
        algorithms::extract_real(&PropertyValue::Boolean(true)),
        None
    );
    let bits = PropertyValue::BitString {
        unused_bits: 4,
        data: vec![0xF0],
    };
    assert_eq!(algorithms::extract_bitstring(&bits), Some(vec![0xF0]));
}

/// The legacy byte-layout encoders, for the `Opaque` path, each give
/// something.
#[test]
fn the_legacy_parameter_encoders_encode_each_form() {
    for (bytes, event_type) in [
        (
            algorithms::encode_out_of_range_params(80.0, 0.0, 1.0),
            EventType::OUT_OF_RANGE,
        ),
        (
            algorithms::encode_change_of_state_params(&[1, 2]),
            EventType::CHANGE_OF_STATE,
        ),
        (
            algorithms::encode_change_of_value_params(0.5),
            EventType::CHANGE_OF_VALUE,
        ),
        (
            algorithms::encode_floating_limit_params(20.0, 5.0, 5.0, 1.0),
            EventType::FLOATING_LIMIT,
        ),
        (
            algorithms::encode_change_of_bitstring_params(&[0xF0], &[0xA0]),
            EventType::CHANGE_OF_BITSTRING,
        ),
    ] {
        assert!(!bytes.is_empty(), "{event_type:?}");
    }
}

#[test]
fn fault_algorithms_judge_a_numeric_range() {
    let db = bacnet_objects::database::ObjectDatabase::new();
    let none = SupportedFaultAlgorithm::None;
    assert_eq!(
        evaluate_fault_algorithm(&db, &none, None),
        FaultAlgorithmEvaluation::Healthy
    );
    let range = SupportedFaultAlgorithm::OutOfRange {
        min_normal: 0.0,
        max_normal: 100.0,
    };
    let at = |value: f32| evaluate_fault_algorithm(&db, &range, Some(&PropertyValue::Real(value)));
    assert_eq!(at(50.0), FaultAlgorithmEvaluation::Healthy);
    assert!(
        matches!(at(150.0), FaultAlgorithmEvaluation::Fault(_)),
        "{:?}",
        at(150.0)
    );
    assert!(
        matches!(at(-1.0), FaultAlgorithmEvaluation::Fault(_)),
        "{:?}",
        at(-1.0)
    );
}

#[test]
fn a_missing_required_property_is_malformed_and_any_other_failure_waits() {
    let unknown = Error::protocol(ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY);
    assert_eq!(
        classify_required_property_read_error(&unknown),
        LocalConfigurationReadError::Malformed
    );
    let busy = Error::protocol(ErrorClass::DEVICE, ErrorCode::DEVICE_BUSY);
    assert_eq!(
        classify_required_property_read_error(&busy),
        LocalConfigurationReadError::Unavailable
    );
}

#[test]
fn the_fingerprint_follows_the_parameters_and_the_reference() {
    let ai = ObjectIdentifier::new(bacnet_types::enums::ObjectType::ANALOG_INPUT, 1).unwrap();
    let pv = PropertyIdentifier::PRESENT_VALUE;
    let reference = MonitoredReference::local(ai, pv, None);
    let params = |high_limit| bacnet_types::constructed::BACnetEventParameter::OutOfRange {
        time_delay: 1,
        low_limit: 0.0,
        high_limit,
        deadband: 1.0,
    };
    let out = EventType::OUT_OF_RANGE;
    let first = super::reference::params_fingerprint(&params(80.0), 1, out, &reference).unwrap();
    let again = super::reference::params_fingerprint(&params(80.0), 1, out, &reference).unwrap();
    let other = super::reference::params_fingerprint(&params(90.0), 1, out, &reference).unwrap();
    assert_eq!(first, again);
    assert_ne!(first, other);
}
