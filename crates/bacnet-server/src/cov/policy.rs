//! The server's COV telemetry atomics and its confirmed in-flight tracker.

use bacnet_device_core::cov::{CovCounters, CovRecipient};
use bacnet_types::sync::MutexExt;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use bacnet_transport::warn_throttle::WarnThrottle;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Atomic storage for COV telemetry counters.
#[derive(Debug, Default)]
pub struct AtomicCovCounters {
    /// Number of currently active subscriptions.
    pub subscriptions_active: AtomicU64,
    /// Total number of subscriptions created.
    pub subscriptions_created: AtomicU64,
    /// Total number of subscriptions rejected due to per-peer quota.
    pub subscriptions_rejected_quota: AtomicU64,
    /// Total number of subscriptions rejected due to global or reserved capacity.
    pub subscriptions_rejected_capacity: AtomicU64,
    /// Total number of subscriptions rejected due to indefinite policy or quota.
    pub subscriptions_rejected_indefinite: AtomicU64,
    /// Total number of subscriptions explicitly cancelled.
    pub subscriptions_cancelled: AtomicU64,
    /// Total number of expired subscriptions purged.
    pub subscriptions_purged: AtomicU64,
    /// Total number of COV notifications sent (confirmed + unconfirmed).
    pub notifications_sent: AtomicU64,
    /// Total number of confirmed COV notifications sent.
    pub notifications_confirmed: AtomicU64,
    /// Total number of unconfirmed COV notifications sent.
    pub notifications_unconfirmed: AtomicU64,
    /// Total number of notification payload bytes sent.
    pub notification_bytes_sent: AtomicU64,
    /// Total number of notifications throttled due to per-event fanout or byte budget.
    pub notifications_throttled_fanout: AtomicU64,
    /// Total number of confirmed notifications held back because their
    /// peer already had its limit in flight, each counted once: a
    /// subscription held back at several changes counts several times but
    /// waits once, and goes with the value then current once its peer has
    /// room, which may take more than one of the peer's reports ending
    /// (#2671).
    pub notifications_throttled_peer: AtomicU64,
    /// Total number of pending timestamped COV-multiple changes discarded while
    /// their reference stayed subscribed: evicted on overflow of the
    /// per-context history bound, superseded by a newer delivered change when
    /// an older notification failed, or, counted once per change, losing a
    /// value too large for any notification to the subscriber even alone (the
    /// rest of such a change still goes out). Changes split across several
    /// notifications, one value per notification included, are not counted.
    /// Only the first drop of each cause in a context logs a warning until the
    /// context is admitted afresh, so this count is the running signal, for
    /// instance for a subscriber whose maximum APDU cannot hold one timestamped
    /// value.
    pub timed_changes_dropped: AtomicU64,
    /// Total number of times a COV-multiple report left out an untimestamped
    /// reference because its values alone exceed one notification to the
    /// subscriber. The value is not queued: the reference is evaluated again
    /// at its next fanout, and each report that leaves it out counts once.
    pub untimed_references_oversized: AtomicU64,
    /// Times a reference entered the queue of those waiting for room for
    /// a confirmed report: when its report found no invoke ID free (#2661),
    /// its peer at its in-flight limit or the pool of in-flight slots full
    /// (#2671), unless it waited already, and again each time a look at it
    /// was refused for want of room. Not in [`CovCounters`] yet, so the
    /// server's API doesn't report these three. `queued` is `delivered`
    /// plus `dropped` plus those still waiting.
    pub(crate) owed_revisits_queued: AtomicU64,
    /// Of those, handed to the fanout once room came back, or sent by
    /// another fanout that found room first.
    pub(crate) owed_revisits_delivered: AtomicU64,
    /// Of those, dropped while they waited: the subscription was
    /// cancelled, expired, removed or replaced (a replacement reports for
    /// itself), or the server stopped.
    pub(crate) owed_revisits_dropped: AtomicU64,
    /// Confirmed notifications held back because the pool of in-flight
    /// slots was full (#2671), each counted once, as
    /// `notifications_throttled_peer` counts its own. Not in
    /// [`CovCounters`] yet; the log line is throttled, so this is the count.
    pub(crate) notifications_throttled_pool: AtomicU64,
    /// The same for want of an invoke ID (#2661).
    pub(crate) notifications_throttled_invoke_id: AtomicU64,
    /// Confirmed reports refused because their subscriber, relayed from
    /// another network, is at a broadcast or group form there (the one
    /// octet `0xFF`, two octets starting `0x00` or `0xFF`, three of `0xFF`,
    /// or six starting `FF:FF:FF:FF`; #2732): one per fanout that would
    /// have sent the subscription a confirmed report. Its baseline never
    /// moves, so once its value leaves the baseline every later fanout
    /// counts again, a forced one (the initial report, a renewal) included.
    /// Its unconfirmed notifications aren't held back. Not in
    /// [`CovCounters`] yet; the warning is throttled, so this is the
    /// count.
    pub(crate) confirmed_broadcast_subscriber: AtomicU64,
}

