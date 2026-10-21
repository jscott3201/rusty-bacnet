//! Elevator Group, Lift and Escalator properties over WriteProperty and
//! ReadProperty: the Elevator Group's Table 12-76 property set (#997) and
//! indexed Group_Members (#1034), the Lift's Car_Moving_Direction domain
//! (#998), the Lift and Escalator Table 12-77 / 12-78 rows and datatypes
//! (#1021, #1022), the Lift's out-of-service door simulation (#1035),
//! Energy_Meter_Ref (#1036), and, in `lift_simulation`, the Lift's call,
//! command and car-state rows (#1052).

use super::*;
use bacnet_objects::elevator::{ElevatorGroupObject, EscalatorObject, LiftObject};
use bacnet_types::constructed::BACnetDeviceObjectReference;
use bacnet_types::enums::{DoorStatus, LiftCarDirection, LiftFault};

mod lift_simulation;

fn db_with(object: Box<dyn BACnetObject>) -> (ObjectDatabase, ObjectIdentifier) {
    let oid = object.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(object).unwrap();
    (db, oid)
}

fn write(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: PropertyValue,
) -> Result<(), Error> {
    let mut property_value = BytesMut::new();
    encode_property_value(&mut property_value, &value).unwrap();
    write_at(db, oid, property, None, &property_value)
}

/// A WriteProperty carrying `property_value` verbatim.
fn write_at(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    array_index: Option<u32>,
    property_value: &[u8],
) -> Result<(), Error> {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: array_index,
        property_value: property_value.to_vec(),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    handle_write_property(db, &request).map(|_| ())
}

/// The raw propertyValue a ReadProperty ACK carries.
fn read_wire(db: &ObjectDatabase, oid: ObjectIdentifier, property: PropertyIdentifier) -> Vec<u8> {
    read_wire_at(db, oid, property, None).unwrap()
}

fn read_wire_at(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    array_index: Option<u32>,
) -> Result<Vec<u8>, Error> {
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: array_index,
    }
    .encode(&mut request);
    let mut response = BytesMut::new();
    handle_read_property(db, &request, &mut response)?;
    Ok(ReadPropertyACK::decode(&response).unwrap().property_value)
}

fn assert_property_error(result: Result<(), Error>, expected: ErrorCode, context: &str) {
    assert!(
        matches!(result, Err(Error::Protocol { class, code })
            if class == ErrorClass::PROPERTY
                && code == expected),
        "{context}: expected PROPERTY/{expected:?}, got {result:?}"
    );
}

#[test]
fn wp_elevator_group_properties_outside_table_12_76_are_unknown() {
    let (mut db, oid) = db_with(Box::new(ElevatorGroupObject::new(1, "EG-1").unwrap()));
    let before = read_wire(&db, oid, PropertyIdentifier::PROPERTY_LIST);
    for (property, value) in [
        (
            PropertyIdentifier::STATUS_FLAGS,
            PropertyValue::BitString {
                unused_bits: 4,
                data: vec![0],
            },
        ),
        (
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyValue::Boolean(true),
        ),
        (
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyValue::Boolean(false),
        ),
        (
            PropertyIdentifier::RELIABILITY,
            PropertyValue::Enumerated(0),
        ),
    ] {
        assert_property_error(
            write(&mut db, oid, property, value),
            ErrorCode::UNKNOWN_PROPERTY,
            &format!("{property:?}"),
        );
    }
    assert_eq!(
        read_wire(&db, oid, PropertyIdentifier::PROPERTY_LIST),
        before
    );
}

