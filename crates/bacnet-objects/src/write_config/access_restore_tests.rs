//! What taking saved event algorithm parameters does to an Access Door and
//! an Access Zone as they are built (#2331). A saved value goes where a
//! write of it puts it. A door then does what it does after a write of one
//! of its lists: a Door_Alarm_State the lists no longer admit goes back to
//! NORMAL, once, with every saved list in place. Nothing is evaluated, so
//! the door or zone is left as a write of the same values leaves it and
//! raises only what that write would.

use bacnet_types::enums::{
    AccessZoneOccupancyState as Z, DoorAlarmState as S, EventState, PropertyIdentifier as P,
    Reliability,
};
use bacnet_types::primitives::PropertyValue;

use super::access_parameters_tests::{door, list_row, states, zone};
use super::parameters_tests::{evaluated, holding, read, write};
use super::reporting_tests::persistence;
use super::{AlgorithmWriteConfig, ReportingWriteConfig, WriteConfigSnapshot};
use crate::access_control::{AccessDoorObject, AccessZoneObject};
use crate::traits::BACnetObject;
use crate::write_config::test_storage::MemoryConfig;

/// Everything a door serves that its lists or its detector can move.
const DOOR_SERVED: [P; 10] = [
    P::DOOR_ALARM_STATE,
    P::ALARM_VALUES,
    P::FAULT_VALUES,
    P::MASKED_ALARM_VALUES,
    P::RELIABILITY,
    P::EVENT_STATE,
    P::STATUS_FLAGS,
    P::SECURED_STATUS,
    P::ACKED_TRANSITIONS,
    P::TIME_DELAY,
];

/// The same of a zone.
const ZONE_SERVED: [P; 8] = [
    P::OCCUPANCY_STATE,
    P::OCCUPANCY_COUNT,
    P::ALARM_VALUES,
    P::RELIABILITY,
    P::EVENT_STATE,
    P::STATUS_FLAGS,
    P::ACKED_TRANSITIONS,
    P::TIME_DELAY,
];

fn served(object: &dyn BACnetObject, properties: &[P]) -> Vec<(P, PropertyValue)> {
    properties
        .iter()
        .map(|property| (*property, read(object, *property)))
        .collect()
}

fn alarm_state(state: S) -> PropertyValue {
    PropertyValue::Enumerated(state.to_raw())
}

/// The door as its application builds it, reporting `state`.
fn door_in(state: S) -> AccessDoorObject {
    let mut door = door(None).unwrap();
    door.set_door_alarm_state(state).unwrap();
    door
}

fn restored(door: AccessDoorObject, saved: AlgorithmWriteConfig) -> AccessDoorObject {
    door.with_reporting_persistence(persistence(&holding(saved)))
        .unwrap()
}

/// One saved parameter, the state the door is in as it takes it, and the
/// state it is left in.
struct Case {
    held: S,
    property: P,
    written: PropertyValue,
    saved: AlgorithmWriteConfig,
    left: S,
    proposes: Option<EventState>,
}

fn list_case(held: S, property: P, list: &[S], left: S, proposes: Option<EventState>) -> Case {
    let raw: Vec<u32> = list.iter().map(|state| state.to_raw()).collect();
    Case {
        held,
        property,
        written: states(&raw),
        saved: list_row(property, raw),
        left,
        proposes,
    }
}

