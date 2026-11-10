//! The event algorithm parameters a write sets on an Access Door or an
//! Access Zone, kept with its reporting settings (#2331): the two time
//! delays and Alarm_Values on both, and a door's Fault_Values and
//! Masked_Alarm_Values. Each comes back on a rebuild and the detector uses
//! it, a write whose save fails is refused, a refused write saves nothing,
//! and a stored list is validated as a written one. The setters, the
//! clear, what a type doesn't keep and a request's staged writes are in
//! `access_parameter_rules_tests`; what a restore does to a door's
//! Door_Alarm_State is in `access_restore_tests`.

use alloc::sync::Arc;

use bacnet_types::enums::{
    AccessZoneOccupancyState as Z, DoorAlarmState as S, ErrorClass, ErrorCode, EventState,
    ObjectType, PropertyIdentifier as P, Reliability,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::parameters_tests::{evaluated, holding, read, saved, ticked, write};
use super::reporting_tests::{persistence, refused_with};
use super::state_parameters_tests::commit;
use super::test_storage::MemoryConfig;
use super::{AlgorithmWriteConfig, ReportingWriteConfig};
use crate::access_control::{AccessDoorObject, AccessZoneObject};
use crate::traits::BACnetObject;

pub(super) const KINDS: [ObjectType; 2] = [ObjectType::ACCESS_DOOR, ObjectType::ACCESS_ZONE];

/// The lists each type keeps.
pub(super) fn lists(kind: ObjectType) -> &'static [P] {
    match kind {
        ObjectType::ACCESS_DOOR => &[P::ALARM_VALUES, P::FAULT_VALUES, P::MASKED_ALARM_VALUES],
        _ => &[P::ALARM_VALUES],
    }
}

/// Door 1 as an application builds it at each start: it alarms on
/// FORCED_OPEN, faults on DOOR_FAULT and masks nothing. Then it attaches
/// `storage`, if any.
pub(super) fn door(storage: Option<&Arc<MemoryConfig>>) -> Result<AccessDoorObject, Error> {
    let mut door = AccessDoorObject::new(1, "Door")?;
    door.set_alarm_values([S::FORCED_OPEN])?;
    door.set_fault_values([S::DOOR_FAULT])?;
    match storage {
        Some(storage) => door.with_reporting_persistence(persistence(storage)),
        None => Ok(door),
    }
}

/// Zone 1 as an application builds it at each start: an upper limit of
/// five, three inside, and an alarm above the limit. Then it attaches
/// `storage`, if any.
pub(super) fn zone(storage: Option<&Arc<MemoryConfig>>) -> Result<AccessZoneObject, Error> {
    let mut zone = AccessZoneObject::new(1, "Zone")?;
    zone.set_occupancy_limits(0, 5)?;
    zone.set_occupancy_count(3);
    zone.set_alarm_values([Z::ABOVE_UPPER_LIMIT])?;
    match storage {
        Some(storage) => zone.with_reporting_persistence(persistence(storage)),
        None => Ok(zone),
    }
}

/// The state an object is put in: none of its alarm values, the one the
/// application configures, or the one [`covered`]'s Alarm_Values write
/// names.
#[derive(Clone, Copy, Debug)]
pub(super) enum Held {
    Quiet,
    Configured,
    Written,
}

/// What these tests do to a door and a zone alike.
pub(super) trait Access: BACnetObject {
    /// Put the object in `state`, as its application does; a door refuses
    /// a state its lists don't admit.
    fn hold(&mut self, state: Held) -> Result<(), Error>;
    fn clear(&mut self) -> Result<(), Error>;
    fn written(&self) -> ReportingWriteConfig;
}

impl Access for AccessDoorObject {
    fn hold(&mut self, state: Held) -> Result<(), Error> {
        self.set_door_alarm_state(match state {
            Held::Quiet => S::NORMAL,
            Held::Configured => S::FORCED_OPEN,
            Held::Written => S::TAMPER,
        })
    }

    fn clear(&mut self) -> Result<(), Error> {
        self.clear_written_reporting()
    }

    fn written(&self) -> ReportingWriteConfig {
        self.written_reporting()
    }
}

impl Access for AccessZoneObject {
    fn hold(&mut self, state: Held) -> Result<(), Error> {
        self.set_occupancy_count(match state {
            Held::Quiet => 3,
            Held::Configured => 7,
            Held::Written => 5,
        });
        Ok(())
    }

