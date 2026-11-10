//! The rules the binary and multi-state objects' kept parameters follow
//! (#2331): the `set_alarm_values` setter gives way to a saved list until
//! the clear, the clear drops the parameters with the rest, a list naming a
//! state the rebuilt object lacks fails the build, a restored list is taken
//! as a write of it is, a request's writes save once, and a staged step
//! is checked by the family's own write path against the object as it is
//! when the write is made.

use bacnet_types::enums::{
    ErrorClass, ErrorCode, ObjectType, PropertyIdentifier as P, Reliability,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::parameters_tests::{read, saved, write};
use super::reporting_tests::{persistence, refused_with};
use super::state_parameters_tests::{KINDS, STATES, has_alarm_values, is_binary, open, states};
use super::test_storage::MemoryConfig;
use super::{AlgorithmWriteConfig, ReportingWriteConfig, WriteConfigSnapshot};
use crate::multistate::{MultiStateInputObject, MultiStateValueObject};
use crate::traits::BACnetObject;

/// Run `$body` on a Multi-state Input and on a Multi-state Value: `$Object`
/// names the type, and `$new` builds object 1 of it with so many states.
macro_rules! on_each_listed {
    (|$new:ident, $Object:ident| $body:block) => {{
        {
            type $Object = MultiStateInputObject;
            let $new = |states: u32| $Object::new(1, "MSI", states).unwrap();
            $body
        }
        {
            type $Object = MultiStateValueObject;
            let $new = |states: u32| $Object::new(1, "MSV", states).unwrap();
            $body
        }
    }};
}

/// A saved Alarm_Values list wins over `set_alarm_values`, on the object
/// that took the write and on one rebuilt on its storage, until
/// `clear_written_reporting`; then the setter applies again, and at the
/// next start what the application configures. Without storage, and with
/// storage holding no written list, the setter applies as it always has.
#[test]
fn a_saved_list_wins_over_its_setter_until_the_clear() {
    on_each_listed!(|new, Object| {
        let storage = MemoryConfig::new();
        let attach = |mut object: Object| {
            // What the application configures at each start.
            object.set_alarm_values(vec![2]);
            object
                .with_reporting_persistence(persistence(&storage))
                .unwrap()
        };
        let mut plain = new(STATES);
        plain.set_alarm_values(vec![2]);
        plain.set_alarm_values(vec![3]);
        assert_eq!(read(&plain, P::ALARM_VALUES), states(&[3]));

        let mut object = attach(new(STATES));
        // Nothing written yet: the setter applies, and saves nothing.
        object.set_alarm_values(vec![3]);
        assert_eq!(read(&object, P::ALARM_VALUES), states(&[3]));
        assert_eq!(storage.saves(), 0);
        write(&mut object, P::ALARM_VALUES, states(&[4, 5]));
        object.set_alarm_values(vec![1]);
        assert_eq!(read(&object, P::ALARM_VALUES), states(&[4, 5]));
        drop(object);

        let mut rebuilt = attach(new(STATES));
        assert_eq!(read(&rebuilt, P::ALARM_VALUES), states(&[4, 5]));
        rebuilt.set_alarm_values(vec![1]);
        assert_eq!(read(&rebuilt, P::ALARM_VALUES), states(&[4, 5]));
        let kept = rebuilt.written_reporting();
        assert_eq!(kept.algorithm.alarm_values, Some(vec![4, 5]));

        storage.fail(true);
        assert!(rebuilt.clear_written_reporting().is_err());
        rebuilt.set_alarm_values(vec![1]);
        assert_eq!(
            read(&rebuilt, P::ALARM_VALUES),
            states(&[4, 5]),
            "a failed clear keeps the list over the setter"
        );
        storage.fail(false);
        rebuilt.clear_written_reporting().unwrap();
        assert_eq!(rebuilt.written_reporting(), ReportingWriteConfig::default());
        // The value served stays until a setter or a write changes it.
        assert_eq!(read(&rebuilt, P::ALARM_VALUES), states(&[4, 5]));
        rebuilt.set_alarm_values(vec![1]);
        assert_eq!(read(&rebuilt, P::ALARM_VALUES), states(&[1]));
        drop(rebuilt);
        assert_eq!(saved(&storage), ReportingWriteConfig::default());
        assert_eq!(read(&attach(new(STATES)), P::ALARM_VALUES), states(&[2]));
    });
}

/// The clear drops every parameter a type keeps with the reporting rows
/// and Description, in one save; the values served stay until the next
/// start, when what the application configures applies again.
#[test]
fn the_clear_drops_parameters_reporting_rows_and_description_together() {
    for kind in KINDS {
        let storage = MemoryConfig::new();
        let mut object = open(kind, Some(&storage));
        write(object.as_mut(), P::TIME_DELAY, PropertyValue::Unsigned(7));
        let three = PropertyValue::Unsigned(3);
        write(object.as_mut(), P::TIME_DELAY_NORMAL, three);
        let nine = PropertyValue::Unsigned(9);
        write(object.as_mut(), P::NOTIFICATION_CLASS, nine);
        let label = PropertyValue::CharacterString("lobby".into());
        write(object.as_mut(), P::DESCRIPTION, label);
        let own = match (is_binary(kind), has_alarm_values(kind)) {
            (true, true) => Some((P::ALARM_VALUE, PropertyValue::Enumerated(0))),
            (false, true) => Some((P::ALARM_VALUES, states(&[4, 5]))),
            _ => None,
        };
        if let Some((property, value)) = own.clone() {
            write(object.as_mut(), property, value);
        }
        let kept = saved(&storage);
        assert_eq!(kept.algorithm.time_delay, Some(7));
        assert_eq!(kept.algorithm.time_delay_normal, Some(3));
        assert_eq!(
            kept.algorithm.alarm_value.is_some() || kept.algorithm.alarm_values.is_some(),
            has_alarm_values(kind)
        );
        assert_eq!(kept.notification_class, Some(9));
        assert_eq!(kept.description.as_deref(), Some("lobby"));
        drop(object);
        let saves = storage.saves();

        macro_rules! clear {
            ($object:expr) => {{
                let mut object = $object
                    .and_then(|object| object.with_reporting_persistence(persistence(&storage)))
                    .unwrap();
                assert_eq!(object.written_reporting(), kept);
                storage.fail(true);
                assert!(object.clear_written_reporting().is_err());
                assert_eq!(
                    object.written_reporting(),
                    kept,
                    "a failed clear keeps them"
                );
                storage.fail(false);
                object.clear_written_reporting().unwrap();
                assert_eq!(object.written_reporting(), ReportingWriteConfig::default());
                assert_eq!(read(&object, P::TIME_DELAY), PropertyValue::Unsigned(7));
                if let Some((property, value)) = own.clone() {
                    assert_eq!(read(&object, property), value);
                }
            }};
        }
        use crate::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
        use crate::multistate::MultiStateOutputObject;
        match kind {
            ObjectType::BINARY_INPUT => clear!(BinaryInputObject::new(1, "BI")),
            ObjectType::BINARY_OUTPUT => clear!(BinaryOutputObject::new(1, "BO")),
            ObjectType::BINARY_VALUE => clear!(BinaryValueObject::new(1, "BV")),
            ObjectType::MULTI_STATE_INPUT => clear!(MultiStateInputObject::new(1, "MSI", STATES)),
            ObjectType::MULTI_STATE_OUTPUT => {
                clear!(MultiStateOutputObject::new(1, "MSO", STATES))
            }
            _ => clear!(MultiStateValueObject::new(1, "MSV", STATES)),
        }
        assert_eq!(storage.saves(), saves + 1, "{kind}: one save clears all");
        assert_eq!(saved(&storage), ReportingWriteConfig::default(), "{kind}");
        let rebuilt = open(kind, Some(&storage));
        let zero = PropertyValue::Unsigned(0);
        assert_eq!(read(rebuilt.as_ref(), P::TIME_DELAY), zero, "{kind}");
        assert_eq!(read(rebuilt.as_ref(), P::NOTIFICATION_CLASS), zero);
        match own {
            Some((P::ALARM_VALUE, _)) => {
                assert_eq!(
                    read(rebuilt.as_ref(), P::ALARM_VALUE),
                    PropertyValue::Enumerated(1)
                );
            }
            Some((property, _)) => assert_eq!(read(rebuilt.as_ref(), property), states(&[2])),
            None => {}
        }
    }
}

/// Storage holding `list` as written Alarm_Values.
fn holding_list(list: &[u32]) -> alloc::sync::Arc<MemoryConfig> {
    MemoryConfig::holding(WriteConfigSnapshot::Reporting(ReportingWriteConfig {
        algorithm: AlgorithmWriteConfig {
            alarm_values: Some(list.to_vec()),
            ..Default::default()
        },
        ..Default::default()
    }))
}

/// An object rebuilt with fewer states than a saved Alarm_Values entry
/// names doesn't open on that storage, as a write of the list to it would
/// be refused, and the storage is left as it was. Opened with enough
/// states, the object clears it, and the smaller one then opens on what the
/// application configures.
#[test]
fn a_saved_list_past_the_rebuilt_objects_states_fails_the_build() {
    on_each_listed!(|new, Object| {
        let _: fn(u32) -> Object = new;
        let storage = MemoryConfig::new();
        let mut object = new(STATES)
            .with_reporting_persistence(persistence(&storage))
            .unwrap();
        write(&mut object, P::ALARM_VALUES, states(&[2, STATES]));
        drop(object);
        let kept = saved(&storage);

        let fewer = new(STATES - 1).with_reporting_persistence(persistence(&storage));
        let Err(error) = fewer else {
            panic!(
                "a list naming state {STATES} opened on {} states",
                STATES - 1
            );
        };
        assert!(matches!(error, Error::Encoding(_)), "{error:?}");
        assert!(error.to_string().contains("Number_Of_States"), "{error}");
        assert_eq!(saved(&storage), kept);
        assert_eq!(storage.saves(), 1);

        let mut enough = new(STATES)
            .with_reporting_persistence(persistence(&storage))
            .unwrap();
        enough.clear_written_reporting().unwrap();
        drop(enough);
        let mut fewer = new(STATES - 1);
        fewer.set_alarm_values(vec![1]);
        let fewer = fewer
            .with_reporting_persistence(persistence(&storage))
            .unwrap();
        assert_eq!(read(&fewer, P::ALARM_VALUES), states(&[1]));
    });
}

/// Number_Of_States isn't kept. A peer that gives an object more states by
/// writing State_Text, and then an Alarm_Values entry naming one of them,
/// leaves a list the object as the application builds it doesn't open on.
/// And the wire can't get there the other way: a State_Text write that
/// would drop a state the held list names is refused.
#[test]
fn a_list_naming_a_state_a_peer_added_fails_the_next_build() {
    on_each_listed!(|new, Object| {
        let _: fn(u32) -> Object = new;
        let storage = MemoryConfig::new();
        let mut object = new(STATES)
            .with_reporting_persistence(persistence(&storage))
            .unwrap();
        let grown = PropertyValue::Unsigned(u64::from(STATES + 1));
        object
            .write_property(P::STATE_TEXT, Some(0), grown, None)
            .unwrap();
        write(&mut object, P::ALARM_VALUES, states(&[STATES + 1]));
        let shrunk = PropertyValue::Unsigned(u64::from(STATES));
        let result = object.write_property(P::STATE_TEXT, Some(0), shrunk, None);
        assert!(
            refused_with(&result, ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE),
            "{result:?}"
        );
        drop(object);
        assert!(
            new(STATES)
                .with_reporting_persistence(persistence(&storage))
                .is_err()
        );
        assert!(
            new(STATES + 1)
                .with_reporting_persistence(persistence(&storage))
                .is_ok()
        );
    });
}

/// A Multi-state Value works its Reliability out again when Alarm_Values is
/// written, and so when a saved list goes over the configured one: the
/// CONFIGURATION_ERROR a configured state past the count raised doesn't
/// outlive that list.
#[test]
fn a_restored_list_is_taken_as_a_write_of_it_is() {
    let mut written = MultiStateValueObject::new(1, "MSV", STATES).unwrap();
    written.set_alarm_values(vec![STATES + 4]);
    let fault = PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw());
    assert_eq!(read(&written, P::RELIABILITY), fault);
    write(&mut written, P::ALARM_VALUES, states(&[2]));
    let sound = PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw());
    assert_eq!(read(&written, P::RELIABILITY), sound);

    let mut restored = MultiStateValueObject::new(1, "MSV", STATES).unwrap();
    restored.set_alarm_values(vec![STATES + 4]);
    assert_eq!(read(&restored, P::RELIABILITY), fault);
    let restored = restored
        .with_reporting_persistence(persistence(&holding_list(&[2])))
        .unwrap();
    assert_eq!(read(&restored, P::ALARM_VALUES), states(&[2]));
    assert_eq!(read(&restored, P::RELIABILITY), sound);
    // Storage without a list leaves the configured one, and its fault.
    let mut unwritten = MultiStateValueObject::new(1, "MSV", STATES).unwrap();
    unwritten.set_alarm_values(vec![STATES + 4]);
    let unwritten = unwritten
        .with_reporting_persistence(persistence(&MemoryConfig::new()))
        .unwrap();
    assert_eq!(read(&unwritten, P::RELIABILITY), fault);
}

