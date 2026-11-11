//! A multi-state object's written State_Text, and with it its state count,
//! kept with its reporting settings (#2414). Each way of writing it is
//! kept and comes back on a rebuild over the application's count; the
//! setters that size the states give way to it until the clear; storage
//! refuses a label or a count past its caps with the write; storage holding
//! one no write leaves fails the build; and a restore leaves the object as
//! a write of the same labels would.

use alloc::sync::Arc;

use bacnet_types::enums::{
    ErrorClass, ErrorCode, ObjectType, PropertyIdentifier as P, Reliability,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::parameters_tests::{holding, read, saved, write};
use super::reporting_tests::{persistence, refused_with};
use super::state_parameters_tests::{STATES, labels, written_texts};
use super::test_storage::MemoryConfig;
use super::{AlgorithmWriteConfig, ReportingWriteConfig};
use crate::multistate::{
    MAX_KEPT_STATE_TEXT_OCTETS, MAX_NUMBER_OF_STATES, MultiStateInputObject,
    MultiStateOutputObject, MultiStateValueObject,
};
use crate::traits::BACnetObject;

/// Run `$body` on each multi-state type: `$new` builds object 1 of it with
/// so many states, `$Object` names the type.
macro_rules! on_each {
    (|$new:ident, $Object:ident| $body:block) => {{
        {
            type $Object = MultiStateInputObject;
            let $new = |states: u32| $Object::new(1, "MSI", states).unwrap();
            $body
        }
        {
            type $Object = MultiStateOutputObject;
            let $new = |states: u32| $Object::new(1, "MSO", states).unwrap();
            $body
        }
        {
            type $Object = MultiStateValueObject;
            let $new = |states: u32| $Object::new(1, "MSV", states).unwrap();
            $body
        }
    }};
}

fn text(label: &str) -> PropertyValue {
    PropertyValue::CharacterString(label.into())
}

fn count(states: u32) -> PropertyValue {
    PropertyValue::Unsigned(u64::from(states))
}

/// The default labels of states 1 to `states`, as a new object has them.
fn defaults(states: u32) -> Vec<String> {
    (1..=states).map(|state| format!("State {state}")).collect()
}

/// State_Text written whole, its size at index 0 and one label are each
/// saved once, before the write is answered, and an object built on the
/// storage at the next start, with the application's count, serves the
/// labels and the count they give it. A Multi-state Output keeps them as
/// the Input and the Value do.
#[test]
fn each_way_of_writing_state_text_is_kept_over_the_configured_count() {
    let grown = defaults(STATES + 2);
    let mut relabelled = defaults(STATES);
    relabelled[1] = "Low".into();
    let writes = [
        (None, labels(&written_texts()), written_texts()),
        (Some(0), count(STATES + 2), grown),
        (Some(2), text("Low"), relabelled),
    ];
    on_each!(|new, Object| {
        for (index, value, kept) in writes.clone() {
            let storage = MemoryConfig::new();
            let mut object: Object = new(STATES)
                .with_reporting_persistence(persistence(&storage))
                .unwrap();
            object
                .write_property(P::STATE_TEXT, index, value, None)
                .unwrap();
            assert_eq!(storage.saves(), 1, "{index:?}");
            assert_eq!(saved(&storage).algorithm.state_text, Some(kept.clone()));
            drop(object);
            let rebuilt = new(STATES)
                .with_reporting_persistence(persistence(&storage))
                .unwrap();
            assert_eq!(read(&rebuilt, P::STATE_TEXT), labels(&kept), "{index:?}");
            let states = u32::try_from(kept.len()).unwrap();
            assert_eq!(read(&rebuilt, P::NUMBER_OF_STATES), count(states));
            assert_eq!(rebuilt.written_reporting().algorithm.state_text, Some(kept));
        }
    });
}

/// A saved State_Text wins over `set_number_of_states`, on the object that
/// took the write and on one rebuilt on its storage: the setter still
/// refuses zero, and otherwise leaves the labels and the count. A failed
/// clear keeps it so. Once the clear has saved, the setter applies again,
/// and the next start has the application's count and labels. Without
/// storage, or before a write, the setter applies as it always has.
#[test]
fn a_saved_state_text_wins_over_the_count_setter_until_the_clear() {
    on_each!(|new, Object| {
        let storage = MemoryConfig::new();
        let attach = |object: Object| {
            object
                .with_reporting_persistence(persistence(&storage))
                .unwrap()
        };
        let mut plain = new(STATES);
        plain.set_number_of_states(2).unwrap();
        assert_eq!(read(&plain, P::NUMBER_OF_STATES), count(2));

        let mut object = attach(new(STATES));
        object.set_number_of_states(STATES + 1).unwrap();
        assert_eq!(read(&object, P::NUMBER_OF_STATES), count(STATES + 1));
        assert_eq!(storage.saves(), 0, "a setter saves nothing");
        write(&mut object, P::STATE_TEXT, labels(&written_texts()));
        object.set_number_of_states(2).unwrap();
        assert!(object.set_number_of_states(0).is_err());
        assert_eq!(read(&object, P::STATE_TEXT), labels(&written_texts()));
        drop(object);

        let mut rebuilt = attach(new(STATES));
        rebuilt.set_number_of_states(2).unwrap();
        assert_eq!(read(&rebuilt, P::STATE_TEXT), labels(&written_texts()));
        storage.fail(true);
        assert!(rebuilt.clear_written_reporting().is_err());
        rebuilt.set_number_of_states(2).unwrap();
        let kept = count(STATES + 1);
        assert_eq!(read(&rebuilt, P::NUMBER_OF_STATES), kept, "kept on");
        storage.fail(false);
        rebuilt.clear_written_reporting().unwrap();
        assert_eq!(rebuilt.written_reporting(), ReportingWriteConfig::default());
        // The labels served stay until a setter or a write changes them.
        assert_eq!(read(&rebuilt, P::STATE_TEXT), labels(&written_texts()));
        rebuilt.set_number_of_states(2).unwrap();
        assert_eq!(read(&rebuilt, P::NUMBER_OF_STATES), count(2));
        drop(rebuilt);
        let restarted = attach(new(STATES));
        assert_eq!(read(&restarted, P::STATE_TEXT), labels(&defaults(STATES)));
        assert_eq!(read(&restarted, P::NUMBER_OF_STATES), count(STATES));
    });
}

/// A saved State_Text wins over `set_state_texts` (#2445) as it does over
/// the count setter: the setter still refuses no labels at all, and
/// otherwise leaves the saved labels and count, until the clear. Without
/// storage, or before a write, it applies.
#[test]
fn a_saved_state_text_wins_over_the_labels_setter_until_the_clear() {
    let mine: Vec<String> = ["Closed", "Open"].map(String::from).to_vec();
    on_each!(|new, Object| {
        let storage = MemoryConfig::new();
        let mut plain = new(STATES);
        plain.set_state_texts(mine.clone()).unwrap();
        assert_eq!(read(&plain, P::STATE_TEXT), labels(&mine));

        let mut object: Object = new(STATES)
            .with_reporting_persistence(persistence(&storage))
            .unwrap();
        object.set_state_texts(mine.clone()).unwrap();
        assert_eq!(read(&object, P::STATE_TEXT), labels(&mine));
        assert_eq!(storage.saves(), 0, "a setter saves nothing");
        write(&mut object, P::STATE_TEXT, labels(&written_texts()));
        object.set_state_texts(mine.clone()).unwrap();
        assert!(object.set_state_texts(Vec::new()).is_err());
        assert_eq!(read(&object, P::STATE_TEXT), labels(&written_texts()));
        let saved_count = count(u32::try_from(written_texts().len()).unwrap());
        assert_eq!(read(&object, P::NUMBER_OF_STATES), saved_count);
        object.clear_written_reporting().unwrap();
        object.set_state_texts(mine.clone()).unwrap();
        assert_eq!(read(&object, P::STATE_TEXT), labels(&mine));
        assert_eq!(read(&object, P::NUMBER_OF_STATES), count(2));
    });
}

/// With storage, `set_state_texts` refuses labels storage can't keep as a
/// write of them is refused (#2445), and nothing changes, so a peer's later
/// write of one label still goes through. Without storage it takes them.
#[test]
fn storage_refuses_labels_past_its_cap_with_the_setter() {
    let long = "é".repeat(MAX_KEPT_STATE_TEXT_OCTETS / 2) + "x";
    let mut texts = defaults(STATES);
    texts[0] = long;
    on_each!(|new, Object| {
        let storage = MemoryConfig::new();
        let mut object: Object = new(STATES)
            .with_reporting_persistence(persistence(&storage))
            .unwrap();
        let before = read(&object, P::STATE_TEXT);
        let result = object.set_state_texts(texts.clone());
        assert!(
            refused_with(
                &result,
                ErrorClass::RESOURCES,
                ErrorCode::NO_SPACE_TO_WRITE_PROPERTY
            ),
            "{result:?}"
        );
        assert_eq!(read(&object, P::STATE_TEXT), before);
        object
            .write_property(P::STATE_TEXT, Some(2), text("Low"), None)
            .unwrap();
        let mut plain = new(STATES);
        plain.set_state_texts(texts.clone()).unwrap();
        assert_eq!(read(&plain, P::STATE_TEXT), labels(&texts));
    });
}

/// With storage, a write that would leave a label past 512 octets, or more
/// than 1,024 states, is refused with RESOURCES /
/// NO_SPACE_TO_WRITE_PROPERTY before anything is saved: a label written
/// whole or alone, and any write to an object the application built with
/// more states than storage keeps. A label at the cap is taken, in octets
/// of UTF-8, not characters. Without storage the same writes are taken.
#[test]
fn storage_refuses_a_state_text_past_its_caps_with_the_write() {
    let long = "é".repeat(MAX_KEPT_STATE_TEXT_OCTETS / 2) + "x";
    let at_cap = "é".repeat(MAX_KEPT_STATE_TEXT_OCTETS / 2);
    on_each!(|new, Object| {
        let past: [(u32, Option<u32>, PropertyValue); 4] = [
            (STATES, Some(1), text(&long)),
            (STATES, None, labels(&[long.clone(), "b".into()])),
            (MAX_NUMBER_OF_STATES + 1, Some(1), text("one")),
            (
                MAX_NUMBER_OF_STATES + 1,
                None,
                labels(&defaults(MAX_NUMBER_OF_STATES + 1)),
            ),
        ];
        for (states, index, value) in past {
            let storage = MemoryConfig::new();
            let mut object: Object = new(states)
                .with_reporting_persistence(persistence(&storage))
                .unwrap();
            let before = read(&object, P::STATE_TEXT);
            let result = object.write_property(P::STATE_TEXT, index, value.clone(), None);
            assert!(
                refused_with(
                    &result,
                    ErrorClass::RESOURCES,
                    ErrorCode::NO_SPACE_TO_WRITE_PROPERTY
                ),
                "{states} {index:?}: {result:?}"
            );
            assert_eq!(read(&object, P::STATE_TEXT), before);
            assert_eq!(storage.saves(), 0);
            let mut plain = new(states);
            plain
                .write_property(P::STATE_TEXT, index, value, None)
                .unwrap();
        }
        let storage = MemoryConfig::new();
        let mut object: Object = new(STATES)
            .with_reporting_persistence(persistence(&storage))
            .unwrap();
        write(
            &mut object,
            P::STATE_TEXT,
            labels(core::slice::from_ref(&at_cap)),
        );
        drop(object);
        let rebuilt = new(STATES)
            .with_reporting_persistence(persistence(&storage))
            .unwrap();
        assert_eq!(
            read(&rebuilt, P::STATE_TEXT),
            labels(core::slice::from_ref(&at_cap))
        );
    });
}

/// What a write leaves storage holding always reopens, at the edges of what
/// a write takes: one state, every state an object has with each label at
/// its cap, labels past ASCII and empty ones.
#[test]
fn every_state_text_a_write_saves_reopens() {
    let full: Vec<String> = (0..MAX_NUMBER_OF_STATES)
        .map(|state| format!("{state:04}").repeat(MAX_KEPT_STATE_TEXT_OCTETS / 4))
        .collect();
    let cases = [
        vec![String::new()],
        vec!["Ein".into(), String::new(), "ζ ✓ 🌡".into()],
        full,
    ];
    on_each!(|new, Object| {
        for texts in &cases {
            let storage = MemoryConfig::new();
            let mut object: Object = new(STATES)
                .with_reporting_persistence(persistence(&storage))
                .unwrap();
            write(&mut object, P::STATE_TEXT, labels(texts));
            drop(object);
            let rebuilt = new(STATES)
                .with_reporting_persistence(persistence(&storage))
                .unwrap();
            assert_eq!(read(&rebuilt, P::STATE_TEXT), labels(texts));
        }
    });
}

/// Storage holding a State_Text no write leaves (no label, more than 1,024
/// labels, a label past its cap) fails the build, on each type; a
/// non-multi-state type holding one at all does too.
#[test]
fn a_stored_state_text_no_write_leaves_fails_the_build() {
    let stored = |texts: Vec<String>| {
        holding(AlgorithmWriteConfig {
            state_text: Some(texts),
            ..Default::default()
        })
    };
    let refused = [
        vec![],
        vec!["x".into(); MAX_NUMBER_OF_STATES as usize + 1],
        vec!["x".repeat(MAX_KEPT_STATE_TEXT_OCTETS + 1)],
    ];
    on_each!(|new, Object| {
        let _: fn(u32) -> Object = new;
        for texts in &refused {
            let built = new(STATES).with_reporting_persistence(persistence(&stored(texts.clone())));
            assert!(matches!(built, Err(Error::Encoding(_))), "{}", texts.len());
        }
    });
    let binary = crate::binary::BinaryValueObject::new(1, "BV")
        .unwrap()
        .with_reporting_persistence(persistence(&stored(written_texts())));
    assert!(matches!(binary, Err(Error::Encoding(_))));
}

/// A restore goes on a fresh object, whose Present_Value, Relinquish_Default
/// and commands are state 1, so no restored count strands them and the
/// object reports no fault. What the application set before the builder
/// call is held as a local shrink leaves it, and Reliability is worked out
/// again, as after a write: a Multi-state Value whose default names a state
/// past the restored count reports CONFIGURATION_ERROR, and a Multi-state
/// Input whose value is past it MULTI_STATE_OUT_OF_RANGE.
#[test]
fn a_restore_strands_nothing_on_a_fresh_object() {
    let one_label = || {
        MemoryConfig::holding(super::WriteConfigSnapshot::Reporting(
            ReportingWriteConfig {
                algorithm: AlgorithmWriteConfig {
                    state_text: Some(vec!["Only".into()]),
                    ..Default::default()
                },
                ..Default::default()
            },
        ))
    };
    let sound = PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw());
    on_each!(|new, Object| {
        let fresh: Object = new(STATES)
            .with_reporting_persistence(persistence(&one_label()))
            .unwrap();
        assert_eq!(read(&fresh, P::NUMBER_OF_STATES), count(1));
        assert_eq!(read(&fresh, P::PRESENT_VALUE), count(1));
        assert_eq!(read(&fresh, P::RELIABILITY), sound);
    });
    let mut value = MultiStateValueObject::new(1, "MSV", STATES).unwrap();
    value.set_relinquish_default(3).unwrap();
    let value = value
        .with_reporting_persistence(persistence(&one_label()))
        .unwrap();
    let fault = PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw());
    assert_eq!(read(&value, P::RELIABILITY), fault);
    let mut input = MultiStateInputObject::new(1, "MSI", STATES).unwrap();
    input.set_present_value(3);
    let input = input
        .with_reporting_persistence(persistence(&one_label()))
        .unwrap();
    let out = PropertyValue::Enumerated(Reliability::MULTI_STATE_OUT_OF_RANGE.to_raw());
    assert_eq!(read(&input, P::RELIABILITY), out);
}

