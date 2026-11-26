//! The answers a station gives the server's own confirmed requests, counted
//! for a binding check that watches it (#2825).
//!
//! A check of a bound device's address reads the device's identifier there,
//! twice a minute apart, before it moves the binding; an answer from that
//! station to any request of the server's between and during the two (a
//! Command or Channel write, an event notification, an Audit record) is word
//! that the device is still there, and the move is off. Only an answer the
//! pool admitted counts: one from the station the request went to, completing
//! a request still waiting for it. An I-Am or a notification the station sent
//! unasked is not an answer, so it counts for nothing here.
//!
//! The count is kept under the transactions' state lock, which an answer
//! takes anyway, and only for stations a check listens to: with none, an
//! answer costs one emptiness test.

use std::collections::HashMap;
use std::sync::Arc;

use bacnet_endpoint_core::coordinator::CanonicalPeer;

use super::{NotificationCore, NotificationTransactions};

/// The stations listened to, with each one's listeners and answers.
#[derive(Debug, Default)]
pub(super) struct Listening(HashMap<CanonicalPeer, Heard>);

#[derive(Debug)]
struct Heard {
    listeners: usize,
    answers: u64,
}

impl Listening {
    /// `peer` answered one of the server's requests.
    #[inline]
    pub(super) fn answered(&mut self, peer: &CanonicalPeer) {
        if self.0.is_empty() {
            return;
        }
        if let Some(heard) = self.0.get_mut(peer) {
            heard.answers = heard.answers.wrapping_add(1);
        }
    }
}

/// Counts one station's answers to the server's requests from its making
/// until it is dropped.
pub(in crate::server) struct AnswerListener {
    core: Arc<NotificationCore>,
    peer: CanonicalPeer,
    /// The station's count when listening began; `None` when the state
    /// could not be read (a panic poisoned it).
    from: Option<u64>,
}

impl NotificationTransactions {
    /// Count `peer`'s answers to the server's requests from now on.
    pub(in crate::server) fn listen(&self, peer: CanonicalPeer) -> AnswerListener {
        // lock-poison: fail-closed (a listener that can't count says every
        // answer may have come)
        let from = self.core.state.lock().ok().map(|mut state| {
            let heard = state.listening.0.entry(peer.clone()).or_insert(Heard {
                listeners: 0,
                answers: 0,
            });
            heard.listeners += 1;
            heard.answers
        });
        AnswerListener {
            core: Arc::clone(&self.core),
            peer,
            from,
        }
    }
}

impl AnswerListener {
    /// The answers its station gave since listening began, or `None` when
    /// they can't be counted: take that as an answer.
    pub(in crate::server) fn answers(&self) -> Option<u64> {
        let from = self.from?;
        let state = self.core.state.lock().ok()?;
        let heard = state.listening.0.get(&self.peer)?;
        Some(heard.answers.wrapping_sub(from))
    }
}

impl Drop for AnswerListener {
    fn drop(&mut self) {
        if self.from.is_none() {
            return;
        }
        // lock-poison: fail-closed (the entry stays; it only counts)
        let Ok(mut state) = self.core.state.lock() else {
            return;
        };
        if let Some(heard) = state.listening.0.get_mut(&self.peer) {
            heard.listeners = heard.listeners.saturating_sub(1);
            if heard.listeners == 0 {
                state.listening.0.remove(&self.peer);
            }
        }
    }
}
