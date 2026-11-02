//! Write domains of the Elevator trio's served rows (Clauses 12.58-12.60):
//! routed arms store values of their table datatypes verbatim, while mistyped
//! and out-of-range values, and rows with no network write route, are refused
//! without changing state.

use super::super::*;
use super::metadata::assert_error;
use crate::property_metadata::is_writable_property;
use bacnet_types::enums::PropertyIdentifier as P;
use bacnet_types::enums::{
    ErrorCode, EscalatorFault, EscalatorMode, EscalatorOperationDirection, LiftCarDirection,
    LiftFault,
};

#[test]
fn property_metadata_elevator_group_writes_store_verbatim() {
    let mut object = ElevatorGroupObject::new(1, "EG-1").unwrap();
    // Routed arms store verbatim.
    for (p, value, expected) in [
        (
            P::GROUP_ID,
            PropertyValue::Unsigned(47),
            PropertyValue::Unsigned(47),
        ),
        (
            P::GROUP_MODE,
            PropertyValue::Enumerated(2),
            PropertyValue::Enumerated(2),
        ),
        // BACnetLandingCallStatus: floor [0] 5, direction [1] UP.
        (
            P::LANDING_CALL_CONTROL,
            PropertyValue::ApplicationData(vec![0x09, 0x05, 0x19, 0x03]),
            PropertyValue::ApplicationData(vec![0x09, 0x05, 0x19, 0x03]),
        ),
    ] {
        object.write_property(p, None, value, None).unwrap();
        assert_eq!(object.read_property(p, None).unwrap(), expected);
    }
    // Mistyped values are rejected without changing state.
    for (p, value) in [
        (P::GROUP_ID, PropertyValue::Enumerated(47)),
        (P::GROUP_MODE, PropertyValue::Unsigned(2)),
        (P::LANDING_CALL_CONTROL, PropertyValue::Unsigned(1)),
        (P::DESCRIPTION, PropertyValue::Unsigned(1)),
    ] {
        assert_error(
            object.write_property(p, None, value, None).unwrap_err(),
            ErrorCode::INVALID_DATA_TYPE,
        );
    }
    // Machine_Room_ID, Group_Members and Landing_Calls have no network
    // write route: even their read-back values are denied on write.
    for p in [P::MACHINE_ROOM_ID, P::GROUP_MEMBERS, P::LANDING_CALLS] {
        let value = object.read_property(p, None).unwrap();
        assert_error(
            object.write_property(p, None, value, None).unwrap_err(),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
        assert!(!is_writable_property(&object, p));
    }
}

#[test]
fn property_metadata_escalator_domain_validation_matches_dispatch() {
    for out_of_service in [false, true] {
        let mut object = EscalatorObject::new(1, "ESC-1").unwrap();
        object
            .write_property(
                P::OUT_OF_SERVICE,
                None,
                PropertyValue::Boolean(out_of_service),
                None,
            )
            .unwrap();
        // Escalator_Mode and Operation_Direction share the Clause 23.1
        // domain: named values plus 1024..=65535 round-trip, while the
        // reserved gap and oversized values fail atomically.
        for (p, named, prior) in [
            (
                P::ESCALATOR_MODE,
                EscalatorMode::UP.to_raw(),
                EscalatorMode::STOP.to_raw(),
            ),
            (
                P::OPERATION_DIRECTION,
                EscalatorOperationDirection::DOWN_REDUCED_SPEED.to_raw(),
                EscalatorOperationDirection::UP_RATED_SPEED.to_raw(),
            ),
        ] {
            for raw in [named, 1024, 65535] {
                object
                    .write_property(p, None, PropertyValue::Enumerated(raw), None)
                    .unwrap_or_else(|e| panic!("{p:?} {raw} must be accepted: {e:?}"));
                assert_eq!(
                    object.read_property(p, None).unwrap(),
                    PropertyValue::Enumerated(raw)
                );
            }
            object
                .write_property(p, None, PropertyValue::Enumerated(prior), None)
                .unwrap();
            for raw in [6u32, 1023, 65536, u32::MAX] {
                assert_error(
                    object
                        .write_property(p, None, PropertyValue::Enumerated(raw), None)
                        .unwrap_err(),
                    ErrorCode::VALUE_OUT_OF_RANGE,
                );
            }
            assert_eq!(
                object.read_property(p, None).unwrap(),
                PropertyValue::Enumerated(prior)
            );
            assert_error(
                object
                    .write_property(p, None, PropertyValue::Unsigned(prior as u64), None)
                    .unwrap_err(),
                ErrorCode::INVALID_DATA_TYPE,
            );
        }
        // Energy_Meter stores finite values and refuses the rest.
        object
            .write_property(P::ENERGY_METER, None, PropertyValue::Real(42.0), None)
            .unwrap();
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_error(
                object
                    .write_property(P::ENERGY_METER, None, PropertyValue::Real(value), None)
                    .unwrap_err(),
                ErrorCode::VALUE_OUT_OF_RANGE,
            );
        }
        assert_eq!(
            object.read_property(P::ENERGY_METER, None).unwrap(),
            PropertyValue::Real(42.0)
        );
        // Fault_Signals dedups atomically: duplicates, reserved values,
        // and mistyped shapes fail without touching the stored set.
        let prior = PropertyValue::List(vec![
            PropertyValue::Enumerated(EscalatorFault::CONTROLLER_FAULT.to_raw()),
            PropertyValue::Enumerated(1024),
        ]);
        object
            .write_property(P::FAULT_SIGNALS, None, prior.clone(), None)
            .unwrap();
        for (values, expected) in [
            (
                vec![
                    PropertyValue::Enumerated(1024),
                    PropertyValue::Enumerated(1024),
                ],
                ErrorCode::VALUE_OUT_OF_RANGE,
            ),
            (
                vec![PropertyValue::Enumerated(9)],
                ErrorCode::VALUE_OUT_OF_RANGE,
            ),
            (
                vec![PropertyValue::Enumerated(1023)],
                ErrorCode::VALUE_OUT_OF_RANGE,
            ),
            (
                vec![PropertyValue::Unsigned(8)],
                ErrorCode::INVALID_DATA_TYPE,
            ),
        ] {
            assert_error(
                object
                    .write_property(P::FAULT_SIGNALS, None, PropertyValue::List(values), None)
                    .unwrap_err(),
                expected,
            );
            assert_eq!(object.read_property(P::FAULT_SIGNALS, None).unwrap(), prior);
        }
        // Energy_Meter_Ref has no network write route; Power_Mode and
        // Passenger_Alarm store Booleans verbatim.
        let energy_ref = object.read_property(P::ENERGY_METER_REF, None).unwrap();
        assert_error(
            object
                .write_property(P::ENERGY_METER_REF, None, energy_ref, None)
                .unwrap_err(),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
        for p in [P::POWER_MODE, P::PASSENGER_ALARM] {
            object
                .write_property(p, None, PropertyValue::Boolean(true), None)
                .unwrap();
            assert_eq!(
                object.read_property(p, None).unwrap(),
                PropertyValue::Boolean(true)
            );
            assert_error(
                object
                    .write_property(p, None, PropertyValue::Enumerated(1), None)
                    .unwrap_err(),
                ErrorCode::INVALID_DATA_TYPE,
            );
        }
    }
}

#[test]
fn property_metadata_lift_writes_store_verbatim_with_range_gates() {
    for out_of_service in [false, true] {
        let mut object = LiftObject::new(1, "LIFT-1", 3).unwrap();
        object
            .write_property(
                P::OUT_OF_SERVICE,
                None,
                PropertyValue::Boolean(out_of_service),
                None,
            )
            .unwrap();
        // Each routed arm stores a value of its table datatype verbatim.
        for (p, value) in [
            (P::CAR_POSITION, PropertyValue::Unsigned(255)),
            (
                P::CAR_MOVING_DIRECTION,
                PropertyValue::Enumerated(LiftCarDirection::DOWN.to_raw()),
            ),
            (P::CAR_LOAD, PropertyValue::Real(312.5)),
            (P::PASSENGER_ALARM, PropertyValue::Boolean(true)),
            (P::ENERGY_METER, PropertyValue::Real(-1.5)),
            (
                P::FAULT_SIGNALS,
                PropertyValue::List(vec![
                    PropertyValue::Enumerated(LiftFault::POSITION_LOST.to_raw()),
                    PropertyValue::Enumerated(1024),
                ]),
            ),
        ] {
            object
                .write_property(p, None, value.clone(), None)
                .unwrap_or_else(|e| panic!("{p:?} must accept {value:?}: {e:?}"));
            assert_eq!(object.read_property(p, None).unwrap(), value, "{p:?}");
        }
        // Out-of-range values fail atomically (the per-property suites in
        // tests/lift_properties.rs cover each domain).
        for (p, value) in [
            (P::CAR_POSITION, PropertyValue::Unsigned(256)),
            (P::CAR_MOVING_DIRECTION, PropertyValue::Enumerated(6)),
            (P::CAR_LOAD, PropertyValue::Real(f32::NAN)),
            (P::ENERGY_METER, PropertyValue::Real(f32::INFINITY)),
            (P::FAULT_SIGNALS, PropertyValue::Enumerated(17)),
        ] {
            let before = object.read_property(p, None).unwrap();
            assert_error(
                object.write_property(p, None, value, None).unwrap_err(),
                ErrorCode::VALUE_OUT_OF_RANGE,
            );
            assert_eq!(object.read_property(p, None).unwrap(), before, "{p:?}");
        }
        // Mistyped values are rejected without changing state.
        for (p, value) in [
            (P::CAR_POSITION, PropertyValue::Enumerated(2)),
            (P::CAR_MOVING_DIRECTION, PropertyValue::Unsigned(2)),
            (P::CAR_LOAD, PropertyValue::Unsigned(50)),
            (P::PASSENGER_ALARM, PropertyValue::Enumerated(1)),
            (P::ENERGY_METER, PropertyValue::Unsigned(1)),
            (P::FAULT_SIGNALS, PropertyValue::Unsigned(1)),
            (P::DESCRIPTION, PropertyValue::Unsigned(1)),
            (
                P::OUT_OF_SERVICE,
                PropertyValue::CharacterString("invalid".into()),
            ),
        ] {
            let before = object.read_property(p, None).unwrap();
            assert_error(
                object.write_property(p, None, value, None).unwrap_err(),
                ErrorCode::INVALID_DATA_TYPE,
            );
            assert_eq!(object.read_property(p, None).unwrap(), before, "{p:?}");
        }
        // The membership rows, Floor_Text, Car_Load_Units and
        // Energy_Meter_Ref have no network write route: even their
        // read-back values are denied.
        for p in [
            P::ELEVATOR_GROUP,
            P::GROUP_ID,
            P::INSTALLATION_ID,
            P::FLOOR_TEXT,
            P::CAR_LOAD_UNITS,
            P::ENERGY_METER_REF,
        ] {
            let value = object.read_property(p, None).unwrap();
            assert_error(
                object.write_property(p, None, value, None).unwrap_err(),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
            assert!(!is_writable_property(&object, p));
        }
        // The per-door arrays and car-state rows take their read-back
        // values only out of service.
        for p in [
            P::ASSIGNED_LANDING_CALLS,
            P::MAKING_CAR_CALL,
            P::REGISTERED_CAR_CALL,
            P::CAR_ASSIGNED_DIRECTION,
            P::CAR_DOOR_STATUS,
            P::CAR_DOOR_COMMAND,
            P::CAR_DOOR_ZONE,
            P::CAR_MODE,
            P::NEXT_STOPPING_FLOOR,
            P::CAR_DRIVE_STATUS,
            P::LANDING_DOOR_STATUS,
        ] {
            let value = object.read_property(p, None).unwrap();
            let result = object.write_property(p, None, value, None);
            if out_of_service {
                result.unwrap();
            } else {
                assert_error(result.unwrap_err(), ErrorCode::WRITE_ACCESS_DENIED);
            }
            assert!(is_writable_property(&object, p));
        }
    }
}
