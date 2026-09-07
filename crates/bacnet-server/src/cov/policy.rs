//! COV policies, rate accounting keys, and telemetry counters.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use bacnet_encoding::npdu::NpduAddress;
use bacnet_types::MacAddr;
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
    /// List of MAC addresses of peers permitted to use reserved subscription capacity.
    pub reserved_peers: Vec<MacAddr>,
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
            allow_indefinite_subscriptions: true,
            max_indefinite_per_peer: usize::MAX,
            max_notifications_per_event: usize::MAX,
            max_notification_bytes_per_event: usize::MAX,
            max_confirmed_in_flight_per_peer: usize::MAX,
        }
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
    pub fn is_peer_reserved(&self, peer: &CovPeerKey) -> bool {
        self.reserved_peers.contains(peer.mac())
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
        }
    }
}

/// Canonical representation of peer identity for COV quota and rate accounting.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CovPeerKey {
    /// Directly connected peer identified by its local MAC address.
    Direct(MacAddr),
    /// Routed peer identified by its network number and remote MAC address.
    Routed(u16, MacAddr),
}

impl CovPeerKey {
    /// Create a direct peer key.
    pub fn direct(mac: MacAddr) -> Self {
        Self::Direct(mac)
    }

    /// Create a routed peer key.
    pub fn routed(network: u16, mac: MacAddr) -> Self {
        Self::Routed(network, mac)
    }

    /// Derive canonical peer key from local MAC and optional routed network address.
    pub fn from_endpoint(mac: &MacAddr, network: Option<&NpduAddress>) -> Self {
        match network {
            Some(dest) if !dest.mac_address.is_empty() => {
                Self::Routed(dest.network, MacAddr::from_slice(&dest.mac_address))
            }
            _ => Self::Direct(mac.clone()),
        }
    }

    /// Get a reference to the underlying MAC address.
    pub fn mac(&self) -> &MacAddr {
        match self {
            Self::Direct(mac) => mac,
            Self::Routed(_, mac) => mac,
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
    peer_in_flight: Mutex<HashMap<CovPeerKey, usize>>,
}

/// RAII permit for an in-flight confirmed notification holding both a peer slot and a global permit.
pub struct CovInFlightGuard {
    peer: CovPeerKey,
    tracker: Arc<CovInFlightTracker>,
    _global_permit: OwnedSemaphorePermit,
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
        peer: CovPeerKey,
        max_per_peer: usize,
        global_semaphore: &Arc<Semaphore>,
    ) -> Result<CovInFlightGuard, InFlightAcquireError> {
        let mut guard = self.peer_in_flight.lock().unwrap();
        let current = guard.entry(peer.clone()).or_insert(0);
        if *current >= max_per_peer {
            return Err(InFlightAcquireError::PeerLimitExceeded);
        }
        let global_permit = match global_semaphore.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => return Err(InFlightAcquireError::GlobalPoolExhausted),
        };
        *current += 1;
        Ok(CovInFlightGuard {
            peer,
            tracker: Arc::clone(self),
            _global_permit: global_permit,
        })
    }

    fn release(&self, peer: &CovPeerKey) {
        let mut guard = self.peer_in_flight.lock().unwrap();
        if let Some(count) = guard.get_mut(peer) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                guard.remove(peer);
            }
        }
    }
}
