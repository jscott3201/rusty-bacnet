//! Access Door alarm lists and intrinsic reporting (Clause 12.26, Table
//! 12-30, Clauses 12.26.20, 12.26.21, 12.26.25 and 12.26.26, Clauses 13.3.2
//! and 13.4.5, #1149): Alarm_Values, Fault_Values and Masked_Alarm_Values,
//! the Door_Alarm_State they admit, CHANGE_OF_STATE on Door_Alarm_State and
//! the FAULT_STATE check behind Reliability.

use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::enums::PropertyIdentifier as P;
use bacnet_types::enums::{DoorAlarmState as S, ErrorClass, ErrorCode, EventType};

use super::*;
use crate::event::{commit_test_proposal, EventStateChange, TransitionOutcome};

const LISTS: [P; 3] = [P::ALARM_VALUES, P::FAULT_VALUES, P::MASKED_ALARM_VALUES];

fn read(door: &AccessDoorObject, property: P) -> PropertyValue {
    door.read_property(property, None).unwrap()
}

fn write(door: &mut AccessDoorObject, property: P, value: PropertyValue) -> Result<(), Error> {
    door.write_property(property, None, value, None)
}

fn states(states: &[S]) -> PropertyValue {
    PropertyValue::List(
        states
            .iter()
            .map(|state| PropertyValue::Enumerated(state.to_raw()))
            .collect(),
    )
}

fn enumerated(raw: u32) -> PropertyValue {
    PropertyValue::Enumerated(raw)
}

fn alarm_state(door: &AccessDoorObject) -> PropertyValue {
    read(door, P::DOOR_ALARM_STATE)
}

fn set_out_of_service(door: &mut AccessDoorObject, out_of_service: bool) {
    write(
        door,
        P::OUT_OF_SERVICE,
        PropertyValue::Boolean(out_of_service),
    )
    .unwrap();
}

fn status_flags(bits: StatusFlags) -> PropertyValue {
    PropertyValue::BitString {
        unused_bits: 4,
        data: vec![bits.bits() << 4],
    }
}

fn change(from: EventState, to: EventState) -> EventStateChange {
    EventStateChange { from, to }
}

/// A PROPERTY / VALUE_OUT_OF_RANGE refusal; a list setter's names the
/// element too.
fn assert_out_of_range(result: Result<(), Error>) {
    assert!(
        matches!(result, Err(Error::Protocol { class, code } | Error::Structured { class, code, .. })
            if class == ErrorClass::PROPERTY
                && code == ErrorCode::VALUE_OUT_OF_RANGE),
        "expected PROPERTY / VALUE_OUT_OF_RANGE, got {result:?}"
    );
}

/// A door that alarms on FORCED_OPEN and DOOR_OPEN_TOO_LONG after
/// `time_delay` seconds, faults on DOOR_FAULT, and distributes every
/// transition, configured as a client would configure it.
fn alarming_door(time_delay: u64) -> AccessDoorObject {
    let mut door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    for (property, value) in [
        (
            P::ALARM_VALUES,
            states(&[S::FORCED_OPEN, S::DOOR_OPEN_TOO_LONG]),
        ),
        (P::FAULT_VALUES, states(&[S::DOOR_FAULT])),
        (P::TIME_DELAY, PropertyValue::Unsigned(time_delay)),
        (
            P::EVENT_ENABLE,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![EventTransitionBits::all().to_bacnet()],
            },
        ),
    ] {
        write(&mut door, property, value).unwrap();
    }
    door
}

#[test]
fn access_door_serves_the_event_rows_and_alarm_lists() {
    let door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    let no_stamp = PropertyValue::ApplicationData(vec![0x19, 0x00]);
    for (property, value) in [
        (P::TIME_DELAY, PropertyValue::Unsigned(0)),
        (P::NOTIFICATION_CLASS, PropertyValue::Unsigned(0)),
        (P::ALARM_VALUES, PropertyValue::List(vec![])),
        (P::FAULT_VALUES, PropertyValue::List(vec![])),
        (P::MASKED_ALARM_VALUES, PropertyValue::List(vec![])),
        (
            P::EVENT_ENABLE,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0x00],
            },
        ),
        (
            P::ACKED_TRANSITIONS,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0xe0],
            },
        ),
        (P::NOTIFY_TYPE, enumerated(0)),
        (
            P::EVENT_TIME_STAMPS,
            PropertyValue::List(vec![no_stamp.clone(), no_stamp.clone(), no_stamp]),
        ),
        (
            P::EVENT_MESSAGE_TEXTS,
            PropertyValue::List(vec![PropertyValue::CharacterString(String::new()); 3]),
        ),
        (P::EVENT_DETECTION_ENABLE, PropertyValue::Boolean(true)),
        (P::TIME_DELAY_NORMAL, PropertyValue::Unsigned(0)),
        (P::EVENT_STATE, enumerated(EventState::NORMAL.to_raw())),
        (
            P::RELIABILITY,
            enumerated(Reliability::NO_FAULT_DETECTED.to_raw()),
        ),
    ] {
        assert_eq!(read(&door, property), value, "{property:?}");
        assert!(door.property_list().contains(&property), "{property:?}");
    }
    assert_eq!(
        door.enrollment_summary_capability_internal()
            .map(|capability| capability.event_type),
        Some(EventType::CHANGE_OF_STATE)
    );
}

