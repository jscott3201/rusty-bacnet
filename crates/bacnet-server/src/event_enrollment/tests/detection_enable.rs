//! `Event_Detection_Enable` gating of the Event Enrollment evaluator
//! (ASHRAE 135-2020 Clause 13.2.2.1).
//!
//! Split out of `tests.rs` to keep every file under the 700-LOC cap.
//!
//! What these tests pin, precisely: that a disabled enrollment produces no
//! transition and holds `Event_State` at NORMAL. They do **not** isolate the
//! evaluator's own `Event_Detection_Enable` guard — deleting that guard leaves
//! every test here passing, because `set_event_state_internal` refuses the
//! write anyway and the evaluator only records a transition when that call
//! succeeds. Confirmed by mutation rather than assumed. The two guards are
//! deliberate defense in depth serving different sentences of Clause 13.2.2.1;
//! the object-level one is isolated by
//! `disabled_detection_refuses_non_normal_internal_state` in bacnet-objects.

use super::super::*;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetEventParameter};

/// Build an enrollment that would transition NORMAL -> HIGH_LIMIT, with
/// `Event_Detection_Enable` set from `detection_enabled`.
fn setup(detection_enabled: bool) -> (ObjectDatabase, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();

    let mut ai = AnalogInputObject::new(20, "AI-20", 62).unwrap();
    ai.set_present_value(85.0); // above high_limit
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    let mut ee =
        EventEnrollmentObject::new(20, "EE-det", EventType::OUT_OF_RANGE.to_raw()).unwrap();
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
    ee.set_event_enable(0x07);
    if !detection_enabled {
        ee.write_property(
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();
    }
    let ee_oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();

    (db, ee_oid)
}

/// Control: with detection enabled the same fixture does transition, so the
/// disabled cases below are showing the flag at work rather than a fixture
/// that never fires.
#[test]
fn detection_enabled_evaluates_normally() {
    let (mut db, _ee_oid) = setup(true);
    let transitions = evaluate_event_enrollments(&mut db);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.to, EventState::HIGH_LIMIT);
}

/// Clause 13.2.2.1: "If the Event_Detection_Enable property is FALSE, then this
/// state machine is not evaluated. In this case, no transitions shall occur".
#[test]
fn detection_disabled_yields_no_transitions() {
    let (mut db, _ee_oid) = setup(false);
    assert!(
        evaluate_event_enrollments(&mut db).is_empty(),
        "no transition may occur while Event_Detection_Enable is FALSE"
    );
}

/// Clause 12.12 states the disabled case as an invariant — "When this property
/// is FALSE, Event_State shall be NORMAL" — so an evaluation pass over a
/// disabled enrollment must leave NORMAL in place, not merely decline to
/// report a transition it already applied.
#[test]
fn detection_disabled_holds_event_state_at_normal() {
    let (mut db, ee_oid) = setup(false);
    evaluate_event_enrollments(&mut db);

    let obj = db.get(&ee_oid).unwrap();
    assert_eq!(
        obj.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::NORMAL.to_raw())
    );
}

/// Re-enabling detection resumes evaluation, and the out-of-range condition
/// that was ignored while disabled is now detected afresh from NORMAL.
///
/// The standard is silent on the FALSE -> TRUE edge for this property — note
/// the contrast with `Event_Algorithm_Inhibit`, which the standard gives an
/// explicit re-arm rule ("any condition shall hold for its regular time delay
/// after the change to FALSE"). Resuming from NORMAL is this implementation's
/// choice, and follows from the reset having already run.
#[test]
fn re_enabling_detection_resumes_evaluation() {
    let (mut db, ee_oid) = setup(false);
    assert!(evaluate_event_enrollments(&mut db).is_empty());

    db.get_mut(&ee_oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();

    let transitions = evaluate_event_enrollments(&mut db);
    assert_eq!(transitions.len(), 1, "detection resumes once re-enabled");
    assert_eq!(transitions[0].change.from, EventState::NORMAL);
    assert_eq!(transitions[0].change.to, EventState::HIGH_LIMIT);
}
