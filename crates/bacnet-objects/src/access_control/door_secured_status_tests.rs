//! Secured_Status derived from what the door serves (Clause 12.26.14,
//! #1148, #1149): each input moves it between SECURED and UNSECURED, an
//! input the door's own monitor can't tell makes it UNKNOWN, and pulses,
//! simulated values, the event algorithm, Masked_Alarm_Values and the return
//! to service all show through.

#[cfg(feature = "event-reporting")]
use crate::event::test_clock::test_now;
#[cfg(feature = "event-reporting")]
use crate::traits::EventReporting as _;
use crate::traits::Timed;
use std::time::Duration;

use bacnet_types::enums::PropertyIdentifier as P;

use super::*;
#[cfg(feature = "event-reporting")]
use crate::event::commit_test_proposal;

const SECURED: DoorSecuredStatus = DoorSecuredStatus::SECURED;
const UNSECURED: DoorSecuredStatus = DoorSecuredStatus::UNSECURED;
const UNKNOWN: DoorSecuredStatus = DoorSecuredStatus::UNKNOWN;

fn door() -> AccessDoorObject {
    AccessDoorObject::new(1, "DOOR-1").unwrap()
}

fn secured(door: &AccessDoorObject) -> DoorSecuredStatus {
    match door.read_property(P::SECURED_STATUS, None).unwrap() {
        PropertyValue::Enumerated(raw) => DoorSecuredStatus::from_raw(raw),
        other => panic!("Secured_Status must be Enumerated, got {other:?}"),
    }
}

fn command(door: &mut AccessDoorObject, value: Option<DoorValue>, priority: u8) {
    let value = value.map_or(PropertyValue::Null, |v| {
        PropertyValue::Enumerated(v.to_raw())
    });
    door.write_property(P::PRESENT_VALUE, None, value, Some(priority))
        .unwrap();
}

/// A client's write of Door_Status or Lock_Status, taken out of service.
fn simulate(door: &mut AccessDoorObject, property: P, raw: u32) {
    door.write_property(property, None, PropertyValue::Enumerated(raw), None)
        .unwrap();
}

fn set_out_of_service(door: &mut AccessDoorObject, out_of_service: bool) {
    door.write_property(
        P::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(out_of_service),
        None,
    )
    .unwrap();
}

#[cfg(feature = "event-reporting")]
fn status_flags(bits: StatusFlags) -> PropertyValue {
    PropertyValue::BitString {
        unused_bits: 4,
        data: vec![bits.bits() << 4],
    }
}

#[test]
fn access_door_secured_status_is_secured_with_every_input_met() {
    let mut door = door();
    // A new door is commanded LOCK, closed, locked and not in alarm.
    assert_eq!(secured(&door), SECURED);
    // A door with no contact or no lock monitor fitted meets those inputs.
    door.set_door_status(DoorStatus::UNUSED);
    assert_eq!(secured(&door), SECURED);
    door.set_lock_status(LockStatus::UNUSED);
    assert_eq!(secured(&door), SECURED);
    // Door_Alarm_State isn't an input: only Event_State sets IN_ALARM, and
    // no event algorithm has run on it.
    door.set_alarm_values([DoorAlarmState::FORCED_OPEN])
        .unwrap();
    door.set_door_alarm_state(DoorAlarmState::FORCED_OPEN)
        .unwrap();
    assert_eq!(secured(&door), SECURED);
}

#[test]
fn access_door_secured_status_follows_present_value() {
    let mut door = door();
    for value in [
        DoorValue::UNLOCK,
        DoorValue::PULSE_UNLOCK,
        DoorValue::EXTENDED_PULSE_UNLOCK,
    ] {
        command(&mut door, Some(value), 8);
        assert_eq!(secured(&door), UNSECURED, "{value:?}");
        command(&mut door, None, 8);
        assert_eq!(secured(&door), SECURED, "{value:?} relinquished");
    }
    // A LOCK command outranking an UNLOCK secures the door.
    command(&mut door, Some(DoorValue::UNLOCK), 10);
    command(&mut door, Some(DoorValue::LOCK), 5);
    assert_eq!(secured(&door), SECURED);
    command(&mut door, None, 5);
    assert_eq!(secured(&door), UNSECURED);
    command(&mut door, None, 10);
    // Present_Value taken from Relinquish_Default counts the same way.
    door.set_relinquish_default(DoorValue::UNLOCK).unwrap();
    assert_eq!(secured(&door), UNSECURED);
    door.set_relinquish_default(DoorValue::LOCK).unwrap();
    assert_eq!(secured(&door), SECURED);
}

