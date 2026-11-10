//! The event algorithm parameters a write sets on a binary or multi-state
//! object, kept with its reporting settings (#2331): the two time delays on
//! all six types, Alarm_Value on a Binary Input or Value, Alarm_Values on a
//! Multi-state Input or Value. Each comes back on a rebuild and the
//! detector uses it, a write whose save fails is refused, a refused write
//! saves nothing, a stored value is validated as a written one, and a type
//! keeps only its own.

use alloc::sync::Arc;

use bacnet_types::bitstring::LimitEnable;
use bacnet_types::enums::{
    BinaryPV, ErrorClass, ErrorCode, EventState, ObjectType, PropertyIdentifier as P,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, PropertyValue};

use super::parameters_tests::{evaluated, holding, read, saved, ticked, write};
use super::reporting_tests::{persistence, refused_with};
use super::test_storage::MemoryConfig;
use super::{AlgorithmWriteConfig, ReportingWriteConfig};
use crate::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use crate::event::{EventStateChange, EventTransition, EventTransitionCommit};
use crate::multistate::{MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject};
use crate::traits::BACnetObject;

pub(super) const KINDS: [ObjectType; 6] = [
    ObjectType::BINARY_INPUT,
    ObjectType::BINARY_OUTPUT,
    ObjectType::BINARY_VALUE,
    ObjectType::MULTI_STATE_INPUT,
    ObjectType::MULTI_STATE_OUTPUT,
    ObjectType::MULTI_STATE_VALUE,
];

/// The states the application gives a multi-state object at each start.
pub(super) const STATES: u32 = 5;

pub(super) fn is_binary(kind: ObjectType) -> bool {
    matches!(
        kind,
        ObjectType::BINARY_INPUT | ObjectType::BINARY_OUTPUT | ObjectType::BINARY_VALUE
    )
}

/// Whether `kind` runs CHANGE_OF_STATE on alarm values of its own: the
/// inputs and values. The outputs run COMMAND_FAILURE.
pub(super) fn has_alarm_values(kind: ObjectType) -> bool {
    !matches!(
        kind,
        ObjectType::BINARY_OUTPUT | ObjectType::MULTI_STATE_OUTPUT
    )
}

/// Object 1 of `kind` as an application builds it at each start: a
/// multi-state object has [`STATES`] states, and an input or value among
/// them alarms on state 2; a binary input or value alarms on ACTIVE, as it
/// is built; an output, which is built with event detection off, has it
/// turned on. Then it attaches `storage`, if any.
pub(super) fn try_open(
    kind: ObjectType,
    storage: Option<&Arc<MemoryConfig>>,
) -> Result<Box<dyn BACnetObject>, Error> {
    macro_rules! build {
        ($object:expr) => {{
            let object = $object?;
            let object = match storage {
                Some(storage) => object.with_reporting_persistence(persistence(storage))?,
                None => object,
            };
            Ok(Box::new(object))
        }};
    }
    match kind {
        ObjectType::BINARY_INPUT => build!(BinaryInputObject::new(1, "BI")),
        ObjectType::BINARY_OUTPUT => build!(BinaryOutputObject::new(1, "BO").and_then(detecting)),
        ObjectType::BINARY_VALUE => build!(BinaryValueObject::new(1, "BV")),
        ObjectType::MULTI_STATE_INPUT => {
            build!(
                MultiStateInputObject::new(1, "MSI", STATES).map(|mut object| {
                    object.set_alarm_values(vec![2]);
                    object
                })
            )
        }
        ObjectType::MULTI_STATE_OUTPUT => {
            build!(MultiStateOutputObject::new(1, "MSO", STATES).and_then(detecting))
        }
        _ => build!(
            MultiStateValueObject::new(1, "MSV", STATES).map(|mut object| {
                object.set_alarm_values(vec![2]);
                object
            })
        ),
    }
}