#[cfg(feature = "durable-writes")]
mod staged {
    use bacnet_types::enums::BinaryPV;

    use super::super::parameters_tests::pending;
    use super::*;
    use crate::durable::StageStep;

    const LISTED: [ObjectType; 2] = [ObjectType::MULTI_STATE_INPUT, ObjectType::MULTI_STATE_VALUE];

    /// The two kept parameters a request to `kind` writes around a
    /// reporting row, and what storage keeps of them.
    fn request(kind: ObjectType) -> ([(P, PropertyValue); 2], AlgorithmWriteConfig) {
        let delay = (P::TIME_DELAY, PropertyValue::Unsigned(7));
        let none = AlgorithmWriteConfig::default;
        match (is_binary(kind), has_alarm_values(kind)) {
            (true, true) => (
                [(P::ALARM_VALUE, PropertyValue::Enumerated(0)), delay],
                AlgorithmWriteConfig {
                    alarm_value: Some(BinaryPV::INACTIVE),
                    time_delay: Some(7),
                    ..none()
                },
            ),
            (false, true) => (
                [(P::ALARM_VALUES, states(&[4, 5])), delay],
                AlgorithmWriteConfig {
                    alarm_values: Some(vec![4, 5]),
                    time_delay: Some(7),
                    ..none()
                },
            ),
            _ => (
                [(P::TIME_DELAY_NORMAL, PropertyValue::Unsigned(3)), delay],
                AlgorithmWriteConfig {
                    time_delay_normal: Some(3),
                    time_delay: Some(7),
                    ..none()
                },
            ),
        }
    }

