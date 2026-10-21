//! Request-side timer progress, carried in the transaction's shared owner.
//!
//! The TSM publishes a phase change (SEGMENTED_CONF entered, segment
//! activity, back to AWAIT_CONFIRMATION) and the one task waiting for the
//! response restarts its timer. This used to be a `tokio::sync::watch`
//! channel per request; its creation and the eight waiter lists it notifies
//! on drop cost several hundred instructions on every confirmed request
//! (#1727). The state now lives in the [`TransactionOwner`] allocation each
//! transaction already makes, behind one short-held mutex and one `Notify`.
//!
//! Every publish bumps a version, as `watch::Sender::send_replace` did, so a
//! publish that leaves the value unchanged still reads as a change. When the
//! transaction leaves the TSM nothing is published: the response oneshot,
//! which every waiter selects alongside [`ProgressReceiver::changed`],
//! resolves at that moment instead.

use super::TransactionOwner;
use std::sync::{Mutex, PoisonError};
use tokio::sync::Notify;

/// Request-side timer state delivered to the task waiting for a response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransactionProgress {
    AwaitingResponse,
    SegmentedResponse { generation: u64 },
}

#[derive(Debug)]
struct ProgressState {
    /// Bumped by every publish; the receiver compares it with what it saw.
    version: u64,
    value: TransactionProgress,
}

/// Progress state shared by the TSM entry and its waiting task.
#[derive(Debug)]
pub(super) struct TransactionShared {
    // Held only to copy two words in or out, never across an await.
    state: Mutex<ProgressState>,
    notify: Notify,
}

impl TransactionShared {
    pub(super) fn new() -> Self {
        Self {
            state: Mutex::new(ProgressState {
                version: 0,
                value: TransactionProgress::AwaitingResponse,
            }),
            notify: Notify::new(),
        }
    }

    fn snapshot(&self) -> (u64, TransactionProgress) {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        (state.version, state.value)
    }

    fn version(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .version
    }
}

impl TransactionOwner {
    /// Publish new progress and wake the waiting task.
    ///
    /// Called only under the TSM lock, so publishes are serialized.
    pub(super) fn publish_progress(&self, value: TransactionProgress) {
        {
            let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.version = state.version.wrapping_add(1);
            state.value = value;
        }
        // `notify_one`, not `notify_waiters`: there is exactly one receiver,
        // and `notify_one` stores a permit when that receiver is between its
        // version check and its `notified().await`, so the wakeup can't be
        // lost. A permit left over after the receiver already saw the new
        // version only costs it one extra check in `changed`.
        self.0.notify.notify_one();
    }
}

/// The waiting task's view of its transaction's progress.
///
/// Deliberately not `Clone`: [`TransactionOwner::publish_progress`] wakes a
/// single waiter, which is correct only while there is a single receiver.
#[derive(Debug)]
pub(crate) struct ProgressReceiver {
    owner: TransactionOwner,
    seen: u64,
}

impl ProgressReceiver {
    pub(super) fn new(owner: TransactionOwner) -> Self {
        Self { owner, seen: 0 }
    }

    /// The current progress, marking it seen.
    pub(crate) fn borrow_and_update(&mut self) -> TransactionProgress {
        let (version, value) = self.owner.0.snapshot();
        self.seen = version;
        value
    }

    /// The current progress, without marking it seen.
    #[cfg(test)]
    pub(crate) fn current(&self) -> TransactionProgress {
        self.owner.0.snapshot().1
    }

    /// Resolve once progress has been published since the last
    /// [`borrow_and_update`](Self::borrow_and_update).
    ///
    /// Never resolves after the transaction has left the TSM; callers select
    /// it alongside the response oneshot, which resolves then. Cancel safe:
    /// it holds no lock across its await, and a dropped `Notified` that had
    /// been handed the wakeup passes it back as a stored permit.
    pub(crate) async fn changed(&mut self) {
        loop {
            if self.owner.0.version() != self.seen {
                return;
            }
            // A publish after the check above leaves a permit (see
            // `publish_progress`), so this await returns at once.
            self.owner.0.notify.notified().await;
        }
    }
}

#[cfg(test)]
#[path = "progress_tests.rs"]
mod tests;
