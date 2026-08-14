mod array_index;
mod change_of_bitstring;
mod change_of_state;
mod change_of_value;
mod compat;
mod custom_state;
mod delays;
mod detection_enable;
mod floating_limit;
mod foreign_state;
mod integration;
mod out_of_range;
mod same_state;

use super::*;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::binary::BinaryInputObject;
use bacnet_objects::event_enrollment::EventEnrollmentObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventParameter, BACnetPropertyStates,
};
/// Helper: create an EventEnrollment monitoring an AnalogInput with OUT_OF_RANGE.
fn setup_out_of_range(
    present_value: f32,
    high_limit: f32,
    low_limit: f32,
    deadband: f32,
) -> (ObjectDatabase, ObjectIdentifier, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();

    // Monitored analog input
    let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    ai.set_present_value(present_value);
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    // Event enrollment
    let mut ee = EventEnrollmentObject::new(1, "EE-OOR", EventType::OUT_OF_RANGE.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    ee.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit,
        high_limit,
        deadband,
    });
    ee.set_event_enable(0x07); // all transitions
    let ee_oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();

    (db, ee_oid, ai_oid)
}

/// Helper: create an EventEnrollment monitoring an AnalogInput with FLOATING_LIMIT.
///
/// The setpoint is held by a separate AnalogInput so the
/// `setpoint_reference` resolves to `setpoint` rather than the monitored value.
fn setup_floating_limit(
    present_value: f32,
    setpoint: f32,
    high_diff: f32,
    low_diff: f32,
    deadband: f32,
) -> (ObjectDatabase, ObjectIdentifier, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();

    let mut ai = AnalogInputObject::new(2, "AI-2", 62).unwrap();
    ai.set_present_value(present_value);
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    // Separate setpoint object the floating-limit reference resolves to.
    let mut sp = AnalogInputObject::new(3, "AI-SP", 62).unwrap();
    sp.set_present_value(setpoint);
    let sp_oid = sp.object_identifier();
    db.add(Box::new(sp)).unwrap();

    let mut ee =
        EventEnrollmentObject::new(2, "EE-FL", EventType::FLOATING_LIMIT.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    ee.set_event_parameters(BACnetEventParameter::FloatingLimit {
        time_delay: 0,
        setpoint_reference: BACnetDeviceObjectPropertyReference::new_local(
            sp_oid,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ),
        low_diff_limit: low_diff,
        high_diff_limit: high_diff,
        deadband,
    });
    ee.set_event_enable(0x07);
    let ee_oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();

    (db, ee_oid, ai_oid)
}

/// Helper: create an EventEnrollment monitoring a BinaryInput with CHANGE_OF_STATE.
fn setup_change_of_state(
    present_value: u32,
    alarm_values: &[u32],
) -> (ObjectDatabase, ObjectIdentifier, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();

    let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
    bi.set_present_value(present_value);
    let bi_oid = bi.object_identifier();
    db.add(Box::new(bi)).unwrap();

    let mut ee =
        EventEnrollmentObject::new(3, "EE-COS", EventType::CHANGE_OF_STATE.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        bi_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    ee.set_event_parameters(BACnetEventParameter::ChangeOfState {
        time_delay: 0,
        list_of_values: alarm_values
            .iter()
            .map(|v| BACnetPropertyStates::UnsignedValue(*v))
            .collect(),
    });
    ee.set_event_enable(0x07);
    let ee_oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();

    (db, ee_oid, bi_oid)
}