#[test]
fn access_door_alarm_lists_take_door_alarm_states_only() {
    let mut door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    for list in LISTS {
        let two = states(&[S::FORCED_OPEN, S::TAMPER]);
        write(&mut door, list, two.clone()).unwrap();
        assert_eq!(read(&door, list), two, "{list:?}");
        // WriteProperty hands over a one-element list as the element alone.
        write(&mut door, list, enumerated(S::LOCK_DOWN.to_raw())).unwrap();
        assert_eq!(read(&door, list), states(&[S::LOCK_DOWN]), "{list:?}");
        // A proprietary state is a BACnetDoorAlarmState too.
        let proprietary = PropertyValue::List(vec![enumerated(256), enumerated(65_535)]);
        write(&mut door, list, proprietary.clone()).unwrap();
        assert_eq!(read(&door, list), proprietary, "{list:?}");

        for (value, class, code, position) in [
            // 9 to 255 are reserved for ASHRAE.
            (
                PropertyValue::List(vec![enumerated(3), enumerated(9)]),
                ErrorClass::PROPERTY,
                ErrorCode::VALUE_OUT_OF_RANGE,
                2,
            ),
            (
                PropertyValue::List(vec![enumerated(65_536)]),
                ErrorClass::PROPERTY,
                ErrorCode::VALUE_OUT_OF_RANGE,
                1,
            ),
            (
                PropertyValue::List(vec![PropertyValue::Unsigned(3)]),
                ErrorClass::PROPERTY,
                ErrorCode::INVALID_DATA_TYPE,
                1,
            ),
        ] {
            crate::common::assert_list_element_refused(
                write(&mut door, list, value),
                class,
                code,
                position,
                &format!("{list:?}"),
            );
            assert_eq!(read(&door, list), proprietary, "{list:?}");
        }
        assert!(door
            .write_property(list, Some(1), enumerated(3), None)
            .is_err());
        assert_eq!(read(&door, list), proprietary, "{list:?}");
        write(&mut door, list, PropertyValue::List(vec![])).unwrap();
    }

    // No list takes NORMAL, the state with no alarm or fault: as an alarm
    // or fault value it would put a quiet door in alarm or fault, and
    // masked it would leave the door nothing to hold.
    for list in LISTS {
        crate::common::assert_list_element_refused(
            write(&mut door, list, states(&[S::TAMPER, S::NORMAL])),
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
            2,
            &format!("NORMAL in {list:?}"),
        );
        assert_eq!(read(&door, list), states(&[]), "{list:?}");
    }

    // The setters check the same way and keep the list on a refusal.
    door.set_alarm_values([S::FORCED_OPEN]).unwrap();
    door.set_fault_values([S::DOOR_FAULT]).unwrap();
    door.set_masked_alarm_values([S::TAMPER]).unwrap();
    assert_out_of_range(door.set_alarm_values([S::from_raw(9)]));
    assert_out_of_range(door.set_alarm_values([S::NORMAL]));
    assert_out_of_range(door.set_fault_values([S::ALARM, S::from_raw(255)]));
    assert_out_of_range(door.set_fault_values([S::NORMAL]));
    assert_out_of_range(door.set_masked_alarm_values([S::NORMAL]));
    for (list, expected) in [
        (P::ALARM_VALUES, S::FORCED_OPEN),
        (P::FAULT_VALUES, S::DOOR_FAULT),
        (P::MASKED_ALARM_VALUES, S::TAMPER),
    ] {
        assert_eq!(read(&door, list), states(&[expected]), "{list:?}");
    }
}