    fn clear(&mut self) -> Result<(), Error> {
        self.clear_written_reporting()
    }

    fn written(&self) -> ReportingWriteConfig {
        self.written_reporting()
    }
}

fn try_access(
    kind: ObjectType,
    storage: Option<&Arc<MemoryConfig>>,
) -> Result<Box<dyn Access>, Error> {
    Ok(match kind {
        ObjectType::ACCESS_DOOR => Box::new(door(storage)?),
        _ => Box::new(zone(storage)?),
    })
}

pub(super) fn access(kind: ObjectType, storage: Option<&Arc<MemoryConfig>>) -> Box<dyn Access> {
    try_access(kind, storage).unwrap()
}

/// Object 1 of `kind` as [`door`] or [`zone`] builds it.
pub(super) fn try_open(
    kind: ObjectType,
    storage: Option<&Arc<MemoryConfig>>,
) -> Result<Box<dyn BACnetObject>, Error> {
    Ok(match kind {
        ObjectType::ACCESS_DOOR => Box::new(door(storage)?),
        _ => Box::new(zone(storage)?),
    })
}

pub(super) fn open(kind: ObjectType, storage: Option<&Arc<MemoryConfig>>) -> Box<dyn BACnetObject> {
    try_open(kind, storage).unwrap()
}

/// A list of states as a write carries it.
pub(super) fn states(values: &[u32]) -> PropertyValue {
    PropertyValue::List(
        values
            .iter()
            .copied()
            .map(PropertyValue::Enumerated)
            .collect(),
    )
}

/// Storage's row for `property`, a list, holding `values`.
pub(super) fn list_row(property: P, values: Vec<u32>) -> AlgorithmWriteConfig {
    let mut row = AlgorithmWriteConfig::default();
    match property {
        P::ALARM_VALUES => row.alarm_values = Some(values),
        P::FAULT_VALUES => row.fault_values = Some(values),
        P::MASKED_ALARM_VALUES => row.masked_alarm_values = Some(values),
        other => panic!("{other:?} is no list"),
    }
    row
}

/// The list each list property's write in [`covered`] carries: none is what
/// the application configures.
pub(super) fn written_list(kind: ObjectType, property: P) -> Vec<u32> {
    match (kind, property) {
        (ObjectType::ACCESS_DOOR, P::ALARM_VALUES) => vec![S::TAMPER.to_raw(), 300],
        (ObjectType::ACCESS_DOOR, P::FAULT_VALUES) => vec![S::LOCK_DOWN.to_raw()],
        (ObjectType::ACCESS_DOOR, _) => vec![S::FREE_ACCESS.to_raw(), 65_535],
        _ => vec![Z::AT_UPPER_LIMIT.to_raw(), 100],
    }
}

/// Each parameter `kind` keeps: a write of it, and what storage keeps of
/// that write alone.
pub(super) fn covered(kind: ObjectType) -> Vec<(P, PropertyValue, AlgorithmWriteConfig)> {
    let none = AlgorithmWriteConfig::default;
    let mut rows = vec![
        (
            P::TIME_DELAY,
            PropertyValue::Unsigned(7),
            AlgorithmWriteConfig {
                time_delay: Some(7),
                ..none()
            },
        ),
        (
            P::TIME_DELAY_NORMAL,
            PropertyValue::Unsigned(3),
            AlgorithmWriteConfig {
                time_delay_normal: Some(3),
                ..none()
            },
        ),
    ];
    for &property in lists(kind) {
        let list = written_list(kind, property);
        rows.push((property, states(&list), list_row(property, list)));
    }
    rows
}

