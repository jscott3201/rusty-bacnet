//! The server's store of timestamped COV-multiple histories: the plain
//! [`TimedHistories`] behind the server's lock, with what only a host keeps
//! beside them: the wake-up of the deadline wait, the telemetry atomics and
//! the log lines for dropped changes (#1039).
//!
//! The histories count, report and flag; the store publishes. Each
//! [`HistoryStore::lock`] hands out a guard that, when it drops, adds what the
//! section counted to the atomics, logs what it dropped, and wakes
//! [`HistoryStore::next_due`] if a deadline may have moved earlier. So every
//! caller sees the counters and the warnings as it did when the histories
//! held them, and nothing is published outside the lock. One thing differs
//! in the log: the histories keep 64 repeat reports for one section, so a
//! section that drops more gets one debug line for the rest, not one each.
//!
//! The histories settle a claim only when told. The server's fanout and
//! workers return from many places, and a worker can be aborted, so here a
//! claim settles itself when dropped: [`ClaimGuard`] gives its changes back
//! unless it was committed, [`LodgedClaim`] does the same for a claim the
//! histories keep for a confirmed worker, and [`SendTurn`] ends an
//! unconfirmed context's report. Each takes only this store's mutex, a leaf
//! lock, for a synchronous section.

use std::collections::BTreeSet;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard};

use bacnet_device_core::cov::{CovSubscriptionKey, MultipleContextKey};
use bacnet_time::Instant;
use tokio::sync::Notify;
use tracing::{debug, warn};

use super::{AtomicCovCounters, CovRevisits};
use crate::runtime_clock;
use bacnet_device_core::cov::timed::{
    self, ClaimId, DropReason, Dropped, Settlement, TimedChange, TimedCounts, TimedHistories,
    ValueFit,
};

/// What a store's handles share.
#[derive(Debug)]
struct Shared {
    histories: Mutex<TimedHistories>,
    /// Wakes [`HistoryStore::next_due`] when a deadline may have moved earlier.
    wake: Notify,
    counters: Arc<AtomicCovCounters>,
}

/// Shared handle to the table's timestamped histories. Locked only for short
/// synchronous sections; never held across an await, and never while a
/// [`ClaimGuard`], a [`LodgedClaim`] or a [`SendTurn`] of the same store is
/// dropped.
#[derive(Debug, Clone)]
pub(crate) struct HistoryStore {
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
        let drops = self.histories.take_drops();
        for dropped in &drops {
            log_dropped(dropped);
        }
        // The histories keep only so many repeat reports for one section;
        // one line stands for the drops past them.
        let more = unreported(&counts, &drops);
        if more != 0 {
            debug!(
                count = more,
                "Dropped further pending timestamped COV-multiple changes in the same step; \
                 counted in timed_changes_dropped"
            );
        }
        if self.histories.take_moved() {
            self.shared.wake.notify_one();
        }
    }
}

impl HistoryStore {
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
            let (due, next) = {
                let mut histories = self.lock();
                let (due, next) = histories.take_due(runtime_clock::core_now());
                // With nothing due, every deadline is still ahead.
                debug_assert!(!due.is_empty() || next == histories.next_deadline());
                (due, next)
            };
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

/// Changes drained into one notification, and the untimestamped references
/// whose prepared values it carries: a [`timed::TimedClaim`] that settles
/// itself. Unless [`ClaimGuard::commit`] is called once the notification is
/// delivered, dropping it gives the claim back to the histories
/// ([`TimedHistories::give_back`]), on any early return or failed send.
///
/// Its methods are the plain claim's, which has the policy.
#[derive(Debug)]
pub(crate) struct ClaimGuard {
    store: HistoryStore,
    claim: timed::TimedClaim,
}

impl ClaimGuard {
    pub(crate) fn new(store: HistoryStore) -> Self {
        Self {
            store,
            claim: timed::TimedClaim::new(),
        }
    }

    /// `claim` of this claim's store, settling itself as this one does.
    fn wrap(&self, claim: timed::TimedClaim) -> Self {
        Self {
            store: self.store.clone(),
            claim,
        }
    }

    /// Take the plain claim out, leaving nothing to give back.
    fn take(&mut self) -> timed::TimedClaim {
        std::mem::take(&mut self.claim)
    }

    /// See [`timed::TimedClaim::add_untimed`].
    pub(crate) fn add_untimed(
        &mut self,
        key: CovSubscriptionKey,
        generation: u64,
        owed_since: Option<Instant>,
    ) {
        self.claim.add_untimed(key, generation, owed_since);
    }

    /// See [`timed::TimedClaim::split_untimed`].
    pub(crate) fn split_untimed(&mut self, keys: &BTreeSet<CovSubscriptionKey>) -> Self {
        let part = self.claim.split_untimed(|key| keys.contains(key));
        self.wrap(part)
    }

    /// See [`TimedHistories::forgo_untimed`].
    pub(crate) fn forgo_untimed(&mut self) {
        self.store.lock().forgo_untimed(&mut self.claim);
    }

    /// See [`timed::TimedClaim::without_eviction`].
    pub(crate) fn without_eviction(mut self) -> Self {
        self.claim = self.take().without_eviction();
        self
    }

    /// See [`timed::TimedClaim::owing`].
    pub(crate) fn owing(mut self) -> Self {
        self.claim = self.take().owing();
        self
    }