#[test]
fn wp_elevator_group_machine_room_id_reads_but_refuses_writes() {
    let (mut db, oid) = db_with(Box::new(ElevatorGroupObject::new(1, "EG-1").unwrap()));
    // Positive Integer Value (type 48), instance 4194303: no machine room number.
    let none = [0xC4, 0x0C, 0x3F, 0xFF, 0xFF];
    assert_eq!(
        read_wire(&db, oid, PropertyIdentifier::MACHINE_ROOM_ID),
        none
    );
    let piv = ObjectIdentifier::new(ObjectType::POSITIVE_INTEGER_VALUE, 9).unwrap();
    assert_property_error(
        write(
            &mut db,
            oid,
            PropertyIdentifier::MACHINE_ROOM_ID,
            PropertyValue::ObjectIdentifier(piv),
        ),
        ErrorCode::WRITE_ACCESS_DENIED,
        "Machine_Room_ID write",
    );
    assert_eq!(
        read_wire(&db, oid, PropertyIdentifier::MACHINE_ROOM_ID),
        none
    );
}

#[test]
fn wp_elevator_group_group_id_holds_to_unsigned8() {
    let (mut db, oid) = db_with(Box::new(ElevatorGroupObject::new(1, "EG-1").unwrap()));
    write(
        &mut db,
        oid,
        PropertyIdentifier::GROUP_ID,
        PropertyValue::Unsigned(255),
    )
    .unwrap();
    assert_eq!(
        read_wire(&db, oid, PropertyIdentifier::GROUP_ID),
        [0x21, 0xFF]
    );
    for raw in [256u64, 70_000, u64::from(u32::MAX)] {
        assert_property_error(
            write(
                &mut db,
                oid,
                PropertyIdentifier::GROUP_ID,
                PropertyValue::Unsigned(raw),
            ),
            ErrorCode::VALUE_OUT_OF_RANGE,
            &format!("Group_ID {raw}"),
        );
        assert_eq!(
            read_wire(&db, oid, PropertyIdentifier::GROUP_ID),
            [0x21, 0xFF]
        );
    }
}

#[test]
fn wp_lift_car_moving_direction_accepts_the_bacnet_lift_car_direction_domain() {
    let (mut db, oid) = db_with(Box::new(LiftObject::new(1, "LIFT-1", 3).unwrap()));
    let cmd = PropertyIdentifier::CAR_MOVING_DIRECTION;
    // A fresh lift reads STOPPED (2).
    assert_eq!(read_wire(&db, oid, cmd), [0x91, 0x02]);
    for (raw, wire) in [
        (LiftCarDirection::DOWN.to_raw(), &[0x91, 0x04][..]),
        (LiftCarDirection::UP_AND_DOWN.to_raw(), &[0x91, 0x05]),
        (1024, &[0x92, 0x04, 0x00]),
        (65_535, &[0x92, 0xFF, 0xFF]),
    ] {
        write(&mut db, oid, cmd, PropertyValue::Enumerated(raw))
            .unwrap_or_else(|e| panic!("Car_Moving_Direction {raw} must be accepted: {e:?}"));
        assert_eq!(read_wire(&db, oid, cmd), wire, "{raw}");
    }
    for raw in [6u32, 1023, 65_536, u32::MAX] {
        assert_property_error(
            write(&mut db, oid, cmd, PropertyValue::Enumerated(raw)),
            ErrorCode::VALUE_OUT_OF_RANGE,
            &format!("Car_Moving_Direction {raw}"),
        );
        assert_eq!(read_wire(&db, oid, cmd), [0x92, 0xFF, 0xFF], "{raw}");
    }
}

