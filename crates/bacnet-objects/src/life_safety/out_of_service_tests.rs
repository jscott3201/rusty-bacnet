//! Tracking_Value and Reliability writes while Out_Of_Service is TRUE
//! (Clauses 12.15.11 and 12.16.11, #1108).

use std::sync::{Arc, Mutex};

use bacnet_types::enums::{
    ErrorClass, ErrorCode, LifeSafetyOperation, LifeSafetyState, PropertyIdentifier as P,
    Reliability, SilencedState,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use crate::traits::BACnetObject;

use super::*;

fn point() -> LifeSafetyPointObject {
    LifeSafetyPointObject::new(1, "LSP-1").unwrap()
}

fn zone() -> LifeSafetyZoneObject {
    LifeSafetyZoneObject::new(1, "LSZ-1").unwrap()
}

fn read(object: &dyn BACnetObject, property: P) -> PropertyValue {
    object.read_property(property, None).unwrap()
}

fn write(object: &mut dyn BACnetObject, property: P, value: PropertyValue) -> Result<(), Error> {
    object.write_property(property, None, value, None)
}

fn set_out_of_service(object: &mut dyn BACnetObject, out_of_service: bool) {
    write(
        object,
        P::OUT_OF_SERVICE,
        PropertyValue::Boolean(out_of_service),
    )
    .unwrap();
}

fn assert_property_error(result: Result<(), Error>, code: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Protocol { class, code: actual })
            if class == ErrorClass::PROPERTY
                && actual == code),
        "expected PROPERTY / {code:?}, got {result:?}"
    );
}

/// The FAULT bit of Status_Flags.
fn fault(object: &dyn BACnetObject) -> bool {
    match read(object, P::STATUS_FLAGS) {
        PropertyValue::BitString { data, .. } => data[0] & 0x40 != 0,
        other => panic!("expected Status_Flags bits, got {other:?}"),
    }
}

fn enumerated(raw: u32) -> PropertyValue {
    PropertyValue::Enumerated(raw)
}

