//! The two local nonrouter controls for an explicitly opted-in single link.
use bacnet_network::layer::ReceivedNetworkControl;
use bacnet_network::network_number::{number_is_reply, LocalNetworkNumber, NumberControl};
use bacnet_objects::database::ObjectDatabase;
use bacnet_types::network_number::NetworkNumber;
use bacnet_types::primitives::ObjectIdentifier;
use std::sync::Arc;
use tokio::sync::RwLock;

enum State {
    Registered(Arc<RwLock<ObjectDatabase>>, ObjectIdentifier),
    Unregistered(NetworkNumber),
}
/// Hidden shared control adapter, not a second configuration or lifecycle API.
#[doc(hidden)]
pub struct NetworkNumberOwner {
    state: State,
    published: Option<LocalNetworkNumber>,
}
impl NetworkNumberOwner {
    /// Explicit registration alone provides configured-number authority.
    #[doc(hidden)]
    pub fn new(selected: Option<(Arc<RwLock<ObjectDatabase>>, ObjectIdentifier)>) -> Self {
        Self {
            state: match selected {
                Some((db, oid)) => State::Registered(db, oid),
                None => State::Unregistered(NetworkNumber::default()),
            },
            published: None,
        }
    }
    /// Copy the state into `slot` after every control, so senders read the
    /// local network number without the database lock. With a registered
    /// port the copy is taken from the port's state under the same write
    /// guard that changed it, so the port stays the one authority.
    #[doc(hidden)]
    pub fn publishing_to(mut self, slot: LocalNetworkNumber) -> Self {
        self.published = Some(slot);
        self
    }
    /// Validate and process one control, returning a complete local-control NPDU.
    /// Other network messages retain the owners' existing discard behavior.
    #[doc(hidden)]
    pub async fn handle(&mut self, control: ReceivedNetworkControl) -> Option<Vec<u8>> {
        let announcement = match NumberControl::parse(&control)? {
            NumberControl::WhatIs => None,
            NumberControl::NumberIs { number, flag } => Some((number, flag)),
        };
        let publish = |state| {
            if let Some(slot) = &self.published {
                slot.publish(state);
            }
        };
        let state = match &mut self.state {
            State::Registered(db, oid) => {
                let mut db = db.write().await;
                let state = db.network_number_internal(*oid, announcement)?;
                publish(state);
                state
            }
            State::Unregistered(state) => {
                if let Some((number, flag)) = announcement {
                    state.observe(number, flag);
                }
                publish(*state);
                *state
            }
        };
        if announcement.is_some() {
            return None;
        }
        number_is_reply(state).map(|npdu| npdu.to_vec())
    }
}

impl Drop for NetworkNumberOwner {
    /// A registered owner's handle on the database goes through
    /// [`drop_database_off_runtime`](crate::server::drop_database_off_runtime),
    /// so the task owning it, finished or aborted, never drops the objects
    /// on a runtime worker should it hold the last handle (#1561).
    fn drop(&mut self) {
        let state = std::mem::replace(
            &mut self.state,
            State::Unregistered(NetworkNumber::default()),
        );
        if let State::Registered(db, _) = state {
            drop(crate::server::drop_database_off_runtime(db));
        }
    }
}
