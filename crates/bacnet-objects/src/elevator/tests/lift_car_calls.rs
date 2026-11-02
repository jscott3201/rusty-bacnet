//! Assigned_Landing_Calls, Making_Car_Call, Registered_Car_Call and
//! Car_Door_Command, the Lift's per-door call and command arrays (Clause
//! 12.59, Table 12-77; #1052): one element per car door, set by the
//! application in service and written over the network only while the Lift
//! is out of service.

use super::super::*;
use super::{
    assert_invalid_data_type, assert_property_error, assert_value_out_of_range, frame,
    set_out_of_service,
};
use crate::property_metadata::{is_array_property, is_writable_property};
use bacnet_types::constructed::{
    AssignedLandingCall, BACnetAssignedLandingCalls, BACnetLiftCarCallList,
};
use bacnet_types::enums::{DoorStatus, ErrorCode, LiftCarDirection, LiftCarDoorCommand};

type P = PropertyIdentifier;

/// The four arrays this suite covers.
const CALL_ARRAYS: [P; 4] = [
    P::ASSIGNED_LANDING_CALLS,
    P::MAKING_CAR_CALL,
    P::REGISTERED_CAR_CALL,
    P::CAR_DOOR_COMMAND,
];

/// A two-door Lift, out of service.
fn simulated_lift() -> LiftObject {
    let mut lift = LiftObject::new(1, "LIFT-1", 3).unwrap();
    lift.set_car_door_status(vec![DoorStatus::CLOSED, DoorStatus::CLOSED])
        .unwrap();
    set_out_of_service(&mut lift, true);
    lift
}

fn calls(entries: &[(u8, LiftCarDirection)]) -> BACnetAssignedLandingCalls {
    BACnetAssignedLandingCalls {
        landing_calls: entries
            .iter()
            .map(|&(floor_number, direction)| AssignedLandingCall {
                floor_number,
                direction,
            })
            .collect(),
    }
}

fn floors(floor_numbers: &[u8]) -> BACnetLiftCarCallList {
    BACnetLiftCarCallList {
        floor_numbers: floor_numbers.to_vec(),
    }
}

fn command(command: LiftCarDoorCommand) -> PropertyValue {
    PropertyValue::Enumerated(command.to_raw())
}

#[test]
fn lift_call_arrays_start_with_one_empty_element_per_door() {
    let lift = LiftObject::new(1, "LIFT-1", 3).unwrap();
    assert_eq!(
        lift.assigned_landing_calls(),
        [BACnetAssignedLandingCalls::default()]
    );
    assert_eq!(lift.making_car_call(), [0]);
    assert_eq!(
        lift.registered_car_call(),
        [BACnetLiftCarCallList::default()]
    );
    assert_eq!(lift.car_door_command(), [LiftCarDoorCommand::NONE]);
    for (p, element) in [
        (P::ASSIGNED_LANDING_CALLS, frame(&[0x0E, 0x0F])),
        (P::MAKING_CAR_CALL, PropertyValue::Unsigned(0)),
        (P::REGISTERED_CAR_CALL, frame(&[0x0E, 0x0F])),
        (P::CAR_DOOR_COMMAND, PropertyValue::Enumerated(0)),
    ] {
        assert!(is_array_property(&lift, p), "{p:?}");
        assert_eq!(
            lift.read_property(p, None).unwrap(),
            PropertyValue::List(vec![element.clone()]),
            "{p:?}"
        );
        assert_eq!(
            lift.read_property(p, Some(0)).unwrap(),
            PropertyValue::Unsigned(1)
        );
        assert_eq!(lift.read_property(p, Some(1)).unwrap(), element, "{p:?}");
        assert_property_error(
            lift.read_property(p, Some(2)).map(|_| ()),
            ErrorCode::INVALID_ARRAY_INDEX,
            &format!("{p:?} [2]"),
        );
    }
}

