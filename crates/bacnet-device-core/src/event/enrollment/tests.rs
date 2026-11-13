//! The moved algorithms on their own (#2319): each event algorithm both
//! ways, the fault algorithms, the parameter encoders and the reference's
//! fingerprint, under `alloc` with no server.

use alloc::boxed::Box;
use alloc::vec;

use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::traits::BACnetObject;

use bacnet_types::constructed::{BACnetPropertyStates, ChangeOfValueCriteria};
use bacnet_types::enums::{
    EngineeringUnits, ErrorClass, ErrorCode, EventState, EventType, ObjectType, PropertyIdentifier,
    Reliability,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::algorithms::{self, PropertyStateValue};
use super::fault::{
    FaultAlgorithmEvaluation, MonitoredReliability, SupportedFaultAlgorithm,
    evaluate_fault_algorithm, read_event_parameters, read_fault_algorithm,
    read_monitored_reliability,
};
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
fn the_none_algorithm_returns_an_offnormal_state_to_normal_only() {
    let none = |state| target(algorithms::eval_none_arm(state));
    assert_eq!(none(EventState::NORMAL), None);
    assert_eq!(
        none(EventState::FAULT),
        None,
        "FAULT is the fault algorithm's"
    );
    for state in [
        EventState::OFFNORMAL,
        EventState::HIGH_LIMIT,
        EventState::LOW_LIMIT,
    ] {
        assert_eq!(none(state), Some(EventState::NORMAL), "{state:?}");
    }
}

#[test]
fn values_extract_by_type() {
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

/// The legacy byte-layout encoders, for the `Opaque` path: little-endian
/// REALs and counts, as the byte evaluators read them.
#[test]
fn the_legacy_parameter_encoders_write_their_layout() {
    let le = |value: f32| value.to_le_bytes();
    let out_of_range = [le(80.0), le(0.0), le(1.0)].concat();
    assert_eq!(
        algorithms::encode_out_of_range_params(80.0, 0.0, 1.0),
        out_of_range
    );
    let floating = [le(20.0), le(5.0), le(4.0), le(1.0)].concat();
    assert_eq!(
        algorithms::encode_floating_limit_params(20.0, 5.0, 4.0, 1.0),
        floating
    );
    let states = [2u32.to_le_bytes(), 1u32.to_le_bytes(), 7u32.to_le_bytes()].concat();
    assert_eq!(algorithms::encode_change_of_state_params(&[1, 7]), states);
    assert_eq!(
        algorithms::encode_change_of_value_params(0.5),
        le(0.5).to_vec()
    );
    // The shorter of the two lengths, then the mask, then the alarm bits.
    let bits = [1u32.to_le_bytes().as_slice(), &[0xF0], &[0xA0]].concat();
    assert_eq!(
        algorithms::encode_change_of_bitstring_params(&[0xF0, 0x0F], &[0xA0]),
        bits
    );
}

#[test]
fn fault_out_of_range_names_the_side() {
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
    let at = |value: &PropertyValue| evaluate_fault_algorithm(&db, &range, Some(value));
    assert_eq!(
        at(&PropertyValue::Real(50.0)),
        FaultAlgorithmEvaluation::Healthy
    );
    let over = FaultAlgorithmEvaluation::Fault(Reliability::OVER_RANGE);
    let under = FaultAlgorithmEvaluation::Fault(Reliability::UNDER_RANGE);
    assert_eq!(at(&PropertyValue::Real(150.0)), over);
    assert_eq!(at(&PropertyValue::Signed(-1)), under);
    assert_eq!(at(&PropertyValue::Unsigned(101)), over);
    let nan = PropertyValue::Real(f32::NAN);
    assert_eq!(at(&nan), FaultAlgorithmEvaluation::ConfigurationError);
    assert_eq!(
        evaluate_fault_algorithm(&db, &range, None),
        FaultAlgorithmEvaluation::ConfigurationError
    );
}

/// AI-1, its Reliability `reliability` when given one.
fn input(reliability: Option<Reliability>) -> AnalogInputObject {
    let mut input = AnalogInputObject::new(1, "AI-1", EngineeringUnits::NO_UNITS).unwrap();
    if let Some(reliability) = reliability {
        let eval = input.reliability_eval().unwrap();
        eval.set_reliability(reliability).unwrap();
    }
    input
}

#[test]
fn fault_status_flags_reads_the_referenced_objects_fault_bit() {
    let algorithm = |object_identifier| SupportedFaultAlgorithm::StatusFlags { object_identifier };
    let ai = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let mut db = ObjectDatabase::new();
    assert_eq!(
        evaluate_fault_algorithm(&db, &algorithm(ai), None),
        FaultAlgorithmEvaluation::ObservationUnavailable,
        "no such object"
    );
    db.add(Box::new(input(None))).unwrap();
    assert_eq!(
        evaluate_fault_algorithm(&db, &algorithm(ai), None),
        FaultAlgorithmEvaluation::Healthy
    );
    let mut db = ObjectDatabase::new();
    db.add(Box::new(input(Some(Reliability::NO_SENSOR))))
        .unwrap();
    assert_eq!(
        evaluate_fault_algorithm(&db, &algorithm(ai), None),
        FaultAlgorithmEvaluation::Fault(Reliability::MEMBER_FAULT)
    );
}

#[test]
fn the_reads_classify_what_an_object_lacks() {
    let sensor = input(Some(Reliability::NO_SENSOR));
    assert_eq!(
        read_monitored_reliability(&sensor),
        MonitoredReliability::Value(Reliability::NO_SENSOR)
    );
    // An Analog Input has no Event_Parameters or Object_Property_Reference.
    assert_eq!(
        read_event_parameters(&sensor),
        Err(LocalConfigurationReadError::Malformed)
    );
    assert_eq!(
        super::reference::read_object_property_ref(&sensor),
        Err(LocalConfigurationReadError::Malformed)
    );
    // An Event Enrollment without Fault_Parameters runs FAULT_NONE.
    let db = ObjectDatabase::new();
    let enrollment = EventEnrollmentObject::new(1, "EE-1", EventType::OUT_OF_RANGE).unwrap();
    assert_eq!(
        read_fault_algorithm(&enrollment, db.local_device()),
        Ok(SupportedFaultAlgorithm::None)
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
fn the_fingerprint_follows_each_input_but_the_device() {
    let ai = |instance| ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap();
    let pv = PropertyIdentifier::PRESENT_VALUE;
    let params = |high_limit| bacnet_types::constructed::BACnetEventParameter::OutOfRange {
        time_delay: 1,
        low_limit: 0.0,
        high_limit,
        deadband: 1.0,
    };
    let base = (
        params(80.0),
        1,
        EventType::OUT_OF_RANGE,
        MonitoredReference::local(ai(1), pv, None),
    );
    let print =
        |(params, delay, event_type, reference): &(_, u64, EventType, MonitoredReference)| {
            super::reference::params_fingerprint(params, *delay, *event_type, reference).unwrap()
        };
    let first = print(&base);
    assert_eq!(first, print(&base.clone()), "stable");
    let other = PropertyIdentifier::RELIABILITY;
    let reference = |object, property, index| MonitoredReference::local(object, property, index);
    let changes = [
        (params(90.0), base.1, base.2, base.3),
        (base.0.clone(), 2, base.2, base.3),
        (base.0.clone(), base.1, EventType::FLOATING_LIMIT, base.3),
        (base.0.clone(), base.1, base.2, reference(ai(2), pv, None)),
        (
            base.0.clone(),
            base.1,
            base.2,
            reference(ai(1), other, None),
        ),
        (
            base.0.clone(),
            base.1,
            base.2,
            reference(ai(1), pv, Some(0)),
        ),
    ];
    for change in &changes {
        assert_ne!(print(change), first, "{change:?}");
    }
    // The device qualifier isn't: qualified and unqualified resolve alike.
    let mut qualified = base.3;
    qualified.device_identifier = Some(ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap());
    assert_eq!(print(&(base.0.clone(), base.1, base.2, qualified)), first);
}
