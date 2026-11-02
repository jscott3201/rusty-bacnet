//! Car_Assigned_Direction, Car_Door_Zone, Car_Mode, Next_Stopping_Floor and
//! Car_Drive_Status, the Lift's single-valued car-state rows (Clause 12.59,
//! Table 12-77; #1052): set by the application in service and written over
//! the network only while the Lift is out of service.

use super::super::*;
use super::{
    assert_invalid_data_type, assert_property_error, assert_value_out_of_range, set_out_of_service,
};
use crate::property_metadata::{is_array_property, is_writable_property};
use bacnet_types::enums::{ErrorCode, LiftCarDirection, LiftCarDriveStatus, LiftCarMode};

type P = PropertyIdentifier;

/// The five rows this suite covers, with a well-typed value for each that a
/// new Lift doesn't already hold.
fn rows() -> [(P, PropertyValue); 5] {
    [
        (
            P::CAR_ASSIGNED_DIRECTION,
            PropertyValue::Enumerated(LiftCarDirection::UP_AND_DOWN.to_raw()),
        ),
        (P::CAR_DOOR_ZONE, PropertyValue::Boolean(true)),
        (
            P::CAR_MODE,
            PropertyValue::Enumerated(LiftCarMode::FIREFIGHTER_CONTROL.to_raw()),
        ),
        (P::NEXT_STOPPING_FLOOR, PropertyValue::Unsigned(255)),
        (
            P::CAR_DRIVE_STATUS,
            PropertyValue::Enumerated(LiftCarDriveStatus::MULTI_FLOOR_JUMP.to_raw()),
        ),
    ]
}

#[test]
fn lift_car_state_rows_start_unknown_and_follow_the_setters() {
    let mut lift = LiftObject::new(1, "LIFT-1", 3).unwrap();
    for (p, expected) in [
        (P::CAR_ASSIGNED_DIRECTION, PropertyValue::Enumerated(0)),
        (P::CAR_DOOR_ZONE, PropertyValue::Boolean(false)),
        (P::CAR_MODE, PropertyValue::Enumerated(0)),
        (P::NEXT_STOPPING_FLOOR, PropertyValue::Unsigned(1)),
        (P::CAR_DRIVE_STATUS, PropertyValue::Enumerated(0)),
    ] {
        assert_eq!(lift.read_property(p, None).unwrap(), expected, "{p:?}");
        assert!(!is_array_property(&lift, p), "{p:?}");
    }
    lift.set_car_assigned_direction(LiftCarDirection::DOWN)
        .unwrap();
    lift.set_car_door_zone(true);
    lift.set_car_mode(LiftCarMode::OCCUPANT_EVACUATION).unwrap();
    lift.set_next_stopping_floor(9);
    lift.set_car_drive_status(LiftCarDriveStatus::BRAKING)
        .unwrap();
    assert_eq!(lift.car_assigned_direction(), LiftCarDirection::DOWN);
    assert!(lift.car_door_zone());
    assert_eq!(lift.car_mode(), LiftCarMode::OCCUPANT_EVACUATION);
    assert_eq!(lift.next_stopping_floor(), 9);
    assert_eq!(lift.car_drive_status(), LiftCarDriveStatus::BRAKING);
    for (p, expected) in [
        (P::CAR_ASSIGNED_DIRECTION, PropertyValue::Enumerated(4)),
        (P::CAR_DOOR_ZONE, PropertyValue::Boolean(true)),
        (P::CAR_MODE, PropertyValue::Enumerated(13)),
        (P::NEXT_STOPPING_FLOOR, PropertyValue::Unsigned(9)),
        (P::CAR_DRIVE_STATUS, PropertyValue::Enumerated(2)),
    ] {
        assert_eq!(lift.read_property(p, None).unwrap(), expected, "{p:?}");
    }
}

#[test]
fn lift_car_state_setters_take_named_and_proprietary_values() {
    let mut lift = LiftObject::new(1, "LIFT-1", 3).unwrap();
    for &(_, direction) in LiftCarDirection::ALL_NAMED {
        lift.set_car_assigned_direction(direction).unwrap();
        assert_eq!(lift.car_assigned_direction(), direction);
    }
    for &(_, mode) in LiftCarMode::ALL_NAMED {
        lift.set_car_mode(mode).unwrap();
        assert_eq!(lift.car_mode(), mode);
    }
    for &(_, status) in LiftCarDriveStatus::ALL_NAMED {
        lift.set_car_drive_status(status).unwrap();
        assert_eq!(lift.car_drive_status(), status);
    }
    for raw in [1024, 65_535] {
        lift.set_car_assigned_direction(LiftCarDirection::from_raw(raw))
            .unwrap();
        lift.set_car_mode(LiftCarMode::from_raw(raw)).unwrap();
        lift.set_car_drive_status(LiftCarDriveStatus::from_raw(raw))
            .unwrap();
        for p in [P::CAR_ASSIGNED_DIRECTION, P::CAR_MODE, P::CAR_DRIVE_STATUS] {
            assert_eq!(
                lift.read_property(p, None).unwrap(),
                PropertyValue::Enumerated(raw)
            );
        }
    }
}