#[test]
fn lift_call_arrays_follow_the_car_door_count() {
    let mut lift = LiftObject::new(1, "LIFT-1", 3).unwrap();
    lift.set_assigned_landing_calls(vec![calls(&[(2, LiftCarDirection::UP)])])
        .unwrap();
    lift.set_making_car_call(vec![3]).unwrap();
    lift.set_registered_car_call(vec![floors(&[3])]).unwrap();
    lift.set_car_door_command(vec![LiftCarDoorCommand::OPEN])
        .unwrap();
    // A second door starts with no calls and no pending command.
    lift.set_car_door_status(vec![DoorStatus::CLOSED, DoorStatus::OPENED])
        .unwrap();
    assert_eq!(
        lift.assigned_landing_calls(),
        [
            calls(&[(2, LiftCarDirection::UP)]),
            BACnetAssignedLandingCalls::default()
        ]
    );
    assert_eq!(lift.making_car_call(), [3, 0]);
    assert_eq!(
        lift.registered_car_call(),
        [floors(&[3]), BACnetLiftCarCallList::default()]
    );
    assert_eq!(
        lift.car_door_command(),
        [LiftCarDoorCommand::OPEN, LiftCarDoorCommand::NONE]
    );
    for p in CALL_ARRAYS {
        assert_eq!(
            lift.read_property(p, Some(0)).unwrap(),
            PropertyValue::Unsigned(2),
            "{p:?}"
        );
    }
    // Dropping the first door's status drops the second door's elements.
    lift.set_car_door_status(vec![DoorStatus::CLOSED]).unwrap();
    assert_eq!(lift.making_car_call(), [3]);
    assert_eq!(lift.car_door_command(), [LiftCarDoorCommand::OPEN]);
    assert_eq!(lift.assigned_landing_calls().len(), 1);
    assert_eq!(lift.registered_car_call(), [floors(&[3])]);
}

#[test]
fn lift_call_array_setters_refuse_mismatches_atomically() {
    let mut lift = simulated_lift();
    let assigned = vec![
        calls(&[
            (1, LiftCarDirection::DOWN),
            (4, LiftCarDirection::from_raw(1024)),
        ]),
        calls(&[]),
    ];
    lift.set_assigned_landing_calls(assigned.clone()).unwrap();
    lift.set_making_car_call(vec![2, 0]).unwrap();
    lift.set_registered_car_call(vec![floors(&[2, 5]), floors(&[])])
        .unwrap();
    lift.set_car_door_command(vec![LiftCarDoorCommand::CLOSE, LiftCarDoorCommand::NONE])
        .unwrap();
    let before: Vec<_> = CALL_ARRAYS
        .iter()
        .map(|&p| lift.read_property(p, None).unwrap())
        .collect();
    // One element for two car doors, then a direction or command outside
    // its enumeration.
    for (result, context) in [
        (
            lift.set_assigned_landing_calls(vec![calls(&[])]),
            "assigned calls size",
        ),
        (lift.set_making_car_call(vec![1, 2, 3]), "car call size"),
        (
            lift.set_registered_car_call(vec![floors(&[])]),
            "registered calls size",
        ),
        (
            lift.set_car_door_command(vec![LiftCarDoorCommand::OPEN]),
            "command size",
        ),
        (
            lift.set_assigned_landing_calls(vec![
                calls(&[]),
                calls(&[(1, LiftCarDirection::from_raw(6))]),
            ]),
            "reserved direction 6",
        ),
        (
            lift.set_assigned_landing_calls(vec![
                calls(&[(1, LiftCarDirection::from_raw(65_536))]),
                calls(&[]),
            ]),
            "direction 65536",
        ),
        (
            lift.set_car_door_command(vec![
                LiftCarDoorCommand::NONE,
                LiftCarDoorCommand::from_raw(3),
            ]),
            "command 3",
        ),
        (
            lift.set_car_door_command(vec![
                LiftCarDoorCommand::from_raw(1024),
                LiftCarDoorCommand::NONE,
            ]),
            "command 1024 (no proprietary range)",
        ),
    ] {
        assert_value_out_of_range(result, context);
    }
    let after: Vec<_> = CALL_ARRAYS
        .iter()
        .map(|&p| lift.read_property(p, None).unwrap())
        .collect();
    assert_eq!(after, before);
    assert_eq!(lift.assigned_landing_calls(), assigned);
}

