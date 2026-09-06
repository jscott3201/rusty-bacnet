use super::*;
use bacnet_types::enums::{ErrorClass, ErrorCode, EscalatorMode, EscalatorOperationDirection};

mod escalator_status_writability;

/// Escalator write-domain tests run with Out_Of_Service enabled so they do not
/// set policy for writes while the object is in service.
fn oos_escalator() -> EscalatorObject {
    let mut esc = EscalatorObject::new(1, "ESC-1").unwrap();
    esc.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    esc
}

fn assert_value_out_of_range(result: Result<(), Error>, context: &str) {
    match result.expect_err(&format!("{context}: write must be refused")) {
        Error::Protocol { class, code } => {
            assert_eq!(
                class,
                ErrorClass::PROPERTY.to_raw() as u32,
                "{context}: wrong error class"
            );
            assert_eq!(
                code,
                ErrorCode::VALUE_OUT_OF_RANGE.to_raw() as u32,
                "{context}: wrong error code"
            );
        }
        other => panic!("{context}: expected PROPERTY/VALUE_OUT_OF_RANGE, got {other:?}"),
    }
}

fn assert_invalid_data_type(result: Result<(), Error>, context: &str) {
    match result.expect_err(&format!("{context}: write must be refused")) {
        Error::Protocol { class, code } => {
            assert_eq!(
                class,
                ErrorClass::PROPERTY.to_raw() as u32,
                "{context}: wrong error class"
            );
            assert_eq!(
                code,
                ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
                "{context}: wrong error code"
            );
        }
        other => panic!("{context}: expected PROPERTY/INVALID_DATA_TYPE, got {other:?}"),
    }
}

fn read_mode(esc: &EscalatorObject) -> u32 {
    match esc
        .read_property(PropertyIdentifier::ESCALATOR_MODE, None)
        .unwrap()
    {
        PropertyValue::Enumerated(raw) => raw,
        other => panic!("expected Enumerated readback, got {other:?}"),
    }
}

fn read_direction(esc: &EscalatorObject) -> u32 {
    match esc
        .read_property(PropertyIdentifier::OPERATION_DIRECTION, None)
        .unwrap()
    {
        PropertyValue::Enumerated(raw) => raw,
        other => panic!("expected Enumerated readback, got {other:?}"),
    }
}

// --- ElevatorGroupObject ---

#[test]
fn elevator_group_create_and_read_defaults() {
    let eg = ElevatorGroupObject::new(1, "EG-1").unwrap();
    assert_eq!(eg.object_name(), "EG-1");
    assert_eq!(
        eg.read_property(PropertyIdentifier::GROUP_ID, None)
            .unwrap(),
        PropertyValue::Unsigned(0)
    );
    assert_eq!(
        eg.read_property(PropertyIdentifier::GROUP_MEMBERS, None)
            .unwrap(),
        PropertyValue::List(vec![])
    );
    assert_eq!(
        eg.read_property(PropertyIdentifier::GROUP_MODE, None)
            .unwrap(),
        PropertyValue::Enumerated(0) // Unknown
    );
}

#[test]
fn elevator_group_object_type() {
    let eg = ElevatorGroupObject::new(1, "EG-1").unwrap();
    assert_eq!(
        eg.read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap(),
        PropertyValue::Enumerated(ObjectType::ELEVATOR_GROUP.to_raw())
    );
}

#[test]
fn elevator_group_add_members() {
    let mut eg = ElevatorGroupObject::new(1, "EG-1").unwrap();
    let lift1 = ObjectIdentifier::new(ObjectType::LIFT, 1).unwrap();
    let lift2 = ObjectIdentifier::new(ObjectType::LIFT, 2).unwrap();
    eg.add_member(lift1);
    eg.add_member(lift2);
    assert_eq!(
        eg.read_property(PropertyIdentifier::GROUP_MEMBERS, None)
            .unwrap(),
        PropertyValue::List(vec![
            PropertyValue::ObjectIdentifier(lift1),
            PropertyValue::ObjectIdentifier(lift2),
        ])
    );
}