    /// See [`timed::TimedClaim::add`].
    pub(crate) fn add(
        &mut self,
        key: CovSubscriptionKey,
        incarnation: u64,
        changes: Vec<TimedChange>,
    ) {
        self.claim.add(key, incarnation, changes);
    }

    /// See [`timed::TimedClaim::split_oldest`].
    pub(crate) fn split_oldest(&mut self, count: usize) -> Self {
        let part = self.claim.split_oldest(count);
        self.wrap(part)
    }

    /// See [`timed::TimedClaim::split_values`]; the changes that lost a
    /// value are counted here ([`TimedHistories::lost_values`]).
    pub(crate) fn split_values(
        mut self,
        fit: impl Fn(&CovSubscriptionKey, &TimedChange) -> ValueFit,
    ) -> Vec<Self> {
        let (parts, lost) = self.take().split_values(fit);
        if !lost.is_empty() {
            self.store.lock().lost_values(&lost);
        }
        parts.into_iter().map(|part| self.wrap(part)).collect()
    }

    /// See [`timed::TimedClaim::in_order`].
    pub(crate) fn in_order(&self) -> Vec<(&CovSubscriptionKey, &TimedChange)> {
        self.claim.in_order()
    }

    /// See [`timed::TimedClaim::last_changes`].
    pub(crate) fn last_changes(&self) -> impl Iterator<Item = (&CovSubscriptionKey, &TimedChange)> {
        self.claim.last_changes()
    }

    /// The notification carrying these changes and values was delivered; see
    /// [`TimedHistories::commit`].
    pub(crate) fn commit(mut self) {
        let claim = self.take();
        self.store.lock().commit(claim);
    }

    /// See [`TimedHistories::going_out`].
    pub(crate) fn going_out(&self) {
        self.store.lock().going_out(&self.claim);
    }

    /// Hand the claim to the histories for a confirmed worker, until
    /// `deadline`, its report's own ([`TimedHistories::lodge`]). The worker
    /// settles it through the handle returned; the histories give it back
    /// themselves if the worker has not by then.
    pub(crate) fn lodge(mut self, deadline: Instant) -> LodgedClaim {
        let claim = self.take();
        let id = self.store.lock().lodge(claim, deadline);
        LodgedClaim {
            store: self.store.clone(),
            id: Some(id),
        }
    }
}

impl Drop for ClaimGuard {
    fn drop(&mut self) {
        if self.claim.is_empty() {
            return;
        }
        let claim = self.take();
        self.store
            .lock()
            .give_back(claim, runtime_clock::core_now());
    }
}

/// A confirmed worker's hold on the claim its report conveys, lodged with
/// the histories. Unless [`LodgedClaim::commit`] is called on the Ack,
/// dropping it settles the claim as returned: on a failure, a fence, DCC, a
/// closed transaction, or when the worker is aborted. A worker that settles
/// after the claim's deadline finds it given back already, and changes
/// nothing.
#[derive(Debug)]
pub(crate) struct LodgedClaim {
    store: HistoryStore,
    id: Option<ClaimId>,
}

impl LodgedClaim {
    fn settle(&mut self, outcome: Settlement) {
        if let Some(id) = self.id.take() {
            self.store
                .lock()
                .settle(id, outcome, runtime_clock::core_now());
        }
    }

    /// The report was acknowledged: retire what it conveyed.
    pub(crate) fn commit(mut self) {
        self.settle(Settlement::Delivered);
    }
}

impl Drop for LodgedClaim {
    fn drop(&mut self) {
        self.settle(Settlement::Returned);
    }
}

/// The one report an unconfirmed context is sending (#986, #1038); see
/// [`TimedHistories::begin_send`].
///
/// Its parts go out one await apart. While a turn is held, other fanouts of
/// the context stand back and leave their changes queued. Dropping the turn,
/// after the last part or on any early return, hands the context to one
/// follow-up if a fanout stood back.
#[must_use = "dropping the turn ends the report"]
pub(crate) struct SendTurn {
    store: HistoryStore,
    context: MultipleContextKey,
    revisits: Arc<CovRevisits>,
    keys: Vec<CovSubscriptionKey>,
}

impl SendTurn {
    /// Take the turn of `context`, unless a report of it is still going out.
    /// `keys` are the context's references, for the follow-up.
    pub(crate) fn begin(
        store: &HistoryStore,
        context: &MultipleContextKey,
        revisits: &Arc<CovRevisits>,
        keys: Vec<CovSubscriptionKey>,
    ) -> Option<Self> {
        store.lock().begin_send(context).then(|| Self {
            store: store.clone(),
            context: context.clone(),
            revisits: Arc::clone(revisits),
            keys,
        })
    }
}

impl Drop for SendTurn {
    fn drop(&mut self) {
        let owed = self.store.lock().end_send(&self.context);
        if owed {
            self.revisits.request(std::mem::take(&mut self.keys));
        }
    }
}

/// The changes a locked section dropped that none of its reports names:
/// past a number of repeats the histories count a drop without reporting it
/// (#2320). Both come from one section, so the counts hold every report's.
fn unreported(counts: &TimedCounts, drops: &[Dropped]) -> u64 {
    let reported: u64 = drops.iter().map(|dropped| dropped.count as u64).sum();
    counts.changes_dropped.saturating_sub(reported)
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

#[cfg(test)]
#[path = "timed_store_tests.rs"]
mod tests;
