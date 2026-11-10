//! The rules the binary and multi-state objects' kept parameters follow
//! (#2331): the `set_alarm_values` setter gives way to a saved list until
//! the clear, the clear drops the parameters with the rest, a list naming a
//! state the rebuilt object lacks is restored as written, a count a peer
//! raised comes back with the list naming it (#2414), a restored list is
//! taken as a write of it is, a request's writes save once, and a staged
//! step is checked by the family's own write path against the state count
//! the write will meet.

use bacnet_types::enums::{
    ErrorClass, ErrorCode, EventState, ObjectType, PropertyIdentifier as P, Reliability,
};
use bacnet_types::primitives::PropertyValue;

use super::parameters_tests::{evaluated, read, saved, write};
use super::reporting_tests::{persistence, refused_with};
use super::state_parameters_tests::{
    KINDS, STATES, has_alarm_values, is_binary, open, set_state, states,
};
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
            object.set_alarm_values(vec![2]).unwrap();
            object
                .with_reporting_persistence(persistence(&storage))
                .unwrap()
        };
        let mut plain = new(STATES);
        plain.set_alarm_values(vec![2]).unwrap();
        plain.set_alarm_values(vec![3]).unwrap();
        assert_eq!(read(&plain, P::ALARM_VALUES), states(&[3]));

        let mut object = attach(new(STATES));
        // Nothing written yet: the setter applies, and saves nothing.
        object.set_alarm_values(vec![3]).unwrap();
        assert_eq!(read(&object, P::ALARM_VALUES), states(&[3]));
        assert_eq!(storage.saves(), 0);
        write(&mut object, P::ALARM_VALUES, states(&[4, 5]));
        object.set_alarm_values(vec![1]).unwrap();
        assert_eq!(read(&object, P::ALARM_VALUES), states(&[4, 5]));
        drop(object);

        let mut rebuilt = attach(new(STATES));
        assert_eq!(read(&rebuilt, P::ALARM_VALUES), states(&[4, 5]));
        rebuilt.set_alarm_values(vec![1]).unwrap();
        assert_eq!(read(&rebuilt, P::ALARM_VALUES), states(&[4, 5]));
        let kept = rebuilt.written_reporting();
        assert_eq!(kept.algorithm.alarm_values, Some(vec![4, 5]));

        storage.fail(true);
        assert!(rebuilt.clear_written_reporting().is_err());
        rebuilt.set_alarm_values(vec![1]).unwrap();
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
        rebuilt.set_alarm_values(vec![1]).unwrap();
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

fn reliability(reliability: Reliability) -> PropertyValue {
    PropertyValue::Enumerated(reliability.to_raw())
}

/// The Reliability an object holding an Alarm_Values state past its count
/// shows: a Multi-state Value's CONFIGURATION_ERROR, and no fault on a
/// Multi-state Input, which doesn't look at its list.
fn stranded(object: &dyn BACnetObject) -> PropertyValue {
    let value = object.object_identifier().object_type() == ObjectType::MULTI_STATE_VALUE;
    reliability(if value {
        Reliability::CONFIGURATION_ERROR
    } else {
        Reliability::NO_FAULT_DETECTED
    })
}

/// What an object holding a list with a state past its count does, the
/// list being `list` and state 2 one of its entries within the count. It
/// serves the list. The Input still alarms on state 2. The Value goes to
/// FAULT at its first evaluation, and is sound again once the states are
/// there again or a list within them is written. A write of the same list
/// is refused, as without storage.
fn holds_a_stranded_list(object: &mut dyn BACnetObject, list: &[u32]) {
    let kind = object.object_identifier().object_type();
    let sound = reliability(Reliability::NO_FAULT_DETECTED);
    assert_eq!(read(object, P::ALARM_VALUES), states(list), "{kind}");
    assert_eq!(read(object, P::RELIABILITY), stranded(object), "{kind}");
    set_state(object, 2);
    let expected = if stranded(object) == sound {
        EventState::OFFNORMAL
    } else {
        EventState::FAULT
    };
    assert_eq!(evaluated(object), Some(expected), "{kind}");
    let result = object.write_property(P::ALARM_VALUES, None, states(list), None);
    // As an object with as many states and no storage refuses it.
    let mut plain = open(kind, None);
    let count = read(object, P::NUMBER_OF_STATES);
    plain
        .write_property(P::STATE_TEXT, Some(0), count, None)
        .unwrap();
    let refusal = plain
        .write_property(P::ALARM_VALUES, None, states(list), None)
        .unwrap_err();
    assert_eq!(result.unwrap_err().to_string(), refusal.to_string());
    assert_eq!(read(object, P::ALARM_VALUES), states(list), "{kind}");
}