#[test]
fn elevator_group_read_landing_calls() {
    let eg = ElevatorGroupObject::new(1, "EG-1").unwrap();
    assert_eq!(
        eg.read_property(PropertyIdentifier::LANDING_CALLS, None)
            .unwrap(),
        PropertyValue::Unsigned(0)
    );
}

#[test]
fn elevator_group_property_list() {
    let eg = ElevatorGroupObject::new(1, "EG-1").unwrap();
    let list = eg.property_list();
    assert!(list.contains(&PropertyIdentifier::GROUP_ID));
    assert!(list.contains(&PropertyIdentifier::GROUP_MEMBERS));
    assert!(list.contains(&PropertyIdentifier::GROUP_MODE));
    assert!(list.contains(&PropertyIdentifier::LANDING_CALLS));
    assert!(list.contains(&PropertyIdentifier::LANDING_CALL_CONTROL));
    assert!(list.contains(&PropertyIdentifier::STATUS_FLAGS));
}

// --- EscalatorObject ---

#[test]
fn escalator_create_and_read_defaults() {
    let esc = EscalatorObject::new(1, "ESC-1").unwrap();
    assert_eq!(esc.object_name(), "ESC-1");
    assert_eq!(
        esc.read_property(PropertyIdentifier::ESCALATOR_MODE, None)
            .unwrap(),
        PropertyValue::Enumerated(0) // unknown
    );
    assert_eq!(
        esc.read_property(PropertyIdentifier::ENERGY_METER, None)
            .unwrap(),
        PropertyValue::Real(0.0)
    );
    assert_eq!(
        esc.read_property(PropertyIdentifier::POWER_MODE, None)
            .unwrap(),
        PropertyValue::Boolean(false)
    );
    assert_eq!(
        esc.read_property(PropertyIdentifier::PASSENGER_ALARM, None)
            .unwrap(),
        PropertyValue::Boolean(false)
    );
}

#[test]
fn escalator_object_type() {
    let esc = EscalatorObject::new(1, "ESC-1").unwrap();
    assert_eq!(
        esc.read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap(),
        PropertyValue::Enumerated(ObjectType::ESCALATOR.to_raw())
    );
}

#[test]
fn escalator_read_operation_direction() {
    let esc = EscalatorObject::new(1, "ESC-1").unwrap();
    assert_eq!(
        esc.read_property(PropertyIdentifier::OPERATION_DIRECTION, None)
            .unwrap(),
        PropertyValue::Enumerated(0) // unknown
    );
}

#[test]
fn escalator_read_fault_signals() {
    let esc = EscalatorObject::new(1, "ESC-1").unwrap();
    assert_eq!(
        esc.read_property(PropertyIdentifier::FAULT_SIGNALS, None)
            .unwrap(),
        PropertyValue::List(vec![])
    );
}

#[test]
fn escalator_property_list() {
    let esc = EscalatorObject::new(1, "ESC-1").unwrap();
    let list = esc.property_list();
    assert!(list.contains(&PropertyIdentifier::ESCALATOR_MODE));
    assert!(list.contains(&PropertyIdentifier::FAULT_SIGNALS));
    assert!(list.contains(&PropertyIdentifier::ENERGY_METER));
    assert!(list.contains(&PropertyIdentifier::ENERGY_METER_REF));
    assert!(list.contains(&PropertyIdentifier::POWER_MODE));
    assert!(list.contains(&PropertyIdentifier::OPERATION_DIRECTION));
    assert!(list.contains(&PropertyIdentifier::PASSENGER_ALARM));
    assert!(list.contains(&PropertyIdentifier::STATUS_FLAGS));
}

// --- Escalator_Mode domain (#400) ---

#[test]
fn escalator_mode_field_is_typed_and_defaults_to_unknown() {
    let esc = EscalatorObject::new(1, "ESC-1").unwrap();
    assert_eq!(esc.escalator_mode, EscalatorMode::UNKNOWN);
}