/// Each parameter written on each type saves once, before it answers,
/// storage keeps that parameter and no other, and an object rebuilt on the
/// storage serves it over what the application configures.
#[test]
fn each_written_parameter_survives_a_rebuild_and_only_it_is_kept() {
    for kind in KINDS {
        let rows = covered(kind);
        assert_eq!(rows.len(), 2 + lists(kind).len(), "{kind}");
        let configured = open(kind, None);
        for (property, value, kept) in rows {
            let storage = MemoryConfig::new();
            let mut object = open(kind, Some(&storage));
            let before = read(object.as_ref(), property);
            write(object.as_mut(), property, value.clone());
            assert_eq!(storage.saves(), 1, "{kind} {property:?}");
            let written = read(object.as_ref(), property);
            assert_eq!(written, value, "{kind} {property:?}");
            assert_ne!(written, before, "{kind} {property:?} changed nothing");
            let expected = ReportingWriteConfig {
                algorithm: kept,
                ..Default::default()
            };
            assert_eq!(saved(&storage), expected, "{kind} {property:?}");
            drop(object);
            let rebuilt = open(kind, Some(&storage));
            assert_eq!(read(rebuilt.as_ref(), property), written, "{kind}");
            // A parameter no write set is the application's, as before.
            for (other, _, _) in covered(kind) {
                // An unwritten Time_Delay_Normal reads as Time_Delay.
                let follows = property == P::TIME_DELAY && other == P::TIME_DELAY_NORMAL;
                if other != property && !follows {
                    assert_eq!(
                        read(rebuilt.as_ref(), other),
                        read(configured.as_ref(), other),
                        "{kind} {other:?} with {property:?} written"
                    );
                }
            }
        }
    }
}

/// A written list may be empty. It is kept as an empty list, not as
/// nothing written, and comes back over the configured one.
#[test]
fn an_empty_list_is_kept_as_one() {
    for kind in KINDS {
        for &property in lists(kind) {
            let storage = MemoryConfig::new();
            let mut object = open(kind, Some(&storage));
            // A door masks nothing as built, so its list is written first.
            write(object.as_mut(), property, states(&[1]));
            write(object.as_mut(), property, states(&[]));
            assert_eq!(saved(&storage).algorithm, list_row(property, vec![]));
            drop(object);
            let rebuilt = open(kind, Some(&storage));
            assert_eq!(read(rebuilt.as_ref(), property), states(&[]), "{kind}");
        }
    }
}

/// A rebuilt object's detector uses the restored parameters, on hand-set
/// time. The state only the restored Alarm_Values names raises OFFNORMAL
/// and the one only the configured list names raises nothing; the restored
/// Time_Delay of two seconds runs first, counted by hand; and the return to
/// NORMAL waits the restored Time_Delay_Normal of one second, not
/// Time_Delay's two. An object without the storage alarms at once on its
/// configured state.
#[test]
fn a_rebuilt_detector_uses_the_restored_parameters() {
    for kind in KINDS {
        let storage = MemoryConfig::new();
        let mut object = open(kind, Some(&storage));
        let list = written_list(kind, P::ALARM_VALUES);
        write(object.as_mut(), P::ALARM_VALUES, states(&list));
        write(object.as_mut(), P::TIME_DELAY, PropertyValue::Unsigned(2));
        let one = PropertyValue::Unsigned(1);
        write(object.as_mut(), P::TIME_DELAY_NORMAL, one);
        drop(object);

        let mut rebuilt = access(kind, Some(&storage));
        // What the configured list alone would alarm on. A door doesn't
        // even hold it: its restored lists don't admit the state.
        let held = rebuilt.hold(Held::Configured);
        assert_eq!(held.is_ok(), kind == ObjectType::ACCESS_ZONE, "{kind}");
        assert_eq!(evaluated(rebuilt.as_mut()), None, "{kind}");
        for _ in 0..3 {
            assert_eq!(ticked(rebuilt.as_mut()), None, "{kind}");
        }
        rebuilt.hold(Held::Written).unwrap();
        assert_eq!(evaluated(rebuilt.as_mut()), None, "{kind}");
        assert_eq!(ticked(rebuilt.as_mut()), None, "{kind}");
        assert_eq!(
            ticked(rebuilt.as_mut()),
            Some(EventState::OFFNORMAL),
            "{kind}"
        );
        commit(rebuilt.as_mut(), EventState::NORMAL, EventState::OFFNORMAL);
        rebuilt.hold(Held::Quiet).unwrap();
        assert_eq!(evaluated(rebuilt.as_mut()), None, "{kind}");
        assert_eq!(ticked(rebuilt.as_mut()), Some(EventState::NORMAL), "{kind}");

        // Without the storage: the configured list, and no delay.
        let mut plain = access(kind, None);
        if plain.hold(Held::Written).is_ok() {
            assert_eq!(evaluated(plain.as_mut()), None, "{kind}");
            assert_eq!(ticked(plain.as_mut()), None, "{kind}");
        }
        plain.hold(Held::Configured).unwrap();
        assert_eq!(
            evaluated(plain.as_mut()),
            Some(EventState::OFFNORMAL),
            "{kind}"
        );
    }
}

