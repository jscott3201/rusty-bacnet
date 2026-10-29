//! COV policies, canonical-recipient accounting, and telemetry counters.

use super::CovRecipient;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use bacnet_types::MacAddr;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::error::Error;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Configuration policy for COV subscriptions and notification work budgets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CovPolicy {
    /// Global maximum number of active COV subscriptions across all peers.
    pub max_subscriptions_global: usize,
    /// Maximum number of active COV subscriptions allowed for a single peer.
    pub max_subscriptions_per_peer: usize,
    /// Number of subscription slots reserved for reserved peers.
    pub reserved_capacity: usize,
    /// List of MAC addresses of direct peers permitted to use reserved subscription capacity.
    pub reserved_peers: Vec<MacAddr>,
    /// List of canonical peer keys permitted to use reserved subscription capacity.
    pub reserved_recipients: Vec<CovRecipient>,
    /// Whether indefinite (infinite lifetime) subscriptions are permitted.
    pub allow_indefinite_subscriptions: bool,
    /// Maximum number of indefinite subscriptions allowed for a single peer.
    pub max_indefinite_per_peer: usize,
    /// Maximum notifications emitted per single COV event (fanout budget).
    pub max_notifications_per_event: usize,
    /// Maximum bytes across all notifications emitted per single COV event.
    pub max_notification_bytes_per_event: usize,
    /// Maximum in-flight confirmed COV notifications per peer.
    pub max_confirmed_in_flight_per_peer: usize,
}

impl Default for CovPolicy {
    fn default() -> Self {
        Self {
            max_subscriptions_global: 1024,
            max_subscriptions_per_peer: 64,
            reserved_capacity: 64,
            reserved_peers: Vec::new(),
            reserved_recipients: Vec::new(),
            allow_indefinite_subscriptions: true,
            max_indefinite_per_peer: 16,
            max_notifications_per_event: 64,
            max_notification_bytes_per_event: 65_536,
            max_confirmed_in_flight_per_peer: 16,
        }
    }
}

impl CovPolicy {
    /// Return an unlimited policy with maximum quotas and no restrictions.
    pub fn unlimited() -> Self {
        Self {
            max_subscriptions_global: usize::MAX,
            max_subscriptions_per_peer: usize::MAX,
            reserved_capacity: 0,
            reserved_peers: Vec::new(),
            reserved_recipients: Vec::new(),
            allow_indefinite_subscriptions: true,
            max_indefinite_per_peer: usize::MAX,
            max_notifications_per_event: usize::MAX,
            max_notification_bytes_per_event: usize::MAX,
            max_confirmed_in_flight_per_peer: usize::MAX,
        }
    }

    /// Reject a policy that would refuse all COV work or reserve capacity for
    /// a peer that can never be matched. The server runs this before it
    /// starts a transport.
    ///
    /// The subscription caps, the per-event notification and byte budgets
    /// and the confirmed in-flight limit must be positive. `reserved_capacity`
    /// and `max_indefinite_per_peer` may be zero, and a value larger than the
    /// cap it shares is clamped by [`sanitized`](Self::sanitized), not
    /// refused. Reserved entries follow the DCC source restriction rule: a
    /// MAC of 1 to [`BACnetAddress::MAX_MAC_LEN`] (18) octets, and for a
    /// routed recipient a network in 1..=65534. The network layer drops any
    /// other source, direct or routed (#1141, #1198), so such an entry could
    /// never match a subscriber (#1199).
    pub fn validate(&self) -> Result<(), Error> {
        for (name, value) in [
            ("max_subscriptions_global", self.max_subscriptions_global),
            (
                "max_subscriptions_per_peer",
                self.max_subscriptions_per_peer,
            ),
            (
                "max_notifications_per_event",
                self.max_notifications_per_event,
            ),
            (
                "max_notification_bytes_per_event",
                self.max_notification_bytes_per_event,
            ),
            (
                "max_confirmed_in_flight_per_peer",
                self.max_confirmed_in_flight_per_peer,
            ),
        ] {
            if value == 0 {
                return Err(Error::Encoding(format!(
                    "COV policy {name} must be positive"
                )));
            }
        }
        let mac_length = |field: &str, mac: &MacAddr| {
            if (1..=BACnetAddress::MAX_MAC_LEN).contains(&mac.len()) {
                Ok(())
            } else {
                Err(Error::Encoding(format!(
                    "COV policy {field} entries need a MAC of 1..={} octets",
                    BACnetAddress::MAX_MAC_LEN
                )))
            }
        };
        for mac in &self.reserved_peers {
            mac_length("reserved_peers", mac)?;
        }
        for recipient in &self.reserved_recipients {
            match recipient {
                CovRecipient::Direct(mac) => mac_length("reserved_recipients", mac)?,
                CovRecipient::Routed(source) => {
                    if !(1..=65534).contains(&source.network) {
                        return Err(Error::Encoding(
                            "COV policy reserved_recipients networks must be 1..=65534".into(),
                        ));
                    }
                    mac_length("reserved_recipients", &source.mac_address)?;
                }
            }
        }
        Ok(())
    }