/// An object rebuilt with fewer states than a saved Alarm_Values entry
/// names opens on that storage with the list as it was written, though a
/// write of the list to it is refused: with no State_Text saved, the count
/// is the one the application builds the object with, and a saved list
/// mustn't stop an object from starting. Storage is left as it
/// was, and a later kept write saves the list with it, unchanged. A
/// Multi-state Value shows the fault from the start, until its states are
/// there again or a list within them is written.
#[test]
fn a_saved_list_past_the_rebuilt_objects_states_is_restored_as_written() {
    on_each_listed!(|new, Object| {
        let _: fn(u32) -> Object = new;
        let list = [2, STATES];
        let storage = MemoryConfig::new();
        let mut object = new(STATES)
            .with_reporting_persistence(persistence(&storage))
            .unwrap();
        write(&mut object, P::ALARM_VALUES, states(&list));
        drop(object);
        let kept = saved(&storage);

        let mut fewer = new(STATES - 1)
            .with_reporting_persistence(persistence(&storage))
            .unwrap();
        assert_eq!(saved(&storage), kept);
        assert_eq!(storage.saves(), 1);
        holds_a_stranded_list(&mut fewer, &list);
        assert_eq!(storage.saves(), 1, "the refused write saved nothing");

        write(&mut fewer, P::TIME_DELAY, PropertyValue::Unsigned(7));
        let later = saved(&storage).algorithm;
        assert_eq!(later.alarm_values, Some(list.to_vec()));
        assert_eq!(later.time_delay, Some(7));

        let sound = reliability(Reliability::NO_FAULT_DETECTED);
        fewer.set_number_of_states(STATES).unwrap();
        assert_eq!(read(&fewer, P::RELIABILITY), sound);
        fewer.set_number_of_states(STATES - 1).unwrap();
        assert_eq!(read(&fewer, P::RELIABILITY), stranded(&fewer));
        write(&mut fewer, P::ALARM_VALUES, states(&[2]));
        assert_eq!(read(&fewer, P::RELIABILITY), sound);
        assert_eq!(saved(&storage).algorithm.alarm_values, Some(vec![2]));
    });
}

/// A peer that gives an object more states by writing State_Text, and
/// then an Alarm_Values entry naming one of them, leaves storage holding
/// both (#2414). The object built on it at the next start has the count
/// the peer gave it, over the application's, serves the labels and the
/// list, and alarms on the added state; the peer can write the list back.
/// The wire can't strand the list the other way: a State_Text write that
/// would drop a state the held list names is refused.
#[test]
fn a_count_a_peer_raised_comes_back_with_the_list_naming_it() {
    on_each_listed!(|new, Object| {
        let _: fn(u32) -> Object = new;
        let list = [2, STATES + 1];
        let storage = MemoryConfig::new();
        let mut object = new(STATES)
            .with_reporting_persistence(persistence(&storage))
            .unwrap();
        let grown = PropertyValue::Unsigned(u64::from(STATES + 1));
        object
            .write_property(P::STATE_TEXT, Some(0), grown.clone(), None)
            .unwrap();
        write(&mut object, P::ALARM_VALUES, states(&list));
        let shrunk = PropertyValue::Unsigned(u64::from(STATES));
        let result = object.write_property(P::STATE_TEXT, Some(0), shrunk, None);
        assert!(
            refused_with(&result, ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE),
            "{result:?}"
        );
        let texts = read(&object, P::STATE_TEXT);
        drop(object);

        let mut rebuilt = new(STATES)
            .with_reporting_persistence(persistence(&storage))
            .unwrap();
        assert_eq!(read(&rebuilt, P::NUMBER_OF_STATES), grown);
        assert_eq!(read(&rebuilt, P::STATE_TEXT), texts);
        assert_eq!(read(&rebuilt, P::ALARM_VALUES), states(&list));
        let sound = reliability(Reliability::NO_FAULT_DETECTED);
        assert_eq!(read(&rebuilt, P::RELIABILITY), sound);
        set_state(&mut rebuilt, STATES + 1);
        assert_eq!(evaluated(&mut rebuilt), Some(EventState::OFFNORMAL));
        write(&mut rebuilt, P::ALARM_VALUES, states(&list));
    });
}