#[test]
fn wp_lift_and_escalator_membership_rows_read_but_refuse_writes() {
    let objects: [Box<dyn BACnetObject>; 2] = [
        Box::new(LiftObject::new(1, "LIFT-1", 3).unwrap()),
        Box::new(EscalatorObject::new(1, "ESC-1").unwrap()),
    ];
    let group = ObjectIdentifier::new(ObjectType::ELEVATOR_GROUP, 5).unwrap();
    for object in objects {
        let (mut db, oid) = db_with(object);
        // Elevator Group (type 57) instance 4194303: no group lists it.
        for (property, wire, value) in [
            (
                PropertyIdentifier::ELEVATOR_GROUP,
                &[0xC4, 0x0E, 0x7F, 0xFF, 0xFF][..],
                PropertyValue::ObjectIdentifier(group),
            ),
            (
                PropertyIdentifier::GROUP_ID,
                &[0x21, 0x00],
                PropertyValue::Unsigned(3),
            ),
            (
                PropertyIdentifier::INSTALLATION_ID,
                &[0x21, 0x00],
                PropertyValue::Unsigned(3),
            ),
        ] {
            assert_eq!(read_wire(&db, oid, property), wire, "{oid:?} {property:?}");
            assert_property_error(
                write(&mut db, oid, property, value),
                ErrorCode::WRITE_ACCESS_DENIED,
                &format!("{oid:?} {property:?}"),
            );
            assert_eq!(read_wire(&db, oid, property), wire, "{oid:?} {property:?}");
        }
    }
}

#[test]
fn wp_lift_car_position_holds_to_unsigned8() {
    let (mut db, oid) = db_with(Box::new(LiftObject::new(1, "LIFT-1", 3).unwrap()));
    let car_position = PropertyIdentifier::CAR_POSITION;
    write(&mut db, oid, car_position, PropertyValue::Unsigned(255)).unwrap();
    assert_eq!(read_wire(&db, oid, car_position), [0x21, 0xFF]);
    for raw in [256u64, 70_000, u64::from(u32::MAX)] {
        assert_property_error(
            write(&mut db, oid, car_position, PropertyValue::Unsigned(raw)),
            ErrorCode::VALUE_OUT_OF_RANGE,
            &format!("Car_Position {raw}"),
        );
        assert_eq!(read_wire(&db, oid, car_position), [0x21, 0xFF]);
    }
}

