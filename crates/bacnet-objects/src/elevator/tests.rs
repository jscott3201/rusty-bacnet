use super::*;

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
    assert!(list.contains(&PropertyIdentifier::STATUS_FLAGS));
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
