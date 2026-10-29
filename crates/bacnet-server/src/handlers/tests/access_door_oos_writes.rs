//! Door_Status, Lock_Status and Door_Alarm_State writes on an Access Door over
//! WriteProperty and WritePropertyMultiple: taken while Out_Of_Service is
//! TRUE, refused in service (Clause 12.26.9, Table 12-30 footnote 1, #1131).
//! Also the Secured_Status a ReadProperty derives from the served values and
//! Present_Value (Clause 12.26.14, #1148), and the door's three alarm lists
//! over WriteProperty, WritePropertyMultiple and the list services, with the
//! Door_Alarm_State they admit (Clauses 12.26.20 and 12.26.21, #1149).

use super::*;
use bacnet_objects::access_control::AccessDoorObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_types::enums::{DoorAlarmState, DoorStatus, DoorValue, LockStatus};
use bacnet_types::error::RemoteError;

/// An in-service door whose device reports it open, unlocked and held open
/// too long. Its Alarm_Values admit every alarm state these tests simulate.
fn door_db() -> (ObjectDatabase, ObjectIdentifier) {
    let mut door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    door.set_alarm_values([
        DoorAlarmState::DOOR_OPEN_TOO_LONG,
        DoorAlarmState::FORCED_OPEN,
        DoorAlarmState::TAMPER,
        DoorAlarmState::from_raw(256),
    ])
    .unwrap();
    door.set_door_status(DoorStatus::OPENED);
    door.set_lock_status(LockStatus::UNLOCKED);
    door.set_door_alarm_state(DoorAlarmState::DOOR_OPEN_TOO_LONG)
        .unwrap();
    let oid = door.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(door)).unwrap();
    (db, oid)
}

fn encode(value: &PropertyValue) -> Vec<u8> {
    let mut buf = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut buf, value).unwrap();
    buf.to_vec()
}

fn enumerated(raw: u32) -> PropertyValue {
    PropertyValue::Enumerated(raw)
}

fn write_property(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: PropertyValue,
) -> Result<(), Error> {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
        property_value: encode(&value),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    handle_write_property(db, &request).map(|_| ())
}

fn write_property_multiple(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    writes: &[(PropertyIdentifier, PropertyValue)],
) -> Result<(), Error> {
    let mut request = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: writes
                .iter()
                .map(|(property, value)| BACnetPropertyValue {
                    property_identifier: *property,
                    property_array_index: None,
                    value: encode(value),
                    priority: None,
                })
                .collect(),
        }],
    }
    .encode(&mut request)
    .unwrap();
    handle_write_property_multiple(db, &request).map(|_| ())
}

/// The ReadProperty-ACK value bytes of one property.
fn read_bytes(db: &ObjectDatabase, oid: ObjectIdentifier, property: PropertyIdentifier) -> Vec<u8> {
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
    }
    .encode(&mut request);
    let mut response = BytesMut::new();
    handle_read_property(db, &request, &mut response).unwrap();
    ReadPropertyACK::decode(&response).unwrap().property_value
}

/// Door_Status, Lock_Status and Door_Alarm_State as served, in wire bytes.
fn served(db: &ObjectDatabase, oid: ObjectIdentifier) -> [Vec<u8>; 3] {
    ROWS.map(|property| read_bytes(db, oid, property))
}

fn assert_property_error(result: Result<(), Error>, expected: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Remote(RemoteError::Error { class, code }))
            if class == ErrorClass::PROPERTY
                && code == expected),
        "expected PROPERTY / {expected:?}, got {result:?}"
    );
}

const ROWS: [PropertyIdentifier; 3] = [
    PropertyIdentifier::DOOR_STATUS,
    PropertyIdentifier::LOCK_STATUS,
    PropertyIdentifier::DOOR_ALARM_STATE,
];
/// The device's values: OPENED, UNLOCKED, DOOR_OPEN_TOO_LONG.
const DEVICE: [[u8; 2]; 3] = [[0x91, 1], [0x91, 1], [0x91, 2]];
/// The simulated values: DOOR_FAULT, LOCK_FAULT, FORCED_OPEN.
const SIMULATED: [[u8; 2]; 3] = [[0x91, 3], [0x91, 2], [0x91, 3]];
/// Status_Flags with only OUT_OF_SERVICE set.
const OUT_OF_SERVICE_FLAGS: [u8; 3] = [0x82, 0x04, 0x10];