fn assert_in_service_writes_are_refused(object: &mut dyn BACnetObject) {
    for property in [P::TRACKING_VALUE, P::RELIABILITY] {
        let before = read(object, property);
        assert_property_error(
            write(object, property, enumerated(2)),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
        assert_eq!(read(object, property), before, "{property:?}");
    }
    assert!(!fault(object));
}

fn assert_out_of_service_writes_are_taken(object: &mut dyn BACnetObject) {
    set_out_of_service(object, true);
    write(
        object,
        P::TRACKING_VALUE,
        enumerated(LifeSafetyState::ALARM.to_raw()),
    )
    .unwrap();
    assert_eq!(
        read(object, P::TRACKING_VALUE),
        enumerated(LifeSafetyState::ALARM.to_raw())
    );
    write(
        object,
        P::RELIABILITY,
        enumerated(Reliability::NO_SENSOR.to_raw()),
    )
    .unwrap();
    assert_eq!(
        read(object, P::RELIABILITY),
        enumerated(Reliability::NO_SENSOR.to_raw())
    );
    assert!(fault(object), "a simulated fault sets FAULT");
    // The simulation leaves the properties nothing simulates alone.
    for (property, expected) in [
        (P::PRESENT_VALUE, LifeSafetyState::QUIET.to_raw()),
        (P::SILENCED, SilencedState::UNSILENCED.to_raw()),
        (P::OPERATION_EXPECTED, LifeSafetyOperation::NONE.to_raw()),
    ] {
        assert_eq!(read(object, property), enumerated(expected), "{property:?}");
    }
}

#[test]
fn point_and_zone_refuse_tracking_value_and_reliability_writes_in_service() {
    assert_in_service_writes_are_refused(&mut point());
    assert_in_service_writes_are_refused(&mut zone());
}

#[test]
fn point_and_zone_take_tracking_value_and_reliability_writes_out_of_service() {
    assert_out_of_service_writes_are_taken(&mut point());
    assert_out_of_service_writes_are_taken(&mut zone());
}

fn assert_values_are_checked(object: &mut dyn BACnetObject) {
    set_out_of_service(object, true);
    write(
        object,
        P::TRACKING_VALUE,
        enumerated(LifeSafetyState::PRE_ALARM.to_raw()),
    )
    .unwrap();
    // 35..=255 are reserved for ASHRAE; past 65535 is outside the datatype.
    for raw in [35, 255, 65_536] {
        assert_property_error(
            write(object, P::TRACKING_VALUE, enumerated(raw)),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    for wrong in [PropertyValue::Unsigned(1), PropertyValue::Null] {
        assert_property_error(
            write(object, P::TRACKING_VALUE, wrong.clone()),
            ErrorCode::INVALID_DATA_TYPE,
        );
        assert_property_error(
            write(object, P::RELIABILITY, wrong),
            ErrorCode::INVALID_DATA_TYPE,
        );
    }
    assert_eq!(
        read(object, P::TRACKING_VALUE),
        enumerated(LifeSafetyState::PRE_ALARM.to_raw())
    );
    for raw in [11, 26, 63, 65_536] {
        assert_property_error(
            write(object, P::RELIABILITY, enumerated(raw)),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    assert_eq!(
        read(object, P::RELIABILITY),
        enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
    );
    // The proprietary range reads back verbatim.
    for raw in [256, 65_535] {
        write(object, P::TRACKING_VALUE, enumerated(raw)).unwrap();
        assert_eq!(read(object, P::TRACKING_VALUE), enumerated(raw));
    }
}

#[test]
fn simulated_values_outside_their_datatypes_are_refused_unchanged() {
    assert_values_are_checked(&mut point());
    assert_values_are_checked(&mut zone());
}

#[test]
fn return_to_service_serves_the_device_values_again() {
    let mut point = point();
    point.set_tracking_value(LifeSafetyState::PRE_ALARM);
    point
        .set_reliability_internal(Reliability::OVER_RANGE)
        .unwrap();
    set_out_of_service(&mut point, true);
    write(
        &mut point,
        P::TRACKING_VALUE,
        enumerated(LifeSafetyState::ALARM.to_raw()),
    )
    .unwrap();
    write(
        &mut point,
        P::RELIABILITY,
        enumerated(Reliability::NO_SENSOR.to_raw()),
    )
    .unwrap();
    // The device keeps reporting while the client simulates; the report is
    // held, and the application's Reliability is refused.
    point.set_tracking_value(LifeSafetyState::FAULT);
    assert_property_error(
        point.set_reliability_internal(Reliability::NO_FAULT_DETECTED),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    assert_eq!(
        read(&point, P::TRACKING_VALUE),
        enumerated(LifeSafetyState::ALARM.to_raw())
    );
    set_out_of_service(&mut point, false);
    assert_eq!(
        read(&point, P::TRACKING_VALUE),
        enumerated(LifeSafetyState::FAULT.to_raw())
    );
    assert_eq!(
        read(&point, P::RELIABILITY),
        enumerated(Reliability::OVER_RANGE.to_raw())
    );

    // A simulation the client never writes to returns the entry values.
    let mut zone = zone();
    zone.set_tracking_value(LifeSafetyState::ALARM);
    set_out_of_service(&mut zone, true);
    assert_eq!(
        read(&zone, P::TRACKING_VALUE),
        enumerated(LifeSafetyState::ALARM.to_raw())
    );
    write(
        &mut zone,
        P::TRACKING_VALUE,
        enumerated(LifeSafetyState::QUIET.to_raw()),
    )
    .unwrap();
    // A NULL or same-value Out_Of_Service write is not an edge.
    write(&mut zone, P::OUT_OF_SERVICE, PropertyValue::Null).unwrap();
    set_out_of_service(&mut zone, true);
    assert_eq!(
        read(&zone, P::TRACKING_VALUE),
        enumerated(LifeSafetyState::QUIET.to_raw())
    );
    set_out_of_service(&mut zone, false);
    assert_eq!(
        read(&zone, P::TRACKING_VALUE),
        enumerated(LifeSafetyState::ALARM.to_raw())
    );
    // In service the device's values are served directly again.
    zone.set_tracking_value(LifeSafetyState::PRE_ALARM);
    assert_eq!(
        read(&zone, P::TRACKING_VALUE),
        enumerated(LifeSafetyState::PRE_ALARM.to_raw())
    );
}

#[test]
fn reset_out_of_service_sees_the_simulation_and_holds_a_committed_tracking_value() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let point_seen = Arc::clone(&seen);
    let mut point = point().with_reset_executor(Arc::new(move |context| {
        point_seen.lock().unwrap().push(context.tracking_value);
        Ok(LifeSafetyPointResetCommit {
            present_value: Some(LifeSafetyState::QUIET),
            tracking_value: Some(LifeSafetyState::QUIET),
            silenced: Some(SilencedState::UNSILENCED),
        })
    }));
    let zone_seen = Arc::clone(&seen);
    let mut zone = zone().with_reset_executor(Arc::new(move |context| {
        zone_seen.lock().unwrap().push(context.tracking_value);
        Ok(LifeSafetyZoneResetCommit {
            present_value: Some(LifeSafetyState::QUIET),
            tracking_value: Some(LifeSafetyState::QUIET),
            silenced: Some(SilencedState::UNSILENCED),
        })
    }));
    point.set_present_value(LifeSafetyState::ALARM);
    point.set_silenced(SilencedState::ALL_SILENCED);
    zone.set_present_value(LifeSafetyState::ALARM);
    zone.set_silenced(SilencedState::ALL_SILENCED);
    for object in [
        &mut point as &mut dyn BACnetObject,
        &mut zone as &mut dyn BACnetObject,
    ] {
        set_out_of_service(object, true);
        write(
            object,
            P::TRACKING_VALUE,
            enumerated(LifeSafetyState::ALARM.to_raw()),
        )
        .unwrap();
        object
            .set_life_safety_operation_expected_internal(LifeSafetyOperation::RESET)
            .unwrap();
        let outcome = object
            .apply_life_safety_operation(LifeSafetyOperation::RESET)
            .unwrap();
        // Present_Value and Silenced commit as in service; the simulated
        // Tracking_Value stays, so it is not among the changes.
        assert_eq!(
            outcome.changed_properties,
            vec![P::PRESENT_VALUE, P::SILENCED, P::OPERATION_EXPECTED]
        );
        assert_eq!(
            read(object, P::TRACKING_VALUE),
            enumerated(LifeSafetyState::ALARM.to_raw())
        );
        set_out_of_service(object, false);
        assert_eq!(
            read(object, P::TRACKING_VALUE),
            enumerated(LifeSafetyState::QUIET.to_raw()),
            "the committed value is served from the return to service"
        );
    }
    assert_eq!(
        *seen.lock().unwrap(),
        vec![LifeSafetyState::ALARM, LifeSafetyState::ALARM]
    );
}
