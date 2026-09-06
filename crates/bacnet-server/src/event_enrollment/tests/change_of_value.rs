//! CHANGE_OF_VALUE algorithm tests (Clause 13.3.3, Figure 13-10).
//!
//! Split out of `tests.rs` to keep every file under the 700-LOC cap.
//!
//! Clause 13.3.3 inducts only transitions to NORMAL, driven by a detection
//! baseline: "the value of the monitored value when a transition to NORMAL
//! is indicated shall be used in evaluation of the conditions until the next
//! transition to NORMAL is indicated." The pre-#137 implementation had no
//! baseline and answered OFFNORMAL whenever `|value| >= increment` — a
//! transition the algorithm cannot indicate, dropped again by the pre-#166
//! same-state skip. These tests pin the baseline semantics instead: the
//! first sample initializes the baseline without indicating (the clause's
//! "local matter"), and a change of `>= pIncrement` against that baseline
//! indicates a NORMAL→NORMAL same-state transition whose actions still run
//! (Clause 13.2.2.1.4).

use super::super::*;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventParameter, ChangeOfValueCriteria,
};

/// Helper: an AnalogInput monitored by a CHANGE_OF_VALUE enrollment with the
/// given increment (IEEE-754 REAL criterion).
fn setup_cov(
    present_value: f32,
    increment: f32,
    time_delay: u32,
) -> (ObjectDatabase, ObjectIdentifier, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();

    let mut ai = AnalogInputObject::new(70, "AI-COV", 62).unwrap();
    ai.set_present_value(present_value);
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    let mut ee =
        EventEnrollmentObject::new(70, "EE-COV", EventType::CHANGE_OF_VALUE.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    ee.set_event_parameters(BACnetEventParameter::ChangeOfValue {
        time_delay,
        criteria: ChangeOfValueCriteria::ReferencedPropertyIncrement(increment),
    });
    ee.set_event_enable(0x07);
    let ee_oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();

    (db, ee_oid, ai_oid)
}

/// Move the monitored present value (an input's `Present_Value` accepts
/// writes only while `Out_Of_Service` — the route the out_of_range tests
/// already use).
fn set_monitored(db: &mut ObjectDatabase, ai_oid: &ObjectIdentifier, value: f32) {
    let ai = db.get_mut(ai_oid).unwrap();
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
        PropertyValue::Real(value),
        None,
    )
    .unwrap();
}

/// The FIRST observed sample initializes the detection baseline and never
/// indicates a transition — even when its absolute value dwarfs the
/// increment. Clause 13.3.3: "The initialization of the value used in
/// evaluation before the first transition to NORMAL is indicated is a local
/// matter." (This test replaces the pre-#137 `|value| >= increment →
/// OFFNORMAL` assertion; that behavior is what the issue removed.)
#[test]
fn change_of_value_first_sample_establishes_baseline_without_transition() {
    let (mut db, ee_oid, _ai_oid) = setup_cov(10.0, 5.0, 0);

    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert!(
        transitions.is_empty(),
        "first sample initializes the baseline; it must not indicate: {transitions:?}"
    );
    let obj = db.get(&ee_oid).unwrap();
    assert_eq!(
        obj.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::NORMAL.to_raw())
    );
}

/// A change against the established baseline smaller than the increment
/// indicates nothing (condition (a)'s "equal to or greater than pIncrement").
#[test]
fn change_of_value_within_increment_indicates_nothing() {
    let (mut db, _ee_oid, ai_oid) = setup_cov(3.0, 5.0, 0);
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty()); // baseline = 3.0

    set_monitored(&mut db, &ai_oid, 7.9); // |7.9 - 3.0| = 4.9 < 5.0
    assert!(
        evaluate_event_enrollments(&mut db, 1).is_empty(),
        "a sub-increment change against the baseline indicates no transition"
    );
}