/// A State_Text write whose save fails is refused with DEVICE /
/// OPERATIONAL_PROBLEM on each way of writing it, and the labels and count
/// stay; so does the state count an Alarm_Values entry is checked against.
#[test]
fn a_state_text_write_whose_save_fails_keeps_the_labels_and_count() {
    on_each!(|new, Object| {
        let storage = MemoryConfig::new();
        let mut object: Object = new(STATES)
            .with_reporting_persistence(persistence(&storage))
            .unwrap();
        storage.fail(true);
        for (index, value) in [
            (None, labels(&written_texts())),
            (Some(0), count(STATES + 1)),
            (Some(1), text("Off")),
        ] {
            let result = object.write_property(P::STATE_TEXT, index, value, None);
            assert!(
                refused_with(&result, ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM),
                "{index:?}: {result:?}"
            );
        }
        assert_eq!(read(&object, P::STATE_TEXT), labels(&defaults(STATES)));
        assert_eq!(read(&object, P::NUMBER_OF_STATES), count(STATES));
        assert_eq!(storage.saved(), None);
        let kind = object.object_identifier().object_type();
        if kind != ObjectType::MULTI_STATE_OUTPUT {
            let list = PropertyValue::List(vec![count(STATES + 1)]);
            storage.fail(false);
            let result = object.write_property(P::ALARM_VALUES, None, list, None);
            let refusal = format!("{result:?}");
            assert!(refusal.contains("VALUE_OUT_OF_RANGE"), "{kind}: {refusal}");
        }
    });
}

