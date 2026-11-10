//! The server's store of timestamped COV-multiple histories: the plain
//! [`TimedHistories`] behind the server's lock, with what only a host keeps
//! beside them: the wake-up of the deadline wait, the telemetry atomics and
//! the log lines for dropped changes (#1039).
//!
//! The histories count, report and flag; the store publishes. Each
//! [`TimedStore::lock`] hands out a guard that, when it drops, adds what the
//! section counted to the atomics, logs what it dropped, and wakes
//! [`TimedStore::next_due`] if a deadline may have moved earlier. So every
//! caller sees the counters and the log as it did when the histories held
//! them, and nothing is published outside the lock.

use std::ops::{Deref, DerefMut};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard};

use bacnet_device_core::cov::{CovSubscriptionKey, MultipleContextKey};
use bacnet_time::Instant;
use tokio::sync::Notify;
use tracing::{debug, warn};

use super::AtomicCovCounters;
use super::timed::{DropReason, Dropped, TimedHistories};
use crate::runtime_clock;

/// What a store's handles share.
#[derive(Debug)]
struct Shared {
    histories: Mutex<TimedHistories>,
    /// Wakes [`TimedStore::next_due`] when a deadline may have moved earlier.
    wake: Notify,
    counters: Arc<AtomicCovCounters>,
}

/// Shared handle to the table's timestamped histories. Locked only for short
/// synchronous sections; never held across an await, and never while a
/// [`TimedClaim`](super::timed::TimedClaim) of the same store is dropped.
#[derive(Debug, Clone)]
pub(crate) struct TimedStore {
    shared: Arc<Shared>,
}

/// The locked histories. Dropping it publishes what the section counted,
/// dropped and moved; see the module documentation.
pub(crate) struct TimedGuard<'a> {
    histories: MutexGuard<'a, TimedHistories>,
    shared: &'a Shared,
}

impl Deref for TimedGuard<'_> {
    type Target = TimedHistories;

    fn deref(&self) -> &TimedHistories {
        &self.histories
    }
}

impl DerefMut for TimedGuard<'_> {
    fn deref_mut(&mut self) -> &mut TimedHistories {
        &mut self.histories
    }
}

impl Drop for TimedGuard<'_> {
    fn drop(&mut self) {
        let counts = self.histories.take_counts();
        let counters = &self.shared.counters;
        if counts.changes_dropped != 0 {
            counters
                .timed_changes_dropped
                .fetch_add(counts.changes_dropped, Ordering::Relaxed);
        }
        if counts.untimed_oversized != 0 {
            counters
                .untimed_references_oversized
                .fetch_add(counts.untimed_oversized, Ordering::Relaxed);
        }
        for dropped in self.histories.take_drops() {
            log_dropped(&dropped);
        }
        if self.histories.take_moved() {
            self.shared.wake.notify_one();
        }
    }
}

impl TimedStore {
    pub(super) fn new(max_apdu_length: usize, counters: Arc<AtomicCovCounters>) -> Self {
        Self {
            shared: Arc::new(Shared {
                histories: Mutex::new(TimedHistories::new(max_apdu_length)),
                wake: Notify::new(),
                counters,
            }),
        }
    }

    pub(crate) fn lock(&self) -> TimedGuard<'_> {
        TimedGuard {
            // A panic while holding this lock leaves only bounded queue state.
            histories: self
                .shared
                .histories
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()),
            shared: &self.shared,
        }
    }

    /// Wait until some context's pending changes reach their deadline, and
    /// return that context's references with pending changes for the caller
    /// to fan out. A context that stays blocked comes back after its spacing.
    ///
    /// The histories give the deadline ([`TimedHistories::take_due`]); the
    /// timer and the wake-up are the server's. The `select!` holds no lock
    /// and neither arm takes one: one is a sleep, the other this store's
    /// `Notify`, which a guard fires as it drops.
    pub(crate) async fn next_due(&self) -> Vec<CovSubscriptionKey> {
        loop {
            let notified = self.shared.wake.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let (due, next) = self.lock().take_due(runtime_clock::core_now());
            if !due.is_empty() {
                return due;
            }
            // A deadline past what the host's clock can hold never comes.
            let next = next
                .and_then(|deadline| runtime_clock::core_clock().to_std(deadline))
                .map(tokio::time::Instant::from_std);
            match next {
                Some(deadline) => {
                    tokio::select! {
                        () = tokio::time::sleep_until(deadline) => {}
                        () = notified => {}
                    }
                }
                None => notified.await,
            }
        }
    }

    /// A block on notifications lifted (communication re-enabled): overdue
    /// changes go out at the next scan instead of after a backstop wait.
    pub(crate) fn rearm(&self) {
        self.lock().rearm();
    }

    /// See [`TimedHistories::hold_until`].
    pub(crate) fn hold_until(&self, context: &MultipleContextKey, until: Instant) {
        self.lock().hold_until(context, until);
    }
}

fn describe(reason: DropReason) -> &'static str {
    match reason {
        DropReason::HistoryFull => "context history full",
        DropReason::TooLarge => {
            "a timestamped value exceeds every notification to the subscriber on its own"
        }
        DropReason::Superseded => "superseded by a delivered newer change",
    }
}

/// Log dropped changes: a warning for a context's first drop for a cause,
/// and debug for the rest, which the counter alone tracks (#1039).
fn log_dropped(dropped: &Dropped) {
    let Dropped {
        key,
        count,
        reason,
        apdu,
        first,
    } = dropped;
    if *first {
        let context = key.multiple_context();
        warn!(
            object = ?key.object(),
            recipient = ?context.map(|context| &context.recipient),
            process = context.map(|context| context.process_id),
            apdu,
            count,
            reason = describe(*reason),
            "Dropped pending timestamped COV-multiple changes; further drops in this \
             context for the same reason are only counted in timed_changes_dropped"
        );
    } else {
        debug!(
            object = ?key.object(),
            count,
            reason = describe(*reason),
            "Dropped pending timestamped COV-multiple changes"
        );
    }
}

/// Test hook: counts the drop warnings this module logs on the thread it is
/// installed on, debug-level drops excluded.
#[cfg(test)]
#[derive(Clone, Default)]
pub(crate) struct DropWarningCount(std::sync::Arc<std::sync::atomic::AtomicUsize>);

#[cfg(test)]
impl DropWarningCount {
    /// Count on this thread until the returned guard drops. Under a
    /// current-thread runtime that covers the server's tasks too.
    pub(crate) fn install(&self) -> tracing::subscriber::DefaultGuard {
        // As in the DCC trace tests: keep a second, disabled dispatcher alive
        // so a callsite another test registered first, with no subscriber of
        // its own, is not cached as never enabled.
        static REGISTRATION_PEER: std::sync::OnceLock<tracing::Dispatch> =
            std::sync::OnceLock::new();
        REGISTRATION_PEER
            .get_or_init(|| tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default()));
        tracing::subscriber::set_default(self.clone())
    }

    /// Drop warnings counted so far.
    pub(crate) fn get(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
impl tracing::Subscriber for DropWarningCount {
    fn register_callsite(
        &self,
        _: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        tracing::subscriber::Interest::sometimes()
    }
    fn max_level_hint(&self) -> Option<tracing::metadata::LevelFilter> {
        Some(tracing::metadata::LevelFilter::WARN)
    }
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        metadata.target() == module_path!() && *metadata.level() == tracing::Level::WARN
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        if self.enabled(event.metadata()) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}
