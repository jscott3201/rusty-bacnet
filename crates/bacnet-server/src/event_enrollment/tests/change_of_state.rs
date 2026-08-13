//! CHANGE_OF_STATE algorithm tests.
//!
//! Split out of `tests.rs` to keep every file under the 700-LOC cap.

use super::super::*;
use super::*;

// ---- CHANGE_OF_STATE tests ----

#[test]
fn change_of_state_normal_when_not_in_alarm_set() {
    // Binary INACTIVE (0), alarm on ACTIVE (1)
    let (mut db, _ee_oid, _bi_oid) = setup_change_of_state(0, &[1]);
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert!(transitions.is_empty());
}

#[test]
fn change_of_state_to_offnormal() {
    // Binary ACTIVE (1), alarm on ACTIVE (1)
    let (mut db, ee_oid, bi_oid) = setup_change_of_state(1, &[1]);
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].enrollment_oid, ee_oid);
    assert_eq!(transitions[0].monitored_oid, bi_oid);
    assert_eq!(transitions[0].change.from, EventState::NORMAL);
    assert_eq!(transitions[0].change.to, EventState::OFFNORMAL);
    assert_eq!(transitions[0].event_type, EventType::CHANGE_OF_STATE);
}

#[test]
fn change_of_state_back_to_normal() {
    let (mut db, _ee_oid, bi_oid) = setup_change_of_state(1, &[1]);
    evaluate_event_enrollments(&mut db, 1);

    // Set monitored value to non-alarm
    let bi = db.get_mut(&bi_oid).unwrap();
    bi.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    bi.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(0),
        None,
    )
    .unwrap();

    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.from, EventState::OFFNORMAL);
    assert_eq!(transitions[0].change.to, EventState::NORMAL);
}

#[test]
fn change_of_state_multiple_alarm_values() {
    // Alarm on values 1, 3, 5
    let (mut db, _ee_oid, _bi_oid) = setup_change_of_state(3, &[1, 3, 5]);
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.to, EventState::OFFNORMAL);
}