/// A Multi-state Value works its Reliability out again when Alarm_Values is
/// written, and so when a saved list goes over the configured one: the
/// CONFIGURATION_ERROR a configured state past the count raised doesn't
/// outlive that list.
#[test]
fn a_restored_list_is_taken_as_a_write_of_it_is() {
    // A configured list naming a state the count no longer has: only a
    // local shrink under a held list leaves one (#2415).
    let stranding = || {
        let mut object = MultiStateValueObject::new(1, "MSV", STATES).unwrap();
        object.set_alarm_values(vec![STATES]).unwrap();
        object.set_number_of_states(STATES - 1).unwrap();
        object
    };
    let mut written = stranding();
    let fault = PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw());
    assert_eq!(read(&written, P::RELIABILITY), fault);
    write(&mut written, P::ALARM_VALUES, states(&[2]));
    let sound = PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw());
    assert_eq!(read(&written, P::RELIABILITY), sound);

    let restored = stranding();
    assert_eq!(read(&restored, P::RELIABILITY), fault);
    let restored = restored
        .with_reporting_persistence(persistence(&holding_list(&[2])))
        .unwrap();
    assert_eq!(read(&restored, P::ALARM_VALUES), states(&[2]));
    assert_eq!(read(&restored, P::RELIABILITY), sound);
    // Storage without a list leaves the configured one, and its fault.
    let unwritten = stranding()
        .with_reporting_persistence(persistence(&MemoryConfig::new()))
        .unwrap();
    assert_eq!(read(&unwritten, P::RELIABILITY), fault);
}

#[cfg(feature = "durable-writes")]
mod staged {
    use bacnet_types::enums::BinaryPV;

