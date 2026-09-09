//! Local fail-fast handler policy, not a protocol or throughput guarantee.
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::request_peer::CanonicalRequester;

use bacnet_types::error::Error;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[path = "recovery_classifier.rs"]
mod recovery_classifier;
pub(super) use recovery_classifier::confirmed_class;

/// Global and logical-peer top-level handler limits with a strict recovery reserve.
/// Defaults are provisional owner policy, not benchmark-derived values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestAdmissionPolicy {
    /// Maximum concurrent confirmed handlers (default 64).
    pub max_confirmed_in_flight: usize,
    /// Maximum concurrent unconfirmed handlers (default 32).
    pub max_unconfirmed_in_flight: usize,
    /// Maximum concurrent ordinary confirmed handlers per logical peer (default 16).
    /// Clamped to global capacity minus the reserve; independent of recovery.
    pub max_confirmed_in_flight_per_peer: usize,
    /// Maximum concurrent unconfirmed handlers per logical peer (default 8).
    /// This partitions capacity by identity; it does not guarantee fairness.
    pub max_unconfirmed_in_flight_per_peer: usize,
    /// Strict DCC ENABLE partition inside the confirmed global limit (default 4).
    /// Zero disables protection; otherwise must be smaller than the global limit.
    pub confirmed_recovery_reserve: usize,
    /// Independent protected per-peer limit (default 1), always positive.
    /// Clamped only to the reserve; zero reserve uses ordinary accounting instead.
    pub max_recovery_in_flight_per_peer: usize,
}

impl Default for RequestAdmissionPolicy {
    fn default() -> Self {
        Self {
            max_confirmed_in_flight: 64,
            max_unconfirmed_in_flight: 32,
            max_confirmed_in_flight_per_peer: 16,
            max_unconfirmed_in_flight_per_peer: 8,
            confirmed_recovery_reserve: 4,
            max_recovery_in_flight_per_peer: 1,
        }
    }
}

impl RequestAdmissionPolicy {
    /// Reject zero and limits that the underlying semaphore cannot represent.
    pub fn validate(&self) -> Result<(), Error> {
        for (name, value) in [
            ("max_confirmed_in_flight", self.max_confirmed_in_flight),
            ("max_unconfirmed_in_flight", self.max_unconfirmed_in_flight),
            (
                "max_recovery_in_flight_per_peer",
                self.max_recovery_in_flight_per_peer,
            ),
            (
                "max_confirmed_in_flight_per_peer",
                self.max_confirmed_in_flight_per_peer,
            ),
            (
                "max_unconfirmed_in_flight_per_peer",
                self.max_unconfirmed_in_flight_per_peer,
            ),
        ] {
            if value == 0 || value > Semaphore::MAX_PERMITS {
                return Err(Error::Encoding(format!(
                    "{name} must be in 1..={}",
                    Semaphore::MAX_PERMITS
                )));
            }
        }
        if self.confirmed_recovery_reserve >= self.max_confirmed_in_flight {
            return Err(Error::Encoding(
                "confirmed_recovery_reserve must be smaller than max_confirmed_in_flight".into(),
            ));
        }
        Ok(())
    }
}

/// Independently sampled counters for one server lifetime, not an atomic aggregate.
/// Admitted totals count registrations, not successful responses. Active counts
/// include registered futures not yet polled and exclude completed unreaped tasks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RequestAdmissionCounters {
    /// Live protected handlers, also included in confirmed_active.
    pub recovery_active: usize,
    /// Protected registrations, also included in confirmed_admitted_total.
    pub recovery_admitted_total: u64,
    /// Protected capacity rejections, also included in confirmed_overloaded_total.
    pub recovery_overloaded_total: u64,
    /// Live confirmed handlers.
    pub confirmed_active: usize,
    /// Confirmed handlers registered since startup.
    pub confirmed_admitted_total: u64,
    /// Confirmed requests rejected for exhausted handler capacity.
    pub confirmed_overloaded_total: u64,
    /// Confirmed capacity rejections classified partition-first (ordinary or protected).
    pub confirmed_global_overloaded_total: u64,
    /// Confirmed rejections with a partition permit but no slot in that peer partition.
    pub confirmed_peer_overloaded_total: u64,
    /// Confirmed registrations rejected after shutdown sealed the owner.
    pub confirmed_shutdown_rejected_total: u64,
    /// Live unconfirmed handlers.
    pub unconfirmed_active: usize,
    /// Unconfirmed handlers registered since startup.
    pub unconfirmed_admitted_total: u64,
    /// Unconfirmed requests dropped for exhausted handler capacity.
    pub unconfirmed_overloaded_total: u64,
    /// Unconfirmed capacity rejections classified global-first.
    pub unconfirmed_global_overloaded_total: u64,
    /// Unconfirmed capacity rejections with a global permit but no peer slot.
    pub unconfirmed_peer_overloaded_total: u64,
    /// Unconfirmed registrations rejected after shutdown sealed the owner.
    pub unconfirmed_shutdown_rejected_total: u64,
    /// Live overload Abort send workers (private fixed capacity 8).
    pub abort_active: usize,
    /// Overload Abort workers registered, not necessarily successfully sent.
    pub abort_admitted_total: u64,
    /// Confirmed overloads silently dropped because all Abort workers were busy.
    pub confirmed_fallback_dropped_total: u64,
    /// Abort registrations rejected after shutdown sealed the owner.
    pub abort_shutdown_rejected_total: u64,
}

