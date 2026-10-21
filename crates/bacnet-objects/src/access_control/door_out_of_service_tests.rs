//! Door_Status, Lock_Status and Door_Alarm_State writes while Out_Of_Service
//! is TRUE (Clause 12.26.9, Table 12-30 footnote 1, #1131).

use std::time::Duration;

use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};

use super::*;

/// The three rows footnote 1 marks.
const SIMULATED: [P; 3] = [P::DOOR_STATUS, P::LOCK_STATUS, P::DOOR_ALARM_STATE];

/// A door whose Alarm_Values hold every named BACnetDoorAlarmState but
/// NORMAL, which needs no listing, and the proprietary ones these tests
/// simulate, so only the enumeration's range and the in-service gate refuse
/// a write here (`door_alarm_tests` has the list checks).
fn door() -> AccessDoorObject {
    let mut door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    let proprietary = [256, 65_535].map(DoorAlarmState::from_raw);
    door.set_alarm_values(
        DoorAlarmState::ALL_NAMED
            .iter()
            .map(|&(_, state)| state)
            .filter(|&state| state != DoorAlarmState::NORMAL)
            .chain(proprietary),
    )
    .unwrap();
    door
}

fn read(door: &AccessDoorObject, property: P) -> PropertyValue {
    door.read_property(property, None).unwrap()
}

fn write(door: &mut AccessDoorObject, property: P, value: PropertyValue) -> Result<(), Error> {
    door.write_property(property, None, value, None)
}

fn set_out_of_service(door: &mut AccessDoorObject, out_of_service: bool) {
    write(
        door,
        P::OUT_OF_SERVICE,
        PropertyValue::Boolean(out_of_service),
    )
    .unwrap();
}

fn enumerated(raw: u32) -> PropertyValue {
    PropertyValue::Enumerated(raw)
}

/// Each named value of a production with its number.
fn named<T: Copy>(all: &[(&'static str, T)], to_raw: fn(T) -> u32) -> Vec<(&'static str, u32)> {
    all.iter()
        .map(|&(name, value)| (name, to_raw(value)))
        .collect()
}

/// Door_Status, Lock_Status and Door_Alarm_State as served.
fn served(door: &AccessDoorObject) -> [PropertyValue; 3] {
    SIMULATED.map(|property| read(door, property))
}

fn assert_property_error(result: Result<(), Error>, code: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Protocol { class, code: actual })
            if class == ErrorClass::PROPERTY
                && actual == code),
        "expected PROPERTY / {code:?}, got {result:?}"
    );
}

