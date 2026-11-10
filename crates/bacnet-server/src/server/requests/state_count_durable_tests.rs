//! A multi-state object's written State_Text, and with it its state count,
//! kept with its Alarm_Values (#2414). A peer adds a state through
//! State_Text and lists it: the device restarts on the application's count
//! and the rebuilt object has the peer's, serves its labels and alarms on
//! the added state. One request doing both saves once, off the database
//! guard. Each way of writing State_Text saves, on the three types; a write
//! that can't be saved is refused and the old texts stay; a label past what
//! storage keeps is refused before anything is saved.
//!
//! DEVICE 0, PROPERTY 2, RESOURCES 3; NO_SPACE_TO_WRITE_PROPERTY 20,
//! OPERATIONAL_PROBLEM 25, VALUE_OUT_OF_RANGE 37.

use super::durable_write_wire_tests::{
    ERROR_PDU, SIMPLE_ACK_WPM, SIMPLE_ACK_WRITE, WAIT, while_saving,
};
use super::mutation_list_wire_tests::wire;
use super::mutation_tests::{Fixture, oid, value, wpm};
use super::reporting_durable_tests::{INSTANCE, served_by, write_property};
use super::write_config_durable_tests::ConfigStorage;
use super::*;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_types::enums::{EventState, Reliability};
use std::sync::atomic::Ordering;

use PropertyIdentifier as P;

/// The two types that keep Alarm_Values. The fixture builds each with
/// three states.
const LISTED: [ObjectType; 2] = [ObjectType::MULTI_STATE_INPUT, ObjectType::MULTI_STATE_VALUE];

/// The three types that keep State_Text.
const MULTI_STATE: [ObjectType; 3] = [
    ObjectType::MULTI_STATE_INPUT,
    ObjectType::MULTI_STATE_OUTPUT,
    ObjectType::MULTI_STATE_VALUE,
];

fn text(label: &str) -> PropertyValue {
    PropertyValue::CharacterString(label.into())
}

/// State_Text written whole with one more label than the fixture's three
/// states.
fn four_labels() -> PropertyValue {
    PropertyValue::List(["Off", "Low", "High", "Boost"].map(text).to_vec())
}

/// A list naming the fourth state and one the object is built with.
fn list() -> PropertyValue {
    PropertyValue::List([4, 2].map(PropertyValue::Unsigned).to_vec())
}

async fn write(
    fixture: &Fixture,
    kind: ObjectType,
    property: P,
    index: Option<u32>,
    written: PropertyValue,
) -> Vec<u8> {
    let request = write_property(kind, property, index, value(written));
    wire(fixture, ConfirmedServiceChoice::WRITE_PROPERTY, request).await
}

/// The error a WriteProperty answer carries, as its class and code octets.
fn refusal(response: &[u8]) -> (u8, u8) {
    assert_eq!(response[0], ERROR_PDU, "{response:?}");
    (response[4], response[6])
}

/// The Event_State the object takes once it holds `state`, written over
/// the wire: out of service on the Input, by command on the Value. The
/// server evaluates the object's intrinsic reporting after the write, and
/// the detector runs with no delay, so no time has to pass.
async fn alarm_on(fixture: &Fixture, kind: ObjectType, state: u64) -> PropertyValue {
    if kind == ObjectType::MULTI_STATE_INPUT {
        let out = PropertyValue::Boolean(true);
        let answer = write(fixture, kind, P::OUT_OF_SERVICE, None, out).await;
        assert_eq!(answer, SIMPLE_ACK_WRITE);
    }
    let state = PropertyValue::Unsigned(state);
    let answer = write(fixture, kind, P::PRESENT_VALUE, None, state).await;
    assert_eq!(answer, SIMPLE_ACK_WRITE);
    fixture.read(oid(kind, INSTANCE), P::EVENT_STATE).await
}