/// A door that takes one saved parameter is left as a door that takes a
/// write of it: the same Door_Alarm_State, lists, Reliability, Event_State
/// and flags, and the same proposal at its next evaluation. A state the
/// saved list no longer admits is back at NORMAL; one it still admits
/// stays, and alarms or faults as the list says; a saved delay moves
/// nothing.
#[test]
fn a_door_restoring_a_parameter_is_left_as_a_write_of_it_leaves_one() {
    let offnormal = Some(EventState::OFFNORMAL);
    let fault = Some(EventState::FAULT);
    let cases = [
        // No alarm value any more.
        list_case(
            S::FORCED_OPEN,
            P::ALARM_VALUES,
            &[S::TAMPER],
            S::NORMAL,
            None,
        ),
        list_case(S::FORCED_OPEN, P::ALARM_VALUES, &[], S::NORMAL, None),
        // Still one.
        list_case(
            S::FORCED_OPEN,
            P::ALARM_VALUES,
            &[S::TAMPER, S::FORCED_OPEN],
            S::FORCED_OPEN,
            offnormal,
        ),
        // Masked.
        list_case(
            S::FORCED_OPEN,
            P::MASKED_ALARM_VALUES,
            &[S::FORCED_OPEN],
            S::NORMAL,
            None,
        ),
        list_case(
            S::FORCED_OPEN,
            P::MASKED_ALARM_VALUES,
            &[S::TAMPER],
            S::FORCED_OPEN,
            offnormal,
        ),
        // No fault value any more.
        list_case(
            S::DOOR_FAULT,
            P::FAULT_VALUES,
            &[S::LOCK_DOWN],
            S::NORMAL,
            None,
        ),
        list_case(S::DOOR_FAULT, P::FAULT_VALUES, &[], S::NORMAL, None),
        // An alarm value that is a fault value too now.
        list_case(
            S::FORCED_OPEN,
            P::FAULT_VALUES,
            &[S::FORCED_OPEN],
            S::FORCED_OPEN,
            fault,
        ),
        // A delay: nothing moves, and the alarm waits for it.
        Case {
            held: S::FORCED_OPEN,
            property: P::TIME_DELAY,
            written: PropertyValue::Unsigned(5),
            saved: AlgorithmWriteConfig {
                time_delay: Some(5),
                ..Default::default()
            },
            left: S::FORCED_OPEN,
            proposes: None,
        },
    ];
    for case in cases {
        let what = format!("{:?} over {:?}", case.property, case.held);
        let mut written = door_in(case.held);
        write(&mut written, case.property, case.written.clone());
        let mut restored = restored(door_in(case.held), case.saved);
        assert_eq!(
            read(&restored, P::DOOR_ALARM_STATE),
            alarm_state(case.left),
            "{what}"
        );
        assert_eq!(read(&restored, case.property), case.written, "{what}");
        let normal = PropertyValue::Enumerated(EventState::NORMAL.to_raw());
        assert_eq!(read(&restored, P::EVENT_STATE), normal, "{what}");
        assert_eq!(
            served(&restored, &DOOR_SERVED),
            served(&written, &DOOR_SERVED),
            "{what}"
        );
        assert_eq!(evaluated(&mut restored), case.proposes, "{what}");
        assert_eq!(evaluated(&mut written), case.proposes, "{what}");
    }
}

/// The state is checked once, against every saved list together, as it
/// was checked when the application reported it before the restart: a state
/// that left Alarm_Values and joined Fault_Values in the same file stays,
/// and one Alarm_Values still lists and Masked_Alarm_Values now masks goes.
#[test]
fn a_door_settles_once_with_every_saved_list_in_place() {
    let forced_open = vec![S::FORCED_OPEN.to_raw()];
    let moved = AlgorithmWriteConfig {
        alarm_values: Some(vec![]),
        fault_values: Some(forced_open.clone()),
        ..Default::default()
    };
    let mut door = restored(door_in(S::FORCED_OPEN), moved);
    assert_eq!(
        read(&door, P::DOOR_ALARM_STATE),
        alarm_state(S::FORCED_OPEN)
    );
    let fault = PropertyValue::Enumerated(Reliability::MULTI_STATE_FAULT.to_raw());
    assert_eq!(read(&door, P::RELIABILITY), fault);
    assert_eq!(evaluated(&mut door), Some(EventState::FAULT));

    let masked = AlgorithmWriteConfig {
        alarm_values: Some(vec![S::TAMPER.to_raw(), S::FORCED_OPEN.to_raw()]),
        masked_alarm_values: Some(forced_open),
        ..Default::default()
    };
    let mut door = restored(door_in(S::FORCED_OPEN), masked);
    assert_eq!(read(&door, P::DOOR_ALARM_STATE), alarm_state(S::NORMAL));
    assert_eq!(evaluated(&mut door), None);
}