#[test]
fn lift_call_arrays_take_whole_and_element_writes_out_of_service() {
    let mut lift = simulated_lift();
    // Door 1: floor 2 UP and floor 5 proprietary direction 1024; door 2:
    // floor 1 UP_AND_DOWN.
    let first = [
        0x0E, 0x09, 0x02, 0x19, 0x03, 0x09, 0x05, 0x1A, 0x04, 0x00, 0x0F,
    ];
    let second = [0x0E, 0x09, 0x01, 0x19, 0x05, 0x0F];
    for (p, whole, index, element, decoded) in [
        (
            P::ASSIGNED_LANDING_CALLS,
            vec![frame(&first), frame(&second)],
            2,
            frame(&[0x0E, 0x0F]),
            "landing calls",
        ),
        (
            P::MAKING_CAR_CALL,
            vec![PropertyValue::Unsigned(7), PropertyValue::Unsigned(255)],
            1,
            PropertyValue::Unsigned(0),
            "car calls",
        ),
        (
            P::REGISTERED_CAR_CALL,
            vec![
                frame(&[0x0E, 0x21, 0x07, 0x21, 0x02, 0x0F]),
                frame(&[0x0E, 0x0F]),
            ],
            2,
            frame(&[0x0E, 0x21, 0xFF, 0x0F]),
            "registered calls",
        ),
        (
            P::CAR_DOOR_COMMAND,
            vec![
                command(LiftCarDoorCommand::OPEN),
                command(LiftCarDoorCommand::CLOSE),
            ],
            1,
            command(LiftCarDoorCommand::NONE),
            "commands",
        ),
    ] {
        let whole = PropertyValue::List(whole);
        lift.write_property(p, None, whole.clone(), None)
            .unwrap_or_else(|e| panic!("{decoded}: {e:?}"));
        assert_eq!(lift.read_property(p, None).unwrap(), whole, "{decoded}");
        lift.write_property(p, Some(index), element.clone(), None)
            .unwrap_or_else(|e| panic!("{decoded} [{index}]: {e:?}"));
        assert_eq!(
            lift.read_property(p, Some(index)).unwrap(),
            element,
            "{decoded}"
        );
    }
    assert_eq!(
        lift.assigned_landing_calls(),
        [
            calls(&[
                (2, LiftCarDirection::UP),
                (5, LiftCarDirection::from_raw(1024))
            ]),
            calls(&[]),
        ]
    );
    assert_eq!(lift.making_car_call(), [0, 255]);
    assert_eq!(
        lift.registered_car_call(),
        [floors(&[7, 2]), floors(&[255])]
    );
    assert_eq!(
        lift.car_door_command(),
        [LiftCarDoorCommand::NONE, LiftCarDoorCommand::CLOSE]
    );

    // A one-door car's whole array arrives from the service decoder as the
    // bare element.
    let mut lift = LiftObject::new(1, "LIFT-1", 3).unwrap();
    set_out_of_service(&mut lift, true);
    lift.write_property(P::MAKING_CAR_CALL, None, PropertyValue::Unsigned(4), None)
        .unwrap();
    lift.write_property(
        P::REGISTERED_CAR_CALL,
        None,
        frame(&[0x0E, 0x21, 0x04, 0x0F]),
        None,
    )
    .unwrap();
    assert_eq!(lift.making_car_call(), [4]);
    assert_eq!(lift.registered_car_call(), [floors(&[4])]);
}

