//! OUT_OF_RANGE algorithm tests.
//!
//! Split out of `tests.rs` to keep every file under the 700-LOC cap.

use super::super::*;
use super::*;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetEventParameter};

// ---- OUT_OF_RANGE tests ----

#[test]
fn out_of_range_normal_stays_normal() {
    let (mut db, _ee_oid, _ai_oid) = setup_out_of_range(50.0, 80.0, 20.0, 2.0);
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert!(transitions.is_empty());
}

#[test]
fn out_of_range_normal_to_high_limit() {
    let (mut db, ee_oid, ai_oid) = setup_out_of_range(85.0, 80.0, 20.0, 2.0);
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].enrollment_oid, ee_oid);
    assert_eq!(transitions[0].monitored_oid, ai_oid);
    assert_eq!(transitions[0].change.from, EventState::NORMAL);
    assert_eq!(transitions[0].change.to, EventState::HIGH_LIMIT);
    assert_eq!(transitions[0].event_type, EventType::OUT_OF_RANGE);

    // Verify event_state was persisted
    let obj = db.get(&ee_oid).unwrap();
    assert_eq!(
        obj.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw())
    );
}

#[test]
fn out_of_range_normal_to_low_limit() {
    let (mut db, ee_oid, _ai_oid) = setup_out_of_range(15.0, 80.0, 20.0, 2.0);
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.from, EventState::NORMAL);
    assert_eq!(transitions[0].change.to, EventState::LOW_LIMIT);

    // Verify persisted state
    let obj = db.get(&ee_oid).unwrap();
    assert_eq!(
        obj.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::LOW_LIMIT.to_raw())
    );
}

#[test]
fn out_of_range_high_to_normal_with_deadband() {
    let (mut db, ee_oid, ai_oid) = setup_out_of_range(85.0, 80.0, 20.0, 2.0);
    // First: go to HIGH_LIMIT
    evaluate_event_enrollments(&mut db, 1);

    // Update monitored value — still within deadband (80 - 2 = 78)
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
        PropertyValue::Real(79.0),
        None,
    )
    .unwrap();

    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert!(transitions.is_empty(), "within deadband — no transition");

    // Drop below deadband
    let ai = db.get_mut(&ai_oid).unwrap();
    ai.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(77.0),
        None,
    )
    .unwrap();

    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.from, EventState::HIGH_LIMIT);
    assert_eq!(transitions[0].change.to, EventState::NORMAL);

    let obj = db.get(&ee_oid).unwrap();
    assert_eq!(
        obj.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::NORMAL.to_raw())
    );
}

#[test]
fn out_of_range_no_change_when_already_faulted() {
    let (mut db, _ee_oid, _ai_oid) = setup_out_of_range(85.0, 80.0, 20.0, 2.0);
    let t1 = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(t1.len(), 1);

    // Second evaluation: same state, no new transition
    let t2 = evaluate_event_enrollments(&mut db, 1);
    assert!(t2.is_empty());
}

#[test]
fn out_of_range_event_enable_suppresses_distribution_not_the_transition() {
    let mut db = ObjectDatabase::new();

    let mut ai = AnalogInputObject::new(10, "AI-10", 62).unwrap();
    ai.set_present_value(85.0);
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    let mut ee =
        EventEnrollmentObject::new(10, "EE-sup", EventType::OUT_OF_RANGE.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    ee.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 2.0,
    });
    ee.set_event_enable(0x04); // only TO_NORMAL enabled
    let ee_oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();

    // TO_OFFNORMAL is not enabled, so the notification must be suppressed —
    // but the transition itself is still detected and reported. Clause 12.12
    // scopes Event_Enable to distribution alone.
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert!(
        !transitions[0].distribute,
        "TO_OFFNORMAL disabled: the notification must not be externally distributed"
    );
    assert_eq!(transitions[0].change.to, EventState::HIGH_LIMIT);

    // And Event_State IS updated. Storing the new event state is the first of
    // Clause 13.2.2.1.4's transition actions, and none of them is
    // Event_Enable-scoped — the property disables distribution downstream
    // (Clause 13.2.5). Suppressing the notification must therefore not freeze
    // the state. Leaving it at NORMAL here would also break the next
    // transition, because the evaluator compares against a state that never
    // advanced.
    let obj = db.get(&ee_oid).unwrap();
    assert_eq!(
        obj.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw())
    );
}

/// A suppressed TO_OFFNORMAL must not cost us the *enabled* TO_NORMAL that
/// follows it.
///
/// This is the concrete damage from gating `Event_State` on `Event_Enable`:
/// with TO_OFFNORMAL disabled the old code left the enrollment sitting at
/// NORMAL, so when the value came back into range the evaluator compared
/// NORMAL against NORMAL, saw no change, and dropped a transition the operator
/// had explicitly enabled. Clearing one `Event_Enable` bit silently disabled a
/// different one.
#[test]
fn out_of_range_suppressed_offnormal_still_yields_enabled_return_to_normal() {
    let mut db = ObjectDatabase::new();

    let mut ai = AnalogInputObject::new(11, "AI-11", 62).unwrap();
    ai.set_present_value(85.0);
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    let mut ee =
        EventEnrollmentObject::new(11, "EE-ret", EventType::OUT_OF_RANGE.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    ee.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 2.0,
    });
    ee.set_event_enable(0x04); // only TO_NORMAL enabled
    let ee_oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();

    // Out of range: detected, state advances, notification suppressed.
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert!(!transitions[0].distribute);

    // Back into range, clear of the deadband (80 - 2 = 78).
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
        PropertyValue::Real(70.0),
        None,
    )
    .unwrap();

    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(
        transitions.len(),
        1,
        "TO_NORMAL is enabled and must be reported"
    );
    assert_eq!(transitions[0].change.from, EventState::HIGH_LIMIT);
    assert_eq!(transitions[0].change.to, EventState::NORMAL);
    assert!(
        transitions[0].distribute,
        "TO_NORMAL is enabled — this transition must be marked eligible for \
         distribution (the enrollment path does not send it yet, see #127)"
    );

    let obj = db.get(&ee_oid).unwrap();
    assert_eq!(
        obj.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::NORMAL.to_raw())
    );
}

/// `Event_Enable` of zero suppresses all *external* distribution (Clause 13.2.5
/// leaves the effect on local objects a local matter) but must still track
/// state, so a later re-enable resumes from the true condition rather than a
/// stale NORMAL.
#[test]
fn out_of_range_event_enable_zero_still_tracks_event_state() {
    let mut db = ObjectDatabase::new();

    let mut ai = AnalogInputObject::new(12, "AI-12", 62).unwrap();
    ai.set_present_value(15.0);
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    let mut ee =
        EventEnrollmentObject::new(12, "EE-zero", EventType::OUT_OF_RANGE.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    ee.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 2.0,
    });
    ee.set_event_enable(0x00); // nothing distributed
    let ee_oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();

    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.to, EventState::LOW_LIMIT);
    assert!(!transitions[0].distribute);

    let obj = db.get(&ee_oid).unwrap();
    assert_eq!(
        obj.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::LOW_LIMIT.to_raw()),
        "Event_State is a readable property, not a notification — it tracks \
         the condition regardless of Event_Enable"
    );
}

#[test]
fn out_of_range_skips_out_of_service() {
    let (mut db, ee_oid, _ai_oid) = setup_out_of_range(85.0, 80.0, 20.0, 2.0);

    // Set enrollment to out-of-service
    let obj = db.get_mut(&ee_oid).unwrap();
    obj.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();

    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert!(transitions.is_empty());
}