#[derive(Clone, Copy)]
pub(super) enum Class {
    Confirmed = 0,
    Unconfirmed = 1,
    Abort = 2,
    Recovery = 3,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Rejection {
    Closed,
    Overloaded,
}

pub(super) struct Admission {
    pools: [Arc<Pool>; 3],
    recovery: Arc<Pool>,
    recovery_enabled: bool,
}

struct Pool {
    permits: Arc<Semaphore>,
    peer_limit: usize,
    peers: Mutex<HashMap<CanonicalRequester, usize>>,
    active: AtomicUsize,
    admitted: AtomicU64,
    overloaded: AtomicU64,
    global_overloaded: AtomicU64,
    peer_overloaded: AtomicU64,
    closed: AtomicU64,
}

impl Pool {
    fn new(limit: usize, peer_limit: usize) -> Arc<Self> {
        Arc::new(Self {
            permits: Arc::new(Semaphore::new(limit)),
            peer_limit,
            peers: Mutex::new(HashMap::new()),
            active: AtomicUsize::new(0),
            admitted: AtomicU64::new(0),
            overloaded: AtomicU64::new(0),
            global_overloaded: AtomicU64::new(0),
            peer_overloaded: AtomicU64::new(0),
            closed: AtomicU64::new(0),
        })
    }
}

pub(super) struct Guard {
    pool: Arc<Pool>,
    // Counters only: recovery never registers in the ordinary peer map.
    aggregate: Option<Arc<Pool>>,
    peer: Option<CanonicalRequester>,
    _permit: OwnedSemaphorePermit,
}

impl Drop for Guard {
    fn drop(&mut self) {
        // Remove registration before Rust drops _permit and releases global
        // capacity. This lock never acquires the task owner's lock.
        if let Some(peer) = &self.peer {
            let mut peers = self.pool.peers.lock().unwrap_or_else(|e| e.into_inner());
            let count = peers.get_mut(peer).expect("live peer registration");
            *count -= 1;
            if *count == 0 {
                peers.remove(peer);
            }
        }
        for pool in std::iter::once(&self.pool).chain(self.aggregate.iter()) {
            pool.active.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

impl Admission {
    #[cfg(test)]
    pub(super) fn peer_entries(&self) -> [usize; 3] {
        let mut entries = std::array::from_fn(|i| self.pools[i].peers.lock().unwrap().len());
        // Count entries in both confirmed maps, not distinct confirmed identities.
        entries[0] += self.recovery.peers.lock().unwrap().len();
        entries
    }

    pub(super) fn new(policy: RequestAdmissionPolicy) -> Result<Self, Error> {
        policy.validate()?;
        Ok(Self {
            pools: [
                Pool::new(
                    policy.max_confirmed_in_flight - policy.confirmed_recovery_reserve,
                    policy
                        .max_confirmed_in_flight_per_peer
                        .min(policy.max_confirmed_in_flight - policy.confirmed_recovery_reserve),
                ),
                Pool::new(
                    policy.max_unconfirmed_in_flight,
                    policy
                        .max_unconfirmed_in_flight_per_peer
                        .min(policy.max_unconfirmed_in_flight),
                ),
                Pool::new(8, 8),
            ],
            recovery: Pool::new(
                policy.confirmed_recovery_reserve,
                policy
                    .max_recovery_in_flight_per_peer
                    .min(policy.confirmed_recovery_reserve),
            ),
            recovery_enabled: policy.confirmed_recovery_reserve != 0,
        })
    }

    // Called under the task owner's spawn/close lock. No capacity waiters exist.
    pub(super) fn try_enter(
        &self,
        class: Class,
        peer: CanonicalRequester,
        closed: bool,
    ) -> Result<Guard, Rejection> {
        let (pool, aggregate) = if matches!(class, Class::Recovery) {
            if self.recovery_enabled {
                (&self.recovery, Some(&self.pools[0]))
            } else {
                (&self.pools[0], None)
            }
        } else {
            (&self.pools[class as usize], None)
        };
        if closed {
            pool.closed.fetch_add(1, Ordering::Relaxed);
            if let Some(total) = aggregate {
                total.closed.fetch_add(1, Ordering::Relaxed);
            }
            return Err(Rejection::Closed);
        }
        let permit = Arc::clone(&pool.permits).try_acquire_owned().map_err(|_| {
            pool.overloaded.fetch_add(1, Ordering::Relaxed);
            pool.global_overloaded.fetch_add(1, Ordering::Relaxed);
            if let Some(total) = aggregate {
                total.overloaded.fetch_add(1, Ordering::Relaxed);
                total.global_overloaded.fetch_add(1, Ordering::Relaxed);
            }
            Rejection::Overloaded
        })?;
        // Spawn serialization belongs to RequestTasks. Peer registrations are
        // partition-local, while confirmed counters remain inclusive. Only one
        // map is locked, so no cross-partition registration or rollback is needed.
        // Abort workers are global-only, with no peer registration or charge.
        let peer = if matches!(class, Class::Abort) {
            None
        } else {
            let mut peers = pool.peers.lock().unwrap_or_else(|e| e.into_inner());
            if peers.get(&peer).copied().unwrap_or(0) >= pool.peer_limit {
                pool.overloaded.fetch_add(1, Ordering::Relaxed);
                pool.peer_overloaded.fetch_add(1, Ordering::Relaxed);
                if let Some(total) = aggregate {
                    total.overloaded.fetch_add(1, Ordering::Relaxed);
                    total.peer_overloaded.fetch_add(1, Ordering::Relaxed);
                }
                return Err(Rejection::Overloaded);
            }
            *peers.entry(peer.clone()).or_default() += 1;
            Some(peer)
        };
        pool.active.fetch_add(1, Ordering::Relaxed);
        pool.admitted.fetch_add(1, Ordering::Relaxed);
        if let Some(total) = aggregate {
            total.active.fetch_add(1, Ordering::Relaxed);
            total.admitted.fetch_add(1, Ordering::Relaxed);
        }
        Ok(Guard {
            pool: Arc::clone(pool),
            aggregate: aggregate.map(Arc::clone),
            peer,
            _permit: permit,
        })
    }

    pub(super) fn snapshot(&self) -> RequestAdmissionCounters {
        let [c, u, a] = &self.pools;
        RequestAdmissionCounters {
            recovery_active: self.recovery.active.load(Ordering::Relaxed),
            recovery_admitted_total: self.recovery.admitted.load(Ordering::Relaxed),
            recovery_overloaded_total: self.recovery.overloaded.load(Ordering::Relaxed),
            confirmed_active: c.active.load(Ordering::Relaxed),
            confirmed_admitted_total: c.admitted.load(Ordering::Relaxed),
            confirmed_overloaded_total: c.overloaded.load(Ordering::Relaxed),
            confirmed_global_overloaded_total: c.global_overloaded.load(Ordering::Relaxed),
            confirmed_peer_overloaded_total: c.peer_overloaded.load(Ordering::Relaxed),
            confirmed_shutdown_rejected_total: c.closed.load(Ordering::Relaxed),
            unconfirmed_active: u.active.load(Ordering::Relaxed),
            unconfirmed_admitted_total: u.admitted.load(Ordering::Relaxed),
            unconfirmed_overloaded_total: u.overloaded.load(Ordering::Relaxed),
            unconfirmed_global_overloaded_total: u.global_overloaded.load(Ordering::Relaxed),
            unconfirmed_peer_overloaded_total: u.peer_overloaded.load(Ordering::Relaxed),
            unconfirmed_shutdown_rejected_total: u.closed.load(Ordering::Relaxed),
            abort_active: a.active.load(Ordering::Relaxed),
            abort_admitted_total: a.admitted.load(Ordering::Relaxed),
            confirmed_fallback_dropped_total: a.overloaded.load(Ordering::Relaxed),
            abort_shutdown_rejected_total: a.closed.load(Ordering::Relaxed),
        }
    }
}

impl<T: super::TransportPort + 'static> super::ServerBuilder<T> {
    /// Set positive global and peer handler limits, validated before startup.
    pub fn request_admission_policy(mut self, policy: RequestAdmissionPolicy) -> Self {
        self.config.request_admission_policy = policy;
        self
    }
}

impl super::BipServerBuilder {
    /// Set positive global and peer handler limits, validated before startup.
    pub fn request_admission_policy(mut self, policy: RequestAdmissionPolicy) -> Self {
        self.config.request_admission_policy = policy;
        self
    }
}

#[cfg(feature = "sc-tls")]
impl super::ScServerBuilder {
    /// Set positive global and peer handler limits, validated before TLS dialing.
    pub fn request_admission_policy(mut self, policy: RequestAdmissionPolicy) -> Self {
        self.config.request_admission_policy = policy;
        self
    }
}

impl<T: super::TransportPort + 'static> super::BACnetServer<T> {
    /// Sample admission counters. Available after stop; active counts then are zero.
    pub fn request_admission_counters(&self) -> RequestAdmissionCounters {
        self.request_tasks.counters()
    }
}