fn simulation() -> [(PropertyIdentifier, PropertyValue); 3] {
    [
        (ROWS[0], enumerated(DoorStatus::DOOR_FAULT.to_raw())),
        (ROWS[1], enumerated(LockStatus::LOCK_FAULT.to_raw())),
        (ROWS[2], enumerated(DoorAlarmState::FORCED_OPEN.to_raw())),
    ]
}

#[test]
fn write_property_takes_door_status_rows_only_out_of_service() {
    let (mut db, oid) = door_db();
    for (property, value) in simulation() {
        assert_property_error(
            write_property(&mut db, oid, property, value),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
    }
    assert_eq!(served(&db, oid), DEVICE.map(Vec::from));

    write_property(
        &mut db,
        oid,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    )
    .unwrap();
    for (index, (property, value)) in simulation().into_iter().enumerate() {
        write_property(&mut db, oid, property, value).unwrap();
        assert_eq!(read_bytes(&db, oid, property), SIMULATED[index]);
    }
    assert_eq!(
        read_bytes(&db, oid, PropertyIdentifier::STATUS_FLAGS),
        OUT_OF_SERVICE_FLAGS
    );
    // Present_Value is untouched by the simulation.
    assert_eq!(
        read_bytes(&db, oid, PropertyIdentifier::PRESENT_VALUE),
        [0x91, 0]
    );

    // The return to service serves the device's values again.
    write_property(
        &mut db,
        oid,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(false),
    )
    .unwrap();
    assert_eq!(served(&db, oid), DEVICE.map(Vec::from));
    for (property, value) in simulation() {
        assert_property_error(
            write_property(&mut db, oid, property, value),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
    }
}

#[test]
fn write_property_multiple_takes_door_status_rows_only_out_of_service() {
    let (mut db, oid) = door_db();
    // In service the first write is refused and nothing changes; so is each
    // row on its own.
    assert_property_error(
        write_property_multiple(&mut db, oid, &simulation()),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    for write in simulation() {
        assert_property_error(
            write_property_multiple(&mut db, oid, &[write]),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
    }
    assert_eq!(served(&db, oid), DEVICE.map(Vec::from));

    // Out_Of_Service first, then the three simulated values, in one request.
    let mut writes = vec![(
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    )];
    writes.extend(simulation());
    write_property_multiple(&mut db, oid, &writes).unwrap();
    assert_eq!(served(&db, oid), SIMULATED.map(Vec::from));
    assert_eq!(
        read_bytes(&db, oid, PropertyIdentifier::STATUS_FLAGS),
        OUT_OF_SERVICE_FLAGS
    );

    // A simulated value and the return to service in one request: the
    // device's values come back and the simulation is dropped.
    write_property_multiple(
        &mut db,
        oid,
        &[
            (
                PropertyIdentifier::DOOR_ALARM_STATE,
                enumerated(DoorAlarmState::TAMPER.to_raw()),
            ),
            (
                PropertyIdentifier::OUT_OF_SERVICE,
                PropertyValue::Boolean(false),
            ),
        ],
    )
    .unwrap();
    assert_eq!(served(&db, oid), DEVICE.map(Vec::from));
    assert_property_error(
        write_property_multiple(&mut db, oid, &simulation()),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
}

#[test]
fn door_status_row_writes_outside_their_datatypes_are_refused_unchanged() {
    let (mut db, oid) = door_db();
    write_property(
        &mut db,
        oid,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    )
    .unwrap();
    for (property, value, code) in [
        // 10 is reserved for ASHRAE, 65536 past the datatype.
        (ROWS[0], enumerated(10), ErrorCode::VALUE_OUT_OF_RANGE),
        (ROWS[0], enumerated(65_536), ErrorCode::VALUE_OUT_OF_RANGE),
        (
            ROWS[0],
            PropertyValue::Unsigned(1),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        // BACnetLockStatus has no proprietary range.
        (ROWS[1], enumerated(5), ErrorCode::VALUE_OUT_OF_RANGE),
        (ROWS[1], enumerated(1_024), ErrorCode::VALUE_OUT_OF_RANGE),
        (
            ROWS[1],
            PropertyValue::Unsigned(1),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        // 9 is reserved for ASHRAE.
        (ROWS[2], enumerated(9), ErrorCode::VALUE_OUT_OF_RANGE),
        (ROWS[2], enumerated(65_536), ErrorCode::VALUE_OUT_OF_RANGE),
        (
            ROWS[2],
            PropertyValue::Boolean(true),
            ErrorCode::INVALID_DATA_TYPE,
        ),
    ] {
        assert_property_error(write_property(&mut db, oid, property, value.clone()), code);
        assert_eq!(served(&db, oid), DEVICE.map(Vec::from), "{property:?}");
        // WPM stops at the refused value after committing the valid one
        // before it.
        assert_property_error(
            write_property_multiple(
                &mut db,
                oid,
                &[
                    (
                        PropertyIdentifier::DOOR_STATUS,
                        enumerated(DoorStatus::DOOR_FAULT.to_raw()),
                    ),
                    (property, value),
                ],
            ),
            code,
        );
        assert_eq!(
            served(&db, oid),
            [SIMULATED[0], DEVICE[1], DEVICE[2]].map(Vec::from),
            "{property:?}"
        );
        write_property(
            &mut db,
            oid,
            PropertyIdentifier::DOOR_STATUS,
            enumerated(DoorStatus::OPENED.to_raw()),
        )
        .unwrap();
    }
    // Out of service the rows are writable but not commandable, so a NULL
    // succeeds and leaves each as it is (#1396).
    let before = served(&db, oid);
    for property in ROWS {
        write_property(&mut db, oid, property, PropertyValue::Null).unwrap();
        assert_eq!(served(&db, oid), before, "{property:?}");
    }
    // The proprietary ranges go through and read back.
    for (property, raw, bytes) in [
        (ROWS[0], 1_024, vec![0x92, 0x04, 0x00]),
        (ROWS[2], 256, vec![0x92, 0x01, 0x00]),
    ] {
        write_property(&mut db, oid, property, enumerated(raw)).unwrap();
        assert_eq!(read_bytes(&db, oid, property), bytes);
    }
}

#[test]
fn read_property_derives_secured_status_from_the_served_door() {
    const SECURED: [u8; 2] = [0x91, 0];
    const UNSECURED: [u8; 2] = [0x91, 1];
    const UNKNOWN: [u8; 2] = [0x91, 2];
    let (mut db, oid) = door_db();
    let secured_status =
        |db: &ObjectDatabase| read_bytes(db, oid, PropertyIdentifier::SECURED_STATUS);
    // The device reports the door open and unlocked.
    assert_eq!(secured_status(&db), UNSECURED);

    // Out of service a client simulates a closed, locked door.
    for (property, value) in [
        (
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyValue::Boolean(true),
        ),
        (
            PropertyIdentifier::DOOR_STATUS,
            enumerated(DoorStatus::CLOSED.to_raw()),
        ),
    ] {
        write_property(&mut db, oid, property, value).unwrap();
        assert_eq!(secured_status(&db), UNSECURED, "{property:?}");
    }
    write_property(
        &mut db,
        oid,
        PropertyIdentifier::LOCK_STATUS,
        enumerated(LockStatus::LOCKED.to_raw()),
    )
    .unwrap();
    assert_eq!(secured_status(&db), SECURED);

    // An UNLOCK command unsecures the door until it is relinquished.
    write_property(
        &mut db,
        oid,
        PropertyIdentifier::PRESENT_VALUE,
        enumerated(DoorValue::UNLOCK.to_raw()),
    )
    .unwrap();
    assert_eq!(secured_status(&db), UNSECURED);
    write_property(
        &mut db,
        oid,
        PropertyIdentifier::PRESENT_VALUE,
        PropertyValue::Null,
    )
    .unwrap();
    assert_eq!(secured_status(&db), SECURED);

    // A contact that can't tell whether the door is shut.
    write_property(
        &mut db,
        oid,
        PropertyIdentifier::DOOR_STATUS,
        enumerated(DoorStatus::UNKNOWN.to_raw()),
    )
    .unwrap();
    assert_eq!(secured_status(&db), UNKNOWN);

    // The return to service serves the device's open, unlocked door again.
    write_property(
        &mut db,
        oid,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(false),
    )
    .unwrap();
    assert_eq!(secured_status(&db), UNSECURED);
}

/// An AddListElement or RemoveListElement request on one of the door's alarm
/// lists.
fn list_request(oid: ObjectIdentifier, list: PropertyIdentifier, elements: &[u8]) -> Vec<u8> {
    let mut request = BytesMut::new();
    bacnet_services::list_manipulation::ListElementRequest {
        object_identifier: oid,
        property_identifier: list,
        property_array_index: None,
        list_of_elements: elements.to_vec(),
    }
    .encode(&mut request)
    .unwrap();
    request.to_vec()
}

fn alarm_states(raw: &[u32]) -> PropertyValue {
    PropertyValue::List(raw.iter().copied().map(enumerated).collect())
}

#[test]
fn door_alarm_lists_take_door_alarm_states_over_the_wire() {
    let (mut db, oid) = door_db();
    for list in [
        PropertyIdentifier::ALARM_VALUES,
        PropertyIdentifier::FAULT_VALUES,
        PropertyIdentifier::MASKED_ALARM_VALUES,
    ] {
        // TAMPER alone, then with LOCK_DOWN (#1149). WriteProperty hands the
        // object the one element as a list of one (#1328).
        write_property(&mut db, oid, list, alarm_states(&[4])).unwrap();
        assert_eq!(read_bytes(&db, oid, list), [0x91, 4], "{list:?}");
        write_property_multiple(&mut db, oid, &[(list, alarm_states(&[4, 6]))]).unwrap();
        assert_eq!(read_bytes(&db, oid, list), [0x91, 4, 0x91, 6], "{list:?}");
        // 9 is reserved for ASHRAE, and an Unsigned is the wrong datatype;
        // the refusal names the element and changes nothing.
        for (value, code, element) in [
            (alarm_states(&[4, 9]), ErrorCode::VALUE_OUT_OF_RANGE, 2),
            (
                PropertyValue::List(vec![PropertyValue::Unsigned(4)]),
                ErrorCode::INVALID_DATA_TYPE,
                1,
            ),
        ] {
            assert_eq!(
                list_refusal(write_property(&mut db, oid, list, value)),
                (ErrorClass::PROPERTY, code, element),
                "{list:?}"
            );
            assert_eq!(read_bytes(&db, oid, list), [0x91, 4, 0x91, 6], "{list:?}");
        }

        // The list services edit it too.
        handle_add_list_element(&mut db, &list_request(oid, list, &[0x91, 7])).unwrap();
        assert_eq!(
            read_bytes(&db, oid, list),
            [0x91, 4, 0x91, 6, 0x91, 7],
            "{list:?}"
        );
        assert_eq!(
            list_refusal(handle_add_list_element(
                &mut db,
                &list_request(oid, list, &[0x91, 8, 0x91, 9])
            )),
            (ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE, 2),
            "{list:?}"
        );
        handle_remove_list_element(&mut db, &list_request(oid, list, &[0x91, 4, 0x91, 6])).unwrap();
        assert_eq!(read_bytes(&db, oid, list), [0x91, 7], "{list:?}");
    }
    // No list takes NORMAL, over WriteProperty or AddListElement.
    for list in [
        PropertyIdentifier::ALARM_VALUES,
        PropertyIdentifier::FAULT_VALUES,
        PropertyIdentifier::MASKED_ALARM_VALUES,
    ] {
        assert_eq!(
            list_refusal(write_property(&mut db, oid, list, alarm_states(&[7, 0]))),
            (ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE, 2),
            "{list:?}"
        );
        assert_eq!(
            list_refusal(handle_add_list_element(
                &mut db,
                &list_request(oid, list, &[0x91, 0])
            )),
            (ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE, 1),
            "{list:?}"
        );
        assert_eq!(read_bytes(&db, oid, list), [0x91, 7], "{list:?}");
        // No octets at all is the empty list, over either write service.
        write_property(&mut db, oid, list, alarm_states(&[])).unwrap();
        assert!(read_bytes(&db, oid, list).is_empty(), "{list:?}");
        write_property_multiple(&mut db, oid, &[(list, alarm_states(&[7]))]).unwrap();
        write_property_multiple(&mut db, oid, &[(list, alarm_states(&[]))]).unwrap();
        assert!(read_bytes(&db, oid, list).is_empty(), "{list:?}");
    }
}

#[test]
fn simulated_door_alarm_state_outside_the_lists_is_refused() {
    let (mut db, oid) = door_db();
    write_property(
        &mut db,
        oid,
        PropertyIdentifier::MASKED_ALARM_VALUES,
        alarm_states(&[4]),
    )
    .unwrap();
    write_property(
        &mut db,
        oid,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    )
    .unwrap();
    // LOCK_DOWN (6) is in no list and TAMPER (4) is masked: both refused,
    // over WriteProperty and WritePropertyMultiple, with the device's
    // DOOR_OPEN_TOO_LONG still served.
    for refused in [6, 4] {
        let write = (PropertyIdentifier::DOOR_ALARM_STATE, enumerated(refused));
        assert_property_error(
            write_property(&mut db, oid, write.0, write.1.clone()),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_property_error(
            write_property_multiple(&mut db, oid, &[write]),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_eq!(
            read_bytes(&db, oid, PropertyIdentifier::DOOR_ALARM_STATE),
            [0x91, 2]
        );
    }
    // FORCED_OPEN is an alarm value, and NORMAL is always admitted.
    for taken in [3, 0] {
        write_property(
            &mut db,
            oid,
            PropertyIdentifier::DOOR_ALARM_STATE,
            enumerated(taken),
        )
        .unwrap();
        assert_eq!(
            read_bytes(&db, oid, PropertyIdentifier::DOOR_ALARM_STATE),
            [0x91, taken as u8]
        );
    }
}

#[test]
fn masking_the_door_alarm_state_over_the_wire_returns_it_to_normal() {
    let (mut db, oid) = door_db();
    let alarm_state =
        |db: &ObjectDatabase| read_bytes(db, oid, PropertyIdentifier::DOOR_ALARM_STATE);
    assert_eq!(alarm_state(&db), [0x91, 2]);
    // A WriteProperty of the masked list holding the current state.
    write_property(
        &mut db,
        oid,
        PropertyIdentifier::MASKED_ALARM_VALUES,
        alarm_states(&[2]),
    )
    .unwrap();
    assert_eq!(alarm_state(&db), [0x91, 0]);
    // A masked list fails Secured_Status too.
    assert_eq!(
        read_bytes(&db, oid, PropertyIdentifier::SECURED_STATUS),
        [0x91, 1]
    );

    // AddListElement does the same for a simulated state.
    write_property(
        &mut db,
        oid,
        PropertyIdentifier::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    )
    .unwrap();
    write_property(
        &mut db,
        oid,
        PropertyIdentifier::DOOR_ALARM_STATE,
        enumerated(3),
    )
    .unwrap();
    handle_add_list_element(
        &mut db,
        &list_request(oid, PropertyIdentifier::MASKED_ALARM_VALUES, &[0x91, 3]),
    )
    .unwrap();
    assert_eq!(alarm_state(&db), [0x91, 0]);
}