/// `object` with Event_Detection_Enable written TRUE.
fn detecting<T: BACnetObject>(mut object: T) -> Result<T, Error> {
    let enable = PropertyValue::Boolean(true);
    object.write_property(P::EVENT_DETECTION_ENABLE, None, enable, None)?;
    Ok(object)
}

pub(super) fn open(kind: ObjectType, storage: Option<&Arc<MemoryConfig>>) -> Box<dyn BACnetObject> {
    try_open(kind, storage).unwrap()
}

pub(super) fn states(values: &[u32]) -> PropertyValue {
    let values = values
        .iter()
        .map(|state| PropertyValue::Unsigned(u64::from(*state)));
    PropertyValue::List(values.collect())
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
    match kind {
        ObjectType::BINARY_INPUT | ObjectType::BINARY_VALUE => rows.push((
            P::ALARM_VALUE,
            PropertyValue::Enumerated(0),
            AlgorithmWriteConfig {
                alarm_value: Some(BinaryPV::INACTIVE),
                ..none()
            },
        )),
        ObjectType::MULTI_STATE_INPUT | ObjectType::MULTI_STATE_VALUE => rows.push((
            P::ALARM_VALUES,
            states(&[4, 5]),
            AlgorithmWriteConfig {
                alarm_values: Some(vec![4, 5]),
                ..none()
            },
        )),
        _ => {}
    }
    rows
}

/// A write of each parameter some other type keeps and `kind` doesn't, and
/// storage holding it.
fn uncovered(kind: ObjectType) -> Vec<(P, PropertyValue, AlgorithmWriteConfig)> {
    let none = AlgorithmWriteConfig::default;
    let enable = PropertyValue::BitString {
        unused_bits: 6,
        data: vec![LimitEnable::all().to_bacnet()],
    };
    let mut rows = vec![
        (
            P::HIGH_LIMIT,
            PropertyValue::Real(80.0),
            AlgorithmWriteConfig {
                high_limit: Some(80.0),
                ..none()
            },
        ),
        (
            P::LOW_LIMIT,
            PropertyValue::Real(-10.0),
            AlgorithmWriteConfig {
                low_limit: Some(-10.0),
                ..none()
            },
        ),
        (
            P::DEADBAND,
            PropertyValue::Real(2.5),
            AlgorithmWriteConfig {
                deadband: Some(2.5),
                ..none()
            },
        ),
        (
            P::LIMIT_ENABLE,
            enable,
            AlgorithmWriteConfig {
                limit_enable: Some(LimitEnable::all()),
                ..none()
            },
        ),
    ];
    if is_binary(kind) || !has_alarm_values(kind) {
        rows.push((
            P::ALARM_VALUES,
            states(&[1]),
            AlgorithmWriteConfig {
                alarm_values: Some(vec![1]),
                ..none()
            },
        ));
    }
    if !is_binary(kind) || !has_alarm_values(kind) {
        rows.push((
            P::ALARM_VALUE,
            PropertyValue::Enumerated(0),
            AlgorithmWriteConfig {
                alarm_value: Some(BinaryPV::INACTIVE),
                ..none()
            },
        ));
    }
    rows
}

/// Give `object` the Present_Value `state`: an input's while out of
/// service, an output's or value's by command.
pub(super) fn set_state(object: &mut dyn BACnetObject, state: u32) {
    let kind = object.object_identifier().object_type();
    let value = if is_binary(kind) {
        PropertyValue::Enumerated(state)
    } else {
        PropertyValue::Unsigned(u64::from(state))
    };
    if matches!(
        kind,
        ObjectType::BINARY_INPUT | ObjectType::MULTI_STATE_INPUT
    ) {
        write(object, P::OUT_OF_SERVICE, PropertyValue::Boolean(true));
        write(object, P::PRESENT_VALUE, value);
    } else {
        let origin = crate::command_source::test_origin();
        object
            .write_property_from(P::PRESENT_VALUE, None, value, Some(8), &origin)
            .unwrap();
    }
}