#[test]
fn access_door_secured_status_follows_door_and_lock_status() {
    let mut door = door();
    // Every Door_Status other than CLOSED and UNUSED is a door not shut,
    // except the two that can't tell (the UNKNOWN test has those).
    let open = DoorStatus::ALL_NAMED
        .iter()
        .map(|&(_, status)| status)
        .filter(|&status| {
            !matches!(
                status,
                DoorStatus::CLOSED
                    | DoorStatus::UNUSED
                    | DoorStatus::UNKNOWN
                    | DoorStatus::DOOR_FAULT
            )
        })
        .chain([DoorStatus::from_raw(1_024)]);
    for status in open {
        door.set_door_status(status);
        assert_eq!(secured(&door), UNSECURED, "{status:?}");
        door.set_door_status(DoorStatus::CLOSED);
        assert_eq!(secured(&door), SECURED, "{status:?} closed again");
    }
    door.set_lock_status(LockStatus::UNLOCKED);
    assert_eq!(secured(&door), UNSECURED);
    door.set_lock_status(LockStatus::LOCKED);
    assert_eq!(secured(&door), SECURED);
}

#[cfg(feature = "event-reporting")]
/// Move the door to `state`, then let its event algorithm (Time_Delay 0)
/// commit whatever transition that proposes.
fn report(door: &mut AccessDoorObject, state: DoorAlarmState) {
    door.set_door_alarm_state(state).unwrap();
    let outcome = door.evaluate_intrinsic_reporting(test_now()).unwrap();
    commit_test_proposal(door, outcome);
}

#[cfg(feature = "event-reporting")]
#[test]
fn access_door_secured_status_follows_the_in_alarm_flag() {
    let mut door = door();
    door.set_alarm_values([DoorAlarmState::FORCED_OPEN])
        .unwrap();
    door.set_fault_values([DoorAlarmState::DOOR_FAULT]).unwrap();
    for (state, event_state, flags) in [
        (
            DoorAlarmState::FORCED_OPEN,
            EventState::OFFNORMAL,
            StatusFlags::IN_ALARM,
        ),
        (
            DoorAlarmState::DOOR_FAULT,
            EventState::FAULT,
            StatusFlags::IN_ALARM | StatusFlags::FAULT,
        ),
    ] {
        report(&mut door, state);
        assert_eq!(
            door.read_property(P::EVENT_STATE, None).unwrap(),
            PropertyValue::Enumerated(event_state.to_raw())
        );
        assert_eq!(
            door.read_property(P::STATUS_FLAGS, None).unwrap(),
            status_flags(flags),
            "{state:?}"
        );
        assert_eq!(secured(&door), UNSECURED, "{state:?}");
        report(&mut door, DoorAlarmState::NORMAL);
    }
    assert_eq!(
        door.read_property(P::STATUS_FLAGS, None).unwrap(),
        status_flags(StatusFlags::empty())
    );
    assert_eq!(secured(&door), SECURED);
}

#[test]
fn access_door_secured_status_fails_while_any_state_is_masked() {
    let mut door = door();
    // Masking a state the door isn't in still unsecures it.
    door.set_masked_alarm_values([DoorAlarmState::TAMPER])
        .unwrap();
    assert_eq!(secured(&door), UNSECURED);
    door.write_property(
        P::MASKED_ALARM_VALUES,
        None,
        PropertyValue::List(vec![]),
        None,
    )
    .unwrap();
    assert_eq!(secured(&door), SECURED);
}