#[test]
fn escalator_all_named_modes_round_trip_with_oos() {
    let mut esc = oos_escalator();

    for &(name, value) in EscalatorMode::ALL_NAMED {
        esc.write_property(
            PropertyIdentifier::ESCALATOR_MODE,
            None,
            PropertyValue::Enumerated(value.to_raw()),
            None,
        )
        .unwrap_or_else(|e| panic!("named mode {name} must be accepted: {e:?}"));
        assert_eq!(read_mode(&esc), value.to_raw(), "{name} round-trip");
    }

    assert_eq!(read_mode(&esc), EscalatorMode::OUT_OF_SERVICE.to_raw());
}

#[test]
fn escalator_proprietary_mode_values_round_trip() {
    let mut esc = oos_escalator();

    for raw in [1024u32, 65535] {
        esc.write_property(
            PropertyIdentifier::ESCALATOR_MODE,
            None,
            PropertyValue::Enumerated(raw),
            None,
        )
        .unwrap_or_else(|e| panic!("proprietary raw {raw} must be accepted: {e:?}"));
        assert_eq!(read_mode(&esc), raw, "raw {raw} must not normalize");
    }
}

#[test]
fn escalator_reserved_and_oversized_mode_values_rejected_atomically() {
    let prior = EscalatorMode::OUT_OF_SERVICE.to_raw();
    for raw in [6u32, 1023, 65536, u32::MAX] {
        let mut esc = oos_escalator();
        esc.write_property(
            PropertyIdentifier::ESCALATOR_MODE,
            None,
            PropertyValue::Enumerated(prior),
            None,
        )
        .unwrap();

        assert_value_out_of_range(
            esc.write_property(
                PropertyIdentifier::ESCALATOR_MODE,
                None,
                PropertyValue::Enumerated(raw),
                None,
            ),
            &format!("invalid raw {raw}"),
        );
        assert_eq!(read_mode(&esc), prior, "raw {raw} changed the mode");
    }
}

#[test]
fn escalator_mode_wrong_datatype_rejected_atomically() {
    let mut esc = oos_escalator();
    let prior = EscalatorMode::STOP.to_raw();
    esc.write_property(
        PropertyIdentifier::ESCALATOR_MODE,
        None,
        PropertyValue::Enumerated(prior),
        None,
    )
    .unwrap();

    assert_invalid_data_type(
        esc.write_property(
            PropertyIdentifier::ESCALATOR_MODE,
            None,
            PropertyValue::Unsigned(4),
            None,
        ),
        "Unsigned instead of Enumerated",
    );
    assert_eq!(read_mode(&esc), prior, "wrong datatype changed the mode");
}

// --- Operation_Direction typed behavior (#284) ---

#[test]
fn escalator_operation_direction_field_is_typed_and_defaults_to_unknown() {
    let esc = EscalatorObject::new(1, "ESC-1").unwrap();
    assert_eq!(
        esc.operation_direction,
        EscalatorOperationDirection::UNKNOWN
    );
}

#[test]
fn escalator_operation_direction_default_readback_is_enumerated_zero() {
    let esc = EscalatorObject::new(1, "ESC-1").unwrap();
    assert_eq!(
        esc.read_property(PropertyIdentifier::OPERATION_DIRECTION, None)
            .unwrap(),
        PropertyValue::Enumerated(0)
    );
}

#[test]
fn escalator_all_named_directions_round_trip_with_oos() {
    let mut esc = oos_escalator();
    let mut last_raw = 0;
    for &(name, value) in EscalatorOperationDirection::ALL_NAMED {
        esc.write_property(
            PropertyIdentifier::OPERATION_DIRECTION,
            None,
            PropertyValue::Enumerated(value.to_raw()),
            None,
        )
        .unwrap_or_else(|e| panic!("named direction {name} must be accepted: {e:?}"));
        assert_eq!(read_direction(&esc), value.to_raw(), "{name} round-trip");
        last_raw = value.to_raw();
    }
    assert_eq!(
        last_raw,
        EscalatorOperationDirection::DOWN_REDUCED_SPEED.to_raw()
    );
}

