//! A client's live watches (#2561, #2603): each Python `Watch` runs on one
//! of the high-level client's watch hubs, one for each set of watch
//! settings in use, which hold the high-level client, and so the low-level
//! one, while a watch on them lives.
//!
//! The client keeps each watch here from the moment it is made, under the
//! lease of the call that made it, until it ends: closed, dropped, or ended
//! by the client's stop. A stop ends every watch's iterator as it is
//! admitted; before it takes the low-level client it shuts every hub
//! (`bacnet_ops::Client::shutdown_watches`), which ends every watch on
//! them, cancels what it can within [`END_WAIT`], stops the hubs' tasks and
//! lets go of the client. An entry is added only under a read lease and the
//! map emptied only under the stop's exclusive hold, as for the `Device`
//! handles, so a stop never misses a watch made on the client it stops.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use bacnet_types::sync::MutexExt;

use super::ClientInner;
use super::state::ClientState;
use crate::high_level::{Ended, OpsClient, WatchShared};

/// How long ending a watch waits for its cancels: those not answered by
/// then are left to lapse with their lifetime. A device that answers takes
/// each in one round trip; one gone silent would hold each for a request's
/// whole time with its retries.
pub(crate) const END_WAIT: Duration = Duration::from_secs(5);

/// How long a stop waits, after its watches ended, for their hubs' last
/// tasks to let go of the client (see [`ClientState::end_watches`]).
pub(super) const SETTLE: Duration = Duration::from_secs(2);

/// What names one watch among its client's.
pub(crate) type WatchId = u64;

/// The watches a client keeps (see the module docs).
#[derive(Default)]
pub(crate) struct Watches {
    /// A std mutex, held only to add, remove or take entries.
    live: std::sync::Mutex<HashMap<WatchId, Arc<WatchShared>>>,
    next: AtomicU64,
    /// A watch was made since the last stop: its hub's tasks may still be
    /// letting go of the client when the stop takes it.
    made: AtomicBool,
}

impl ClientState {
    /// Keep `watch` until it ends. Called under a call's lease.
    fn keep_watch(&self, watch: Arc<WatchShared>) -> WatchId {
        let watches = &self.watches;
        let id = watches.next.fetch_add(1, Ordering::Relaxed);
        watches.made.store(true, Ordering::Release);
        watches.live.locked().insert(id, watch);
        id
    }

    /// Forget the watch `id` names: it has ended.
    fn forget_watch(&self, id: WatchId) {
        let forgotten = self.watches.live.locked().remove(&id);
        drop(forgotten);
    }

    /// How many watches the client keeps now.
    #[cfg(test)]
    pub(super) fn live_watches(&self) -> usize {
        self.watches.live.locked().len()
    }

    /// End every watch's iterator, as a stop is admitted: their hubs end
    /// once the stop holds the client ([`Self::end_watches`]).
    pub(super) fn close_watch_streams(&self) {
        let live: Vec<Arc<WatchShared>> = self.watches.live.locked().values().cloned().collect();
        for watch in live {
            watch.close_stream(Ended::Stopped);
        }
    }

    /// A stop's first step for watches: end every watch left, then shut
    /// every hub of `high`, the started high-level client, all at once,
    /// each waiting up to `wait` for its cancels. True when a watch was
    /// made since the last stop: a hub that gave up on a silent device
    /// aborted its subscriptions' renewal tasks, and the runtime drops
    /// those, with their hold on the client, a moment after.
    pub(super) async fn end_watches(&self, wait: Duration, high: Option<OpsClient>) -> bool {
        let live: Vec<Arc<WatchShared>> = {
            let mut live = self.watches.live.locked();
            live.drain().map(|(_, watch)| watch).collect()
        };
        for watch in &live {
            watch.close_stream(Ended::Stopped);
        }
        if let Some(high) = high {
            high.shutdown_watches(wait).await;
        }
        drop(live);
        self.watches.made.swap(false, Ordering::AcqRel)
    }
}

/// One Python `Watch`'s place among its client's watches. Dropping it ends
/// the watch, if nothing ended it before.
pub(crate) struct WatchTie {
    inner: ClientInner,
    id: WatchId,
    watch: Arc<WatchShared>,
    /// Set once an end is under way, so it is started once.
    ending: AtomicBool,
}

impl WatchTie {
    /// Keep `watch` on the client `inner` belongs to. Called under a call's
    /// lease (see the module docs).
    pub(super) fn new(inner: ClientInner, watch: Arc<WatchShared>) -> Self {
        let id = inner.keep_watch(Arc::clone(&watch));
        Self {
            inner,
            id,
            watch,
            ending: AtomicBool::new(false),
        }
    }

    /// End the watch as closed: its iterator ends now, and its end (its
    /// points let go, their cancels waited for) runs on the binding
    /// runtime, so it finishes however its caller fares, then the client
    /// forgets it. Await [`WatchShared::end`] for it to finish. Sends
    /// nothing itself.
    pub(crate) fn close(&self) {
        self.watch.close_stream(Ended::Closed);
        if self.ending.swap(true, Ordering::AcqRel) {
            return;
        }
        let (inner, id, watch) = (Arc::clone(&self.inner), self.id, Arc::clone(&self.watch));
        crate::py_async::spawn_cleanup(async move {
            watch.end(Ended::Closed, END_WAIT).await;
            inner.forget_watch(id);
        });
    }
}

impl Drop for WatchTie {
    fn drop(&mut self) {
        self.close();
    }
}