#[test]
fn access_door_alarm_state_outside_the_lists_is_refused() {
    let mut door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    // A new door's lists are empty, so it can only be NORMAL.
    assert_out_of_range(door.set_door_alarm_state(S::FORCED_OPEN));
    door.set_door_alarm_state(S::NORMAL).unwrap();

    door.set_alarm_values([S::FORCED_OPEN]).unwrap();
    door.set_fault_values([S::DOOR_FAULT]).unwrap();
    door.set_masked_alarm_values([S::TAMPER]).unwrap();
    door.set_door_alarm_state(S::FORCED_OPEN).unwrap();
    // Neither a state in no list nor a masked one is taken, and the state
    // held stays.
    for refused in [S::DOOR_OPEN_TOO_LONG, S::TAMPER, S::from_raw(9)] {
        assert_out_of_range(door.set_door_alarm_state(refused));
        assert_eq!(alarm_state(&door), enumerated(S::FORCED_OPEN.to_raw()));
    }
    door.set_door_alarm_state(S::DOOR_FAULT).unwrap();
    door.set_door_alarm_state(S::NORMAL).unwrap();

    // A simulated value meets the same check.
    set_out_of_service(&mut door, true);
    for refused in [S::DOOR_OPEN_TOO_LONG, S::TAMPER] {
        assert_out_of_range(write(
            &mut door,
            P::DOOR_ALARM_STATE,
            enumerated(refused.to_raw()),
        ));
        assert_eq!(alarm_state(&door), enumerated(S::NORMAL.to_raw()));
    }
    for taken in [S::FORCED_OPEN, S::DOOR_FAULT, S::NORMAL] {
        write(&mut door, P::DOOR_ALARM_STATE, enumerated(taken.to_raw())).unwrap();
        assert_eq!(alarm_state(&door), enumerated(taken.to_raw()));
    }
    // So does the application's report, which waits for the return to
    // service.
    assert_out_of_range(door.set_door_alarm_state(S::TAMPER));
    door.set_door_alarm_state(S::FORCED_OPEN).unwrap();
    assert_eq!(alarm_state(&door), enumerated(S::NORMAL.to_raw()));
    set_out_of_service(&mut door, false);
    assert_eq!(alarm_state(&door), enumerated(S::FORCED_OPEN.to_raw()));
}

#[test]
fn access_door_masking_the_current_state_returns_it_to_normal() {
    let mut door = alarming_door(0);
    door.set_door_alarm_state(S::FORCED_OPEN).unwrap();
    // Masking another state leaves the door as it is.
    write(&mut door, P::MASKED_ALARM_VALUES, states(&[S::TAMPER])).unwrap();
    assert_eq!(alarm_state(&door), enumerated(S::FORCED_OPEN.to_raw()));
    // Masking the state the door is in returns it to NORMAL at once, and
    // the application can't report it again while it is masked.
    write(
        &mut door,
        P::MASKED_ALARM_VALUES,
        states(&[S::TAMPER, S::FORCED_OPEN]),
    )
    .unwrap();
    assert_eq!(alarm_state(&door), enumerated(S::NORMAL.to_raw()));
    assert_out_of_range(door.set_door_alarm_state(S::FORCED_OPEN));
    // Unmasked, it can.
    door.set_masked_alarm_values([]).unwrap();
    door.set_door_alarm_state(S::FORCED_OPEN).unwrap();

    // Out of service the device's own state put aside is masked too, so
    // the return to service serves NORMAL rather than a masked state.
    set_out_of_service(&mut door, true);
    write(
        &mut door,
        P::DOOR_ALARM_STATE,
        enumerated(S::DOOR_OPEN_TOO_LONG.to_raw()),
    )
    .unwrap();
    door.set_masked_alarm_values([S::FORCED_OPEN]).unwrap();
    assert_eq!(
        alarm_state(&door),
        enumerated(S::DOOR_OPEN_TOO_LONG.to_raw())
    );
    set_out_of_service(&mut door, false);
    assert_eq!(alarm_state(&door), enumerated(S::NORMAL.to_raw()));

    // A state taken out of Alarm_Values is no alarm any more, so the door
    // returns to NORMAL as well.
    door.set_door_alarm_state(S::DOOR_OPEN_TOO_LONG).unwrap();
    write(&mut door, P::ALARM_VALUES, states(&[S::FORCED_OPEN])).unwrap();
    assert_eq!(alarm_state(&door), enumerated(S::NORMAL.to_raw()));
}

