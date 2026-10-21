//! Access Door pulse timing and the Table 12-30 rows that drive it (#1073):
//! Door_Pulse_Time, Door_Extended_Pulse_Time, Door_Open_Too_Long_Time and
//! Current_Command_Priority.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bacnet_types::enums::{ErrorClass, ErrorCode};

use super::*;

const PV: PropertyIdentifier = PropertyIdentifier::PRESENT_VALUE;
const CCP: PropertyIdentifier = PropertyIdentifier::CURRENT_COMMAND_PRIORITY;
const PULSE: PropertyIdentifier = PropertyIdentifier::DOOR_PULSE_TIME;
const EXTENDED: PropertyIdentifier = PropertyIdentifier::DOOR_EXTENDED_PULSE_TIME;
const TOO_LONG: PropertyIdentifier = PropertyIdentifier::DOOR_OPEN_TOO_LONG_TIME;

fn command(door: &mut AccessDoorObject, value: Option<DoorValue>, priority: u8) {
    let value = value.map_or(PropertyValue::Null, |v| {
        PropertyValue::Enumerated(v.to_raw())
    });
    door.write_property(PV, None, value, Some(priority))
        .unwrap();
}

fn pv(door: &AccessDoorObject) -> DoorValue {
    match door.read_property(PV, None).unwrap() {
        PropertyValue::Enumerated(raw) => DoorValue::from_raw(raw),
        other => panic!("Present_Value must be Enumerated, got {other:?}"),
    }
}

fn slot(door: &AccessDoorObject, priority: u32) -> PropertyValue {
    door.read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(priority))
        .unwrap()
}

fn ccp(door: &AccessDoorObject) -> PropertyValue {
    door.read_property(CCP, None).unwrap()
}

fn tenths(n: u64) -> Duration {
    Duration::from_millis(n * 100)
}

#[test]
fn access_door_pulse_unlock_relocks_after_door_pulse_time() {
    let mut door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    door.write_property(PULSE, None, PropertyValue::Unsigned(30), None)
        .unwrap();
    assert_eq!(door.next_monotonic_deadline_internal(), None);
    command(&mut door, Some(DoorValue::PULSE_UNLOCK), 8);
    assert_eq!(pv(&door), DoorValue::PULSE_UNLOCK);
    assert_eq!(ccp(&door), PropertyValue::Unsigned(8));
    assert_eq!(door.next_monotonic_deadline_internal(), Some(tenths(30)));

    // One tenth short of the pulse the slot holds; at the pulse time it is
    // relinquished and the door falls back to Relinquish_Default.
    assert!(!door.advance_time_internal(tenths(29)));
    assert_eq!(pv(&door), DoorValue::PULSE_UNLOCK);
    assert!(door.advance_time_internal(tenths(1)));
    assert_eq!(pv(&door), DoorValue::LOCK);
    assert_eq!(slot(&door, 8), PropertyValue::Null);
    assert_eq!(ccp(&door), PropertyValue::Null);
    assert_eq!(door.next_monotonic_deadline_internal(), None);
    assert!(!door.advance_time_internal(tenths(100)));
}

#[test]
fn access_door_extended_pulse_uses_its_own_time_and_reveals_lower_commands() {
    let mut door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    door.set_door_pulse_time(10);
    door.set_door_extended_pulse_time(40);
    command(&mut door, Some(DoorValue::UNLOCK), 12);
    command(&mut door, Some(DoorValue::EXTENDED_PULSE_UNLOCK), 6);
    assert_eq!(pv(&door), DoorValue::EXTENDED_PULSE_UNLOCK);
    assert_eq!(ccp(&door), PropertyValue::Unsigned(6));
    // Door_Pulse_Time has passed, Door_Extended_Pulse_Time hasn't.
    assert!(!door.advance_time_internal(tenths(39)));
    assert!(door.advance_time_internal(tenths(1)));
    // The relock falls through to the next live command, not to the default.
    assert_eq!(pv(&door), DoorValue::UNLOCK);
    assert_eq!(ccp(&door), PropertyValue::Unsigned(12));
}

#[test]
fn access_door_pulse_below_a_live_command_or_of_zero_length_never_takes_effect() {
    let mut door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    command(&mut door, Some(DoorValue::LOCK), 4);
    for value in [DoorValue::PULSE_UNLOCK, DoorValue::EXTENDED_PULSE_UNLOCK] {
        command(&mut door, Some(value), 9);
        assert_eq!(slot(&door, 9), PropertyValue::Null);
        assert_eq!(pv(&door), DoorValue::LOCK);
        assert_eq!(door.next_monotonic_deadline_internal(), None);
    }
    // A pulse written to the live command's own slot replaces it.
    command(&mut door, Some(DoorValue::PULSE_UNLOCK), 4);
    assert_eq!(pv(&door), DoorValue::PULSE_UNLOCK);
    command(&mut door, None, 4);

    door.set_door_pulse_time(0);
    command(&mut door, Some(DoorValue::PULSE_UNLOCK), 8);
    assert_eq!(slot(&door, 8), PropertyValue::Null);
    assert_eq!(pv(&door), DoorValue::LOCK);
    assert_eq!(door.next_monotonic_deadline_internal(), None);
}