#[test]
fn wp_lift_car_load_is_a_real_in_car_load_units() {
    let (mut db, oid) = db_with(Box::new(LiftObject::new(1, "LIFT-1", 3).unwrap()));
    let car_load = PropertyIdentifier::CAR_LOAD;
    // REAL 0.0, in PERCENT (98) until the application sets other units.
    assert_eq!(read_wire(&db, oid, car_load), [0x44, 0, 0, 0, 0]);
    assert_eq!(
        read_wire(&db, oid, PropertyIdentifier::CAR_LOAD_UNITS),
        [0x91, 98]
    );
    // 250.5f32 is 0x437A8000: no percentage cap.
    write(&mut db, oid, car_load, PropertyValue::Real(250.5)).unwrap();
    let stored = [0x44, 0x43, 0x7A, 0x80, 0x00];
    assert_eq!(read_wire(&db, oid, car_load), stored);
    for (value, expected) in [
        (PropertyValue::Unsigned(50), ErrorCode::INVALID_DATA_TYPE),
        (PropertyValue::Real(f32::NAN), ErrorCode::VALUE_OUT_OF_RANGE),
        (
            PropertyValue::Real(f32::INFINITY),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
    ] {
        assert_property_error(
            write(&mut db, oid, car_load, value.clone()),
            expected,
            &format!("Car_Load {value:?}"),
        );
        assert_eq!(read_wire(&db, oid, car_load), stored);
    }
}

#[test]
fn wp_lift_passenger_alarm_and_fault_signals_take_their_table_datatypes() {
    let (mut db, oid) = db_with(Box::new(LiftObject::new(1, "LIFT-1", 3).unwrap()));
    let alarm = PropertyIdentifier::PASSENGER_ALARM;
    let faults = PropertyIdentifier::FAULT_SIGNALS;
    assert_eq!(read_wire(&db, oid, alarm), [0x10]);
    write(&mut db, oid, alarm, PropertyValue::Boolean(true)).unwrap();
    assert_eq!(read_wire(&db, oid, alarm), [0x11]);

    assert_eq!(read_wire(&db, oid, faults), [0u8; 0]);
    // SAFETY_INTERLOCK_FAULT (5) and proprietary 1024.
    let set = PropertyValue::List(vec![
        PropertyValue::Enumerated(LiftFault::SAFETY_INTERLOCK_FAULT.to_raw()),
        PropertyValue::Enumerated(1024),
    ]);
    write(&mut db, oid, faults, set).unwrap();
    let stored = [0x91, 0x05, 0x92, 0x04, 0x00];
    assert_eq!(read_wire(&db, oid, faults), stored);
    // The Lift names the element it refuses (#1048); a lone value is the
    // first, a repeat the second.
    for (value, element, context) in [
        (PropertyValue::Enumerated(17), 1, "reserved 17"),
        (PropertyValue::Enumerated(65_536), 1, "above 65535"),
        (
            PropertyValue::List(vec![
                PropertyValue::Enumerated(4),
                PropertyValue::Enumerated(4),
            ]),
            2,
            "duplicate",
        ),
    ] {
        assert_eq!(
            list_refusal(write(&mut db, oid, faults, value)),
            (ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE, element),
            "{context}"
        );
        assert_eq!(read_wire(&db, oid, faults), stored, "{context}");
    }
    // An empty propertyValue clears the set.
    write(&mut db, oid, faults, PropertyValue::List(vec![])).unwrap();
    assert_eq!(read_wire(&db, oid, faults), [0u8; 0]);
}

#[test]
fn wp_lift_rows_outside_table_12_77_and_read_only_rows() {
    let (mut db, oid) = db_with(Box::new(LiftObject::new(1, "LIFT-1", 3).unwrap()));
    let before = read_wire(&db, oid, PropertyIdentifier::PROPERTY_LIST);
    // Not Table 12-77 rows (#1021).
    for property in [
        PropertyIdentifier::TRACKING_VALUE,
        PropertyIdentifier::FLOOR_NUMBER,
    ] {
        assert_property_error(
            write(&mut db, oid, property, PropertyValue::Unsigned(2)),
            ErrorCode::UNKNOWN_PROPERTY,
            &format!("{property:?}"),
        );
    }
    // Served rows the application owns while the lift is in service.
    for (property, value) in [
        (
            PropertyIdentifier::CAR_DOOR_STATUS,
            PropertyValue::List(vec![PropertyValue::Enumerated(0)]),
        ),
        (
            PropertyIdentifier::LANDING_DOOR_STATUS,
            PropertyValue::ApplicationData(vec![0x0E, 0x0F]),
        ),
        (
            PropertyIdentifier::CAR_LOAD_UNITS,
            PropertyValue::Enumerated(39),
        ),
        (
            PropertyIdentifier::FLOOR_TEXT,
            PropertyValue::List(vec![PropertyValue::CharacterString("L".into())]),
        ),
    ] {
        let wire = read_wire(&db, oid, property);
        assert_property_error(
            write(&mut db, oid, property, value),
            ErrorCode::WRITE_ACCESS_DENIED,
            &format!("{property:?}"),
        );
        assert_eq!(read_wire(&db, oid, property), wire, "{property:?}");
    }
    assert_eq!(
        read_wire(&db, oid, PropertyIdentifier::PROPERTY_LIST),
        before
    );
}

#[test]
fn rp_elevator_group_group_members_takes_an_array_index() {
    let mut group = ElevatorGroupObject::new(1, "EG-1").unwrap();
    group.add_member(ObjectIdentifier::new(ObjectType::LIFT, 1).unwrap());
    group.add_member(ObjectIdentifier::new(ObjectType::ESCALATOR, 4).unwrap());
    let (mut db, oid) = db_with(Box::new(group));
    let members = PropertyIdentifier::GROUP_MEMBERS;
    // LIFT is object type 59 (0x0EC00000) and ESCALATOR type 58 (0x0E800000).
    let lift = [0xC4, 0x0E, 0xC0, 0x00, 0x01];
    let escalator = [0xC4, 0x0E, 0x80, 0x00, 0x04];
    assert_eq!(read_wire(&db, oid, members), [lift, escalator].concat());
    for (index, wire) in [(0, &[0x21, 0x02][..]), (1, &lift), (2, &escalator)] {
        assert_eq!(
            read_wire_at(&db, oid, members, Some(index)).unwrap(),
            wire,
            "[{index}]"
        );
    }
    for index in [3, u32::MAX] {
        assert_property_error(
            read_wire_at(&db, oid, members, Some(index)).map(|_| ()),
            ErrorCode::INVALID_ARRAY_INDEX,
            &format!("Group_Members [{index}]"),
        );
    }
    // Still read-only, whole or one element.
    for index in [None, Some(1)] {
        assert_property_error(
            write_at(&mut db, oid, members, index, &lift),
            ErrorCode::WRITE_ACCESS_DENIED,
            &format!("Group_Members write {index:?}"),
        );
    }
    assert_eq!(read_wire(&db, oid, members), [lift, escalator].concat());
}

#[test]
fn wp_lift_door_arrays_take_simulation_writes_only_out_of_service() {
    let mut lift = LiftObject::new(1, "LIFT-1", 3).unwrap();
    lift.set_car_door_status(vec![DoorStatus::CLOSED, DoorStatus::CLOSED])
        .unwrap();
    let (mut db, oid) = db_with(Box::new(lift));
    let car = PropertyIdentifier::CAR_DOOR_STATUS;
    let landing = PropertyIdentifier::LANDING_DOOR_STATUS;
    // OPENED (1) and SAFETY_LOCKED (8); car door 1 pairs with floor 1
    // CLOSED, car door 2 with no landing door.
    let doors = [0x91, 0x01, 0x91, 0x08];
    let frames = [0x0E, 0x09, 0x01, 0x19, 0x00, 0x0F, 0x0E, 0x0F];
    for (property, index, value) in [
        (car, None, &doors[..]),
        (car, Some(1), &doors[..2]),
        (landing, None, &frames[..]),
        (landing, Some(1), &frames[..6]),
    ] {
        assert_property_error(
            write_at(&mut db, oid, property, index, value),
            ErrorCode::WRITE_ACCESS_DENIED,
            &format!("in service {property:?} {index:?}"),
        );
    }
    assert_eq!(read_wire(&db, oid, car), [0x91, 0x00, 0x91, 0x00]);
    assert_eq!(read_wire(&db, oid, landing), [0x0E, 0x0F, 0x0E, 0x0F]);

    write(
        &mut db,
        oid,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    )
    .unwrap();
    write_at(&mut db, oid, car, None, &doors).unwrap();
    assert_eq!(read_wire(&db, oid, car), doors);
    // CLOSING (6) on car door 1.
    write_at(&mut db, oid, car, Some(1), &[0x91, 0x06]).unwrap();
    assert_eq!(read_wire(&db, oid, car), [0x91, 0x06, 0x91, 0x08]);
    write_at(&mut db, oid, landing, None, &frames).unwrap();
    assert_eq!(read_wire(&db, oid, landing), frames);
    // Car door 2 now pairs with floor 3, proprietary status 1024.
    let second = [0x0E, 0x09, 0x03, 0x1A, 0x04, 0x00, 0x0F];
    write_at(&mut db, oid, landing, Some(2), &second).unwrap();
    assert_eq!(read_wire_at(&db, oid, landing, Some(2)).unwrap(), second);

    // Refusals leave both arrays, and their shared size, as they were.
    let car_before = read_wire(&db, oid, car);
    let landing_before = read_wire(&db, oid, landing);
    for (property, index, value, expected, context) in [
        (
            car,
            None,
            &[0x91, 0x01][..],
            ErrorCode::VALUE_OUT_OF_RANGE,
            "one car door for two",
        ),
        (
            car,
            Some(0),
            &[0x21, 0x03],
            ErrorCode::WRITE_ACCESS_DENIED,
            "size",
        ),
        (
            car,
            Some(3),
            &[0x91, 0x01],
            ErrorCode::INVALID_ARRAY_INDEX,
            "door 3",
        ),
        (
            car,
            Some(1),
            &[0x91, 0x0A],
            ErrorCode::VALUE_OUT_OF_RANGE,
            "reserved status 10",
        ),
        (
            car,
            Some(1),
            &[0x21, 0x01],
            ErrorCode::INVALID_DATA_TYPE,
            "Unsigned",
        ),
        (
            landing,
            None,
            &[0x0E, 0x0F],
            ErrorCode::VALUE_OUT_OF_RANGE,
            "one element for two car doors",
        ),
        (
            landing,
            Some(1),
            &[0x0E, 0x0A, 0x01, 0x00, 0x19, 0x00, 0x0F],
            ErrorCode::VALUE_OUT_OF_RANGE,
            "floor 256",
        ),
        (
            landing,
            Some(1),
            &[0x0E, 0x09, 0x01, 0x19, 0x0A, 0x0F],
            ErrorCode::VALUE_OUT_OF_RANGE,
            "reserved landing door status 10",
        ),
        (
            landing,
            Some(1),
            &[0x0E, 0x09, 0x01, 0x0F],
            ErrorCode::INVALID_DATA_ENCODING,
            "no door-status",
        ),
        (
            landing,
            Some(1),
            &[0x0E, 0x19, 0x00, 0x09, 0x01, 0x0F],
            ErrorCode::INVALID_DATA_ENCODING,
            "members out of order",
        ),
    ] {
        assert_property_error(
            write_at(&mut db, oid, property, index, value),
            expected,
            &format!("{property:?} {index:?} {context}"),
        );
        assert_eq!(read_wire(&db, oid, car), car_before, "{context}");
        assert_eq!(read_wire(&db, oid, landing), landing_before, "{context}");
    }
}

#[test]
fn wp_lift_and_escalator_energy_meter_ref_is_read_only_and_holds_energy_meter_at_zero() {
    let meter = BACnetDeviceObjectReference {
        device_identifier: Some(ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap()),
        object_identifier: ObjectIdentifier::new(ObjectType::ACCUMULATOR, 3).unwrap(),
    };
    let mut lift = LiftObject::new(1, "LIFT-1", 3).unwrap();
    lift.set_energy_meter_ref(meter.clone()).unwrap();
    let mut escalator = EscalatorObject::new(1, "ESC-1").unwrap();
    escalator.set_energy_meter_ref(meter).unwrap();
    let objects: [Box<dyn BACnetObject>; 2] = [Box::new(lift), Box::new(escalator)];
    for object in objects {
        let (mut db, oid) = db_with(object);
        // device [0] Device 9 (type 8), object [1] Accumulator (type 23) 3.
        let wire = [0x0C, 0x02, 0x00, 0x00, 0x09, 0x1C, 0x05, 0xC0, 0x00, 0x03];
        let reference = PropertyIdentifier::ENERGY_METER_REF;
        assert_eq!(read_wire(&db, oid, reference), wire, "{oid:?}");
        for value in [&wire[..], &[0x1C, 0x05, 0xFF, 0xFF, 0xFF]] {
            assert_property_error(
                write_at(&mut db, oid, reference, None, value),
                ErrorCode::WRITE_ACCESS_DENIED,
                &format!("{oid:?} Energy_Meter_Ref {value:02X?}"),
            );
        }
        assert_eq!(read_wire(&db, oid, reference), wire, "{oid:?}");

        let energy = PropertyIdentifier::ENERGY_METER;
        assert_eq!(read_wire(&db, oid, energy), [0x44, 0, 0, 0, 0], "{oid:?}");
        assert_property_error(
            write(&mut db, oid, energy, PropertyValue::Real(5.0)),
            ErrorCode::VALUE_OUT_OF_RANGE,
            &format!("{oid:?} Energy_Meter 5.0"),
        );
        write(&mut db, oid, energy, PropertyValue::Real(0.0)).unwrap();
        assert_eq!(read_wire(&db, oid, energy), [0x44, 0, 0, 0, 0], "{oid:?}");
    }
}