    use super::super::parameters_tests::pending;
    use super::*;
    use crate::durable::{PendingWrite, StageStep};

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
    /// Number_Of_States. A list staged against five states and written
    /// after another request's State_Text write leaves four (a kept write
    /// of its own, which drops the staged steps) is refused as the object
    /// refuses it, although it is the request's last: once the request
    /// releases, storage holds what the object serves, the list neither
    /// served nor kept, and the writes made before it kept.
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
            let four = PropertyValue::Unsigned(u64::from(STATES - 1));
            let rebuilt = open(kind, Some(&storage));
            assert_eq!(read(rebuilt.as_ref(), P::ALARM_VALUES), states(&[2]));
            assert_eq!(read(rebuilt.as_ref(), P::TIME_DELAY), delay, "{kind}");
            assert_eq!(read(rebuilt.as_ref(), P::NUMBER_OF_STATES), four);
        }
    }

    /// The other way round, when the state comes from another request: a
    /// list the stage refused, because it named a state the object didn't
    /// have and the request didn't add, stages nothing. It is checked again
    /// when it is written, against the count the object has by then, and
    /// saved where it is written, beside the State_Text the other request
    /// saved.
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
            assert_eq!(storage.saves(), 2, "{kind}");
            let kept = saved(&storage).algorithm;
            assert_eq!(kept.alarm_values, Some(vec![STATES + 1]));
            assert_eq!(kept.state_text.map(|texts| texts.len()), Some(6));
        }
    }

    /// When the request itself adds the state, by State_Text's size at
    /// index 0 or by State_Text written whole, the stage checks the list
    /// against the count that write leaves: the request stages one save of
    /// both, ahead of its writes, and they take their steps without saving
    /// again. A State_Text write that doesn't add the state is staged
    /// alone, and the list refused at the stage.
    #[test]
    fn a_request_that_adds_a_state_and_lists_it_stages_one_save() {
        let size = |count: u32| PendingWrite {
            property: P::STATE_TEXT,
            array_index: Some(0),
            value: PropertyValue::Unsigned(u64::from(count)),
        };
        let whole = |count: u32| {
            let labels = (1..=count).map(|state| format!("S{state}"));
            let labels = labels.map(PropertyValue::CharacterString);
            pending(P::STATE_TEXT, PropertyValue::List(labels.collect()))
        };
        let one_label = PendingWrite {
            property: P::STATE_TEXT,
            array_index: Some(2),
            value: PropertyValue::CharacterString("two".into()),
        };
        let list = pending(P::ALARM_VALUES, states(&[STATES + 1]));
        for kind in LISTED {
            for grows in [size(STATES + 1), whole(STATES + 1)] {
                let storage = MemoryConfig::new();
                let mut object = open(kind, Some(&storage));
                let request = [grows, list.clone()];
                let durable = object.durable_writes().unwrap();
                let StageStep::Staged(wait) = durable.stage_writes(&request) else {
                    panic!("{kind}: the request stages");
                };
                wait.block();
                assert_eq!(storage.saves(), 1, "{kind}");
                assert_eq!(read(object.as_ref(), P::ALARM_VALUES), states(&[2]));
                for write in &request {
                    let (index, value) = (write.array_index, write.value.clone());
                    object
                        .write_property(write.property, index, value, None)
                        .unwrap();
                }
                object.durable_writes().unwrap().release_staged_write(&wait);
                assert_eq!(read(object.as_ref(), P::ALARM_VALUES), list.value);
                drop(object);
                assert_eq!(storage.saves(), 1, "{kind}: the writes saved again");
                let kept = saved(&storage).algorithm;
                assert_eq!(kept.alarm_values, Some(vec![STATES + 1]));
                assert_eq!(kept.state_text.map(|texts| texts.len()), Some(6));
            }
            for stays in [size(STATES), whole(STATES), one_label.clone()] {
                let storage = MemoryConfig::new();
                let mut object = open(kind, Some(&storage));
                let durable = object.durable_writes().unwrap();
                let StageStep::Staged(wait) = durable.stage_writes(&[stays, list.clone()]) else {
                    panic!("{kind}: the State_Text write stages");
                };
                wait.block();
                assert_eq!(storage.saves(), 1, "{kind}");
                let kept = saved(&storage).algorithm;
                assert_eq!(kept.alarm_values, None, "{kind}: the list staged");
                assert_eq!(kept.state_text.map(|texts| texts.len()), Some(5));
                object.durable_writes().unwrap().release_staged_write(&wait);
            }
        }
    }
    /// State_Text written whole as one label, which arrives as that
    /// CharacterString alone, leaves one state. A list naming state 2 after
    /// it in the same request is checked against that one state: refused at
    /// the stage, so the save ahead of the writes holds the label alone,
    /// and once the object refuses the list, storage holds what it serves.
    #[test]
    fn a_single_label_leaves_one_state_for_the_staged_list() {
        let label = pending(P::STATE_TEXT, PropertyValue::CharacterString("only".into()));
        let request = [label, pending(P::ALARM_VALUES, states(&[2]))];
        on_each_listed!(|new, Object| {
            let storage = MemoryConfig::new();
            // No alarm state configured, so the object takes the one label.
            let mut object: Object = new(STATES)
                .with_reporting_persistence(persistence(&storage))
                .unwrap();
            let durable = object.durable_writes().unwrap();
            let StageStep::Staged(wait) = durable.stage_writes(&request) else {
                panic!("the label stages");
            };
            wait.block();
            assert_eq!(storage.saves(), 1);
            let only = Some(vec![String::from("only")]);
            assert_eq!(saved(&storage).algorithm.state_text, only);
            assert_eq!(saved(&storage).algorithm.alarm_values, None);
            let [label, list] = request.clone().map(|write| write.value);
            object
                .write_property(P::STATE_TEXT, None, label, None)
                .unwrap();
            assert_eq!(
                read(&object, P::NUMBER_OF_STATES),
                PropertyValue::Unsigned(1)
            );
            assert!(
                object
                    .write_property(P::ALARM_VALUES, None, list, None)
                    .is_err()
            );
            object.durable_writes().unwrap().release_staged_write(&wait);
            assert_eq!(storage.saves(), 1);
            assert_eq!(saved(&storage).algorithm.alarm_values, None);
            // One state does take a list naming it, staged with the label.
            let one = [request[0].clone(), pending(P::ALARM_VALUES, states(&[1]))];
            let durable = object.durable_writes().unwrap();
            let StageStep::Staged(wait) = durable.stage_writes(&one) else {
                panic!("a list within the one state stages");
            };
            wait.block();
            object.durable_writes().unwrap().release_staged_write(&wait);
        });
    }
}