/// Give an output the Feedback_Value `state`.
fn set_feedback(object: &mut dyn BACnetObject, state: u32) {
    let kind = object.object_identifier().object_type();
    let value = if is_binary(kind) {
        PropertyValue::Enumerated(state)
    } else {
        PropertyValue::Unsigned(u64::from(state))
    };
    write(object, P::FEEDBACK_VALUE, value);
}

/// Put `object` in its offnormal condition, or back out of it. With
/// `restored` the condition is one only the parameters written in
/// [`a_rebuilt_detector_uses_the_restored_parameters`] make offnormal;
/// without, one the configured parameters do. An output's condition is its
/// feedback disagreeing with its command, whatever its parameters.
fn set_condition(object: &mut dyn BACnetObject, offnormal: bool, restored: bool) {
    let kind = object.object_identifier().object_type();
    match (is_binary(kind), has_alarm_values(kind)) {
        // Written Alarm_Value INACTIVE, configured ACTIVE.
        (true, true) => set_state(object, u32::from(offnormal != restored)),
        // Written Alarm_Values 4 and 5, configured 2; state 1 is in neither.
        (false, true) => {
            let alarm = if restored { 5 } else { 2 };
            set_state(object, if offnormal { alarm } else { 1 });
        }
        (true, false) => {
            set_state(object, 1);
            set_feedback(object, u32::from(!offnormal));
        }
        (false, false) => {
            set_state(object, 3);
            set_feedback(object, if offnormal { 2 } else { 3 });
        }
    }
}

/// Commit the transition the detector proposed, from `from` to `to`.
fn commit(object: &mut dyn BACnetObject, from: EventState, to: EventState) {
    let reporting = object.event_reporting_mut().unwrap();
    reporting
        .commit_event_transition(EventTransitionCommit {
            change: EventStateChange { from, to },
            coordinate: EventTransition::for_target_state(to),
            ack_required: false,
            timestamp: BACnetTimeStamp::SequenceNumber(1),
            message_text: None,
        })
        .unwrap();
}

/// Each parameter written on each type saves once, before it answers,
/// storage keeps that parameter and no other, and an object rebuilt on the
/// storage serves it over what the application configures.
#[test]
fn each_written_parameter_survives_a_rebuild_and_only_it_is_kept() {
    for kind in KINDS {
        let rows = covered(kind);
        assert_eq!(rows.len(), if has_alarm_values(kind) { 3 } else { 2 });
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
            if property != P::TIME_DELAY {
                assert_eq!(
                    read(rebuilt.as_ref(), P::TIME_DELAY),
                    PropertyValue::Unsigned(0),
                    "{kind}"
                );
            }
        }
    }
}

/// A written list may be empty: no state alarms. It is kept as an empty
/// list, not as nothing written, and comes back over the configured one.
#[test]
fn an_empty_alarm_values_list_is_kept_as_one() {
    for kind in [ObjectType::MULTI_STATE_INPUT, ObjectType::MULTI_STATE_VALUE] {
        let storage = MemoryConfig::new();
        let mut object = open(kind, Some(&storage));
        write(object.as_mut(), P::ALARM_VALUES, states(&[]));
        assert_eq!(saved(&storage).algorithm.alarm_values, Some(vec![]));
        drop(object);
        let mut rebuilt = open(kind, Some(&storage));
        assert_eq!(read(rebuilt.as_ref(), P::ALARM_VALUES), states(&[]));
        set_state(rebuilt.as_mut(), 2);
        assert_eq!(evaluated(rebuilt.as_mut()), None, "{kind}");
    }
}