    /// Return a sanitized copy with valid bounds.
    pub fn sanitized(&self) -> Self {
        let mut policy = self.clone();
        policy.reserved_capacity = policy
            .reserved_capacity
            .min(policy.max_subscriptions_global);
        policy.max_indefinite_per_peer = policy
            .max_indefinite_per_peer
            .min(policy.max_subscriptions_per_peer);
        policy
    }

    /// Check if a peer is in the reserved peers list.
    pub fn is_peer_reserved(&self, peer: &CovRecipient) -> bool {
        if self.reserved_recipients.contains(peer) {
            return true;
        }
        match peer {
            CovRecipient::Direct(mac) => self.reserved_peers.contains(mac),
            CovRecipient::Routed(_) => false,
        }
    }

    /// Return the effective unreserved capacity available to unreserved peers.
    ///
    /// When neither `reserved_peers` nor `reserved_recipients` is configured or
    /// `reserved_capacity` is 0, no capacity is set aside, and the entire global
    /// capacity is available to unreserved peers.
    pub fn effective_unreserved_capacity(&self) -> usize {
        if (self.reserved_peers.is_empty() && self.reserved_recipients.is_empty())
            || self.reserved_capacity == 0
        {
            self.max_subscriptions_global
        } else {
            self.max_subscriptions_global
                .saturating_sub(self.reserved_capacity)
        }
    }
}

/// Telemetry counters for COV operations.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CovCounters {
    /// Number of currently active subscriptions.
    pub subscriptions_active: u64,
    /// Total number of subscriptions created.
    pub subscriptions_created: u64,
    /// Total number of subscriptions rejected due to per-peer quota.
    pub subscriptions_rejected_quota: u64,
    /// Total number of subscriptions rejected due to global or reserved capacity.
    pub subscriptions_rejected_capacity: u64,
    /// Total number of subscriptions rejected due to indefinite policy or quota.
    pub subscriptions_rejected_indefinite: u64,
    /// Total number of subscriptions explicitly cancelled.
    pub subscriptions_cancelled: u64,
    /// Total number of expired subscriptions purged.
    pub subscriptions_purged: u64,
    /// Total number of COV notifications sent (confirmed + unconfirmed).
    pub notifications_sent: u64,
    /// Total number of confirmed COV notifications sent.
    pub notifications_confirmed: u64,
    /// Total number of unconfirmed COV notifications sent.
    pub notifications_unconfirmed: u64,
    /// Total number of notification payload bytes sent.
    pub notification_bytes_sent: u64,
    /// Total number of notifications throttled due to per-event fanout or byte budget.
    pub notifications_throttled_fanout: u64,
    /// Total number of notifications throttled due to peer in-flight confirmed limit.
    pub notifications_throttled_peer: u64,
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
    pub timed_changes_dropped: u64,
    /// Total number of times a COV-multiple report left out an untimestamped
    /// reference because its values alone exceed one notification to the
    /// subscriber. The value is not queued: the reference is evaluated again
    /// at its next fanout, and each report that leaves it out counts once.
    pub untimed_references_oversized: u64,
}

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
    /// Total number of notifications throttled due to peer in-flight confirmed limit.
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
}

impl AtomicCovCounters {
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
}

/// RAII permit for an in-flight confirmed notification holding both a peer slot and a global permit.
pub struct CovInFlightGuard {
    peer: CovRecipient,
    tracker: Arc<CovInFlightTracker>,
    _global_permit: OwnedSemaphorePermit,
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
        let mut guard = self.peer_in_flight.lock().unwrap();
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
            _global_permit: global_permit,
        })
    }

    #[cfg(test)]
    pub(crate) fn active_peer_count(&self) -> usize {
        self.peer_in_flight.lock().unwrap().len()
    }

    fn release(&self, peer: &CovRecipient) {
        let mut guard = self.peer_in_flight.lock().unwrap();
        if let Some(count) = guard.get_mut(peer) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                guard.remove(peer);
            }
        }
    }
}