#[test]
fn access_door_rewriting_a_pulse_slot_cancels_or_rearms_its_deadline() {
    let mut door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    door.set_door_pulse_time(20);
    command(&mut door, Some(DoorValue::PULSE_UNLOCK), 8);
    door.advance_time_internal(tenths(15));
    // A second pulse at the same slot restarts the pulse from now.
    command(&mut door, Some(DoorValue::PULSE_UNLOCK), 8);
    assert_eq!(door.next_monotonic_deadline_internal(), Some(tenths(35)));
    assert!(!door.advance_time_internal(tenths(19)));
    assert!(door.advance_time_internal(tenths(1)));

    // A steady command or a relinquish in the slot cancels the deadline.
    for replacement in [Some(DoorValue::UNLOCK), None] {
        command(&mut door, Some(DoorValue::PULSE_UNLOCK), 8);
        command(&mut door, replacement, 8);
        assert_eq!(door.next_monotonic_deadline_internal(), None);
        assert!(!door.advance_time_internal(tenths(50)));
        assert_eq!(
            slot(&door, 8),
            replacement.map_or(PropertyValue::Null, |v| PropertyValue::Enumerated(
                v.to_raw()
            ))
        );
    }
    // Changing the pulse time leaves an armed deadline where it was.
    command(&mut door, Some(DoorValue::PULSE_UNLOCK), 8);
    let armed = door.next_monotonic_deadline_internal();
    door.set_door_pulse_time(500);
    assert_eq!(door.next_monotonic_deadline_internal(), armed);
}

#[test]
fn access_door_pulses_at_two_priorities_expire_independently() {
    let mut door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    door.set_door_pulse_time(10);
    door.set_door_extended_pulse_time(30);
    command(&mut door, Some(DoorValue::EXTENDED_PULSE_UNLOCK), 10);
    command(&mut door, Some(DoorValue::PULSE_UNLOCK), 5);
    assert_eq!(door.next_monotonic_deadline_internal(), Some(tenths(10)));
    assert!(door.advance_time_internal(tenths(10)));
    assert_eq!(pv(&door), DoorValue::EXTENDED_PULSE_UNLOCK);
    assert_eq!(door.next_monotonic_deadline_internal(), Some(tenths(30)));
    assert!(door.advance_monotonic_time_internal(tenths(30)));
    assert_eq!(pv(&door), DoorValue::LOCK);
}

#[test]
fn access_door_pulse_deadline_comes_from_the_bound_monotonic_clock() {
    let now = Arc::new(AtomicU64::new(1_000));
    let reader = Arc::clone(&now);
    let mut door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    door.bind_monotonic_clock_internal(Some(Arc::new(move || {
        Duration::from_millis(reader.load(Ordering::SeqCst))
    })));
    door.set_door_pulse_time(25);
    command(&mut door, Some(DoorValue::PULSE_UNLOCK), 8);
    let deadline = Duration::from_millis(3_500);
    assert_eq!(door.next_monotonic_deadline_internal(), Some(deadline));
    now.store(3_499, Ordering::SeqCst);
    assert!(!door.advance_monotonic_time_internal(Duration::from_millis(3_499)));
    assert!(door.advance_monotonic_time_internal(deadline));
    assert_eq!(pv(&door), DoorValue::LOCK);
    // The relock is visible to COV through a snapshot of the new state.
    let snapshot = door.cov_snapshot_internal().unwrap();
    assert_eq!(
        snapshot.read_property(PV, None).unwrap(),
        PropertyValue::Enumerated(0)
    );
}

#[test]
fn access_door_current_command_priority_tracks_the_winning_slot() {
    let mut door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    door.set_relinquish_default(DoorValue::UNLOCK).unwrap();
    assert_eq!(ccp(&door), PropertyValue::Null);
    command(&mut door, Some(DoorValue::LOCK), 16);
    assert_eq!(ccp(&door), PropertyValue::Unsigned(16));
    command(&mut door, Some(DoorValue::UNLOCK), 1);
    assert_eq!(ccp(&door), PropertyValue::Unsigned(1));
    command(&mut door, None, 1);
    assert_eq!(ccp(&door), PropertyValue::Unsigned(16));
    command(&mut door, None, 16);
    assert_eq!(ccp(&door), PropertyValue::Null);
}

#[test]
fn access_door_times_are_unsigned32_tenths() {
    let mut door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    for property in [PULSE, EXTENDED, TOO_LONG] {
        assert!(door.is_writable_property(property));
        door.write_property(
            property,
            None,
            PropertyValue::Unsigned(u32::MAX.into()),
            None,
        )
        .unwrap();
        assert_eq!(
            door.read_property(property, None).unwrap(),
            PropertyValue::Unsigned(u32::MAX.into())
        );
        for (value, code) in [
            (
                PropertyValue::Unsigned(1 << 32),
                ErrorCode::VALUE_OUT_OF_RANGE,
            ),
            (PropertyValue::Signed(5), ErrorCode::INVALID_DATA_TYPE),
            (PropertyValue::Null, ErrorCode::INVALID_DATA_TYPE),
        ] {
            match door.write_property(property, None, value, None) {
                Err(Error::Protocol { class, code: c }) => {
                    assert_eq!(class, ErrorClass::PROPERTY);
                    assert_eq!(c, code, "{property:?}");
                }
                other => panic!("expected {code:?}, got {other:?}"),
            }
            assert_eq!(
                door.read_property(property, None).unwrap(),
                PropertyValue::Unsigned(u32::MAX.into())
            );
        }
    }
    // Current_Command_Priority is derived and read-only.
    assert!(!door.is_writable_property(CCP));
    assert!(door
        .write_property(CCP, None, PropertyValue::Unsigned(8), None)
        .is_err());
}