/// A rebuilt door's fault check and its mask use the restored lists: the
/// restored fault value makes Reliability MULTI_STATE_FAULT and proposes
/// FAULT, the configured one is no state the door holds any more, and a
/// restored masked state is refused although Alarm_Values lists it.
#[test]
fn a_rebuilt_door_faults_and_masks_by_the_restored_lists() {
    let storage = MemoryConfig::new();
    let mut written = door(Some(&storage)).unwrap();
    let lock_down = states(&[S::LOCK_DOWN.to_raw()]);
    write(&mut written, P::FAULT_VALUES, lock_down);
    let forced_open = states(&[S::FORCED_OPEN.to_raw()]);
    write(&mut written, P::MASKED_ALARM_VALUES, forced_open);
    drop(written);

    let mut rebuilt = door(Some(&storage)).unwrap();
    // Configured as a fault value, restored as none.
    assert!(rebuilt.set_door_alarm_state(S::DOOR_FAULT).is_err());
    // Configured as an alarm value still, and restored as masked.
    assert!(rebuilt.set_door_alarm_state(S::FORCED_OPEN).is_err());
    let no_fault = PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw());
    assert_eq!(read(&rebuilt, P::RELIABILITY), no_fault);
    assert_eq!(evaluated(&mut rebuilt), None);
    rebuilt.set_door_alarm_state(S::LOCK_DOWN).unwrap();
    let fault = PropertyValue::Enumerated(Reliability::MULTI_STATE_FAULT.to_raw());
    assert_eq!(read(&rebuilt, P::RELIABILITY), fault);
    assert_eq!(evaluated(&mut rebuilt), Some(EventState::FAULT));

    // As the application alone configures it.
    let mut plain = door(None).unwrap();
    assert!(plain.set_door_alarm_state(S::LOCK_DOWN).is_err());
    plain.set_door_alarm_state(S::DOOR_FAULT).unwrap();
    assert_eq!(evaluated(&mut plain), Some(EventState::FAULT));
}

/// A parameter write whose save fails is refused with DEVICE /
/// OPERATIONAL_PROBLEM, and the old value stays: on the wire, in the
/// detector, in a door's lists and in storage.
#[test]
fn a_parameter_write_whose_save_fails_keeps_the_old_value() {
    for kind in KINDS {
        let storage = MemoryConfig::new();
        let mut object = access(kind, Some(&storage));
        write(
            object.as_mut(),
            P::NOTIFICATION_CLASS,
            PropertyValue::Unsigned(9),
        );
        let kept = saved(&storage);
        storage.fail(true);
        for (property, value, _) in covered(kind) {
            let before = read(object.as_ref(), property);
            let result = object.write_property(property, None, value, None);
            assert!(
                refused_with(&result, ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM),
                "{kind} {property:?}: {result:?}"
            );
            assert_eq!(read(object.as_ref(), property), before, "{kind}");
        }
        assert_eq!(saved(&storage), kept, "{kind}");
        assert_eq!(storage.saves(), 1, "{kind}");
        // The detector still has what was configured: the configured alarm
        // value and no delay, not the refused list.
        if object.hold(Held::Written).is_ok() {
            assert_eq!(evaluated(object.as_mut()), None, "{kind}");
            assert_eq!(ticked(object.as_mut()), None, "{kind}");
        }
        object.hold(Held::Configured).unwrap();
        assert_eq!(
            evaluated(object.as_mut()),
            Some(EventState::OFFNORMAL),
            "{kind}"
        );
    }
}

/// A state past the end of `kind`'s named states and short of its
/// proprietary ones.
pub(super) fn outside(kind: ObjectType) -> u32 {
    if kind == ObjectType::ACCESS_DOOR {
        9
    } else {
        7
    }
}