#[test]
fn escalator_down_directions_are_accepted() {
    for raw in [
        EscalatorOperationDirection::DOWN_RATED_SPEED.to_raw(),
        EscalatorOperationDirection::DOWN_REDUCED_SPEED.to_raw(),
    ] {
        let mut esc = oos_escalator();
        esc.write_property(
            PropertyIdentifier::OPERATION_DIRECTION,
            None,
            PropertyValue::Enumerated(raw),
            None,
        )
        .unwrap_or_else(|e| panic!("raw {raw} must be accepted: {e:?}"));
        assert_eq!(read_direction(&esc), raw);
    }
}

// --- Proprietary boundaries (Clause 23.1 / Table 23-1) ---

#[test]
fn escalator_proprietary_direction_values_round_trip() {
    for raw in [1024u32, 65535] {
        let mut esc = oos_escalator();
        esc.write_property(
            PropertyIdentifier::OPERATION_DIRECTION,
            None,
            PropertyValue::Enumerated(raw),
            None,
        )
        .unwrap_or_else(|e| panic!("proprietary raw {raw} must be accepted: {e:?}"));
        assert_eq!(read_direction(&esc), raw, "raw {raw} must not normalize");
    }
}

// --- Invalid boundaries ---

#[test]
fn escalator_reserved_direction_values_rejected() {
    for raw in [6u32, 1023] {
        let mut esc = oos_escalator();
        assert_value_out_of_range(
            esc.write_property(
                PropertyIdentifier::OPERATION_DIRECTION,
                None,
                PropertyValue::Enumerated(raw),
                None,
            ),
            &format!("reserved raw {raw}"),
        );
    }
}

#[test]
fn escalator_above_maximum_direction_values_rejected() {
    for raw in [65536u32, u32::MAX] {
        let mut esc = oos_escalator();
        assert_value_out_of_range(
            esc.write_property(
                PropertyIdentifier::OPERATION_DIRECTION,
                None,
                PropertyValue::Enumerated(raw),
                None,
            ),
            &format!("above-maximum raw {raw}"),
        );
    }
}

#[test]
fn escalator_operation_direction_wrong_datatype_rejected() {
    let mut esc = oos_escalator();
    assert_invalid_data_type(
        esc.write_property(
            PropertyIdentifier::OPERATION_DIRECTION,
            None,
            PropertyValue::Unsigned(4),
            None,
        ),
        "Unsigned instead of Enumerated",
    );
}

// --- Atomicity ---

#[test]
fn escalator_failed_writes_leave_prior_direction_unchanged() {
    let mut esc = oos_escalator();
    let prior = EscalatorOperationDirection::UP_RATED_SPEED.to_raw();
    esc.write_property(
        PropertyIdentifier::OPERATION_DIRECTION,
        None,
        PropertyValue::Enumerated(prior),
        None,
    )
    .unwrap();

    assert_value_out_of_range(
        esc.write_property(
            PropertyIdentifier::OPERATION_DIRECTION,
            None,
            PropertyValue::Enumerated(6),
            None,
        ),
        "reserved 6",
    );
    assert_eq!(
        read_direction(&esc),
        prior,
        "value changed after reserved rejection"
    );

    assert_value_out_of_range(
        esc.write_property(
            PropertyIdentifier::OPERATION_DIRECTION,
            None,
            PropertyValue::Enumerated(u32::MAX),
            None,
        ),
        "above maximum",
    );
    assert_eq!(
        read_direction(&esc),
        prior,
        "value changed after above-maximum rejection"
    );

    assert_invalid_data_type(
        esc.write_property(
            PropertyIdentifier::OPERATION_DIRECTION,
            None,
            PropertyValue::Unsigned(4),
            None,
        ),
        "wrong datatype",
    );
    assert_eq!(
        read_direction(&esc),
        prior,
        "value changed after datatype rejection"
    );
}

// --- LiftObject ---