/// A rebuilt object's detector uses the restored parameters, on hand-set
/// time. The restored Alarm_Value or Alarm_Values entry raises OFFNORMAL
/// where the configured one would not, and not where it would; the
/// restored Time_Delay of two seconds runs first, counted by hand; and the
/// return to NORMAL waits the restored Time_Delay_Normal of one second, not
/// Time_Delay's two. An object without the storage alarms at once on its
/// configured parameters.
#[test]
fn a_rebuilt_detector_uses_the_restored_parameters() {
    for kind in KINDS {
        let storage = MemoryConfig::new();
        let mut object = open(kind, Some(&storage));
        match (is_binary(kind), has_alarm_values(kind)) {
            (true, true) => write(
                object.as_mut(),
                P::ALARM_VALUE,
                PropertyValue::Enumerated(0),
            ),
            (false, true) => write(object.as_mut(), P::ALARM_VALUES, states(&[4, 5])),
            _ => {}
        }
        write(object.as_mut(), P::TIME_DELAY, PropertyValue::Unsigned(2));
        let one = PropertyValue::Unsigned(1);
        write(object.as_mut(), P::TIME_DELAY_NORMAL, one);
        drop(object);

        let mut rebuilt = open(kind, Some(&storage));
        if has_alarm_values(kind) {
            // What the configured alarm value alone would alarm on.
            set_condition(rebuilt.as_mut(), true, false);
            assert_eq!(evaluated(rebuilt.as_mut()), None, "{kind}");
            for _ in 0..3 {
                assert_eq!(ticked(rebuilt.as_mut()), None, "{kind}");
            }
        }
        set_condition(rebuilt.as_mut(), true, true);
        assert_eq!(evaluated(rebuilt.as_mut()), None, "{kind}");
        assert_eq!(ticked(rebuilt.as_mut()), None, "{kind}");
        assert_eq!(
            ticked(rebuilt.as_mut()),
            Some(EventState::OFFNORMAL),
            "{kind}"
        );
        commit(rebuilt.as_mut(), EventState::NORMAL, EventState::OFFNORMAL);
        set_condition(rebuilt.as_mut(), false, true);
        assert_eq!(evaluated(rebuilt.as_mut()), None, "{kind}");
        assert_eq!(ticked(rebuilt.as_mut()), Some(EventState::NORMAL), "{kind}");

        // Without the storage: the configured parameters, and no delay.
        let mut plain = open(kind, None);
        if has_alarm_values(kind) {
            set_condition(plain.as_mut(), true, true);
            assert_eq!(evaluated(plain.as_mut()), None, "{kind}");
            assert_eq!(ticked(plain.as_mut()), None, "{kind}");
        }
        set_condition(plain.as_mut(), true, false);
        assert_eq!(
            evaluated(plain.as_mut()),
            Some(EventState::OFFNORMAL),
            "{kind}"
        );
    }
}