/// Only what a write set is kept: an object whose other rows a write saved
/// keeps no State_Text, and its count stays the application's.
#[test]
fn a_write_of_another_row_keeps_no_state_text() {
    let storage: Arc<MemoryConfig> = MemoryConfig::new();
    let mut object = MultiStateInputObject::new(1, "MSI", STATES)
        .unwrap()
        .with_reporting_persistence(persistence(&storage))
        .unwrap();
    write(&mut object, P::TIME_DELAY, PropertyValue::Unsigned(4));
    assert_eq!(saved(&storage).algorithm.state_text, None);
    let object: &dyn BACnetObject = &object;
    assert_eq!(read(object, P::NUMBER_OF_STATES), count(STATES));
}

/// Run `$body` on a Multi-state Input and a Multi-state Value: `$new`
/// builds object 1 of the type with so many states.
macro_rules! on_both_listed {
    (|$new:ident| $body:block) => {{
        {
            let $new = |states: u32| MultiStateInputObject::new(1, "MSI", states).unwrap();
            $body
        }
        {
            let $new = |states: u32| MultiStateValueObject::new(1, "MSV", states).unwrap();
            $body
        }
    }};
}

/// The class and code of a refusal, whether or not it names an element.
fn class_and_code(result: &Result<(), Error>) -> Option<(ErrorClass, ErrorCode)> {
    match result {
        Err(Error::Structured { class, code, .. }) => Some((*class, *code)),
        Err(Error::Remote(bacnet_types::error::RemoteError::Error { class, code })) => {
            Some((*class, *code))
        }
        _ => None,
    }
}

