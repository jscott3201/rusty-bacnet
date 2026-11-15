//! The two local nonrouter controls for an explicitly opted-in single link.
use bacnet_network::layer::ReceivedNetworkControl;
use bacnet_network::network_number::{
    DelayedAnswer, LocalNetworkNumber, NumberControl, number_is_reply,
};
use bacnet_objects::database::ObjectDatabase;
use bacnet_types::network_number::NetworkNumber;
use bacnet_types::primitives::ObjectIdentifier;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static CONTROL_WRITERS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many controls on this thread have asked a registered port's database
/// for the write guard. A test that holds the database and wants to know its
/// own control is the writer behind it waits for this to rise, not for "some
/// writer is queued": the server's own tasks queue for the guard too (#2507).
/// Counted per thread, so it is exact for a test on a current-thread runtime.
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub fn control_writers_requested() -> usize {
    CONTROL_WRITERS.with(std::cell::Cell::get)
}

enum State {
    Registered(Arc<RwLock<ObjectDatabase>>, ObjectIdentifier),
    Unregistered(NetworkNumber),
}
/// What the task that owns the Number controls does next
/// ([`NetworkNumberOwner::next`]).
#[doc(hidden)]
pub enum NumberTurn {
    /// A control arrived: hand it to [`NetworkNumberOwner::handle`].
    Control(ReceivedNetworkControl),
    /// The answer a broadcast question left waiting came due: send it, when
    /// there is one.
    Answer(Option<Vec<u8>>),
    /// The control queue closed: the task ends.
    Closed,
}

/// Hidden shared control adapter, not a second configuration or lifecycle API.
#[doc(hidden)]
pub struct NetworkNumberOwner {
    state: State,
    published: Option<LocalNetworkNumber>,
    /// The answer to a broadcast What-Is-Network-Number, held for its
    /// random wait (#2597).
    answer: DelayedAnswer,
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
            answer: DelayedAnswer::new(),
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
    /// Validate and process one control, returning a complete local-control
    /// NPDU when an answer goes now: to a What-Is-Network-Number sent to
    /// this node alone. A broadcast one is answered later, through
    /// [`Self::next`], unless another node's answer is heard
    /// first (Clause 6.4.19, [`DelayedAnswer`]). Other network messages
    /// retain the owners' existing discard behavior.
    #[doc(hidden)]
    pub async fn handle(&mut self, control: ReceivedNetworkControl) -> Option<Vec<u8>> {
        let control = NumberControl::parse(&control)?;
        let announcement = match control {
            NumberControl::WhatIs { .. } => None,
            NumberControl::NumberIs { number, flag } => Some((number, flag)),
        };
        let publish = |state| {
            if let Some(slot) = &self.published {
                slot.publish(state);
            }
        };
        let state = match &mut self.state {
            State::Registered(db, oid) => {
                // Counted in the poll that first asks, so a pending guard
                // means this control is the writer waiting.
                #[cfg(any(test, feature = "test-support"))]
                CONTROL_WRITERS.with(|count| count.set(count.get() + 1));
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
        match control {
            NumberControl::NumberIs { number, flag } => {
                // After the state took the announcement in, so a number
                // just adopted from it counts as agreeing.
                self.answer.heard(state, number, flag);
                None
            }
            NumberControl::WhatIs { broadcast } => self
                .answer
                .asked(state, broadcast)
                .then(|| number_is_reply(state))
                .flatten()
                .map(|npdu| npdu.to_vec()),
        }
    }

    /// Wait for the owner's next piece of work: a control from `controls`,
    /// or the answer a broadcast question left waiting. The future that
    /// loses is dropped before this returns, so the caller's work on the
    /// turn runs with nothing left queued for the database. A waiting
    /// answer cut short by a control stays pending and comes due again.
    #[doc(hidden)]
    pub async fn next(
        &mut self,
        controls: &mut mpsc::Receiver<ReceivedNetworkControl>,
    ) -> NumberTurn {
        tokio::select! {
            control = controls.recv() => control.map_or(NumberTurn::Closed, NumberTurn::Control),
            answer = self.delayed_answer() => NumberTurn::Answer(answer),
        }
    }

    /// Wait for the answer a broadcast What-Is-Network-Number left pending,
    /// and give it as a complete local-control NPDU, made from the number
    /// held when it comes due. Never returns while nothing is pending.
    /// Dropping the wait keeps the answer pending: it is taken only when
    /// this returns.
    async fn delayed_answer(&mut self) -> Option<Vec<u8>> {
        self.answer.due().await;
        let state = match &mut self.state {
            State::Registered(db, oid) => db.write().await.network_number_internal(*oid, None),
            State::Unregistered(state) => Some(*state),
        };
        self.answer.taken();
        number_is_reply(state?).map(|npdu| npdu.to_vec())
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
