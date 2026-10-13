//! The `bacnet-router-core` engine as the hosted router runs it (#1679).
//!
//! One [`Engine`] per router holds the core's [`Router`] over boxed storage,
//! behind a std mutex. Every port's dispatch, the sweep task and the table
//! guard take that lock for one synchronous call and the drain of what the
//! call asks to send, and none of them holds it across an await. The
//! engine's hooks (group rules, the control gate and its authorizer, the
//! logs) run under it too, so none of them may block or call back into the
//! router. If a hook panics, the lock is poisoned and is still taken as it
//! is: whatever that call left unsent is dropped, and counted as
//! `undrained`, by the engine's next frame.
//!
//! The engine's counts are copied out to atomics before each lock is let
//! go ([`EngineGuard`]), so reading them never takes the lock.
//!
//! The engine runs on Tokio time from one origin, taken when the router
//! starts ([`RouterClock`]), so paused-time tests drive its timers as they
//! drive the transports'.

use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use bacnet_router_core::router::{ConfigError, Counters, Limits, Options, PortConfig};
use bacnet_router_core::{Duration, Instant, Router, Storage};
use bacnet_transport::port::NpduLimits;

use crate::router_table::{HOLD_RECORDS, ROUTE_CAP};

/// The most ports a router has: a routing table's Port ID names 255.
pub(super) const PORTS: usize = 255;

/// The unknown networks the router looks for at once (Clause 6.6.3.1).
pub(super) const SEARCHES: usize = 256;

/// The output scratch: 65,535 octets, the largest BVLC message, plus the
/// SNET, SLEN and longest SADR a rewrite adds. A BACnet/SC NPDU is shorter
/// than that BVLC bound (Table 6-1 puts it at 61,327 octets, and Clause 6.6
/// lets two BACnet/SC ports route NPDUs that long), so any of them fits
/// with its added source address.
pub(super) const SCRATCH: usize = 65_535 + 2 + 1 + bacnet_router_core::MAX_MAC_LEN;

/// What a hosted router's engine keeps, boxed: about 115 KB.
type HostedStorage = Storage<PORTS, ROUTE_CAP, HOLD_RECORDS, SEARCHES, SCRATCH>;

/// The engine as the hosted router runs it.
pub(super) type HostedRouter =
    Router<Box<HostedStorage>, PORTS, ROUTE_CAP, HOLD_RECORDS, SEARCHES, SCRATCH>;

/// The hosted router's options: its policy values and timers, and the
/// 256-route cap.
const OPTIONS: Options = Options::HOSTED;

// The engine's route cap is the table's size, which the crate publishes as
// `ROUTE_CAP`: the two can't drift apart.
const _: () = assert!(
    OPTIONS.route_cap as usize == ROUTE_CAP,
    "the hosted route cap is ROUTE_CAP"
);

/// Tokio time on the engine's nanosecond timeline, from the router's start.
#[derive(Debug, Clone, Copy)]
pub(super) struct RouterClock {
    origin: tokio::time::Instant,
}

impl RouterClock {
    /// A clock whose zero is now.
    pub(super) fn start() -> Self {
        Self {
            origin: tokio::time::Instant::now(),
        }
    }

    /// The engine's time now.
    pub(super) fn now(&self) -> Instant {
        let elapsed = tokio::time::Instant::now().saturating_duration_since(self.origin);
        // Saturates some 584 years after the router started.
        Instant::ZERO + Duration::saturating_from_core(elapsed)
    }

    /// The Tokio instant of an engine time. Both ways are exact, so a timer
    /// set here never wakes before the engine's deadline is due.
    pub(super) fn instant(&self, at: Instant) -> tokio::time::Instant {
        let offset = std::time::Duration::from_nanos(at.as_nanos());
        self.origin
            .checked_add(offset)
            .unwrap_or_else(|| self.origin + std::time::Duration::from_secs(86_400 * 365))
    }
}

/// The engine of one router, with its clock and its published counts.
pub(super) struct Engine {
    router: Mutex<HostedRouter>,
    clock: RouterClock,
    published: Published,
}

impl Engine {
    /// An engine on `ports`, in port order, started on `clock`: each port's
    /// network installed as a direct route and the first sweep a period
    /// away. Refused as the core refuses: no port, more than 255, two on
    /// one network, or a router MAC longer than 18 octets.
    pub(super) fn new(ports: &[PortConfig], clock: RouterClock) -> Result<Self, ConfigError> {
        let router = Router::new(boxed_storage(), ports, OPTIONS, clock.now())?;
        Ok(Self {
            router: Mutex::new(router),
            clock,
            published: Published::default(),
        })
    }

