//! Multiple-enrollment integration tests.
//!
//! Split out of `tests.rs` to keep every file under the 700-LOC cap.

use super::super::*;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetEventParameter};

// ---- Integration: multiple enrollments ----

#[test]
fn evaluates_multiple_enrollments() {
    let mut db = ObjectDatabase::new();

    // Two analog inputs
    let mut ai1 = AnalogInputObject::new(80, "AI-80", 62).unwrap();
    ai1.set_present_value(90.0); // will trigger HIGH_LIMIT
    let ai1_oid = ai1.object_identifier();
    db.add(Box::new(ai1)).unwrap();

    let mut ai2 = AnalogInputObject::new(81, "AI-81", 62).unwrap();
    ai2.set_present_value(50.0); // normal
    let ai2_oid = ai2.object_identifier();
    db.add(Box::new(ai2)).unwrap();

    // Two enrollments
    let mut ee1 =
        EventEnrollmentObject::new(80, "EE-80", EventType::OUT_OF_RANGE.to_raw()).unwrap();
    ee1.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai1_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    ee1.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 2.0,
    });
    ee1.set_event_enable(0x07);
    db.add(Box::new(ee1)).unwrap();

    let mut ee2 =
        EventEnrollmentObject::new(81, "EE-81", EventType::OUT_OF_RANGE.to_raw()).unwrap();
    ee2.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai2_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    ee2.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 2.0,
    });
    ee2.set_event_enable(0x07);
    db.add(Box::new(ee2)).unwrap();

    let transitions = evaluate_event_enrollments(&mut db);
    // Only AI-80 triggers (90 > 80)
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].monitored_oid, ai1_oid);
}

#[test]
fn missing_monitored_object_is_skipped() {
    let mut db = ObjectDatabase::new();

    let fake_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 999).unwrap();
    let mut ee =
        EventEnrollmentObject::new(90, "EE-miss", EventType::OUT_OF_RANGE.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        fake_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    ee.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 20.0,
        high_limit: 80.0,
        deadband: 2.0,
    });
    ee.set_event_enable(0x07);
    db.add(Box::new(ee)).unwrap();

    // Should not panic or return transitions
    let transitions = evaluate_event_enrollments(&mut db);
    assert!(transitions.is_empty());
}