    /// A request writing two parameters and a reporting row stages one
    /// save of them all, ahead of the object serving any; its writes then
    /// take their steps without saving again. When that save fails each
    /// write is refused and the old values stay.
    #[test]
    fn a_request_of_parameters_and_a_reporting_row_saves_once() {
        for kind in KINDS {
            let ([first, second], algorithm) = request(kind);
            let request = [
                pending(first.0, first.1),
                pending(P::NOTIFICATION_CLASS, PropertyValue::Unsigned(9)),
                pending(second.0, second.1),
            ];
            for fails in [false, true] {
                let storage = MemoryConfig::new();
                let mut object = open(kind, Some(&storage));
                let before = request
                    .each_ref()
                    .map(|write| read(object.as_ref(), write.property));
                storage.fail(fails);
                let durable = object.durable_writes().unwrap();
                let StageStep::Staged(wait) = durable.stage_writes(&request) else {
                    panic!("{kind}: the request stages");
                };
                wait.block();
                assert_eq!(storage.saves(), usize::from(!fails), "{kind}");
                for (write, before) in request.iter().zip(&before) {
                    assert_eq!(&read(object.as_ref(), write.property), before, "{kind}");
                }
                for (write, before) in request.iter().zip(&before) {
                    let value = write.value.clone();
                    let result = object.write_property(write.property, None, value, None);
                    if fails {
                        assert!(
                            refused_with(
                                &result,
                                ErrorClass::DEVICE,
                                ErrorCode::OPERATIONAL_PROBLEM
                            ),
                            "{kind} {:?}: {result:?}",
                            write.property
                        );
                        assert_eq!(&read(object.as_ref(), write.property), before, "{kind}");
                    } else {
                        result.unwrap();
                        assert_eq!(read(object.as_ref(), write.property), write.value);
                    }
                }
                object.durable_writes().unwrap().release_staged_write(&wait);
                drop(object);
                assert_eq!(storage.saves(), usize::from(!fails), "{kind}");
                if fails {
                    assert_eq!(storage.saved(), None, "{kind}");
                    continue;
                }
                let kept = saved(&storage);
                assert_eq!(kept.notification_class, Some(9));
                assert_eq!(kept.algorithm, algorithm, "{kind}");
            }
        }
    }

