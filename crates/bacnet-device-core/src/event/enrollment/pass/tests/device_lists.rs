//! An Event Enrollment monitoring the local Device's Database_Revision sees
//! the database's revision (#2669), which the Device object on its own
//! reads as 0.

use super::super::*;
use super::*;
use bacnet_objects::device::{DeviceConfig, DeviceObject};

#[test]
fn an_enrollment_on_the_device_revision_sees_it_move() {
    let mut db = ObjectDatabase::new();
    let device = DeviceObject::new(DeviceConfig {
        identity: bacnet_objects::DeviceIdentity::new(9, 0).unwrap(),
        ..DeviceConfig::default()
    })
    .unwrap();
    let device_oid = device.object_identifier();
    db.add(Box::new(device)).unwrap();
    let mut ee = EventEnrollmentObject::new(1, "EE-REV", EventType::OUT_OF_RANGE).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        device_oid,
        PropertyIdentifier::DATABASE_REVISION.to_raw(),
    )))
    .unwrap();
    // In range at revisions 1 and 2; high past 2.
    ee.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 0.5,
        high_limit: 2.5,
        deadband: 0.0,
    });
    ee.set_event_enable(EventTransitionBits::all());
    let ee_oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();
    assert_eq!(db.database_revision(), 2);
    assert!(evaluate_event_enrollments(&mut db, 1).is_empty());

    let ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::NO_UNITS).unwrap();
    db.add(Box::new(ai)).unwrap();
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].enrollment_oid, ee_oid);
    assert_eq!(transitions[0].monitored_oid, device_oid);
    assert_eq!(transitions[0].change.to, EventState::HIGH_LIMIT);
}
