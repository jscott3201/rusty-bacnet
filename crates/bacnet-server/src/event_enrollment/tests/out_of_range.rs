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
    let transitions = evaluate_event_enrollments(&mut db);
    assert!(transitions.is_empty());
}

#[test]
fn out_of_range_normal_to_high_limit() {
    let (mut db, ee_oid, ai_oid) = setup_out_of_range(85.0, 80.0, 20.0, 2.0);
    let transitions = evaluate_event_enrollments(&mut db);
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
    let transitions = evaluate_event_enrollments(&mut db);
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
    evaluate_event_enrollments(&mut db);

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

    let transitions = evaluate_event_enrollments(&mut db);
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

    let transitions = evaluate_event_enrollments(&mut db);
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
    let t1 = evaluate_event_enrollments(&mut db);
    assert_eq!(t1.len(), 1);

    // Second evaluation: same state, no new transition
    let t2 = evaluate_event_enrollments(&mut db);
    assert!(t2.is_empty());
}

#[test]
fn out_of_range_event_enable_suppresses_notification() {
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

    // TO_OFFNORMAL not enabled — should not appear in transitions
    let transitions = evaluate_event_enrollments(&mut db);
    assert!(transitions.is_empty());

    // But event_state should NOT have been updated (notification suppressed)
    let obj = db.get(&ee_oid).unwrap();
    assert_eq!(
        obj.read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap(),
        PropertyValue::Enumerated(EventState::NORMAL.to_raw())
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

    let transitions = evaluate_event_enrollments(&mut db);
    assert!(transitions.is_empty());
}