/// The peer sequence from #2414. A peer writes State_Text with one more
/// label, and then Alarm_Values naming the new state, each saved once.
/// The device restarts and the application builds the object with three
/// states again: the rebuilt object has the peer's four, serves its labels
/// and the list, reports no fault, and alarms on the fourth state. The
/// peer can write the list it reads back.
#[tokio::test(start_paused = true)]
async fn a_count_a_peer_raised_comes_back_with_the_list_naming_it() {
    for kind in LISTED {
        let object = oid(kind, INSTANCE);
        let storage = Arc::new(ConfigStorage::default());
        let fixture = served_by(kind, &storage).await;
        let grown = write(&fixture, kind, P::STATE_TEXT, None, four_labels()).await;
        assert_eq!(grown, SIMPLE_ACK_WRITE, "{kind:?}");
        let listed = write(&fixture, kind, P::ALARM_VALUES, None, list()).await;
        assert_eq!(listed, SIMPLE_ACK_WRITE, "{kind:?}");
        assert_eq!(storage.saves.load(Ordering::SeqCst), 2, "{kind:?}");
        drop(fixture);

        let rebuilt = served_by(kind, &storage).await;
        let four = PropertyValue::Unsigned(4);
        assert_eq!(rebuilt.read(object, P::NUMBER_OF_STATES).await, four);
        assert_eq!(rebuilt.read(object, P::STATE_TEXT).await, four_labels());
        assert_eq!(rebuilt.read(object, P::ALARM_VALUES).await, list());
        let sound = PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw());
        assert_eq!(
            rebuilt.read(object, P::RELIABILITY).await,
            sound,
            "{kind:?}"
        );
        let again = write(&rebuilt, kind, P::ALARM_VALUES, None, list()).await;
        assert_eq!(again, SIMPLE_ACK_WRITE, "{kind:?}");
        let offnormal = PropertyValue::Enumerated(EventState::OFFNORMAL.to_raw());
        assert_eq!(alarm_on(&rebuilt, kind, 4).await, offnormal, "{kind:?}");
    }
}

/// State_Text written whole, its size at index 0 and one label at a time
/// each save once, off the guard on WriteProperty too, on the three types,
/// and an object rebuilt on the storage serves them with their count.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_state_text_write_saves_and_survives_a_rebuild() {
    let writes = [
        (None, four_labels(), 4),
        (Some(0), PropertyValue::Unsigned(5), 5),
        (Some(2), text("Low"), 3),
    ];
    for kind in MULTI_STATE {
        for (index, written, count) in writes.clone() {
            let object = oid(kind, INSTANCE);
            let storage = Arc::new(ConfigStorage::default());
            let fixture = served_by(kind, &storage).await;
            let request = write_property(kind, P::STATE_TEXT, index, value(written));
            let service = ConfirmedServiceChoice::WRITE_PROPERTY;
            let (response, available) =
                while_saving(&fixture, &storage, service, request, WAIT).await;
            assert_eq!(response, SIMPLE_ACK_WRITE, "{kind:?} {index:?}");
            assert!(available, "{kind:?} {index:?}: saved under the guard");
            assert_eq!(storage.saves.load(Ordering::SeqCst), 1, "{kind:?}");
            let served = fixture.read(object, P::STATE_TEXT).await;
            drop(fixture);
            let rebuilt = served_by(kind, &storage).await;
            assert_eq!(rebuilt.read(object, P::STATE_TEXT).await, served);
            let count = PropertyValue::Unsigned(count);
            assert_eq!(rebuilt.read(object, P::NUMBER_OF_STATES).await, count);
        }
    }
}