impl AtomicCovCounters {
    /// Add the counts a table mutation or evaluation reports (subscriptions,
    /// and notifications cut off at their budget), and take its live count.
    pub(crate) fn add(&self, counts: &CovCounters) {
        for (counter, count) in [
            (&self.subscriptions_created, counts.subscriptions_created),
            (
                &self.subscriptions_rejected_quota,
                counts.subscriptions_rejected_quota,
            ),
            (
                &self.subscriptions_rejected_capacity,
                counts.subscriptions_rejected_capacity,
            ),
            (
                &self.subscriptions_rejected_indefinite,
                counts.subscriptions_rejected_indefinite,
            ),
            (
                &self.subscriptions_cancelled,
                counts.subscriptions_cancelled,
            ),
            (&self.subscriptions_purged, counts.subscriptions_purged),
            (
                &self.notifications_throttled_fanout,
                counts.notifications_throttled_fanout,
            ),
        ] {
            if count != 0 {
                counter.fetch_add(count, Ordering::Relaxed);
            }
        }
        self.subscriptions_active
            .store(counts.subscriptions_active, Ordering::Relaxed);
    }

    /// Return an instantaneous snapshot of all counters.
    pub fn snapshot(&self) -> CovCounters {
        CovCounters {
            subscriptions_active: self.subscriptions_active.load(Ordering::Relaxed),
            subscriptions_created: self.subscriptions_created.load(Ordering::Relaxed),
            subscriptions_rejected_quota: self.subscriptions_rejected_quota.load(Ordering::Relaxed),
            subscriptions_rejected_capacity: self
                .subscriptions_rejected_capacity
                .load(Ordering::Relaxed),
            subscriptions_rejected_indefinite: self
                .subscriptions_rejected_indefinite
                .load(Ordering::Relaxed),
            subscriptions_cancelled: self.subscriptions_cancelled.load(Ordering::Relaxed),
            subscriptions_purged: self.subscriptions_purged.load(Ordering::Relaxed),
            notifications_sent: self.notifications_sent.load(Ordering::Relaxed),
            notifications_confirmed: self.notifications_confirmed.load(Ordering::Relaxed),
            notifications_unconfirmed: self.notifications_unconfirmed.load(Ordering::Relaxed),
            notification_bytes_sent: self.notification_bytes_sent.load(Ordering::Relaxed),
            notifications_throttled_fanout: self
                .notifications_throttled_fanout
                .load(Ordering::Relaxed),
            notifications_throttled_peer: self.notifications_throttled_peer.load(Ordering::Relaxed),
            timed_changes_dropped: self.timed_changes_dropped.load(Ordering::Relaxed),
            untimed_references_oversized: self.untimed_references_oversized.load(Ordering::Relaxed),
        }
    }
}

