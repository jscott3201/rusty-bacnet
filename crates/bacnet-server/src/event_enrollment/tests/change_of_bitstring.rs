//! CHANGE_OF_BITSTRING algorithm tests.
//!
//! Split out of `tests.rs` to keep every file under the 700-LOC cap.

use super::super::*;
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetEventParameter};

// ---- CHANGE_OF_BITSTRING tests ----

#[test]
fn change_of_bitstring_normal() {
    let mut db = ObjectDatabase::new();

    // Create an object with a bitstring property (using a multistate or similar)
    // For testing, we'll use an EventEnrollment monitoring another enrollment's EVENT_ENABLE
    let mut target = EventEnrollmentObject::new(50, "Target", EventType::NONE.to_raw()).unwrap();
    // EVENT_ENABLE is a 3-bit bitstring
    target.set_event_enable(0x05); // bits: TO_OFFNORMAL | TO_NORMAL
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();

    let mut ee =
        EventEnrollmentObject::new(51, "EE-COBS", EventType::CHANGE_OF_BITSTRING.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        target_oid,
        PropertyIdentifier::EVENT_ENABLE.to_raw(),
    )));
    // mask=0xFF, alarm_pattern=0xE0 (all 3 high bits set)
    ee.set_event_parameters(BACnetEventParameter::ChangeOfBitstring {
        time_delay: 0,
        bitmask: (0, vec![0xFF]),
        list_of_values: vec![(0, vec![0xE0])],
    });
    ee.set_event_enable(0x07);
    db.add(Box::new(ee)).unwrap();

    let transitions = evaluate_event_enrollments(&mut db);
    // internal 0x05 -> wire 0xA0 (MSB-first), mask 0xFF → 0xA0, alarm 0xE0 → no match → NORMAL
    assert!(transitions.is_empty());
}

#[test]
fn change_of_bitstring_offnormal() {
    let mut db = ObjectDatabase::new();

    let mut target = EventEnrollmentObject::new(60, "Target2", EventType::NONE.to_raw()).unwrap();
    target.set_event_enable(0x07); // all 3 bits set
    let target_oid = target.object_identifier();
    db.add(Box::new(target)).unwrap();

    let mut ee =
        EventEnrollmentObject::new(61, "EE-COBS2", EventType::CHANGE_OF_BITSTRING.to_raw())
            .unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        target_oid,
        PropertyIdentifier::EVENT_ENABLE.to_raw(),
    )));
    // mask=0xE0, alarm_pattern=0xE0 (all 3 high bits)
    ee.set_event_parameters(BACnetEventParameter::ChangeOfBitstring {
        time_delay: 0,
        bitmask: (0, vec![0xE0]),
        list_of_values: vec![(0, vec![0xE0])],
    });
    ee.set_event_enable(0x07);
    db.add(Box::new(ee)).unwrap();

    let transitions = evaluate_event_enrollments(&mut db);
    // internal 0x07 -> wire 0xE0 (MSB-first), mask 0xE0 → 0xE0, alarm 0xE0 → match → OFFNORMAL
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.to, EventState::OFFNORMAL);
}