    /// A staged step runs the write path of the object's own family: it
    /// refuses what that object refuses, ending the steps, so a later write
    /// isn't saved ahead of one that will never be made. A value another
    /// family's check would take, or refuse, doesn't change that.
    #[test]
    fn a_staged_step_is_checked_by_the_objects_own_write_path() {
        for kind in KINDS {
            // What `kind` refuses for its own parameter, or for a delay.
            let refused = match (is_binary(kind), has_alarm_values(kind)) {
                (true, true) => pending(P::ALARM_VALUE, PropertyValue::Enumerated(2)),
                (false, true) => pending(P::ALARM_VALUES, states(&[STATES + 1])),
                _ => pending(P::TIME_DELAY_NORMAL, PropertyValue::Real(1.0)),
            };
            let storage = MemoryConfig::new();
            let mut object = open(kind, Some(&storage));
            let request = [
                pending(P::TIME_DELAY, PropertyValue::Unsigned(7)),
                refused,
                pending(P::NOTIFICATION_CLASS, PropertyValue::Unsigned(9)),
            ];
            let durable = object.durable_writes().unwrap();
            let StageStep::Staged(wait) = durable.stage_writes(&request) else {
                panic!("{kind}: the request stages");
            };
            wait.block();
            let expected = ReportingWriteConfig {
                algorithm: AlgorithmWriteConfig {
                    time_delay: Some(7),
                    ..Default::default()
                },
                ..Default::default()
            };
            assert_eq!(saved(&storage), expected, "{kind}");
            object.durable_writes().unwrap().release_staged_write(&wait);
            drop(object);
            // The parameter each family takes stages alone.
            let (taken, _) = request_of(kind);
            let storage = MemoryConfig::new();
            let mut object = open(kind, Some(&storage));
            let durable = object.durable_writes().unwrap();
            let StageStep::Staged(wait) = durable.stage_writes(&[taken]) else {
                panic!("{kind}: its own parameter stages");
            };
            wait.block();
            assert_eq!(storage.saves(), 1, "{kind}");
            object.durable_writes().unwrap().release_staged_write(&wait);
        }
    }