    /// A port on `network` with the router's `mac` on it, carrying the
    /// NPDUs its transport reports (`TransportPort::max_npdu`, #1665). A
    /// report under [`Limits::MIN`] counts as that: the engine refuses a
    /// smaller limit, and the transport still refuses what it can't carry.
    pub(super) fn port(
        network: u16,
        mac: bacnet_router_core::Mac,
        max_npdu: NpduLimits,
    ) -> PortConfig {
        PortConfig {
            network,
            mac,
            max_npdu: Limits {
                unicast: max_npdu.unicast.max(Limits::MIN),
                broadcast: max_npdu.broadcast.max(Limits::MIN),
            },
        }
    }

    /// The engine, locked. Hold the guard for one call and its drain, never
    /// across an await. The lock isn't reentrant: a thread that holds the
    /// guard and asks for it again waits for itself forever.
    pub(super) fn lock(&self) -> EngineGuard<'_> {
        let router = self.router.lock().unwrap_or_else(PoisonError::into_inner);
        EngineGuard {
            before: router.counters(),
            router,
            published: &self.published,
        }
    }

    /// The engine's counts as of the last lock to be let go. Reading them
    /// takes no lock, so a thread that holds the guard, or a hook running
    /// under it, may read them too.
    pub(super) fn counters(&self) -> Counters {
        self.published.load()
    }

    /// Whether someone holds the engine's lock now.
    #[cfg(test)]
    pub(super) fn is_locked(&self) -> bool {
        matches!(
            self.router.try_lock(),
            Err(std::sync::TryLockError::WouldBlock)
        )
    }

    /// The engine's clock.
    pub(super) fn clock(&self) -> RouterClock {
        self.clock
    }

    /// The engine's time now.
    pub(super) fn now(&self) -> Instant {
        self.clock.now()
    }
}

/// The engine, locked. Dropping it publishes the engine's counts, when the
/// call under it changed them, and then lets the lock go.
pub(super) struct EngineGuard<'a> {
    router: MutexGuard<'a, HostedRouter>,
    published: &'a Published,
    /// The counts when the lock was taken.
    before: Counters,
}

impl Deref for EngineGuard<'_> {
    type Target = HostedRouter;

    fn deref(&self) -> &HostedRouter {
        &self.router
    }
}

impl DerefMut for EngineGuard<'_> {
    fn deref_mut(&mut self) -> &mut HostedRouter {
        &mut self.router
    }
}

impl Drop for EngineGuard<'_> {
    fn drop(&mut self) {
        // Still under the lock, so publications come in the engine's order
        // and a count never goes back.
        let counters = self.router.counters();
        if counters != self.before {
            self.published.store(&counters);
        }
    }
}

/// Declares [`Published`]: one atomic for each named field of [`Counters`],
/// and the copies each way.
macro_rules! published {
    ($($field:ident),+ $(,)?) => {
        /// The engine's counts, one atomic each. They are stored under the
        /// engine's lock after its sends are queued (release) and read with
        /// no lock (acquire), so a reader who sees a count also sees the
        /// sends of the call that made it.
        #[derive(Debug, Default)]
        struct Published {
            $($field: AtomicU64,)+
        }

        impl Published {
            /// How many counts it keeps.
            const FIELDS: usize = [$(stringify!($field)),+].len();

            fn store(&self, counters: &Counters) {
                $(self.$field.store(counters.$field, Ordering::Release);)+
            }

            fn load(&self) -> Counters {
                let mut counters = Counters::default();
                $(counters.$field = self.$field.load(Ordering::Acquire);)+
                counters
            }
        }
    };
}

published!(
    address_length_drops,
    global_broadcast_dadr_drops,
    broadcast_pdu_type_drops,
    group_dadr_drops,
    malformed,
    bad_messages,
    unknown_port,
    hop_count_drops,
    denied,
    unroutable_rejects,
    rejects,
    solicitations,
    write_failures,
    egress_too_long,
    refused,
    undrained,
);

// `Counters` holds only `u64` counts, so its size says how many there are:
// a count the engine adds fails this until it is published too.
const _: () = assert!(
    std::mem::size_of::<Counters>() == Published::FIELDS * std::mem::size_of::<u64>(),
    "publish every count in the engine's Counters"
);

/// Zeroed storage on the heap. Built here, outside any async function, so
/// the 115 KB never sit in a future's state.
#[inline(never)]
fn boxed_storage() -> Box<HostedStorage> {
    Box::new(Storage::new())
}