/// The writes of `kind`'s parameters that the object refuses.
fn refusals(kind: ObjectType) -> Vec<(P, Option<u32>, PropertyValue)> {
    let mut refusals = vec![
        (P::TIME_DELAY, None, PropertyValue::Unsigned(1 << 32)),
        (P::TIME_DELAY_NORMAL, None, PropertyValue::Real(1.0)),
        (P::TIME_DELAY, Some(1), PropertyValue::Unsigned(4)),
        (P::TIME_DELAY_NORMAL, None, PropertyValue::Null),
    ];
    let long: Vec<u32> = (0..=1024).map(|_| 1).collect();
    for &list in lists(kind) {
        refusals.extend([
            // NORMAL, which is zero in both enumerations.
            (list, None, states(&[0])),
            (list, None, states(&[1, 0])),
            (list, None, states(&[1, outside(kind)])),
            (list, None, states(&[65_536])),
            (list, None, states(&long)),
            (list, None, PropertyValue::Unsigned(1)),
            (
                list,
                None,
                PropertyValue::List(vec![PropertyValue::Unsigned(1)]),
            ),
            (list, Some(1), states(&[1])),
        ]);
    }
    refusals
}

/// A write the object refuses is refused as without storage and saves
/// nothing.
#[test]
fn a_refused_parameter_write_saves_nothing() {
    for kind in KINDS {
        let storage = MemoryConfig::new();
        let mut kept = open(kind, Some(&storage));
        let mut plain = open(kind, None);
        for (property, index, value) in refusals(kind) {
            let before = read(kept.as_ref(), property);
            let refusal = plain
                .write_property(property, index, value.clone(), None)
                .unwrap_err();
            let result = kept.write_property(property, index, value.clone(), None);
            assert_eq!(
                result.unwrap_err().to_string(),
                refusal.to_string(),
                "{kind} {property:?} {index:?}"
            );
            assert_eq!(read(kept.as_ref(), property), before);
        }
        assert_eq!(storage.saves(), 0, "{kind}");
        assert_eq!(storage.saved(), None, "{kind}");
    }
}

/// A stored list is taken or refused exactly as a write of it is, on each
/// list of each type: every state from 0 to 10, the edges of the two
/// proprietary ranges and the top of the Unsigned range, alone and after a
/// state the list takes, and a list at and past the cap. Nothing a write
/// refuses loads and nothing it takes is refused, so no check on these
/// lists is left out on load. NORMAL, which is zero, is refused both ways.
#[test]
fn a_stored_list_is_validated_as_a_write_of_it_is() {
    let mut candidates: Vec<u32> = (0..=10).collect();
    candidates.extend([63, 64, 65, 255, 256, 257, 65_535, 65_536, u32::MAX]);
    for kind in KINDS {
        for &property in lists(kind) {
            let mut cases: Vec<Vec<u32>> = vec![vec![]];
            for &state in &candidates {
                cases.push(vec![state]);
                cases.push(vec![1, state]);
            }
            cases.push((0..1024).map(|_| 2).collect());
            cases.push((0..1025).map(|_| 2).collect());
            let (mut taken, mut refused) = (0, 0);
            for list in cases {
                let value = states(&list);
                let written = open(kind, None).write_property(property, None, value.clone(), None);
                let built = try_open(kind, Some(&holding(list_row(property, list.clone()))));
                assert_eq!(
                    built.is_ok(),
                    written.is_ok(),
                    "{kind} {property:?} {list:?}: a write gives {written:?}"
                );
                match built {
                    Ok(object) => {
                        assert_eq!(read(object.as_ref(), property), value);
                        taken += 1;
                    }
                    Err(error) => {
                        assert!(matches!(error, Error::Encoding(_)), "{error:?}");
                        refused += 1;
                    }
                }
            }
            // A door's named states are 1 to 8 and a zone's 1 to 6; of the
            // rest, a door takes four and a zone seven. Each twice, then
            // the empty list and the full one.
            let named = if kind == ObjectType::ACCESS_DOOR {
                8
            } else {
                6
            };
            let proprietary = if kind == ObjectType::ACCESS_DOOR {
                3
            } else {
                6
            };
            assert_eq!(taken, 2 * (named + proprietary) + 2, "{kind} {property:?}");
            assert_eq!(refused, 2 * candidates.len() + 3 - taken, "{kind}");
            let normal = holding(list_row(property, vec![0]));
            assert!(
                try_open(kind, Some(&normal)).is_err(),
                "{kind} {property:?}"
            );
        }
    }
}
