//! The Lift serves its Table 12-77 rows with their table datatypes (Clause
//! 12.59; #1021): an Unsigned8 Car_Position, a REAL Car_Load with its
//! Car_Load_Units, BACnetARRAYs of BACnetDoorStatus and
//! BACnetLandingDoorStatus per car door, and the required Passenger_Alarm and
//! Fault_Signals. Tracking_Value and Floor_Number, which the table doesn't
//! define, are gone.

use super::super::*;
use super::{assert_invalid_data_type, assert_value_out_of_range};
use bacnet_types::constructed::{BACnetLandingDoorStatus, LandingDoor};
use bacnet_types::enums::{DoorStatus, EngineeringUnits, ErrorClass, ErrorCode, LiftFault};

type P = PropertyIdentifier;

fn lift() -> LiftObject {
    LiftObject::new(1, "LIFT-1", 3).unwrap()
}

fn read(lift: &LiftObject, property: P) -> PropertyValue {
    lift.read_property(property, None).unwrap()
}

fn write(lift: &mut LiftObject, property: P, value: PropertyValue) -> Result<(), Error> {
    lift.write_property(property, None, value, None)
}

fn assert_property_error(result: Result<impl std::fmt::Debug, Error>, expected: ErrorCode) {
    match result {
        Err(Error::Protocol { class, code }) => {
            assert_eq!(class, ErrorClass::PROPERTY);
            assert_eq!(code, expected, "expected {expected:?}");
        }
        other => panic!("expected PROPERTY/{expected:?}, got {other:?}"),
    }
}

fn landing(doors: &[(u8, DoorStatus)]) -> BACnetLandingDoorStatus {
    BACnetLandingDoorStatus {
        landing_doors: doors
            .iter()
            .map(|&(floor_number, door_status)| LandingDoor {
                floor_number,
                door_status,
            })
            .collect(),
    }
}

#[test]
fn lift_tracking_value_and_floor_number_are_gone() {
    let mut lift = lift();
    for property in [P::TRACKING_VALUE, P::FLOOR_NUMBER] {
        assert!(!lift.property_list().contains(&property), "{property:?}");
        assert_property_error(
            lift.read_property(property, None),
            ErrorCode::UNKNOWN_PROPERTY,
        );
        assert_property_error(
            write(&mut lift, property, PropertyValue::Unsigned(2)),
            ErrorCode::UNKNOWN_PROPERTY,
        );
    }
}

#[test]
fn lift_car_position_accepts_the_unsigned8_range() {
    let mut lift = lift();
    assert_eq!(read(&lift, P::CAR_POSITION), PropertyValue::Unsigned(1));
    for raw in [0u64, 254, 255] {
        write(&mut lift, P::CAR_POSITION, PropertyValue::Unsigned(raw))
            .unwrap_or_else(|e| panic!("Car_Position {raw} must be accepted: {e:?}"));
        assert_eq!(read(&lift, P::CAR_POSITION), PropertyValue::Unsigned(raw));
    }
}

#[test]
fn lift_car_position_refuses_values_above_255_atomically() {
    let mut lift = lift();
    write(&mut lift, P::CAR_POSITION, PropertyValue::Unsigned(12)).unwrap();
    for raw in [256u64, 65_535, u64::from(u32::MAX), u64::MAX] {
        assert_value_out_of_range(
            write(&mut lift, P::CAR_POSITION, PropertyValue::Unsigned(raw)),
            &format!("Car_Position {raw}"),
        );
        assert_eq!(read(&lift, P::CAR_POSITION), PropertyValue::Unsigned(12));
    }
}

#[test]
fn lift_car_load_is_a_real_in_car_load_units() {
    let mut lift = lift();
    assert_eq!(read(&lift, P::CAR_LOAD), PropertyValue::Real(0.0));
    assert_eq!(
        read(&lift, P::CAR_LOAD_UNITS),
        PropertyValue::Enumerated(EngineeringUnits::PERCENT.to_raw())
    );
    // No percentage cap: the value is in Car_Load_Units.
    for value in [101.0f32, 1250.5, -0.5, f32::MAX] {
        write(&mut lift, P::CAR_LOAD, PropertyValue::Real(value)).unwrap();
        assert_eq!(read(&lift, P::CAR_LOAD), PropertyValue::Real(value));
    }
}