#[test]
fn lift_call_array_writes_refuse_bad_values_atomically() {
    let mut lift = simulated_lift();
    let before: Vec<_> = CALL_ARRAYS
        .iter()
        .map(|&p| lift.read_property(p, None).unwrap())
        .collect();
    let cases: &[(P, PropertyValue, ErrorCode, &str)] = &[
        (
            P::ASSIGNED_LANDING_CALLS,
            PropertyValue::Unsigned(0),
            ErrorCode::INVALID_DATA_TYPE,
            "not a frame",
        ),
        (
            P::ASSIGNED_LANDING_CALLS,
            frame(&[0x0E, 0x09, 0x01, 0x19, 0x06, 0x0F]),
            ErrorCode::VALUE_OUT_OF_RANGE,
            "reserved direction 6",
        ),
        (
            P::ASSIGNED_LANDING_CALLS,
            frame(&[0x0E, 0x0A, 0x01, 0x00, 0x19, 0x03, 0x0F]),
            ErrorCode::VALUE_OUT_OF_RANGE,
            "floor 256",
        ),
        (
            P::ASSIGNED_LANDING_CALLS,
            frame(&[0x0E, 0x09, 0x01, 0x0F]),
            ErrorCode::INVALID_DATA_ENCODING,
            "floor without direction",
        ),
        (
            P::ASSIGNED_LANDING_CALLS,
            frame(&[0x0E, 0x0F, 0x0E, 0x0F]),
            ErrorCode::INVALID_DATA_ENCODING,
            "two frames in one element",
        ),
        (
            P::MAKING_CAR_CALL,
            PropertyValue::Unsigned(256),
            ErrorCode::VALUE_OUT_OF_RANGE,
            "floor 256",
        ),
        (
            P::MAKING_CAR_CALL,
            PropertyValue::Enumerated(3),
            ErrorCode::INVALID_DATA_TYPE,
            "Enumerated floor",
        ),
        (
            P::REGISTERED_CAR_CALL,
            frame(&[0x0E, 0x22, 0x01, 0x00, 0x0F]),
            ErrorCode::VALUE_OUT_OF_RANGE,
            "floor 256",
        ),
        (
            P::REGISTERED_CAR_CALL,
            frame(&[0x0E, 0x09, 0x01, 0x0F]),
            ErrorCode::INVALID_DATA_ENCODING,
            "context-tagged floor",
        ),
        (
            P::REGISTERED_CAR_CALL,
            PropertyValue::Unsigned(3),
            ErrorCode::INVALID_DATA_TYPE,
            "bare floor",
        ),
        (
            P::CAR_DOOR_COMMAND,
            PropertyValue::Enumerated(3),
            ErrorCode::VALUE_OUT_OF_RANGE,
            "command 3",
        ),
        (
            P::CAR_DOOR_COMMAND,
            PropertyValue::Enumerated(1024),
            ErrorCode::VALUE_OUT_OF_RANGE,
            "command 1024",
        ),
        (
            P::CAR_DOOR_COMMAND,
            PropertyValue::Unsigned(1),
            ErrorCode::INVALID_DATA_TYPE,
            "Unsigned command",
        ),
    ];
    for (p, value, expected, context) in cases {
        let context = format!("{p:?} {context}");
        assert_property_error(
            lift.write_property(*p, Some(1), value.clone(), None),
            *expected,
            &context,
        );
        // The same element inside a whole-array write fails the same way.
        let good = lift.read_property(*p, Some(2)).unwrap();
        assert_property_error(
            lift.write_property(
                *p,
                None,
                PropertyValue::List(vec![good, value.clone()]),
                None,
            ),
            *expected,
            &context,
        );
    }
    // Neither the size nor an element past it is writable.
    for p in CALL_ARRAYS {
        let element = lift.read_property(p, Some(1)).unwrap();
        assert_value_out_of_range(
            lift.write_property(p, None, PropertyValue::List(vec![element.clone()]), None),
            &format!("{p:?} one element for two doors"),
        );
        assert_property_error(
            lift.write_property(p, Some(0), PropertyValue::Unsigned(1), None),
            ErrorCode::WRITE_ACCESS_DENIED,
            &format!("{p:?} [0]"),
        );
        assert_property_error(
            lift.write_property(p, Some(3), element, None),
            ErrorCode::INVALID_ARRAY_INDEX,
            &format!("{p:?} [3]"),
        );
    }
    assert_invalid_data_type(
        lift.write_property(P::MAKING_CAR_CALL, Some(1), PropertyValue::Null, None),
        "Making_Car_Call Null",
    );
    let after: Vec<_> = CALL_ARRAYS
        .iter()
        .map(|&p| lift.read_property(p, None).unwrap())
        .collect();
    assert_eq!(after, before);
}

#[test]
fn lift_call_arrays_refuse_writes_in_service() {
    let mut lift = LiftObject::new(1, "LIFT-1", 3).unwrap();
    for (p, element) in [
        (
            P::ASSIGNED_LANDING_CALLS,
            frame(&[0x0E, 0x09, 0x02, 0x19, 0x03, 0x0F]),
        ),
        (P::MAKING_CAR_CALL, PropertyValue::Unsigned(2)),
        (P::REGISTERED_CAR_CALL, frame(&[0x0E, 0x21, 0x02, 0x0F])),
        (P::CAR_DOOR_COMMAND, command(LiftCarDoorCommand::OPEN)),
    ] {
        assert!(is_writable_property(&lift, p), "{p:?}");
        let before = lift.read_property(p, None).unwrap();
        for index in [None, Some(1)] {
            assert_property_error(
                lift.write_property(p, index, element.clone(), None),
                ErrorCode::WRITE_ACCESS_DENIED,
                &format!("{p:?} {index:?} in service"),
            );
        }
        assert_eq!(lift.read_property(p, None).unwrap(), before, "{p:?}");
    }
    // Simulated values stay when the Lift returns to service, until the
    // application sets new ones.
    set_out_of_service(&mut lift, true);
    lift.write_property(
        P::MAKING_CAR_CALL,
        Some(1),
        PropertyValue::Unsigned(6),
        None,
    )
    .unwrap();
    set_out_of_service(&mut lift, false);
    assert_eq!(lift.making_car_call(), [6]);
    assert_property_error(
        lift.write_property(
            P::MAKING_CAR_CALL,
            Some(1),
            PropertyValue::Unsigned(0),
            None,
        ),
        ErrorCode::WRITE_ACCESS_DENIED,
        "back in service",
    );
    lift.set_making_car_call(vec![0]).unwrap();
    assert_eq!(lift.making_car_call(), [0]);
}