/// A parameter write whose save fails is refused with DEVICE /
/// OPERATIONAL_PROBLEM, and the old value stays: on the wire, in the
/// detector and in storage.
#[test]
fn a_parameter_write_whose_save_fails_keeps_the_old_value() {
    for kind in KINDS {
        let storage = MemoryConfig::new();
        let mut object = open(kind, Some(&storage));
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
        // The detector still has what was configured: no delay, and the
        // configured alarm value, not the refused one.
        if has_alarm_values(kind) {
            set_condition(object.as_mut(), true, true);
            assert_eq!(evaluated(object.as_mut()), None, "{kind}");
            assert_eq!(ticked(object.as_mut()), None, "{kind}");
        }
        set_condition(object.as_mut(), true, false);
        assert_eq!(
            evaluated(object.as_mut()),
            Some(EventState::OFFNORMAL),
            "{kind}"
        );
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
    match (is_binary(kind), has_alarm_values(kind)) {
        (true, true) => refusals.extend([
            (P::ALARM_VALUE, None, PropertyValue::Enumerated(2)),
            (P::ALARM_VALUE, None, PropertyValue::Unsigned(1)),
            (P::ALARM_VALUE, None, PropertyValue::Null),
            (P::ALARM_VALUE, Some(1), PropertyValue::Enumerated(0)),
        ]),
        (false, true) => {
            let long: Vec<u32> = (0..=1024).map(|_| 1).collect();
            refusals.extend([
                (P::ALARM_VALUES, None, states(&[0])),
                (P::ALARM_VALUES, None, states(&[1, STATES + 1])),
                (P::ALARM_VALUES, None, states(&long)),
                (P::ALARM_VALUES, None, PropertyValue::Unsigned(1)),
                (
                    P::ALARM_VALUES,
                    None,
                    PropertyValue::List(vec![PropertyValue::Enumerated(1)]),
                ),
                (P::ALARM_VALUES, Some(1), states(&[1])),
            ]);
        }
        _ => {}
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

/// A stored value is taken or refused as a write of it is: what the object
/// refuses on the wire fails the build, and what it takes is served. One
/// wire rule is left out on load: an Alarm_Values state past the object's
/// Number_Of_States, which a write refuses, is served as it was stored.
#[test]
fn a_stored_parameter_is_validated_as_a_write_of_it_is() {
    let none = AlgorithmWriteConfig::default;
    for kind in KINDS {
        if !has_alarm_values(kind) {
            continue;
        }
        // Each case: the property, a write of the value, the value stored,
        // and whether only the object's state count stands against it.
        let mut cases: Vec<(P, PropertyValue, AlgorithmWriteConfig, bool)> = Vec::new();
        if is_binary(kind) {
            for raw in [0, 1, 2, 7, 256] {
                let stored = AlgorithmWriteConfig {
                    alarm_value: Some(BinaryPV::from_raw(raw)),
                    ..none()
                };
                let written = PropertyValue::Enumerated(raw);
                cases.push((P::ALARM_VALUE, written, stored, false));
            }
        } else {
            let full: Vec<u32> = (0..1024).map(|at| at % STATES + 1).collect();
            let over: Vec<u32> = (0..1025).map(|_| 1).collect();
            let lists = [
                (vec![], false),
                (vec![1], false),
                (vec![STATES], false),
                (vec![3, 3, 1], false),
                (full, false),
                (vec![0], false),
                (vec![2, 0], false),
                (over, false),
                (vec![STATES + 1], true),
                (vec![1, 2, u32::MAX], true),
            ];
            for (list, past_the_count) in lists {
                let stored = AlgorithmWriteConfig {
                    alarm_values: Some(list.clone()),
                    ..none()
                };
                cases.push((P::ALARM_VALUES, states(&list), stored, past_the_count));
            }
        }
        let mut refused = 0;
        for (property, value, stored, past_the_count) in cases {
            let written = open(kind, None).write_property(property, None, value.clone(), None);
            let built = try_open(kind, Some(&holding(stored)));
            assert_eq!(
                built.is_ok(),
                written.is_ok() || past_the_count,
                "{kind} {property:?} {value:?}: a write gives {written:?}"
            );
            assert_eq!(written.is_err(), built.is_err() || past_the_count);
            match built {
                Ok(object) => assert_eq!(read(object.as_ref(), property), value),
                Err(error) => {
                    assert!(matches!(error, Error::Encoding(_)), "{error:?}");
                    refused += 1;
                }
            }
        }
        assert_eq!(refused, 3, "{kind}");
    }
}

/// A type keeps only its own parameters. On each of the six types, storage
/// holding a parameter another type keeps fails the build, and a write of
/// its property is answered as without storage and saves nothing.
#[test]
fn a_type_keeps_only_its_own_parameters() {
    for kind in KINDS {
        let rows = uncovered(kind);
        assert_eq!(rows.len() + covered(kind).len(), 8, "{kind}: every row");
        let storage = MemoryConfig::new();
        let mut kept = open(kind, Some(&storage));
        let mut plain = open(kind, None);
        for (property, value, stored) in rows {
            let built = try_open(kind, Some(&holding(stored)));
            assert!(matches!(built, Err(Error::Encoding(_))), "{kind}");
            let expected = plain.write_property(property, None, value.clone(), None);
            let result = kept.write_property(property, None, value, None);
            assert_eq!(
                result.map_err(|error| error.to_string()),
                expected.map_err(|error| error.to_string()),
                "{kind} {property:?}"
            );
            assert_eq!(storage.saves(), 0, "{kind} {property:?}");
        }
        assert!(
            !super::reporting::kept_row(kind, P::FEEDBACK_VALUE),
            "{kind}"
        );
    }
}