#[test]
fn access_door_refuses_simulated_rows_in_service() {
    let mut door = door();
    let before = served(&door);
    for property in SIMULATED {
        // A valid value, a reserved one and a wrong datatype are all refused
        // for being in service.
        for value in [enumerated(1), enumerated(1_000), PropertyValue::Real(1.0)] {
            assert_property_error(
                write(&mut door, property, value),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
        }
        // The PICS still lists the row writable: the route exists, gated on
        // Out_Of_Service.
        assert!(door.is_writable_property(property), "{property:?}");
    }
    assert_eq!(served(&door), before);
}

#[test]
fn access_door_takes_simulated_rows_out_of_service() {
    let mut door = door();
    set_out_of_service(&mut door, true);
    let rows = [
        (
            P::DOOR_STATUS,
            named(DoorStatus::ALL_NAMED, DoorStatus::to_raw),
        ),
        (
            P::LOCK_STATUS,
            named(LockStatus::ALL_NAMED, LockStatus::to_raw),
        ),
        (
            P::DOOR_ALARM_STATE,
            named(DoorAlarmState::ALL_NAMED, DoorAlarmState::to_raw),
        ),
    ];
    for (property, values) in rows {
        for (name, raw) in values {
            write(&mut door, property, enumerated(raw)).unwrap();
            assert_eq!(
                read(&door, property),
                enumerated(raw),
                "{property:?} {name}"
            );
        }
    }
    // The proprietary ranges read back verbatim.
    for (property, raw) in [
        (P::DOOR_STATUS, 1_024),
        (P::DOOR_STATUS, 65_535),
        (P::DOOR_ALARM_STATE, 256),
        (P::DOOR_ALARM_STATE, 65_535),
    ] {
        write(&mut door, property, enumerated(raw)).unwrap();
        assert_eq!(read(&door, property), enumerated(raw), "{property:?}");
    }
    // The simulation leaves the rows nothing simulates alone. Secured_Status
    // is derived from it instead (door_secured_status_tests.rs).
    assert_eq!(read(&door, P::PRESENT_VALUE), enumerated(0));
    assert_eq!(
        read(&door, P::RELIABILITY),
        enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
    );
    assert_eq!(read(&door, P::EVENT_STATE), enumerated(0));
}

#[test]
fn access_door_simulated_rows_outside_their_datatypes_are_refused_unchanged() {
    let mut door = door();
    set_out_of_service(&mut door, true);
    let before = served(&door);
    for (property, raw) in [
        // 10..=1023 are reserved for ASHRAE.
        (P::DOOR_STATUS, 10),
        (P::DOOR_STATUS, 1_023),
        (P::DOOR_STATUS, 65_536),
        // BACnetLockStatus is closed: past UNKNOWN (4) nothing is in range.
        (P::LOCK_STATUS, 5),
        (P::LOCK_STATUS, 256),
        (P::LOCK_STATUS, 1_024),
        (P::LOCK_STATUS, 65_535),
        // 9..=255 are reserved for ASHRAE.
        (P::DOOR_ALARM_STATE, 9),
        (P::DOOR_ALARM_STATE, 255),
        (P::DOOR_ALARM_STATE, 65_536),
    ] {
        assert_property_error(
            write(&mut door, property, enumerated(raw)),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    for property in SIMULATED {
        for wrong in [
            PropertyValue::Unsigned(1),
            PropertyValue::Null,
            PropertyValue::Boolean(true),
        ] {
            assert_property_error(
                write(&mut door, property, wrong),
                ErrorCode::INVALID_DATA_TYPE,
            );
        }
    }
    assert_eq!(served(&door), before);
}

#[test]
fn access_door_return_to_service_serves_the_device_state_again() {
    let mut door = door();
    door.set_door_status(DoorStatus::OPENED);
    door.set_lock_status(LockStatus::UNLOCKED);
    door.set_door_alarm_state(DoorAlarmState::DOOR_OPEN_TOO_LONG)
        .unwrap();
    let device = served(&door);
    assert_eq!(
        device,
        [
            enumerated(DoorStatus::OPENED.to_raw()),
            enumerated(LockStatus::UNLOCKED.to_raw()),
            enumerated(DoorAlarmState::DOOR_OPEN_TOO_LONG.to_raw()),
        ]
    );
    set_out_of_service(&mut door, true);
    // Entering out of service changes nothing served.
    assert_eq!(served(&door), device);
    let simulated = [
        enumerated(DoorStatus::DOOR_FAULT.to_raw()),
        enumerated(LockStatus::LOCK_FAULT.to_raw()),
        enumerated(DoorAlarmState::TAMPER.to_raw()),
    ];
    for (property, value) in SIMULATED.into_iter().zip(simulated.clone()) {
        write(&mut door, property, value).unwrap();
    }
    assert_eq!(served(&door), simulated);

    // The device keeps reporting while the client simulates; its reports are
    // held for the return to service.
    door.set_door_status(DoorStatus::CLOSED);
    door.set_lock_status(LockStatus::LOCKED);
    door.set_door_alarm_state(DoorAlarmState::NORMAL).unwrap();
    assert_eq!(served(&door), simulated);
    // A NULL or same-value Out_Of_Service write is not an edge.
    write(&mut door, P::OUT_OF_SERVICE, PropertyValue::Null).unwrap();
    set_out_of_service(&mut door, true);
    assert_eq!(served(&door), simulated);

    set_out_of_service(&mut door, false);
    assert_eq!(served(&door), served(&self::door()));
    // In service the device's values are served directly again, and the
    // rows refuse writes once more.
    door.set_door_alarm_state(DoorAlarmState::FORCED_OPEN)
        .unwrap();
    assert_eq!(
        read(&door, P::DOOR_ALARM_STATE),
        enumerated(DoorAlarmState::FORCED_OPEN.to_raw())
    );
    assert_property_error(
        write(&mut door, P::DOOR_ALARM_STATE, enumerated(0)),
        ErrorCode::WRITE_ACCESS_DENIED,
    );

    // A simulation the device never reports through returns the entry values.
    set_out_of_service(&mut door, true);
    write(&mut door, P::DOOR_STATUS, enumerated(1)).unwrap();
    set_out_of_service(&mut door, false);
    assert_eq!(read(&door, P::DOOR_STATUS), enumerated(0));
    assert_eq!(
        read(&door, P::DOOR_ALARM_STATE),
        enumerated(DoorAlarmState::FORCED_OPEN.to_raw())
    );
}

#[test]
fn access_door_simulation_neither_drives_nor_follows_the_pulse_relock() {
    let mut door = door();
    door.set_door_pulse_time(30);
    set_out_of_service(&mut door, true);
    door.write_property(
        P::PRESENT_VALUE,
        None,
        enumerated(DoorValue::PULSE_UNLOCK.to_raw()),
        Some(8),
    )
    .unwrap();
    let deadline = door.next_monotonic_deadline_internal();
    assert_eq!(deadline, Some(Duration::from_secs(3)));

    // An open, unlocked, alarming door doesn't move or cancel the relock.
    let simulated = [
        enumerated(DoorStatus::OPENED.to_raw()),
        enumerated(LockStatus::UNLOCKED.to_raw()),
        enumerated(DoorAlarmState::FORCED_OPEN.to_raw()),
    ];
    for (property, value) in SIMULATED.into_iter().zip(simulated.clone()) {
        write(&mut door, property, value).unwrap();
        assert_eq!(door.next_monotonic_deadline_internal(), deadline);
    }
    assert!(!door.advance_time_internal(Duration::from_millis(2_900)));
    assert_eq!(
        read(&door, P::PRESENT_VALUE),
        enumerated(DoorValue::PULSE_UNLOCK.to_raw())
    );

    // The relock comes at the pulse time and leaves the simulated rows alone.
    assert!(door.advance_time_internal(Duration::from_millis(100)));
    assert_eq!(
        read(&door, P::PRESENT_VALUE),
        enumerated(DoorValue::LOCK.to_raw())
    );
    assert_eq!(served(&door), simulated);
    assert_eq!(door.next_monotonic_deadline_internal(), None);

    // Simulating a closed, locked door with no pulse armed arms nothing.
    for property in SIMULATED {
        write(&mut door, property, enumerated(0)).unwrap();
    }
    assert_eq!(door.next_monotonic_deadline_internal(), None);
    assert!(!door.advance_time_internal(Duration::from_secs(60)));
}