/// Error returned when acquiring an in-flight confirmed notification slot fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InFlightAcquireError {
    /// In-flight limit for this peer was exceeded.
    PeerLimitExceeded,
    /// Global pool of confirmed notification permits was exhausted.
    GlobalPoolExhausted,
}

/// Tracker for concurrent in-flight confirmed notifications per peer.
#[derive(Debug, Default)]
pub struct CovInFlightTracker {
    peer_in_flight: Mutex<HashMap<CovRecipient, usize>>,
    /// Wakes, once a notification's peer slot and pool permit are both
    /// back, what waits for one to end: references owed a look (#2661,
    /// #2671).
    ended: tokio::sync::Notify,
    /// The warning for a confirmed report held back by the full pool, and
    /// the one for a report held back for want of an invoke ID: both wait
    /// and lose nothing, so each warns at most once an interval (#2671).
    pool_warning: WarnThrottle,
    id_warning: WarnThrottle,
    /// The warning for a confirmed report refused because its subscriber is
    /// at a broadcast or group form on another network (#2732).
    broadcast_warning: WarnThrottle,
}

/// RAII permit for an in-flight confirmed notification holding both a peer slot and a global permit.
pub struct CovInFlightGuard {
    peer: CovRecipient,
    tracker: Arc<CovInFlightTracker>,
    /// Given back first when the guard drops, so the wake that follows
    /// finds the pool's permit returned too.
    global_permit: Option<OwnedSemaphorePermit>,
}

impl std::fmt::Debug for CovInFlightGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CovInFlightGuard")
            .field("peer", &self.peer)
            .finish()
    }
}

impl Drop for CovInFlightGuard {
    fn drop(&mut self) {
        drop(self.global_permit.take());
        self.tracker.release(&self.peer);
    }
}

impl CovInFlightTracker {
    /// Attempt to acquire an in-flight confirmed notification slot.
    pub fn try_acquire(
        self: &Arc<Self>,
        peer: CovRecipient,
        max_per_peer: usize,
        global_semaphore: &Arc<Semaphore>,
    ) -> Result<CovInFlightGuard, InFlightAcquireError> {
        let mut guard = self.peer_in_flight.locked();
        let current = guard.get(&peer).copied().unwrap_or(0);
        if current >= max_per_peer {
            return Err(InFlightAcquireError::PeerLimitExceeded);
        }
        let global_permit = match global_semaphore.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => return Err(InFlightAcquireError::GlobalPoolExhausted),
        };
        guard.insert(peer.clone(), current + 1);
        Ok(CovInFlightGuard {
            peer,
            tracker: Arc::clone(self),
            global_permit: Some(global_permit),
        })
    }

    /// The throttle on the warning for a report the full pool held back.
    pub(crate) fn pool_warning(&self) -> &WarnThrottle {
        &self.pool_warning
    }

    /// The throttle on the warning for a report held back for want of an
    /// invoke ID.
    pub(crate) fn id_warning(&self) -> &WarnThrottle {
        &self.id_warning
    }

    /// The throttle on the warning for a confirmed report refused because
    /// its subscriber is at a broadcast or group form (#2732).
    pub(crate) fn broadcast_warning(&self) -> &WarnThrottle {
        &self.broadcast_warning
    }

    /// Each peer's confirmed notifications in flight now.
    pub(crate) fn in_flight(&self) -> HashMap<CovRecipient, usize> {
        self.peer_in_flight.locked().clone()
    }

    /// Completes once a confirmed notification in flight ends after this
    /// is made, whether or not it was polled by then.
    pub(crate) fn ended(&self) -> tokio::sync::futures::Notified<'_> {
        self.ended.notified()
    }

    #[cfg(test)]
    pub(crate) fn active_peer_count(&self) -> usize {
        self.peer_in_flight.lock().unwrap().len()
    }

    fn release(&self, peer: &CovRecipient) {
        let mut guard = self.peer_in_flight.locked();
        if let Some(count) = guard.get_mut(peer) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                guard.remove(peer);
            }
        }
        drop(guard);
        self.ended.notify_waiters();
    }
}
