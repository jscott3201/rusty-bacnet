//! CHANGE_OF_VALUE algorithm tests.
//!
//! Split out of `tests.rs` to keep every file under the 700-LOC cap.

use super::super::*;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventParameter, ChangeOfValueCriteria,
};

// ---- CHANGE_OF_VALUE tests ----

#[test]
fn change_of_value_within_increment() {
    let mut db = ObjectDatabase::new();

    let mut ai = AnalogInputObject::new(70, "AI-COV", 62).unwrap();
    ai.set_present_value(3.0);
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    let mut ee =
        EventEnrollmentObject::new(70, "EE-COV", EventType::CHANGE_OF_VALUE.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    ee.set_event_parameters(BACnetEventParameter::ChangeOfValue {
        time_delay: 0,
        criteria: ChangeOfValueCriteria::ReferencedPropertyIncrement(5.0),
    });
    ee.set_event_enable(0x07);
    db.add(Box::new(ee)).unwrap();

    // |3.0| < 5.0 → NORMAL
    let transitions = evaluate_event_enrollments(&mut db);
    assert!(transitions.is_empty());
}

#[test]
fn change_of_value_exceeds_increment() {
    let mut db = ObjectDatabase::new();

    let mut ai = AnalogInputObject::new(71, "AI-COV2", 62).unwrap();
    ai.set_present_value(10.0);
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    let mut ee =
        EventEnrollmentObject::new(71, "EE-COV2", EventType::CHANGE_OF_VALUE.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    ee.set_event_parameters(BACnetEventParameter::ChangeOfValue {
        time_delay: 0,
        criteria: ChangeOfValueCriteria::ReferencedPropertyIncrement(5.0),
    });
    ee.set_event_enable(0x07);
    db.add(Box::new(ee)).unwrap();

    // |10.0| >= 5.0 → OFFNORMAL
    let transitions = evaluate_event_enrollments(&mut db);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.to, EventState::OFFNORMAL);
}