#[test]
fn lift_car_state_setters_refuse_values_outside_their_enumerations() {
    let mut lift = LiftObject::new(1, "LIFT-1", 3).unwrap();
    lift.set_car_assigned_direction(LiftCarDirection::UP)
        .unwrap();
    lift.set_car_mode(LiftCarMode::NORMAL).unwrap();
    lift.set_car_drive_status(LiftCarDriveStatus::STATIONARY)
        .unwrap();
    // The first reserved value of each, the top of the reserved range, and
    // values past 65535.
    for raw in [1023, 65_536, u32::MAX] {
        assert_value_out_of_range(
            lift.set_car_assigned_direction(LiftCarDirection::from_raw(raw)),
            &format!("direction {raw}"),
        );
        assert_value_out_of_range(
            lift.set_car_mode(LiftCarMode::from_raw(raw)),
            &format!("mode {raw}"),
        );
        assert_value_out_of_range(
            lift.set_car_drive_status(LiftCarDriveStatus::from_raw(raw)),
            &format!("drive status {raw}"),
        );
    }
    assert_value_out_of_range(
        lift.set_car_assigned_direction(LiftCarDirection::from_raw(6)),
        "direction 6",
    );
    assert_value_out_of_range(lift.set_car_mode(LiftCarMode::from_raw(14)), "mode 14");
    assert_value_out_of_range(
        lift.set_car_drive_status(LiftCarDriveStatus::from_raw(10)),
        "drive status 10",
    );
    assert_eq!(lift.car_assigned_direction(), LiftCarDirection::UP);
    assert_eq!(lift.car_mode(), LiftCarMode::NORMAL);
    assert_eq!(lift.car_drive_status(), LiftCarDriveStatus::STATIONARY);
}

#[test]
fn lift_car_state_rows_take_simulation_writes_only_out_of_service() {
    let mut lift = LiftObject::new(1, "LIFT-1", 3).unwrap();
    for (p, value) in rows() {
        assert!(is_writable_property(&lift, p), "{p:?}");
        let before = lift.read_property(p, None).unwrap();
        assert_property_error(
            lift.write_property(p, None, value, None),
            ErrorCode::WRITE_ACCESS_DENIED,
            &format!("{p:?} in service"),
        );
        assert_eq!(lift.read_property(p, None).unwrap(), before, "{p:?}");
    }
    set_out_of_service(&mut lift, true);
    for (p, value) in rows() {
        lift.write_property(p, None, value.clone(), None)
            .unwrap_or_else(|e| panic!("{p:?}: {e:?}"));
        assert_eq!(lift.read_property(p, None).unwrap(), value, "{p:?}");
    }
    // Proprietary values of the extensible enumerations round-trip too.
    for p in [P::CAR_ASSIGNED_DIRECTION, P::CAR_MODE, P::CAR_DRIVE_STATUS] {
        lift.write_property(p, None, PropertyValue::Enumerated(2048), None)
            .unwrap();
        assert_eq!(
            lift.read_property(p, None).unwrap(),
            PropertyValue::Enumerated(2048)
        );
    }
    assert_eq!(
        lift.car_assigned_direction(),
        LiftCarDirection::from_raw(2048)
    );
    assert!(lift.car_door_zone());
    assert_eq!(lift.next_stopping_floor(), 255);
    // The simulated values stay once the Lift is back in service, and
    // writes are refused again.
    set_out_of_service(&mut lift, false);
    for (p, value) in rows() {
        assert_property_error(
            lift.write_property(p, None, value, None),
            ErrorCode::WRITE_ACCESS_DENIED,
            &format!("{p:?} back in service"),
        );
    }
    assert_eq!(lift.car_mode(), LiftCarMode::from_raw(2048));
}

#[test]
fn lift_car_state_writes_refuse_bad_values_atomically() {
    let mut lift = LiftObject::new(1, "LIFT-1", 3).unwrap();
    set_out_of_service(&mut lift, true);
    let before: Vec<_> = rows()
        .iter()
        .map(|(p, _)| lift.read_property(*p, None).unwrap())
        .collect();
    for (p, raw) in [
        (P::CAR_ASSIGNED_DIRECTION, 6u32),
        (P::CAR_ASSIGNED_DIRECTION, 65_536),
        (P::CAR_MODE, 14),
        (P::CAR_MODE, 1023),
        (P::CAR_DRIVE_STATUS, 10),
        (P::CAR_DRIVE_STATUS, u32::MAX),
    ] {
        assert_value_out_of_range(
            lift.write_property(p, None, PropertyValue::Enumerated(raw), None),
            &format!("{p:?} {raw}"),
        );
    }
    assert_value_out_of_range(
        lift.write_property(
            P::NEXT_STOPPING_FLOOR,
            None,
            PropertyValue::Unsigned(256),
            None,
        ),
        "Next_Stopping_Floor 256",
    );
    for (p, value) in [
        (P::CAR_ASSIGNED_DIRECTION, PropertyValue::Unsigned(3)),
        (P::CAR_DOOR_ZONE, PropertyValue::Enumerated(1)),
        (P::CAR_MODE, PropertyValue::Unsigned(1)),
        (P::NEXT_STOPPING_FLOOR, PropertyValue::Enumerated(2)),
        (P::CAR_DRIVE_STATUS, PropertyValue::Boolean(true)),
    ] {
        assert_invalid_data_type(lift.write_property(p, None, value, None), &format!("{p:?}"));
    }
    let after: Vec<_> = rows()
        .iter()
        .map(|(p, _)| lift.read_property(*p, None).unwrap())
        .collect();
    assert_eq!(after, before);
}
