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