#[test]
fn lift_car_load_refuses_non_finite_and_non_real_values_atomically() {
    let mut lift = lift();
    write(&mut lift, P::CAR_LOAD, PropertyValue::Real(42.5)).unwrap();
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_value_out_of_range(
            write(&mut lift, P::CAR_LOAD, PropertyValue::Real(value)),
            &format!("Car_Load {value}"),
        );
        assert_eq!(read(&lift, P::CAR_LOAD), PropertyValue::Real(42.5));
    }
    // The old Unsigned percentage is the wrong datatype now.
    assert_invalid_data_type(
        write(&mut lift, P::CAR_LOAD, PropertyValue::Unsigned(50)),
        "Unsigned Car_Load",
    );
    assert_eq!(read(&lift, P::CAR_LOAD), PropertyValue::Real(42.5));
}

#[test]
fn lift_car_load_units_is_set_by_the_application_only() {
    let mut lift = lift();
    lift.set_car_load_units(EngineeringUnits::KILOGRAMS)
        .unwrap();
    assert_eq!(lift.car_load_units(), EngineeringUnits::KILOGRAMS);
    assert_eq!(
        read(&lift, P::CAR_LOAD_UNITS),
        PropertyValue::Enumerated(EngineeringUnits::KILOGRAMS.to_raw())
    );
    // A vendor unit is kept; a value past 65535 is refused.
    lift.set_car_load_units(EngineeringUnits::from_raw(65_535))
        .unwrap();
    for raw in [65_536u32, u32::MAX] {
        assert_value_out_of_range(
            lift.set_car_load_units(EngineeringUnits::from_raw(raw)),
            &format!("Car_Load_Units {raw}"),
        );
        assert_eq!(lift.car_load_units().to_raw(), 65_535);
    }
    assert!(!lift.is_writable_property(P::CAR_LOAD_UNITS));
    assert_property_error(
        write(&mut lift, P::CAR_LOAD_UNITS, PropertyValue::Enumerated(39)),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
}

#[test]
fn lift_car_door_status_is_an_array_of_door_status() {
    let mut lift = lift();
    // One car door, status UNKNOWN (2).
    assert!(lift.is_array_property(P::CAR_DOOR_STATUS));
    assert_eq!(lift.car_door_status(), [DoorStatus::UNKNOWN]);
    assert_eq!(
        read(&lift, P::CAR_DOOR_STATUS),
        PropertyValue::List(vec![PropertyValue::Enumerated(2)])
    );
    let doors = vec![
        DoorStatus::SAFETY_LOCKED,
        DoorStatus::LIMITED_OPENED,
        DoorStatus::from_raw(1024),
    ];
    lift.set_car_door_status(doors.clone()).unwrap();
    assert_eq!(lift.car_door_status(), doors);
    for (index, expected) in [
        (0, PropertyValue::Unsigned(3)),
        (1, PropertyValue::Enumerated(8)),
        (2, PropertyValue::Enumerated(9)),
        (3, PropertyValue::Enumerated(1024)),
    ] {
        assert_eq!(
            lift.read_property(P::CAR_DOOR_STATUS, Some(index)).unwrap(),
            expected
        );
    }
    for index in [4, u32::MAX] {
        assert_property_error(
            lift.read_property(P::CAR_DOOR_STATUS, Some(index)),
            ErrorCode::INVALID_ARRAY_INDEX,
        );
    }
}

#[test]
fn lift_set_car_door_status_refuses_statuses_outside_door_status_atomically() {
    let mut lift = lift();
    lift.set_car_door_status(vec![DoorStatus::CLOSED, DoorStatus::OPENED])
        .unwrap();
    lift.set_landing_door_status(vec![landing(&[(1, DoorStatus::CLOSED)]), landing(&[])])
        .unwrap();
    let before = lift.landing_door_status().to_vec();
    for raw in [10u32, 1023, 65_536, u32::MAX] {
        assert_value_out_of_range(
            lift.set_car_door_status(vec![DoorStatus::CLOSED, DoorStatus::from_raw(raw)]),
            &format!("Car_Door_Status {raw}"),
        );
        assert_eq!(
            lift.car_door_status(),
            [DoorStatus::CLOSED, DoorStatus::OPENED]
        );
        assert_eq!(lift.landing_door_status(), before);
    }
}

#[test]
fn lift_landing_door_status_holds_one_element_per_car_door() {
    let mut lift = lift();
    assert!(lift.is_array_property(P::LANDING_DOOR_STATUS));
    // One car door with no landing doors: the empty landing-doors frame.
    assert_eq!(
        read(&lift, P::LANDING_DOOR_STATUS),
        PropertyValue::List(vec![PropertyValue::ApplicationData(vec![0x0E, 0x0F])])
    );
    lift.set_car_door_status(vec![DoorStatus::CLOSED, DoorStatus::CLOSED])
        .unwrap();
    lift.set_landing_door_status(vec![
        landing(&[(1, DoorStatus::SAFETY_LOCKED), (2, DoorStatus::NONE)]),
        landing(&[(3, DoorStatus::from_raw(1024))]),
    ])
    .unwrap();
    // Floor [0] / door-status [1] pairs inside opening/closing tag 0.
    let first = vec![0x0E, 0x09, 0x01, 0x19, 0x08, 0x09, 0x02, 0x19, 0x05, 0x0F];
    let second = vec![0x0E, 0x09, 0x03, 0x1A, 0x04, 0x00, 0x0F];
    assert_eq!(
        read(&lift, P::LANDING_DOOR_STATUS),
        PropertyValue::List(vec![
            PropertyValue::ApplicationData(first.clone()),
            PropertyValue::ApplicationData(second.clone()),
        ])
    );
    for (index, expected) in [
        (0, PropertyValue::Unsigned(2)),
        (1, PropertyValue::ApplicationData(first)),
        (2, PropertyValue::ApplicationData(second)),
    ] {
        assert_eq!(
            lift.read_property(P::LANDING_DOOR_STATUS, Some(index))
                .unwrap(),
            expected
        );
    }
    assert_property_error(
        lift.read_property(P::LANDING_DOOR_STATUS, Some(3)),
        ErrorCode::INVALID_ARRAY_INDEX,
    );
}

#[test]
fn lift_landing_door_status_follows_the_car_door_count() {
    let mut lift = lift();
    let first = landing(&[(1, DoorStatus::OPENED)]);
    lift.set_landing_door_status(vec![first.clone()]).unwrap();
    // A second car door starts with no landing doors.
    lift.set_car_door_status(vec![DoorStatus::OPENED, DoorStatus::CLOSED])
        .unwrap();
    assert_eq!(
        lift.landing_door_status(),
        [first.clone(), BACnetLandingDoorStatus::default()]
    );
    // Dropping it drops its landing doors.
    lift.set_car_door_status(vec![DoorStatus::OPENED]).unwrap();
    assert_eq!(lift.landing_door_status(), [first]);
}

#[test]
fn lift_set_landing_door_status_refuses_mismatches_atomically() {
    let mut lift = lift();
    let before = vec![landing(&[(4, DoorStatus::CLOSING)])];
    lift.set_landing_door_status(before.clone()).unwrap();
    for (value, context) in [
        (vec![], "no element for the car door"),
        (
            vec![landing(&[]), landing(&[])],
            "two elements for one car door",
        ),
        (
            vec![landing(&[(4, DoorStatus::from_raw(10))])],
            "reserved landing door status",
        ),
        (
            vec![landing(&[(4, DoorStatus::from_raw(65_536))])],
            "landing door status above 65535",
        ),
    ] {
        assert_value_out_of_range(lift.set_landing_door_status(value), context);
        assert_eq!(lift.landing_door_status(), before, "{context}");
    }
}

#[test]
fn lift_floor_text_and_in_service_door_arrays_refuse_writes() {
    // Floor_Text is read-only over the network. The door arrays take writes
    // only while Out_Of_Service is TRUE (tests/lift_door_simulation.rs).
    for out_of_service in [false, true] {
        let mut lift = lift();
        write(
            &mut lift,
            P::OUT_OF_SERVICE,
            PropertyValue::Boolean(out_of_service),
        )
        .unwrap();
        let denied: &[P] = if out_of_service {
            &[P::FLOOR_TEXT]
        } else {
            &[P::CAR_DOOR_STATUS, P::LANDING_DOOR_STATUS, P::FLOOR_TEXT]
        };
        for &property in denied {
            let before = read(&lift, property);
            assert_eq!(
                lift.is_writable_property(property),
                property != P::FLOOR_TEXT,
                "{property:?}"
            );
            for index in [None, Some(1)] {
                let element = lift.read_property(property, Some(1)).unwrap();
                assert_property_error(
                    lift.write_property(property, index, element, None),
                    ErrorCode::WRITE_ACCESS_DENIED,
                );
            }
            assert_eq!(read(&lift, property), before, "{property:?}");
        }
    }
}

#[test]
fn lift_floor_text_is_an_array_indexed_by_floor() {
    let lift = lift();
    assert!(lift.is_array_property(P::FLOOR_TEXT));
    assert_eq!(
        lift.read_property(P::FLOOR_TEXT, Some(0)).unwrap(),
        PropertyValue::Unsigned(3)
    );
    assert_eq!(
        lift.read_property(P::FLOOR_TEXT, Some(2)).unwrap(),
        PropertyValue::CharacterString("Floor 2".into())
    );
    assert_property_error(
        lift.read_property(P::FLOOR_TEXT, Some(4)),
        ErrorCode::INVALID_ARRAY_INDEX,
    );
}

#[test]
fn lift_passenger_alarm_is_a_writable_boolean() {
    let mut lift = lift();
    assert_eq!(
        read(&lift, P::PASSENGER_ALARM),
        PropertyValue::Boolean(false)
    );
    write(&mut lift, P::PASSENGER_ALARM, PropertyValue::Boolean(true)).unwrap();
    assert_eq!(
        read(&lift, P::PASSENGER_ALARM),
        PropertyValue::Boolean(true)
    );
    assert_invalid_data_type(
        write(&mut lift, P::PASSENGER_ALARM, PropertyValue::Enumerated(0)),
        "Enumerated Passenger_Alarm",
    );
    assert_eq!(
        read(&lift, P::PASSENGER_ALARM),
        PropertyValue::Boolean(true)
    );
}

#[test]
fn lift_fault_signals_accepts_sets_of_lift_faults() {
    let mut lift = lift();
    assert_eq!(read(&lift, P::FAULT_SIGNALS), PropertyValue::List(vec![]));
    assert!(!lift.is_array_property(P::FAULT_SIGNALS));
    // Every named fault at once, then the proprietary bounds, then a
    // wire singleton, then the empty set.
    let named = PropertyValue::List(
        LiftFault::ALL_NAMED
            .iter()
            .map(|&(_, fault)| PropertyValue::Enumerated(fault.to_raw()))
            .collect(),
    );
    assert_eq!(LiftFault::ALL_NAMED.len(), 17);
    for (value, expected) in [
        (named.clone(), named),
        (
            PropertyValue::List(vec![
                PropertyValue::Enumerated(1024),
                PropertyValue::Enumerated(65_535),
            ]),
            PropertyValue::List(vec![
                PropertyValue::Enumerated(1024),
                PropertyValue::Enumerated(65_535),
            ]),
        ),
        (
            PropertyValue::Enumerated(16),
            PropertyValue::List(vec![PropertyValue::Enumerated(16)]),
        ),
        (PropertyValue::List(vec![]), PropertyValue::List(vec![])),
    ] {
        write(&mut lift, P::FAULT_SIGNALS, value).unwrap();
        assert_eq!(read(&lift, P::FAULT_SIGNALS), expected);
    }
}

#[test]
fn lift_fault_signals_refuses_bad_sets_atomically() {
    let mut lift = lift();
    let prior = PropertyValue::List(vec![
        PropertyValue::Enumerated(LiftFault::START_FAILURE.to_raw()),
        PropertyValue::Enumerated(2048),
    ]);
    write(&mut lift, P::FAULT_SIGNALS, prior.clone()).unwrap();
    // The Lift names the element it refuses, counting from 1 (#1048); a
    // value that is no list and no fault names none.
    for (value, code, position) in [
        (
            PropertyValue::Enumerated(17),
            ErrorCode::VALUE_OUT_OF_RANGE,
            Some(1),
        ),
        (
            PropertyValue::Enumerated(1023),
            ErrorCode::VALUE_OUT_OF_RANGE,
            Some(1),
        ),
        (
            PropertyValue::Enumerated(65_536),
            ErrorCode::VALUE_OUT_OF_RANGE,
            Some(1),
        ),
        (
            PropertyValue::List(vec![
                PropertyValue::Enumerated(3),
                PropertyValue::Enumerated(3),
            ]),
            ErrorCode::VALUE_OUT_OF_RANGE,
            Some(2),
        ),
        (
            PropertyValue::List(vec![PropertyValue::Unsigned(3)]),
            ErrorCode::INVALID_DATA_TYPE,
            Some(1),
        ),
        (
            PropertyValue::Boolean(true),
            ErrorCode::INVALID_DATA_TYPE,
            None,
        ),
    ] {
        let result = write(&mut lift, P::FAULT_SIGNALS, value.clone());
        let context = format!("{value:?}");
        match position {
            Some(position) => crate::common::assert_list_element_refused(
                result,
                ErrorClass::PROPERTY,
                code,
                position,
                &context,
            ),
            None => assert_invalid_data_type(result, &context),
        }
        assert_eq!(read(&lift, P::FAULT_SIGNALS), prior, "{value:?}");
    }
}