/// A change of exactly pIncrement crosses the threshold ("equal to or
/// greater than") and indicates the algorithm's only transition: NORMAL →
/// NORMAL (Figure 13-10). The transition actions run for the same-state
/// result per Clause 13.2.2.1.4 — the transition is emitted and Event_State
/// is (trivially) stored — and the baseline advances to the value at the
/// indicated transition.
#[test]
fn change_of_value_threshold_crossing_indicates_normal_to_normal() {
    let (mut db, ee_oid, ai_oid) = setup_cov(3.0, 5.0, 0);
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty()); // baseline = 3.0

    set_monitored(&mut db, &ai_oid, 8.0); // |8.0 - 3.0| = 5.0 >= 5.0
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].event_type, EventType::CHANGE_OF_VALUE);
    assert_eq!(transitions[0].change.from, EventState::NORMAL);
    assert_eq!(
        transitions[0].change.to,
        EventState::NORMAL,
        "CHANGE_OF_VALUE can only indicate NORMAL (Figure 13-10)"
    );

    // Baseline advanced to 8.0: holding at 8.0 indicates nothing further...
    assert!(
        evaluate_event_enrollments(&mut db, 1).is_empty(),
        "no re-fire while the value holds at the new baseline"
    );

    // ...and the next indication is measured from 8.0, not from 3.0: a move
    // that would dwarf the ORIGINAL baseline but is sub-increment from the
    // new one indicates nothing. Note the baseline did NOT advance on that
    // pass — it moves only when a transition to NORMAL is indicated.
    set_monitored(&mut db, &ai_oid, 12.9); // |12.9 - 8.0| = 4.9 < 5.0
    assert!(
        evaluate_event_enrollments(&mut db, 1).is_empty(),
        "the baseline advances at each indicated NORMAL transition"
    );

    // Recovery across the increment from the STILL-CURRENT baseline (8.0,
    // not the unindicated 12.9) indicates again.
    set_monitored(&mut db, &ai_oid, 2.9); // |2.9 - 8.0| = 5.1 >= 5.0
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.from, EventState::NORMAL);
    assert_eq!(transitions[0].change.to, EventState::NORMAL);

    // Event_State never left NORMAL — and was re-stored as NORMAL by the
    // transition actions.
    let obj = db.get(&ee_oid).unwrap();
    assert_eq!(
        obj.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::NORMAL.to_raw())
    );
}

/// Repeated above-threshold changes against an advancing baseline: each
/// crossing re-indicates, each quiet interval does not.
#[test]
fn change_of_value_repeated_changes_each_indicate() {
    let (mut db, _ee_oid, ai_oid) = setup_cov(0.0, 5.0, 0);
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty()); // baseline = 0.0

    for (from_label, to) in [("first", 6.0), ("second", 11.0), ("third", 16.0)] {
        set_monitored(&mut db, &ai_oid, to);
        let transitions = evaluate_event_enrollments(&mut db, 1);
        assert_eq!(
            transitions.len(),
            1,
            "{from_label} +increment crossing must indicate"
        );
        assert_eq!(transitions[0].change.to, EventState::NORMAL);
        assert!(
            evaluate_event_enrollments(&mut db, 1).is_empty(),
            "holding at the new baseline must not re-indicate"
        );
    }
}

/// A non-positive increment never satisfies condition (a) ("a positive REAL
/// increment"): no crash, no transition, whatever the values.
#[test]
fn change_of_value_nonpositive_increment_never_indicates() {
    let (mut db, _ee_oid, ai_oid) = setup_cov(0.0, 0.0, 0);
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty()); // baseline = 0.0
    set_monitored(&mut db, &ai_oid, 1000.0);
    assert!(
        evaluate_event_enrollments(&mut db, 1).is_empty(),
        "non-positive pIncrement can never indicate (13.3.3)"
    );
}

/// The COV change condition is gated by pTimeDelayNormal (13.3.3's only
/// delay reference — the algorithm never indicates offnormal), here via the
/// Time_Delay fallback when Time_Delay_Normal is not configured.
#[test]
fn change_of_value_change_is_gated_by_the_normal_direction_delay() {
    let (mut db, _ee_oid, ai_oid) = setup_cov(0.0, 5.0, 2);
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty()); // baseline = 0.0

    set_monitored(&mut db, &ai_oid, 10.0); // changed by 10 >= 5
    assert!(
        evaluate_event_enrollments(&mut db, 1).is_empty(),
        "pass 1 of the changed condition: countdown seeded, not fired"
    );
    assert!(
        evaluate_event_enrollments(&mut db, 1).is_empty(),
        "pass 2: still counting down"
    );
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(
        transitions.len(),
        1,
        "pass 3: pTimeDelayNormal (= Time_Delay fallback, 2) elapsed — the NORMAL transition fires"
    );
    assert_eq!(transitions[0].change.to, EventState::NORMAL);
}