fn list(states: &[u32]) -> PropertyValue {
    PropertyValue::List(states.iter().map(|state| count(*state)).collect())
}

/// The application configures `[5]` on five states. A peer writes an empty
/// Alarm_Values and then shrinks State_Text to three states, both kept. At
/// the next start the application's `set_alarm_values(vec![5])`, after the
/// builder call, returns `Ok` and leaves the saved list: the saved list
/// wins, so the count the argument would meet doesn't matter. A zero or an
/// over-long list is still refused. Before the builder call, on the
/// application's five states, the list is taken as configured.
#[test]
fn set_alarm_values_gives_way_to_a_saved_list_before_checking_the_count() {
    on_both_listed!(|new| {
        let storage = MemoryConfig::new();
        let mut object = new(STATES);
        object.set_alarm_values(vec![STATES]).unwrap();
        let mut object = object
            .with_reporting_persistence(persistence(&storage))
            .unwrap();
        write(&mut object, P::ALARM_VALUES, list(&[]));
        write(&mut object, P::STATE_TEXT, labels(&defaults(3)));
        drop(object);

        let mut rebuilt = new(STATES);
        rebuilt.set_alarm_values(vec![STATES]).unwrap();
        let mut rebuilt = rebuilt
            .with_reporting_persistence(persistence(&storage))
            .unwrap();
        assert_eq!(read(&rebuilt, P::NUMBER_OF_STATES), count(3));
        rebuilt.set_alarm_values(vec![STATES]).unwrap();
        assert_eq!(read(&rebuilt, P::ALARM_VALUES), list(&[]));
        let zero = rebuilt.set_alarm_values(vec![0]);
        assert_eq!(
            class_and_code(&zero),
            Some((ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE)),
            "{zero:?}"
        );
        let long = vec![1; crate::multistate::MAX_ALARM_VALUES + 1];
        let long = rebuilt.set_alarm_values(long);
        assert_eq!(
            class_and_code(&long),
            Some((ErrorClass::RESOURCES, ErrorCode::NO_SPACE_TO_WRITE_PROPERTY)),
            "{long:?}"
        );
        assert_eq!(read(&rebuilt, P::ALARM_VALUES), list(&[]));
    });
}

