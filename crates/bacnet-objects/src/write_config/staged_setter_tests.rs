//! What lands between a request's stage and its commit (#2431). A
//! multi-state object's `set_number_of_states` or `set_state_texts`
//! changes the labels a staged State_Text element or size write keeps the
//! rest of, and the server's inhibit follower changes a written
//! Event_Algorithm_Inhibit the staged rows carry: the commit then saves the
//! rows the object serves, and a rebuild serves them too. A setter that
//! changes only what a write is checked against makes the commit refuse
//! it, and its release puts storage back. A change the staged rows don't
//! hold, or one after the commit, saves nothing more. A change only a
//! request's earlier step sees is caught at its last, a stale stage whose
//! save in place fails refuses the write, and the labels come back from a
//! real file too.

use alloc::sync::Arc;

use bacnet_encoding::constructed::encode_object_property_reference;
use bacnet_types::constructed::{BACnetEventParameter, BACnetObjectPropertyReference};
use bacnet_types::enums::{
    EngineeringUnits, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier as P,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;

use super::enrollment_parameters_tests::{event_value, open};
use super::parameters_tests::{read, saved, write};
use super::reporting_tests::{persistence, refused_with};
use super::state_parameters_tests::labels;
use super::test_storage::MemoryConfig;
use crate::analog::AnalogInputObject;
use crate::binary::BinaryValueObject;
use crate::database::ObjectDatabase;
use crate::durable::{PendingWrite, StageStep};
use crate::multistate::{MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject};
use crate::traits::BACnetObject;

/// The states each object starts with.
const STATES: u32 = 5;

/// Run `$body` on each multi-state type: `$new` builds object 1 of it on
/// `storage` with so many states, then `$body` runs with `$Object` naming
/// the type.
macro_rules! on_each {
    (|$new:ident, $Object:ident| $body:block) => {{
        {
            type $Object = MultiStateInputObject;
            let $new = |states: u32, storage: &Arc<MemoryConfig>| {
                $Object::new(1, "MSI", states)
                    .unwrap()
                    .with_reporting_persistence(persistence(storage))
                    .unwrap()
            };
            $body
        }
        {
            type $Object = MultiStateOutputObject;
            let $new = |states: u32, storage: &Arc<MemoryConfig>| {
                $Object::new(1, "MSO", states)
                    .unwrap()
                    .with_reporting_persistence(persistence(storage))
                    .unwrap()
            };
            $body
        }
        {
            type $Object = MultiStateValueObject;
            let $new = |states: u32, storage: &Arc<MemoryConfig>| {
                $Object::new(1, "MSV", states)
                    .unwrap()
                    .with_reporting_persistence(persistence(storage))
                    .unwrap()
            };
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

fn pending(property: P, array_index: Option<u32>, value: PropertyValue) -> PendingWrite {
    PendingWrite {
        property,
        array_index,
        value,
    }
}

/// The labels of states 1 to `states` a new object has, with `first` as
/// state 1's.
fn relabelled(first: &str, states: u32) -> Vec<String> {
    let mut texts: Vec<String> = (1..=states).map(|state| format!("State {state}")).collect();
    texts[0] = first.into();
    texts
}

/// The server's order: stage `request` on `object` and wait for its save,
/// let `meanwhile` run as the database guard is free again, then make each
/// write and release the stage. The first refusal ends the request.
fn commit<T: BACnetObject>(
    object: &mut T,
    request: &[PendingWrite],
    meanwhile: impl FnOnce(&mut T),
) -> Result<(), Error> {
    let StageStep::Staged(wait) = object.durable_writes().unwrap().stage_writes(request) else {
        panic!("the request stages");
    };
    wait.block();
    meanwhile(object);
    let made = request.iter().try_for_each(|write| {
        object.write_property(write.property, write.array_index, write.value.clone(), None)
    });
    object.durable_writes().unwrap().release_staged_write(&wait);
    made
}

/// The reviewer's probe: a State_Text element write staged on five states,
/// `set_number_of_states(3)` before its commit. The object serves three
/// states, the file keeps the same three labels, and a rebuild with five
/// serves them.
#[test]
fn a_count_set_before_the_commit_is_what_storage_keeps() {
    on_each!(|new, Object| {
        let storage = MemoryConfig::new();
        let mut object: Object = new(STATES, &storage);
        let request = [pending(P::STATE_TEXT, Some(1), text("Low"))];
        commit(&mut object, &request, |object| {
            object.set_number_of_states(3).unwrap();
        })
        .unwrap();
        object.wait_for_reporting_saves();
        let kept = relabelled("Low", 3);
        assert_eq!(read(&object, P::NUMBER_OF_STATES), count(3));
        assert_eq!(read(&object, P::STATE_TEXT), labels(&kept));
        assert_eq!(saved(&storage).algorithm.state_text, Some(kept.clone()));
        drop(object);
        let rebuilt: Object = new(STATES, &storage);
        assert_eq!(read(&rebuilt, P::NUMBER_OF_STATES), count(3));
        assert_eq!(read(&rebuilt, P::STATE_TEXT), labels(&kept));
    });
}

/// `set_state_texts` before the commit of a size write: the grown labels
/// start from the application's, in the object and the file alike, and
/// come back on a rebuild.
#[test]
fn labels_set_before_the_commit_are_what_storage_keeps() {
    on_each!(|new, Object| {
        let storage = MemoryConfig::new();
        let mut object: Object = new(STATES, &storage);
        let request = [pending(P::STATE_TEXT, Some(0), count(STATES + 1))];
        let application: Vec<String> = ["off", "low", "mid", "high"].map(String::from).into();
        commit(&mut object, &request, |object| {
            object.set_state_texts(application.clone()).unwrap();
        })
        .unwrap();
        object.wait_for_reporting_saves();
        let mut kept = application.clone();
        kept.extend(["State 5", "State 6"].map(String::from));
        assert_eq!(read(&object, P::STATE_TEXT), labels(&kept));
        assert_eq!(saved(&storage).algorithm.state_text, Some(kept.clone()));
        drop(object);
        let rebuilt: Object = new(STATES, &storage);
        assert_eq!(read(&rebuilt, P::NUMBER_OF_STATES), count(STATES + 1));
        assert_eq!(read(&rebuilt, P::STATE_TEXT), labels(&kept));
    });
}

/// A setter whose change the staged rows don't hold saves nothing more:
/// the count set before a Notification_Class write's commit, which keeps
/// no State_Text, and before a whole State_Text write's, which replaces
/// every label. Each request saves once, at its stage.
#[test]
fn a_setter_the_staged_rows_dont_hold_saves_nothing_more() {
    on_each!(|new, Object| {
        let storage = MemoryConfig::new();
        let mut object: Object = new(STATES, &storage);
        let request = [pending(P::NOTIFICATION_CLASS, None, count(9))];
        commit(&mut object, &request, |object| {
            object.set_number_of_states(3).unwrap();
        })
        .unwrap();
        object.wait_for_reporting_saves();
        assert_eq!(storage.saves(), 1);
        assert_eq!(saved(&storage).notification_class, Some(9));
        assert_eq!(saved(&storage).algorithm.state_text, None);
        assert_eq!(read(&object, P::NUMBER_OF_STATES), count(3));

        let whole: Vec<String> = ["a", "b", "c", "d"].map(String::from).into();
        let request = [pending(P::STATE_TEXT, None, labels(&whole))];
        commit(&mut object, &request, |object| {
            object.set_number_of_states(2).unwrap();
        })
        .unwrap();
        object.wait_for_reporting_saves();
        assert_eq!(storage.saves(), 2);
        assert_eq!(saved(&storage).algorithm.state_text, Some(whole.clone()));
        assert_eq!(read(&object, P::STATE_TEXT), labels(&whole));
    });
}

/// A setter after the commit finds the State_Text written and gives way:
/// nothing changes and nothing more is saved.
#[test]
fn a_setter_after_the_commit_changes_nothing() {
    on_each!(|new, Object| {
        let storage = MemoryConfig::new();
        let mut object: Object = new(STATES, &storage);
        let request = [pending(P::STATE_TEXT, Some(1), text("Low"))];
        commit(&mut object, &request, |_| {}).unwrap();
        object.set_number_of_states(3).unwrap();
        object.set_state_texts(relabelled("x", 2)).unwrap();
        object.wait_for_reporting_saves();
        let kept = relabelled("Low", STATES);
        assert_eq!(storage.saves(), 1);
        assert_eq!(read(&object, P::STATE_TEXT), labels(&kept));
        assert_eq!(saved(&storage).algorithm.state_text, Some(kept));
    });
}

/// What a write is only checked against, changed before its commit: a
/// state a staged shrink would strand, held by Alarm_Values the
/// application set on the Input and Relinquish_Default on the Output and
/// the Value. The commit refuses the shrink, its release puts storage back
/// to what the object serves, and a rebuild has the application's states.
#[test]
fn a_change_a_staged_write_is_checked_against_refuses_it_and_storage_goes_back() {
    let request = [pending(P::STATE_TEXT, Some(0), count(3))];
    macro_rules! refused_then_rebuilt {
        ($object:expr, $storage:expr, $meanwhile:expr, $rebuild:expr) => {{
            let mut object = $object;
            let result = commit(&mut object, &request, $meanwhile);
            assert!(
                format!("{result:?}").contains("VALUE_OUT_OF_RANGE"),
                "{result:?}"
            );
            object.wait_for_reporting_saves();
            assert_eq!(saved(&$storage).algorithm.state_text, None);
            drop(object);
            assert_eq!(read(&$rebuild, P::NUMBER_OF_STATES), count(STATES));
        }};
    }
    let storage = MemoryConfig::new();
    let input = || {
        MultiStateInputObject::new(1, "MSI", STATES)
            .unwrap()
            .with_reporting_persistence(persistence(&storage))
            .unwrap()
    };
    refused_then_rebuilt!(
        input(),
        storage,
        |object: &mut MultiStateInputObject| object.set_alarm_values(vec![STATES]).unwrap(),
        input()
    );
    let storage = MemoryConfig::new();
    let output = || {
        MultiStateOutputObject::new(1, "MSO", STATES)
            .unwrap()
            .with_reporting_persistence(persistence(&storage))
            .unwrap()
    };
    refused_then_rebuilt!(
        output(),
        storage,
        |object: &mut MultiStateOutputObject| object.set_relinquish_default(STATES).unwrap(),
        output()
    );
    let storage = MemoryConfig::new();
    let value = || {
        MultiStateValueObject::new(1, "MSV", STATES)
            .unwrap()
            .with_reporting_persistence(persistence(&storage))
            .unwrap()
    };
    refused_then_rebuilt!(
        value(),
        storage,
        |object: &mut MultiStateValueObject| object.set_relinquish_default(STATES).unwrap(),
        value()
    );
}

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// The server's inhibit follower is the change a running object takes
/// between a stage and its commit through the public API. An Analog Input
/// whose Event_Algorithm_Inhibit a write set keeps the followed value with
/// its rows; a follow before a Notification_Class write's commit leaves the
/// file holding the inhibit the object serves, and a rebuild serves it.
#[test]
fn the_inhibit_followed_before_the_commit_is_what_storage_keeps() {
    let storage = MemoryConfig::new();
    let ai = oid(ObjectType::ANALOG_INPUT, 1);
    let switch = oid(ObjectType::BINARY_VALUE, 1);
    let build = || {
        AnalogInputObject::new(1, "AI", EngineeringUnits::NO_UNITS)
            .unwrap()
            .with_reporting_persistence(persistence(&storage))
            .unwrap()
    };
    let mut db = ObjectDatabase::new();
    db.add(Box::new(build())).unwrap();
    db.add(Box::new(BinaryValueObject::new(1, "BV").unwrap()))
        .unwrap();
    let object = db.get_mut(&ai).unwrap();
    write(
        object,
        P::EVENT_ALGORITHM_INHIBIT,
        PropertyValue::Boolean(true),
    );
    let mut reference = BytesMut::new();
    encode_object_property_reference(
        &mut reference,
        &BACnetObjectPropertyReference::new(switch, P::PRESENT_VALUE.to_raw()),
    );
    let reference = PropertyValue::ApplicationData(reference.to_vec());
    write(object, P::EVENT_ALGORITHM_INHIBIT_REF, reference);
    // The switch is INACTIVE: the follower clears the written TRUE.
    assert!(db.follow_event_algorithm_inhibit(&ai));
    let request = [pending(P::NOTIFICATION_CLASS, None, count(9))];
    let object = db.get_mut(&ai).unwrap();
    let StageStep::Staged(wait) = object.durable_writes().unwrap().stage_writes(&request) else {
        panic!("the request stages");
    };
    wait.block();
    db.get_mut(&switch)
        .unwrap()
        .write_property_from(
            P::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(1),
            Some(16),
            &crate::command_source::test_origin(),
        )
        .unwrap();
    assert!(db.follow_event_algorithm_inhibit(&ai));
    let object = db.get_mut(&ai).unwrap();
    object
        .write_property(P::NOTIFICATION_CLASS, None, count(9), None)
        .unwrap();
    object.durable_writes().unwrap().release_staged_write(&wait);
    assert_eq!(
        read(object, P::EVENT_ALGORITHM_INHIBIT),
        PropertyValue::Boolean(true)
    );
    let kept = saved(&storage);
    assert_eq!(kept.notification_class, Some(9));
    assert_eq!(kept.event_algorithm_inhibit, Some(true));
    drop(db);
    let rebuilt = build();
    assert_eq!(
        read(&rebuilt, P::EVENT_ALGORITHM_INHIBIT),
        PropertyValue::Boolean(true)
    );
}

/// Saved rows holding a limit that is no number aren't equal to
/// themselves, so the commit checks their file octets too: a staged write
/// on an Event Enrollment whose kept Event_Parameters hold one takes its
/// step without saving again.
#[test]
fn rows_holding_a_limit_that_is_no_number_take_their_step() {
    let storage = MemoryConfig::new();
    let mut enrollment = open(Some(&storage));
    let parameters = BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: f32::NAN,
        high_limit: 50.0,
        deadband: 0.0,
    };
    write(
        &mut enrollment,
        P::EVENT_PARAMETERS,
        event_value(&parameters),
    );
    assert_eq!(storage.saves(), 1);
    let request = [pending(P::NOTIFICATION_CLASS, None, count(9))];
    commit(&mut enrollment, &request, |_| {}).unwrap();
    enrollment.wait_for_reporting_saves();
    assert_eq!(storage.saves(), 2, "the stage's save, and no other");
    assert_eq!(saved(&storage).notification_class, Some(9));
}

/// A request of two writes, a State_Text element and then
/// Notification_Class, with the count set before its commit: only the first
/// step reads the labels, and the last finds the stage stale, so the file
/// keeps the three labels and the class the object serves, and a rebuild
/// serves both. The same request with nothing in between saves once.
#[test]
fn a_change_only_an_earlier_step_sees_is_caught_at_the_last() {
    let request = [
        pending(P::STATE_TEXT, Some(1), text("Low")),
        pending(P::NOTIFICATION_CLASS, None, count(9)),
    ];
    on_each!(|new, Object| {
        let storage = MemoryConfig::new();
        let mut object: Object = new(STATES, &storage);
        commit(&mut object, &request, |object| {
            object.set_number_of_states(3).unwrap();
        })
        .unwrap();
        object.wait_for_reporting_saves();
        let kept = relabelled("Low", 3);
        assert_eq!(read(&object, P::STATE_TEXT), labels(&kept));
        assert_eq!(saved(&storage).algorithm.state_text, Some(kept.clone()));
        assert_eq!(saved(&storage).notification_class, Some(9));
        drop(object);
        let rebuilt: Object = new(STATES, &storage);
        assert_eq!(read(&rebuilt, P::STATE_TEXT), labels(&kept));
        assert_eq!(read(&rebuilt, P::NOTIFICATION_CLASS), count(9));

        let storage = MemoryConfig::new();
        let mut object: Object = new(STATES, &storage);
        commit(&mut object, &request, |_| {}).unwrap();
        object.wait_for_reporting_saves();
        assert_eq!(storage.saves(), 1);
        assert_eq!(
            saved(&storage).algorithm.state_text,
            Some(relabelled("Low", STATES))
        );
    });
}

/// A stale stage whose save in place fails: the write is refused with
/// DEVICE / OPERATIONAL_PROBLEM, and the object serves what it did before
/// it, the application's three labels, none of them written.
#[test]
fn a_stale_stage_whose_save_in_place_fails_refuses_the_write() {
    on_each!(|new, Object| {
        let storage = MemoryConfig::new();
        let mut object: Object = new(STATES, &storage);
        let request = [pending(P::STATE_TEXT, Some(1), text("Low"))];
        let result = commit(&mut object, &request, |object| {
            object.set_number_of_states(3).unwrap();
            storage.fail(true);
        });
        assert!(
            refused_with(&result, ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM),
            "{result:?}"
        );
        object.wait_for_reporting_saves();
        assert_eq!(read(&object, P::NUMBER_OF_STATES), count(3));
        assert_eq!(
            read(&object, P::STATE_TEXT),
            labels(&relabelled("State 1", 3))
        );
        assert_eq!(object.written_reporting().algorithm.state_text, None);
    });
}

/// The probe on the file backend: the stale stage's correction reaches the
/// file's octets, and an object built on the file at the next start serves
/// the three labels.
#[cfg(feature = "std")]
#[test]
fn the_corrected_labels_come_back_from_a_file() {
    use super::{FileWriteConfigPersistence, WriteConfigPersistence};

    let directory =
        std::env::temp_dir().join(format!("rusty-staged-setter-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("msv.bin");
    let open = || {
        let file: Arc<dyn WriteConfigPersistence> =
            Arc::new(FileWriteConfigPersistence::new(&path).unwrap());
        MultiStateValueObject::new(1, "MSV", STATES)
            .unwrap()
            .with_reporting_persistence(file)
            .unwrap()
    };
    let mut object = open();
    let request = [pending(P::STATE_TEXT, Some(1), text("Low"))];
    commit(&mut object, &request, |object| {
        object.set_number_of_states(3).unwrap();
    })
    .unwrap();
    drop(object);
    let kept = relabelled("Low", 3);
    let octets = std::fs::read(&path).unwrap();
    let oid = oid(ObjectType::MULTI_STATE_VALUE, 1);
    match crate::snapshot::write_config::decode(oid, &octets).unwrap() {
        super::WriteConfigSnapshot::Reporting(file) => {
            assert_eq!(file.algorithm.state_text, Some(kept.clone()));
        }
        other => panic!("the file holds {other:?}"),
    }
    let rebuilt = open();
    assert_eq!(read(&rebuilt, P::NUMBER_OF_STATES), count(3));
    assert_eq!(read(&rebuilt, P::STATE_TEXT), labels(&kept));
    drop(rebuilt);
    std::fs::remove_dir_all(&directory).unwrap();
}