#[test]
fn access_door_secured_status_is_unknown_when_an_input_cannot_be_told() {
    let mut door = door();
    for status in [DoorStatus::UNKNOWN, DoorStatus::DOOR_FAULT] {
        door.set_door_status(status);
        assert_eq!(secured(&door), UNKNOWN, "{status:?}");
        // A failed input settles it whatever the contact can't tell.
        command(&mut door, Some(DoorValue::UNLOCK), 8);
        assert_eq!(secured(&door), UNSECURED, "{status:?} unlocked");
        command(&mut door, None, 8);
        door.set_lock_status(LockStatus::UNLOCKED);
        assert_eq!(secured(&door), UNSECURED, "{status:?} lock open");
        door.set_lock_status(LockStatus::LOCKED);
        door.set_door_status(DoorStatus::CLOSED);
        assert_eq!(secured(&door), SECURED, "{status:?} closed again");
    }
    for status in [LockStatus::UNKNOWN, LockStatus::LOCK_FAULT] {
        door.set_lock_status(status);
        assert_eq!(secured(&door), UNKNOWN, "{status:?}");
        door.set_door_status(DoorStatus::OPENED);
        assert_eq!(secured(&door), UNSECURED, "{status:?} door open");
        door.set_door_status(DoorStatus::CLOSED);
        door.set_lock_status(LockStatus::LOCKED);
        assert_eq!(secured(&door), SECURED, "{status:?} locked again");
    }
    // Both monitors unable to tell is still UNKNOWN, not UNSECURED.
    door.set_door_status(DoorStatus::UNKNOWN);
    door.set_lock_status(LockStatus::LOCK_FAULT);
    assert_eq!(secured(&door), UNKNOWN);
}

#[test]
fn access_door_secured_status_follows_a_pulse_and_its_relock() {
    let mut door = door();
    door.set_door_pulse_time(30);
    door.set_door_extended_pulse_time(50);
    for (value, tenths) in [
        (DoorValue::PULSE_UNLOCK, 30),
        (DoorValue::EXTENDED_PULSE_UNLOCK, 50),
    ] {
        command(&mut door, Some(value), 8);
        assert_eq!(secured(&door), UNSECURED, "{value:?}");
        assert!(!door.advance_time(Duration::from_millis((tenths - 1) * 100)));
        assert_eq!(secured(&door), UNSECURED, "{value:?} one tenth short");
        // The relock relinquishes the slot and the door reads secure again,
        // in a COV snapshot as well.
        assert!(door.advance_time(Duration::from_millis(100)));
        assert_eq!(secured(&door), SECURED, "{value:?} relocked");
        let snapshot = door.cov_snapshot().unwrap();
        assert_eq!(
            snapshot.read_property(P::SECURED_STATUS, None).unwrap(),
            PropertyValue::Enumerated(SECURED.to_raw())
        );
    }
}

#[test]
fn access_door_secured_status_follows_a_simulation_and_the_return_to_service() {
    let mut door = door();
    set_out_of_service(&mut door, true);
    assert_eq!(secured(&door), SECURED);
    simulate(&mut door, P::DOOR_STATUS, DoorStatus::OPENED.to_raw());
    assert_eq!(secured(&door), UNSECURED);
    simulate(&mut door, P::DOOR_STATUS, DoorStatus::CLOSED.to_raw());
    simulate(&mut door, P::LOCK_STATUS, LockStatus::UNLOCKED.to_raw());
    assert_eq!(secured(&door), UNSECURED);
    simulate(&mut door, P::LOCK_STATUS, LockStatus::UNKNOWN.to_raw());
    assert_eq!(secured(&door), UNKNOWN);
    simulate(&mut door, P::LOCK_STATUS, LockStatus::LOCKED.to_raw());
    assert_eq!(secured(&door), SECURED);

    // The device's reports meanwhile are put aside, so they don't move it.
    door.set_door_status(DoorStatus::OPENED);
    door.set_lock_status(LockStatus::UNLOCKED);
    assert_eq!(secured(&door), SECURED);

    // The return to service serves the device's open, unlocked door.
    set_out_of_service(&mut door, false);
    assert_eq!(secured(&door), UNSECURED);
    door.set_door_status(DoorStatus::CLOSED);
    assert_eq!(secured(&door), UNSECURED);
    door.set_lock_status(LockStatus::LOCKED);
    assert_eq!(secured(&door), SECURED);

    // A simulated fault is dropped on the return to service.
    set_out_of_service(&mut door, true);
    simulate(&mut door, P::DOOR_STATUS, DoorStatus::DOOR_FAULT.to_raw());
    assert_eq!(secured(&door), UNKNOWN);
    set_out_of_service(&mut door, false);
    assert_eq!(secured(&door), SECURED);
}