    fn request_of(kind: ObjectType) -> (crate::durable::PendingWrite, AlgorithmWriteConfig) {
        let ([first, _], algorithm) = request(kind);
        (pending(first.0, first.1), algorithm)
    }

    /// A multi-state object's check of Alarm_Values turns on its
    /// Number_Of_States, which a State_Text write changes without the store
    /// counting it. A list staged against five states and written after
    /// the object has four is refused as the object refuses it and takes no
    /// staged step, although it is the request's last: once the request
    /// releases, storage holds what the object serves, the list neither
    /// served nor kept, and the write made before it kept.
    #[test]
    fn a_staged_list_the_object_no_longer_takes_is_refused_and_not_kept() {
        for kind in LISTED {
            let storage = MemoryConfig::new();
            let mut object = open(kind, Some(&storage));
            let request = [
                pending(P::TIME_DELAY, PropertyValue::Unsigned(7)),
                pending(P::ALARM_VALUES, states(&[STATES])),
            ];
            let durable = object.durable_writes().unwrap();
            let StageStep::Staged(wait) = durable.stage_writes(&request) else {
                panic!("{kind}: the request stages");
            };
            wait.block();
            assert_eq!(
                saved(&storage).algorithm.alarm_values,
                Some(vec![STATES]),
                "{kind}: the staged save ran"
            );
            // Another request drops the last state meanwhile.
            let fewer = PropertyValue::Unsigned(u64::from(STATES - 1));
            object
                .write_property(P::STATE_TEXT, Some(0), fewer.clone(), None)
                .unwrap();
            let [delay, list] = request.map(|write| write.value);
            write(object.as_mut(), P::TIME_DELAY, delay.clone());
            // As an object of four states without storage refuses it.
            let mut plain = open(kind, None);
            plain
                .write_property(P::STATE_TEXT, Some(0), fewer, None)
                .unwrap();
            let refusal = plain
                .write_property(P::ALARM_VALUES, None, list.clone(), None)
                .unwrap_err();
            let result = object.write_property(P::ALARM_VALUES, None, list, None);
            assert_eq!(result.unwrap_err().to_string(), refusal.to_string());
            assert_eq!(read(object.as_ref(), P::ALARM_VALUES), states(&[2]));
            // The request stops at its refused write and releases.
            object.durable_writes().unwrap().release_staged_write(&wait);
            drop(object);
            let kept = saved(&storage).algorithm;
            assert_eq!(kept.alarm_values, None, "{kind}: the refused list is kept");
            assert_eq!(kept.time_delay, Some(7), "{kind}");
            let rebuilt = open(kind, Some(&storage));
            assert_eq!(read(rebuilt.as_ref(), P::ALARM_VALUES), states(&[2]));
            assert_eq!(read(rebuilt.as_ref(), P::TIME_DELAY), delay, "{kind}");
        }
    }

    /// The other way round: a list the stage refused, because it named a
    /// state the object didn't have yet, is checked again when it is
    /// written, and saved then.
    #[test]
    fn a_list_refused_at_the_stage_is_checked_again_when_written() {
        for kind in LISTED {
            let storage = MemoryConfig::new();
            let mut object = open(kind, Some(&storage));
            let request = [pending(P::ALARM_VALUES, states(&[STATES + 1]))];
            let durable = object.durable_writes().unwrap();
            assert!(
                matches!(durable.stage_writes(&request), StageStep::Skip),
                "{kind}: nothing to stage"
            );
            let more = PropertyValue::Unsigned(u64::from(STATES + 1));
            object
                .write_property(P::STATE_TEXT, Some(0), more, None)
                .unwrap();
            write(object.as_mut(), P::ALARM_VALUES, request[0].value.clone());
            assert_eq!(storage.saves(), 1, "{kind}");
            assert_eq!(
                saved(&storage).algorithm.alarm_values,
                Some(vec![STATES + 1])
            );
        }
    }
}