#[test]
fn access_door_goes_offnormal_after_time_delay_and_back_to_normal() {
    let mut door = alarming_door(2);
    // The application's door logic finds the door held open too long.
    door.set_door_alarm_state(S::DOOR_OPEN_TOO_LONG).unwrap();
    assert_eq!(door.evaluate_intrinsic_reporting(), None);
    assert_eq!(door.tick_intrinsic_reporting(), None);
    let offnormal = door.tick_intrinsic_reporting().unwrap();
    assert_eq!(
        offnormal,
        TransitionOutcome {
            change: change(EventState::NORMAL, EventState::OFFNORMAL),
            event_type: EventType::CHANGE_OF_STATE,
            distribute: true,
        }
    );
    commit_test_proposal(&mut door, offnormal);
    assert_eq!(
        read(&door, P::EVENT_STATE),
        enumerated(EventState::OFFNORMAL.to_raw())
    );
    assert_eq!(
        read(&door, P::STATUS_FLAGS),
        status_flags(StatusFlags::IN_ALARM)
    );
    // Another alarm value keeps OFFNORMAL without a new transition.
    door.set_door_alarm_state(S::FORCED_OPEN).unwrap();
    assert_eq!(door.evaluate_intrinsic_reporting(), None);
    assert_eq!(door.tick_intrinsic_reporting(), None);

    door.set_door_alarm_state(S::NORMAL).unwrap();
    assert_eq!(door.evaluate_intrinsic_reporting(), None);
    assert_eq!(door.tick_intrinsic_reporting(), None);
    let normal = door.tick_intrinsic_reporting().unwrap();
    assert_eq!(
        normal.change,
        change(EventState::OFFNORMAL, EventState::NORMAL)
    );
    commit_test_proposal(&mut door, normal);
    assert_eq!(
        read(&door, P::STATUS_FLAGS),
        status_flags(StatusFlags::empty())
    );
}

#[test]
fn access_door_fault_values_report_multi_state_fault() {
    let mut door = alarming_door(5);
    door.set_door_alarm_state(S::DOOR_FAULT).unwrap();
    assert_eq!(
        read(&door, P::RELIABILITY),
        enumerated(Reliability::MULTI_STATE_FAULT.to_raw())
    );
    assert_eq!(
        read(&door, P::STATUS_FLAGS),
        status_flags(StatusFlags::FAULT)
    );
    // A fault waits for no Time_Delay.
    let fault = door.evaluate_intrinsic_reporting().unwrap();
    assert_eq!(fault.change, change(EventState::NORMAL, EventState::FAULT));
    assert_eq!(fault.event_type, EventType::CHANGE_OF_RELIABILITY);
    commit_test_proposal(&mut door, fault);
    assert_eq!(
        read(&door, P::STATUS_FLAGS),
        status_flags(StatusFlags::IN_ALARM | StatusFlags::FAULT)
    );

    door.set_door_alarm_state(S::NORMAL).unwrap();
    assert_eq!(
        read(&door, P::RELIABILITY),
        enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
    );
    let recovered = door.evaluate_intrinsic_reporting().unwrap();
    assert_eq!(
        recovered.change,
        change(EventState::FAULT, EventState::NORMAL)
    );
    commit_test_proposal(&mut door, recovered);

    // A fault value taken out of the list clears the fault with it.
    door.set_door_alarm_state(S::DOOR_FAULT).unwrap();
    door.set_fault_values([]).unwrap();
    assert_eq!(alarm_state(&door), enumerated(S::NORMAL.to_raw()));
    assert_eq!(
        read(&door, P::RELIABILITY),
        enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
    );
}

#[test]
fn access_door_simulated_reliability_holds_until_the_return_to_service() {
    let mut door = alarming_door(0);
    let unreliable = enumerated(Reliability::UNRELIABLE_OTHER.to_raw());
    // In service the fault check owns Reliability.
    assert!(matches!(
        write(&mut door, P::RELIABILITY, unreliable.clone()),
        Err(Error::Protocol { code, .. })
            if code == ErrorCode::WRITE_ACCESS_DENIED
    ));
    set_out_of_service(&mut door, true);
    write(&mut door, P::RELIABILITY, unreliable.clone()).unwrap();
    assert_eq!(read(&door, P::RELIABILITY), unreliable);
    // A simulated Reliability takes precedence over the fault check.
    write(
        &mut door,
        P::DOOR_ALARM_STATE,
        enumerated(S::DOOR_FAULT.to_raw()),
    )
    .unwrap();
    assert_eq!(read(&door, P::RELIABILITY), unreliable);
    let fault = door.evaluate_intrinsic_reporting().unwrap();
    assert_eq!(fault.change, change(EventState::NORMAL, EventState::FAULT));
    commit_test_proposal(&mut door, fault);
    for (value, code) in [
        (enumerated(65_536), ErrorCode::VALUE_OUT_OF_RANGE),
        (PropertyValue::Unsigned(0), ErrorCode::INVALID_DATA_TYPE),
    ] {
        assert!(matches!(
            write(&mut door, P::RELIABILITY, value),
            Err(Error::Protocol { code: actual, .. }) if actual == code
        ));
    }

    // The return to service drops the simulation: the device's NORMAL door
    // has no fault.
    set_out_of_service(&mut door, false);
    assert_eq!(
        read(&door, P::RELIABILITY),
        enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
    );
    assert_eq!(
        door.evaluate_intrinsic_reporting()
            .map(|outcome| outcome.change),
        Some(change(EventState::FAULT, EventState::NORMAL))
    );
}
