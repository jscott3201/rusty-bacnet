//! FLOATING_LIMIT algorithm tests.
//!
//! Split out of `tests.rs` to keep every file under the 700-LOC cap.

use super::super::*;
use super::*;

// ---- FLOATING_LIMIT tests ----

#[test]
fn floating_limit_normal_stays_normal() {
    // setpoint=50, high_diff=10, low_diff=10 → limits at 60/40
    let (mut db, _ee_oid, _ai_oid) = setup_floating_limit(50.0, 50.0, 10.0, 10.0, 2.0);
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert!(transitions.is_empty());
}

#[test]
fn floating_limit_to_high() {
    // setpoint=50, high_diff=10 → high_limit=60; value=65 exceeds
    let (mut db, ee_oid, ai_oid) = setup_floating_limit(65.0, 50.0, 10.0, 10.0, 2.0);
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].enrollment_oid, ee_oid);
    assert_eq!(transitions[0].monitored_oid, ai_oid);
    assert_eq!(transitions[0].change.from, EventState::NORMAL);
    assert_eq!(transitions[0].change.to, EventState::HIGH_LIMIT);
    assert_eq!(transitions[0].event_type, EventType::FLOATING_LIMIT);
}

#[test]
fn floating_limit_to_low() {
    // setpoint=50, low_diff=10 → low_limit=40; value=35 below
    let (mut db, _ee_oid, _ai_oid) = setup_floating_limit(35.0, 50.0, 10.0, 10.0, 2.0);
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.to, EventState::LOW_LIMIT);
}

#[test]
fn floating_limit_deadband_hysteresis() {
    // setpoint=50, high_diff=10, deadband=2 → high_limit=60, return threshold=58
    let (mut db, _ee_oid, ai_oid) = setup_floating_limit(65.0, 50.0, 10.0, 10.0, 2.0);
    evaluate_event_enrollments(&mut db, 1);

    // Still above return threshold (58)
    let ai = db.get_mut(&ai_oid).unwrap();
    ai.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    ai.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(59.0),
        None,
    )
    .unwrap();
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert!(transitions.is_empty());

    // Below return threshold
    let ai = db.get_mut(&ai_oid).unwrap();
    ai.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(57.0),
        None,
    )
    .unwrap();
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.to, EventState::NORMAL);
}

#[test]
fn unavailable_setpoint_discards_queued_evaluator_ownership_atomically() {
    let mut db = ObjectDatabase::new();
    let mut monitored = AnalogInputObject::new(4, "AI-floating-monitored", 62).unwrap();
    monitored.set_present_value(65.0);
    let monitored_oid = monitored.object_identifier();
    db.add(Box::new(monitored)).unwrap();

    let missing_setpoint_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 404).unwrap();
    let mut enrollment = EventEnrollmentObject::new(
        4,
        "EE-floating-unavailable",
        EventType::FLOATING_LIMIT.to_raw(),
    )
    .unwrap();
    enrollment.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        monitored_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    enrollment.set_event_parameters(BACnetEventParameter::FloatingLimit {
        time_delay: 2,
        setpoint_reference: BACnetDeviceObjectPropertyReference::new_local(
            missing_setpoint_oid,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ),
        low_diff_limit: 10.0,
        high_diff_limit: 10.0,
        deadband: 2.0,
    });
    let stale_state = EventEnrollmentEvalState {
        pending: Some(EventEnrollmentPending {
            state: EventState::HIGH_LIMIT,
            remaining: 1,
            condition: 1,
            params_fingerprint: 7,
        }),
        cov_baseline: Some(PropertyValue::Real(33.0)),
        last_offnormal_value: Some(2),
    };
    let stale_source = (monitored_oid, PropertyIdentifier::DESCRIPTION, None);
    enrollment
        .set_enrollment_eval_state_internal(stale_state.clone())
        .unwrap();
    enrollment
        .set_enrollment_eval_source_internal(Some(stale_source))
        .unwrap();
    let enrollment_oid = enrollment.object_identifier();
    db.add(Box::new(enrollment)).unwrap();

    let before_reliability = db
        .get(&enrollment_oid)
        .unwrap()
        .read_property(PropertyIdentifier::RELIABILITY, None)
        .unwrap();
    let before_state = db
        .get(&enrollment_oid)
        .unwrap()
        .read_property(PropertyIdentifier::EVENT_STATE, None)
        .unwrap();
    let before_timestamps = db
        .get(&enrollment_oid)
        .unwrap()
        .read_property(PropertyIdentifier::EVENT_TIME_STAMPS, None)
        .unwrap();
    let before_sequence = db.reserve_event_sequence_number().number();

    let report = evaluate_event_enrollments_detailed_report(&mut db, 1);

    assert!(report.transitions.is_empty());
    assert!(report.reliability_results.is_empty());
    assert!(report
        .diagnostics
        .contains(&EventEnrollmentDetailedEvaluationDiagnostic {
            enrollment_oid,
            stage: EventEnrollmentDetailedEvaluationStage::Reliability,
            outcome: EventEnrollmentDetailedEvaluationOutcome::ObservationUnavailable,
        }));
    let enrollment = db.get(&enrollment_oid).unwrap();
    assert_eq!(
        enrollment.enrollment_eval_state_internal(),
        Some(stale_state)
    );
    assert_eq!(
        enrollment.enrollment_eval_source_internal(),
        Some(Some(stale_source))
    );
    assert_eq!(
        enrollment
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap(),
        before_reliability
    );
    assert_eq!(
        enrollment
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        before_state
    );
    assert_eq!(
        enrollment
            .read_property(PropertyIdentifier::EVENT_TIME_STAMPS, None)
            .unwrap(),
        before_timestamps
    );
    assert_eq!(db.reserve_event_sequence_number().number(), before_sequence);
}