/// A State_Text write whose save fails is answered with DEVICE /
/// OPERATIONAL_PROBLEM, and the object keeps its labels and count. A label
/// past the 512 octets storage keeps is refused with RESOURCES /
/// NO_SPACE_TO_WRITE_PROPERTY before anything is saved, and a count the
/// object refuses is refused as without storage.
#[tokio::test(start_paused = true)]
async fn a_state_text_write_storage_can_t_keep_is_refused_and_the_texts_stay() {
    for kind in MULTI_STATE {
        let object = oid(kind, INSTANCE);
        let storage = Arc::new(ConfigStorage::default());
        let fixture = served_by(kind, &storage).await;
        let before = fixture.read(object, P::STATE_TEXT).await;
        storage.fail.store(true, Ordering::SeqCst);
        let failed = write(&fixture, kind, P::STATE_TEXT, None, four_labels()).await;
        assert_eq!(refusal(&failed), (0, 25), "{kind:?}");
        storage.fail.store(false, Ordering::SeqCst);
        let long = text(&"x".repeat(513));
        let refused = write(&fixture, kind, P::STATE_TEXT, Some(1), long).await;
        assert_eq!(refusal(&refused), (3, 20), "{kind:?}");
        let many = PropertyValue::Unsigned(1025);
        let refused = write(&fixture, kind, P::STATE_TEXT, Some(0), many).await;
        assert_eq!(refusal(&refused), (2, 37), "{kind:?}");
        assert_eq!(fixture.read(object, P::STATE_TEXT).await, before);
        let three = PropertyValue::Unsigned(3);
        assert_eq!(fixture.read(object, P::NUMBER_OF_STATES).await, three);
        assert_eq!(storage.saves.load(Ordering::SeqCst), 0, "{kind:?}");
        let fits = text(&"x".repeat(512));
        let taken = write(&fixture, kind, P::STATE_TEXT, Some(1), fits).await;
        assert_eq!(taken, SIMPLE_ACK_WRITE, "{kind:?}");
    }
}

fn attempt(property: P, index: Option<u32>, written: PropertyValue) -> BACnetPropertyValue {
    BACnetPropertyValue {
        property_identifier: property,
        property_array_index: index,
        value: value(written),
        priority: None,
    }
}

/// One WritePropertyMultiple that adds a state and then lists it saves
/// once, with the database guard dropped: the stage checks the list against
/// the count the request's own State_Text write leaves. A server rebuilt on
/// the storage has the four states and serves the list.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_request_that_adds_a_state_and_lists_it_saves_off_the_guard() {
    for kind in LISTED {
        for grows in [
            attempt(P::STATE_TEXT, Some(0), PropertyValue::Unsigned(4)),
            attempt(P::STATE_TEXT, None, four_labels()),
        ] {
            let object = oid(kind, INSTANCE);
            let storage = Arc::new(ConfigStorage::default());
            let fixture = served_by(kind, &storage).await;
            let request = wpm(vec![WriteAccessSpecification {
                object_identifier: object,
                list_of_properties: vec![grows, attempt(P::ALARM_VALUES, None, list())],
            }]);
            let (response, available) = while_saving(
                &fixture,
                &storage,
                ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
                request,
                WAIT,
            )
            .await;
            assert_eq!(response, SIMPLE_ACK_WPM, "{kind:?}");
            assert!(
                available,
                "the database was held while the {kind:?} saved its list"
            );
            assert_eq!(storage.saves.load(Ordering::SeqCst), 1, "{kind:?}");
            assert_eq!(fixture.read(object, P::ALARM_VALUES).await, list());
            let four = PropertyValue::Unsigned(4);
            assert_eq!(fixture.read(object, P::NUMBER_OF_STATES).await, four);
            let texts = fixture.read(object, P::STATE_TEXT).await;
            drop(fixture);
            let rebuilt = served_by(kind, &storage).await;
            assert_eq!(rebuilt.read(object, P::NUMBER_OF_STATES).await, four);
            assert_eq!(rebuilt.read(object, P::STATE_TEXT).await, texts);
            assert_eq!(rebuilt.read(object, P::ALARM_VALUES).await, list());
        }
    }
}

