//! FLOATING_LIMIT algorithm tests.
//!
//! Split out of `tests.rs` to keep every file under the 700-LOC cap.

use super::super::*;
use super::*;
use crate::server::event_notification_payload::CommittedNotificationPayload;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_services::alarm_event::NotificationParameters;
use bacnet_types::primitives::StatusFlags;

// ---- FLOATING_LIMIT tests ----

#[test]
fn floating_limit_normal_stays_normal() {
    // setpoint=50, high_diff=10, low_diff=10 → limits at 60/40
    let (mut db, _ee_oid, _ai_oid) = setup_floating_limit(50.0, 50.0, 10.0, 10.0, 2.0);
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert!(transitions.is_empty());
}

#[test]
fn floating_limit_to_high() {
    // setpoint=50, high_diff=10 → high_limit=60; value=65 exceeds
    let (mut db, ee_oid, ai_oid) = setup_floating_limit(65.0, 50.0, 10.0, 10.0, 2.0);
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].enrollment_oid, ee_oid);
    assert_eq!(transitions[0].monitored_oid, ai_oid);
    assert_eq!(transitions[0].change.from, EventState::NORMAL);
    assert_eq!(transitions[0].change.to, EventState::HIGH_LIMIT);
    assert_eq!(transitions[0].event_type, EventType::FLOATING_LIMIT);
}

#[test]
fn floating_limit_to_low() {
    // setpoint=50, low_diff=10 → low_limit=40; value=35 below
    let (mut db, _ee_oid, _ai_oid) = setup_floating_limit(35.0, 50.0, 10.0, 10.0, 2.0);
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.to, EventState::LOW_LIMIT);
}

#[test]
fn floating_limit_deadband_hysteresis() {
    // setpoint=50, high_diff=10, deadband=2 → high_limit=60, return threshold=58
    let (mut db, _ee_oid, ai_oid) = setup_floating_limit(65.0, 50.0, 10.0, 10.0, 2.0);
    evaluate_event_enrollments(&mut db, 1);

    // Still above return threshold (58)
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
        PropertyValue::Real(59.0),
        None,
    )
    .unwrap();
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert!(transitions.is_empty());

    // Below return threshold
    let ai = db.get_mut(&ai_oid).unwrap();
    ai.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(57.0),
        None,
    )
    .unwrap();
    let transitions = evaluate_event_enrollments(&mut db, 1);
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].change.to, EventState::NORMAL);
}

// ---- Device-qualified setpoint references (#1184) ----

/// `setup_floating_limit(65.0, 50.0, 10.0, 10.0, 2.0)` with Devices
/// `local_devices` added and the setpoint reference naming Device
/// `setpoint_device`. The monitored reference stays unqualified.
fn setup_qualified_setpoint(
    local_devices: &[u32],
    setpoint_device: u32,
) -> (ObjectDatabase, ObjectIdentifier, ObjectIdentifier) {
    let (mut db, ee_oid, ai_oid) = setup_floating_limit(65.0, 50.0, 10.0, 10.0, 2.0);
    for &instance in local_devices {
        db.add(Box::new(
            DeviceObject::new(DeviceConfig {
                identity: bacnet_objects::DeviceIdentity::new(instance, 0)
                    .expect("valid Device identity")
                    .with_name(format!("Device-{instance}")),
                ..DeviceConfig::default()
            })
            .unwrap(),
        ))
        .unwrap();
    }
    // Replace EE-FL with the same enrollment, its setpoint now qualified.
    let setpoint_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap();
    let mut ee = EventEnrollmentObject::new(2, "EE-FL", EventType::FLOATING_LIMIT).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )))
    .unwrap();
    ee.set_event_parameters(BACnetEventParameter::FloatingLimit {
        time_delay: 0,
        setpoint_reference: BACnetDeviceObjectPropertyReference::new_remote(
            setpoint_oid,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
            ObjectIdentifier::new(ObjectType::DEVICE, setpoint_device).unwrap(),
        ),
        low_diff_limit: 10.0,
        high_diff_limit: 10.0,
        deadband: 2.0,
    });
    ee.set_event_enable(EventTransitionBits::all());
    db.add(Box::new(ee)).unwrap();
    (db, ee_oid, ai_oid)
}

#[test]
fn floating_limit_setpoint_naming_this_device_evaluates_and_reports() {
    // With two Devices the lower one is this device.
    for local_devices in [&[100][..], &[100, 200]] {
        let (mut db, ee_oid, ai_oid) = setup_qualified_setpoint(local_devices, 100);
        let batch = evaluate_event_enrollments_for_delivery(&mut db, 1);
        assert_eq!(batch.deliveries.len(), 1, "{local_devices:?}");
        let delivery = &batch.deliveries[0];
        let CommittedEventEnrollmentResult::Normal(transition) = &delivery.result else {
            panic!("expected a FLOATING_LIMIT transition");
        };
        assert_eq!(transition.enrollment_oid, ee_oid);
        assert_eq!(transition.monitored_oid, ai_oid);
        assert_eq!(transition.change.to, EventState::HIGH_LIMIT);
        assert_eq!(
            delivery.event_values,
            CommittedNotificationPayload::for_test(NotificationParameters::FloatingLimit {
                reference_value: 65.0,
                status_flags: StatusFlags::empty(),
                setpoint_value: 50.0,
                error_limit: 10.0,
            })
        );
    }
}

#[test]
fn floating_limit_setpoint_naming_another_device_stays_unavailable() {
    // AI-3 exists here, but the reference names a device this one isn't.
    for local_devices in [&[100][..], &[100, 200], &[]] {
        let (mut db, ee_oid, _) = setup_qualified_setpoint(local_devices, 200);
        let batch = evaluate_event_enrollments_for_delivery(&mut db, 1);
        assert!(batch.deliveries.is_empty(), "{local_devices:?}");
        assert!(batch.report.transitions.is_empty());
        assert!(batch.report.diagnostics.iter().any(|diagnostic| {
            diagnostic.enrollment_oid == ee_oid
                && diagnostic.outcome == EventEnrollmentEvaluationOutcome::ObservationUnavailable
        }));
        assert_eq!(
            db.get(&ee_oid)
                .unwrap()
                .read_property(PropertyIdentifier::EVENT_STATE, None)
                .unwrap(),
            PropertyValue::Enumerated(EventState::NORMAL.to_raw())
        );
    }
}
