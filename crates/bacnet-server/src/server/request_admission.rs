//! Local fail-fast handler policy, not a protocol or throughput guarantee.
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use bacnet_types::error::Error;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Global, independent top-level handler limits. Zero is invalid.
/// Defaults are provisional owner policy, not benchmark-derived values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestAdmissionPolicy {
    /// Maximum concurrent confirmed handlers (default 64).
    pub max_confirmed_in_flight: usize,
    /// Maximum concurrent unconfirmed handlers (default 32).
    pub max_unconfirmed_in_flight: usize,
}

impl Default for RequestAdmissionPolicy {
    fn default() -> Self {
        Self {
            max_confirmed_in_flight: 64,
            max_unconfirmed_in_flight: 32,
        }
    }
}

impl RequestAdmissionPolicy {
    /// Reject zero and limits that the underlying semaphore cannot represent.
    pub fn validate(&self) -> Result<(), Error> {
        for (name, value) in [
            ("max_confirmed_in_flight", self.max_confirmed_in_flight),
            ("max_unconfirmed_in_flight", self.max_unconfirmed_in_flight),
        ] {
            if value == 0 || value > Semaphore::MAX_PERMITS {
                return Err(Error::Encoding(format!(
                    "{name} must be in 1..={}",
                    Semaphore::MAX_PERMITS
                )));
            }
        }
        Ok(())
    }
}

/// Independently sampled counters for one server lifetime, not an atomic aggregate.
/// Admitted totals count registrations, not successful responses. Active counts
/// include registered futures not yet polled and exclude completed unreaped tasks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RequestAdmissionCounters {
    /// Live confirmed handlers.
    pub confirmed_active: usize,
    /// Confirmed handlers registered since startup.
    pub confirmed_admitted_total: u64,
    /// Confirmed requests rejected for exhausted handler capacity.
    pub confirmed_overloaded_total: u64,
    /// Confirmed registrations rejected after shutdown sealed the owner.
    pub confirmed_shutdown_rejected_total: u64,
    /// Live unconfirmed handlers.
    pub unconfirmed_active: usize,
    /// Unconfirmed handlers registered since startup.
    pub unconfirmed_admitted_total: u64,
    /// Unconfirmed requests dropped for exhausted handler capacity.
    pub unconfirmed_overloaded_total: u64,
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
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Rejection {
    Closed,
    Overloaded,
}

pub(super) struct Admission {
    pools: [Arc<Pool>; 3],
}

struct Pool {
    permits: Arc<Semaphore>,
    active: AtomicUsize,
    admitted: AtomicU64,
    overloaded: AtomicU64,
    closed: AtomicU64,
}

impl Pool {
    fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            permits: Arc::new(Semaphore::new(limit)),
            active: AtomicUsize::new(0),
            admitted: AtomicU64::new(0),
            overloaded: AtomicU64::new(0),
            closed: AtomicU64::new(0),
        })
    }
}

pub(super) struct Guard {
    pool: Arc<Pool>,
    _permit: OwnedSemaphorePermit,
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.pool.active.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Admission {
    pub(super) fn new(policy: RequestAdmissionPolicy) -> Result<Self, Error> {
        policy.validate()?;
        Ok(Self {
            pools: [
                Pool::new(policy.max_confirmed_in_flight),
                Pool::new(policy.max_unconfirmed_in_flight),
                Pool::new(8),
            ],
        })
    }

    // Called under the task owner's spawn/close lock. No capacity waiters exist.
    pub(super) fn try_enter(&self, class: Class, closed: bool) -> Result<Guard, Rejection> {
        let pool = &self.pools[class as usize];
        if closed {
            pool.closed.fetch_add(1, Ordering::Relaxed);
            return Err(Rejection::Closed);
        }
        let permit = Arc::clone(&pool.permits).try_acquire_owned().map_err(|_| {
            pool.overloaded.fetch_add(1, Ordering::Relaxed);
            Rejection::Overloaded
        })?;
        pool.active.fetch_add(1, Ordering::Relaxed);
        pool.admitted.fetch_add(1, Ordering::Relaxed);
        Ok(Guard {
            pool: Arc::clone(pool),
            _permit: permit,
        })
    }

    pub(super) fn snapshot(&self) -> RequestAdmissionCounters {
        let [c, u, a] = &self.pools;
        RequestAdmissionCounters {
            confirmed_active: c.active.load(Ordering::Relaxed),
            confirmed_admitted_total: c.admitted.load(Ordering::Relaxed),
            confirmed_overloaded_total: c.overloaded.load(Ordering::Relaxed),
            confirmed_shutdown_rejected_total: c.closed.load(Ordering::Relaxed),
            unconfirmed_active: u.active.load(Ordering::Relaxed),
            unconfirmed_admitted_total: u.admitted.load(Ordering::Relaxed),
            unconfirmed_overloaded_total: u.overloaded.load(Ordering::Relaxed),
            unconfirmed_shutdown_rejected_total: u.closed.load(Ordering::Relaxed),
            abort_active: a.active.load(Ordering::Relaxed),
            abort_admitted_total: a.admitted.load(Ordering::Relaxed),
            confirmed_fallback_dropped_total: a.overloaded.load(Ordering::Relaxed),
            abort_shutdown_rejected_total: a.closed.load(Ordering::Relaxed),
        }
    }
}

impl<T: super::TransportPort + 'static> super::ServerBuilder<T> {
    /// Set positive global handler limits, validated before transport startup.
    pub fn request_admission_policy(mut self, policy: RequestAdmissionPolicy) -> Self {
        self.config.request_admission_policy = policy;
        self
    }
}

impl super::BipServerBuilder {
    /// Set positive global handler limits, validated before transport startup.
    pub fn request_admission_policy(mut self, policy: RequestAdmissionPolicy) -> Self {
        self.config.request_admission_policy = policy;
        self
    }
}

#[cfg(feature = "sc-tls")]
impl super::ScServerBuilder {
    /// Set positive global handler limits, validated before TLS dialing.
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