#[test]
fn lift_create_and_read_defaults() {
    let lift = LiftObject::new(1, "LIFT-1", 10).unwrap();
    assert_eq!(lift.object_name(), "LIFT-1");
    assert_eq!(
        lift.read_property(PropertyIdentifier::TRACKING_VALUE, None)
            .unwrap(),
        PropertyValue::Unsigned(1)
    );
    assert_eq!(
        lift.read_property(PropertyIdentifier::CAR_POSITION, None)
            .unwrap(),
        PropertyValue::Unsigned(1)
    );
    assert_eq!(
        lift.read_property(PropertyIdentifier::CAR_MOVING_DIRECTION, None)
            .unwrap(),
        PropertyValue::Enumerated(1) // stopped
    );
}

#[test]
fn lift_object_type() {
    let lift = LiftObject::new(1, "LIFT-1", 5).unwrap();
    assert_eq!(
        lift.read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap(),
        PropertyValue::Enumerated(ObjectType::LIFT.to_raw())
    );
}

#[test]
fn lift_floor_text() {
    let lift = LiftObject::new(1, "LIFT-1", 3).unwrap();
    assert_eq!(
        lift.read_property(PropertyIdentifier::FLOOR_TEXT, None)
            .unwrap(),
        PropertyValue::List(vec![
            PropertyValue::CharacterString("Floor 1".into()),
            PropertyValue::CharacterString("Floor 2".into()),
            PropertyValue::CharacterString("Floor 3".into()),
        ])
    );
}

#[test]
fn lift_read_car_load() {
    let lift = LiftObject::new(1, "LIFT-1", 5).unwrap();
    assert_eq!(
        lift.read_property(PropertyIdentifier::CAR_LOAD, None)
            .unwrap(),
        PropertyValue::Unsigned(0)
    );
}

#[test]
fn lift_write_tracking_value() {
    let mut lift = LiftObject::new(1, "LIFT-1", 10).unwrap();
    lift.write_property(
        PropertyIdentifier::TRACKING_VALUE,
        None,
        PropertyValue::Unsigned(5),
        None,
    )
    .unwrap();
    assert_eq!(
        lift.read_property(PropertyIdentifier::TRACKING_VALUE, None)
            .unwrap(),
        PropertyValue::Unsigned(5)
    );
}

#[test]
fn lift_write_car_load_out_of_range() {
    let mut lift = LiftObject::new(1, "LIFT-1", 5).unwrap();
    let result = lift.write_property(
        PropertyIdentifier::CAR_LOAD,
        None,
        PropertyValue::Unsigned(101),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn lift_read_landing_doors() {
    let lift = LiftObject::new(1, "LIFT-1", 8).unwrap();
    assert_eq!(
        lift.read_property(PropertyIdentifier::LANDING_DOOR_STATUS, None)
            .unwrap(),
        PropertyValue::Unsigned(8)
    );
}

#[test]
fn lift_read_energy_meter() {
    let lift = LiftObject::new(1, "LIFT-1", 5).unwrap();
    assert_eq!(
        lift.read_property(PropertyIdentifier::ENERGY_METER, None)
            .unwrap(),
        PropertyValue::Real(0.0)
    );
}

#[test]
fn lift_property_list() {
    let lift = LiftObject::new(1, "LIFT-1", 5).unwrap();
    let list = lift.property_list();
    assert!(list.contains(&PropertyIdentifier::TRACKING_VALUE));
    assert!(list.contains(&PropertyIdentifier::CAR_POSITION));
    assert!(list.contains(&PropertyIdentifier::CAR_MOVING_DIRECTION));
    assert!(list.contains(&PropertyIdentifier::CAR_DOOR_STATUS));
    assert!(list.contains(&PropertyIdentifier::CAR_LOAD));
    assert!(list.contains(&PropertyIdentifier::LANDING_DOOR_STATUS));
    assert!(list.contains(&PropertyIdentifier::FLOOR_TEXT));
    assert!(list.contains(&PropertyIdentifier::ENERGY_METER));
    assert!(list.contains(&PropertyIdentifier::STATUS_FLAGS));
}