/// With no list saved, a restored State_Text that gave the object fewer
/// states makes `set_alarm_values` after the builder call refuse a state
/// past them, as a write of the list is refused: that answer is true, and
/// the caller handles it. The same holds for `set_relinquish_default` on
/// the Output and the Value, which nothing saved stands in for.
#[test]
fn after_a_restored_count_the_setters_check_against_it() {
    let shrunk = || {
        holding(AlgorithmWriteConfig {
            state_text: Some(defaults(3)),
            ..Default::default()
        })
    };
    on_both_listed!(|new| {
        let mut object = new(STATES)
            .with_reporting_persistence(persistence(&shrunk()))
            .unwrap();
        let result = object.set_alarm_values(vec![STATES]);
        assert!(
            format!("{result:?}").contains("VALUE_OUT_OF_RANGE"),
            "{result:?}"
        );
        object.set_alarm_values(vec![3]).unwrap();
    });
    let mut output = MultiStateOutputObject::new(1, "MSO", STATES)
        .unwrap()
        .with_reporting_persistence(persistence(&shrunk()))
        .unwrap();
    let result = output.set_relinquish_default(STATES);
    assert!(refused_with(
        &result,
        ErrorClass::PROPERTY,
        ErrorCode::VALUE_OUT_OF_RANGE
    ));
    output.set_relinquish_default(3).unwrap();
    let mut value = MultiStateValueObject::new(1, "MSV", STATES)
        .unwrap()
        .with_reporting_persistence(persistence(&shrunk()))
        .unwrap();
    let result = value.set_relinquish_default(STATES);
    assert!(refused_with(
        &result,
        ErrorClass::PROPERTY,
        ErrorCode::VALUE_OUT_OF_RANGE
    ));
}

