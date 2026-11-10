//! AddListElement and RemoveListElement on the lists an object keeps as
//! event algorithm parameters (#2331): an Access Door's Alarm_Values,
//! Fault_Values and Masked_Alarm_Values, an Access Zone's Alarm_Values and a
//! Multi-state Input's Alarm_Values. Each edit stages one save of the list
//! it leaves, off the database guard, and a server rebuilt on the storage
//! serves that list. An edit the object refuses saves nothing.
//!
//! AddListElement 8; RemoveListElement 9.

use super::durable_write_wire_tests::{
    ERROR_PDU, SIMPLE_ACK_ADD, SIMPLE_ACK_WRITE, WAIT, while_saving,
};
use super::mutation_list_wire_tests::{ADD, REMOVE, list_request, wire};
use super::mutation_tests::{Fixture, oid, value};
use super::reporting_durable_tests::{INSTANCE, served_by, write_property};
use super::write_config_durable_tests::ConfigStorage;
use super::*;
use std::sync::atomic::Ordering;

use PropertyIdentifier as P;

const SIMPLE_ACK_REMOVE: [u8; 3] = [0x20, 5, 9];

/// One kept list: its object type and property, how an element is carried
/// (a door's and a zone's states are Enumerated, a multi-state object's
/// Unsigned), the state it starts with and the state an edit adds.
#[derive(Clone, Copy)]
struct Kept {
    kind: ObjectType,
    property: P,
    element: fn(u32) -> PropertyValue,
    held: u32,
    added: u32,
}

fn enumerated(state: u32) -> PropertyValue {
    PropertyValue::Enumerated(state)
}

fn unsigned(state: u32) -> PropertyValue {
    PropertyValue::Unsigned(u64::from(state))
}

/// The five lists kept as parameters. The fixture's Multi-state Input has
/// three states.
const LISTS: [Kept; 5] = [
    Kept {
        kind: ObjectType::ACCESS_DOOR,
        property: P::ALARM_VALUES,
        element: enumerated,
        held: 3,
        added: 4,
    },
    Kept {
        kind: ObjectType::ACCESS_DOOR,
        property: P::FAULT_VALUES,
        element: enumerated,
        held: 5,
        added: 6,
    },
    Kept {
        kind: ObjectType::ACCESS_DOOR,
        property: P::MASKED_ALARM_VALUES,
        element: enumerated,
        held: 7,
        added: 300,
    },
    Kept {
        kind: ObjectType::ACCESS_ZONE,
        property: P::ALARM_VALUES,
        element: enumerated,
        held: 4,
        added: 100,
    },
    Kept {
        kind: ObjectType::MULTI_STATE_INPUT,
        property: P::ALARM_VALUES,
        element: unsigned,
        held: 2,
        added: 3,
    },
];

impl Kept {
    fn list(self, states: &[u32]) -> PropertyValue {
        PropertyValue::List(states.iter().map(|state| (self.element)(*state)).collect())
    }

    /// A list service's request naming `state`.
    fn edit(self, state: u32) -> Bytes {
        let element = value((self.element)(state));
        list_request(oid(self.kind, INSTANCE), self.property, None, &element)
    }

    /// A server whose object holds `held`, written by WriteProperty: one
    /// save.
    async fn serving(self, storage: &Arc<ConfigStorage>) -> Arc<Fixture> {
        let fixture = served_by(self.kind, storage).await;
        let held = value(self.list(&[self.held]));
        let request = write_property(self.kind, self.property, None, held);
        let response = wire(&fixture, ConfirmedServiceChoice::WRITE_PROPERTY, request).await;
        assert_eq!(response, SIMPLE_ACK_WRITE, "{:?}", self.property);
        assert_eq!(storage.saves.load(Ordering::SeqCst), 1);
        fixture
    }

    async fn served(self, fixture: &Fixture) -> PropertyValue {
        fixture.read(oid(self.kind, INSTANCE), self.property).await
    }
}

/// An AddListElement and then a RemoveListElement on each kept list are
/// acknowledged, save once each with the database guard dropped, and leave
/// the list a rebuilt server serves. A list the server didn't stage would
/// save in place, under the guard.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_list_edit_saves_off_the_guard_and_survives_a_rebuild() {
    for kept in LISTS {
        let what = format!("{:?} {:?}", kept.kind, kept.property);
        let storage = Arc::new(ConfigStorage::default());
        let fixture = kept.serving(&storage).await;

        let (response, available) =
            while_saving(&fixture, &storage, ADD, kept.edit(kept.added), WAIT).await;
        assert_eq!(response, SIMPLE_ACK_ADD, "{what}");
        assert!(available, "the database was held while {what} saved an add");
        assert_eq!(storage.saves.load(Ordering::SeqCst), 2, "{what}");
        let both = kept.list(&[kept.held, kept.added]);
        assert_eq!(kept.served(&fixture).await, both, "{what}");

        let (response, available) =
            while_saving(&fixture, &storage, REMOVE, kept.edit(kept.held), WAIT).await;
        assert_eq!(response, SIMPLE_ACK_REMOVE, "{what}");
        assert!(
            available,
            "the database was held while {what} saved a remove"
        );
        assert_eq!(storage.saves.load(Ordering::SeqCst), 3, "{what}");
        let left = kept.list(&[kept.added]);
        assert_eq!(kept.served(&fixture).await, left, "{what}");

        drop(fixture);
        let rebuilt = served_by(kept.kind, &storage).await;
        assert_eq!(kept.served(&rebuilt).await, left, "{what} after a rebuild");
    }
}

/// An edit the object refuses is answered with an error, saves nothing and
/// leaves the list: a RemoveListElement of a state the list doesn't hold,
/// and an AddListElement of zero, which is NORMAL on a door and a zone and
/// no state of a multi-state object.
#[tokio::test(start_paused = true)]
async fn a_refused_list_edit_saves_nothing() {
    for kept in LISTS {
        let what = format!("{:?} {:?}", kept.kind, kept.property);
        let storage = Arc::new(ConfigStorage::default());
        let fixture = kept.serving(&storage).await;
        let held = kept.list(&[kept.held]);
        for (service, state) in [(REMOVE, kept.added), (ADD, 0)] {
            let response = wire(&fixture, service, kept.edit(state)).await;
            assert_eq!(response[0], ERROR_PDU, "{what} {service:?}: {response:?}");
            assert_eq!(
                storage.saves.load(Ordering::SeqCst),
                1,
                "{what} {service:?}"
            );
            assert_eq!(kept.served(&fixture).await, held, "{what} {service:?}");
        }
        drop(fixture);
        let rebuilt = served_by(kept.kind, &storage).await;
        assert_eq!(kept.served(&rebuilt).await, held, "{what} after a rebuild");
    }
}
