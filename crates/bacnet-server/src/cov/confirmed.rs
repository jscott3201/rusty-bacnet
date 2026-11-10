//! The server's side of a confirmed COV report (#896): the worker's wake-up
//! for a fence, and the queue of references owed a fresh evaluation. The
//! flight itself is plain state in the table; `flight.rs` has the policy.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use bacnet_device_core::cov::CovSubscriptionKey;
use tokio::sync::Notify;

use super::Token;

/// The outstanding confirmed report of one notification, as its worker
/// holds it: the token its outcome goes back with
/// ([`CovSubscriptionTable::delivered`](super::CovSubscriptionTable::delivered))
/// and the wake-up a fence fires.
///
/// Dropping it ends nothing: a worker reports every outcome, and a report
/// whose worker was aborted ends itself at its deadline.
#[derive(Debug)]
pub(crate) struct ConfirmedFlight {
    token: Token,
    wake: Arc<Notify>,
}

impl ConfirmedFlight {
    pub(super) fn new(token: Token, wake: Arc<Notify>) -> Self {
        Self { token, wake }
    }

    /// The report's token.
    pub(crate) fn token(&self) -> Token {
        self.token
    }

    /// Complete once a fresh flight has replaced this report's, after a route
    /// change or a renewal, or once its deadline passed: retrying it would
    /// only land after the replacement's own report. Cancel-safe: a fence
    /// that came first leaves its permit for the next call.
    pub(crate) async fn fenced(&self) {
        self.wake.notified().await;
    }
}

/// References whose confirmed report was acknowledged or fenced off, and so need
/// evaluating again. The server drains them in a background task and fans them
/// out through the usual path, which reports only what differs from each
/// baseline.
#[derive(Debug, Default)]
pub(crate) struct CovRevisits {
    pending: Mutex<HashSet<CovSubscriptionKey>>,
    wake: Notify,
}

impl CovRevisits {
    fn pending(&self) -> MutexGuard<'_, HashSet<CovSubscriptionKey>> {
        // A panic while holding this lock leaves only a set of keys.
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Queue references for evaluation and wake the drain.
    pub(crate) fn request(&self, keys: impl IntoIterator<Item = CovSubscriptionKey>) {
        let mut keys = keys.into_iter().peekable();
        // Nothing to queue: neither the lock nor the drain's wake-up.
        if keys.peek().is_none() {
            return;
        }
        let mut pending = self.pending();
        let before = pending.len();
        pending.extend(keys);
        let added = pending.len() > before;
        drop(pending);
        if added {
            self.wake.notify_one();
        }
    }

    /// Wait for queued references and take them all.
    pub(crate) async fn next(&self) -> Vec<CovSubscriptionKey> {
        loop {
            let keys: Vec<_> = self.pending().drain().collect();
            if !keys.is_empty() {
                return keys;
            }
            self.wake.notified().await;
        }
    }

    /// Queued references, without taking them.
    #[cfg(test)]
    pub(crate) fn queued(&self) -> HashSet<CovSubscriptionKey> {
        self.pending().clone()
    }

    /// Drop a removed reference's pending evaluation.
    pub(super) fn forget(&self, key: &CovSubscriptionKey) {
        self.pending().remove(key);
    }
}

#[cfg(test)]
mod tests;