/// The stage judges a State_Text shrink against the states the object holds
/// before the request; a Present_Value commanded earlier in the same
/// request isn't one of them. So it takes a shrink the object then refuses,
/// and its save briefly holds labels the device answered with an error.
/// The request's release puts storage back to what the object serves: a
/// server rebuilt on it has the configured three states. The same goes for
/// a Relinquish_Default or a command the request writes first.
#[tokio::test(start_paused = true)]
async fn a_shrink_the_stage_took_and_the_object_refused_is_put_back() {
    let kind = ObjectType::MULTI_STATE_VALUE;
    let object = oid(kind, INSTANCE);
    let storage = Arc::new(ConfigStorage::default());
    let fixture = served_by(kind, &storage).await;
    let request = wpm(vec![WriteAccessSpecification {
        object_identifier: object,
        list_of_properties: vec![
            attempt(P::PRESENT_VALUE, None, PropertyValue::Unsigned(3)),
            attempt(P::STATE_TEXT, Some(0), PropertyValue::Unsigned(2)),
        ],
    }]);
    let response = wire(
        &fixture,
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        request,
    )
    .await;
    assert_eq!(response[0], ERROR_PDU, "{response:?}");
    let three = PropertyValue::Unsigned(3);
    assert_eq!(fixture.read(object, P::NUMBER_OF_STATES).await, three);
    assert_eq!(fixture.read(object, P::PRESENT_VALUE).await, three);
    // The staged save, then the one that puts storage back.
    drop(fixture);
    assert_eq!(storage.saves.load(Ordering::SeqCst), 2);
    let rebuilt = served_by(kind, &storage).await;
    assert_eq!(rebuilt.read(object, P::NUMBER_OF_STATES).await, three);
    let kept = storage.load_saved().flatten();
    let Some(bacnet_objects::write_config::WriteConfigSnapshot::Reporting(kept)) = kept else {
        panic!("reporting settings saved");
    };
    assert_eq!(kept.algorithm.state_text, None);
}

/// State_Text is a saved write like the others, so every path hands it to
/// the stage: WriteProperty, a local write and each attempt of a
/// WritePropertyMultiple. An Access Door and an Access Zone have no
/// State_Text, so on them only the list, which they save, is staged.
#[test]
fn state_text_is_staged_by_every_path_on_the_multi_state_types() {
    use crate::server::durable_writes::DurableTarget;

    let (index, count) = (Some(0), PropertyValue::Unsigned(4));
    for kind in MULTI_STATE {
        let object = oid(kind, INSTANCE);
        let lone = write_property(kind, P::STATE_TEXT, index, value(count.clone()));
        assert_eq!(DurableTarget::write_property(&lone).len(), 1, "{kind:?}");
        let local = DurableTarget::local(object, P::STATE_TEXT, index, &count);
        assert_eq!(local.len(), 1, "{kind:?}");
        let request = wpm(vec![WriteAccessSpecification {
            object_identifier: object,
            list_of_properties: vec![
                attempt(P::STATE_TEXT, index, count.clone()),
                attempt(P::STATE_TEXT, Some(1), text("Off")),
            ],
        }]);
        assert_eq!(DurableTarget::write_property_multiple(&request).len(), 2);
    }
    let tamper = PropertyValue::List(vec![PropertyValue::Enumerated(4)]);
    for kind in [ObjectType::ACCESS_DOOR, ObjectType::ACCESS_ZONE] {
        let request = wpm(vec![WriteAccessSpecification {
            object_identifier: oid(kind, INSTANCE),
            list_of_properties: vec![
                attempt(P::STATE_TEXT, index, count.clone()),
                attempt(P::ALARM_VALUES, None, tamper.clone()),
            ],
        }]);
        let staged = DurableTarget::write_property_multiple(&request);
        assert_eq!(staged.len(), 1, "{kind:?}");
        assert_eq!(
            staged[0].0.reference.property_identifier,
            P::ALARM_VALUES.to_raw(),
            "{kind:?}"
        );
    }
}