#[cfg(feature = "durable-writes")]
mod staged {
    use super::*;
    use crate::durable::{PendingWrite, StageStep};

    /// An object that goes away part way through a staged request, with
    /// its first write made and its second not, leaves storage holding what
    /// it served: the first write's labels, and not the second write's
    /// list, which the staged save held.
    #[test]
    fn an_object_dropped_mid_request_leaves_storage_with_the_writes_it_made() {
        on_both_listed!(|new| {
            let storage = MemoryConfig::new();
            let mut object = new(STATES)
                .with_reporting_persistence(persistence(&storage))
                .unwrap();
            let grown = count(STATES + 1);
            let request = [
                PendingWrite {
                    property: P::STATE_TEXT,
                    array_index: Some(0),
                    value: grown.clone(),
                },
                PendingWrite {
                    property: P::ALARM_VALUES,
                    array_index: None,
                    value: list(&[STATES + 1]),
                },
            ];
            let durable = object.durable_writes().unwrap();
            let StageStep::Staged(wait) = durable.stage_writes(&request) else {
                panic!("the request stages");
            };
            wait.block();
            assert_eq!(
                saved(&storage).algorithm.alarm_values,
                Some(vec![STATES + 1])
            );
            object
                .write_property(P::STATE_TEXT, Some(0), grown.clone(), None)
                .unwrap();
            drop(object);
            let kept = saved(&storage).algorithm;
            assert_eq!(kept.alarm_values, None, "the write never made is kept");
            assert_eq!(kept.state_text, Some(defaults(STATES + 1)));
        });
    }
}