/// The door's own state put aside while Out_Of_Service is TRUE settles
/// with the one served, as after a write: neither comes back in a state
/// the saved lists don't admit.
#[test]
fn a_door_out_of_service_settles_both_states() {
    let mut door = door_in(S::FORCED_OPEN);
    write(&mut door, P::OUT_OF_SERVICE, PropertyValue::Boolean(true));
    assert_eq!(
        read(&door, P::DOOR_ALARM_STATE),
        alarm_state(S::FORCED_OPEN)
    );
    let saved = list_row(P::ALARM_VALUES, vec![S::TAMPER.to_raw()]);
    let mut door = restored(door, saved);
    assert_eq!(read(&door, P::DOOR_ALARM_STATE), alarm_state(S::NORMAL));
    write(&mut door, P::OUT_OF_SERVICE, PropertyValue::Boolean(false));
    assert_eq!(read(&door, P::DOOR_ALARM_STATE), alarm_state(S::NORMAL));
}

/// Storage with no saved parameter, or with reporting rows alone, leaves a
/// door's state and lists as the application set them.
#[test]
fn a_door_with_no_saved_parameter_keeps_its_state() {
    let rows = ReportingWriteConfig {
        notification_class: Some(9),
        ..Default::default()
    };
    for storage in [
        MemoryConfig::new(),
        MemoryConfig::holding(WriteConfigSnapshot::Reporting(rows)),
    ] {
        let plain = door_in(S::FORCED_OPEN);
        let mut kept = door_in(S::FORCED_OPEN)
            .with_reporting_persistence(persistence(&storage))
            .unwrap();
        assert_eq!(
            served(&kept, &DOOR_SERVED[..8]),
            served(&plain, &DOOR_SERVED[..8])
        );
        assert_eq!(evaluated(&mut kept), Some(EventState::OFFNORMAL));
    }
}

/// The zone as its application builds it, with `count` inside.
fn zone_with(count: u64) -> AccessZoneObject {
    let mut zone = zone(None).unwrap();
    zone.set_occupancy_count(count);
    zone
}

/// A zone that takes a saved Alarm_Values or delay is left as a zone that
/// takes a write of it: the list or the delay is in its detector and
/// nothing else has moved. Its count and Occupancy_State are as the
/// application left them, and it proposes what the saved list makes of
/// them.
#[test]
fn a_zone_restoring_a_parameter_is_left_as_a_write_of_it_leaves_one() {
    let at_limit = vec![Z::AT_UPPER_LIMIT.to_raw()];
    let cases = [
        // Above the limit, which the saved list no longer names.
        (7, P::ALARM_VALUES, states(&at_limit), None),
        // At the limit, which only the saved list names.
        (
            5,
            P::ALARM_VALUES,
            states(&at_limit),
            Some(EventState::OFFNORMAL),
        ),
        (7, P::TIME_DELAY, PropertyValue::Unsigned(5), None),
    ];
    for (count, property, value, proposes) in cases {
        let saved = match property {
            P::ALARM_VALUES => list_row(property, at_limit.clone()),
            _ => AlgorithmWriteConfig {
                time_delay: Some(5),
                ..Default::default()
            },
        };
        let mut written = zone_with(count);
        write(&mut written, property, value.clone());
        let mut restored = zone_with(count)
            .with_reporting_persistence(persistence(&holding(saved)))
            .unwrap();
        assert_eq!(read(&restored, property), value, "{property:?}");
        assert_eq!(
            read(&restored, P::OCCUPANCY_COUNT),
            PropertyValue::Unsigned(count)
        );
        assert_eq!(
            served(&restored, &ZONE_SERVED),
            served(&written, &ZONE_SERVED),
            "{property:?} at {count}"
        );
        assert_eq!(
            evaluated(&mut restored),
            proposes,
            "{property:?} at {count}"
        );
        assert_eq!(evaluated(&mut written), proposes, "{property:?} at {count}");
    }
}
