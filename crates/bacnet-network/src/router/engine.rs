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
//! The engine's counts are taken, which starts them again from zero, and
//! added to `u64` totals in atomics before each lock is let go
//! ([`EngineGuard`]), so reading them never takes the lock and they count
//! from the router's start ([`Counters`]).
//!
//! The engine runs on Tokio time from one origin, taken when the router
//! starts ([`tokio_clock`]), so paused-time tests drive its timers as they
//! drive the transports'.

use std::ops::{Deref, DerefMut};
use std::sync::{Mutex, MutexGuard, PoisonError};

use bacnet_npdu::NpduLimits;
use bacnet_router_core::router::{
    ConfigError, Counters as EngineCounters, Options, PortConfig, MIN_PORT_LIMIT,
};
use bacnet_router_core::{Instant, Router, Storage};
use bacnet_time::HostClock;

use super::port_groups::PortGroups;
use super::Counters;
use crate::totals::totals;

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
pub(super) const SCRATCH: usize = 65_535 + 2 + 1 + bacnet_npdu::MAX_MAC_LEN;

/// The global broadcasts the hosted router remembers for duplicate
/// suppression ([`RouterOptions::duplicates`](super::RouterOptions::duplicates)):
/// enough for 512 within one time to live. 12 KiB, boxed with the rest.
pub(super) const DUPLICATE_RECORD: usize = 512;

/// What a hosted router's engine keeps, including its owned port rules, boxed.
type HostedStorage =
    Storage<PORTS, ROUTE_CAP, HOLD_RECORDS, SEARCHES, SCRATCH, PortGroups, DUPLICATE_RECORD>;

/// The engine as the hosted router runs it.
pub(super) type HostedRouter = Router<
    Box<HostedStorage>,
    PORTS,
    ROUTE_CAP,
    HOLD_RECORDS,
    SEARCHES,
    SCRATCH,
    PortGroups,
    DUPLICATE_RECORD,
>;

/// The hosted router's options: its policy values and timers, and the
/// 256-route cap. [`RouterOptions`](super::RouterOptions) turns on the
/// opt-in policies over these.
pub(super) const OPTIONS: Options = Options::HOSTED;

// The engine's route cap is the table's size, which the crate publishes as
// `ROUTE_CAP`: the two can't drift apart.
const _: () = assert!(
    matches!(OPTIONS.route_cap, Some(cap) if cap as usize == ROUTE_CAP),
    "the hosted route cap is ROUTE_CAP"
);

/// Tokio time on the engine's nanosecond timeline, from now: the router's
/// start. It reads `tokio::time::Instant`, so paused time moves it.
///
/// bacnet-transport has the same constructor (`runtime_clock.rs`).
pub(super) fn tokio_clock() -> HostClock {
    let now = || tokio::time::Instant::now().into_std();
    HostClock::new(now()).reading(now)
}

/// The engine of one router, with its clock and its published counts.
pub(super) struct Engine {
    router: Mutex<HostedRouter>,
    clock: HostClock,
    published: Published,
}

impl Engine {
    /// An engine on `ports`, in port order, with `options` (the hosted
    /// [`OPTIONS`] and what the router's options turn on), started on
    /// `clock`: each port's network installed as a direct route and the
    /// first sweep a period away. Refused as the core refuses: no port,
    /// more than 255, one on network 0 or 0xFFFF, two on one network, or a
    /// router MAC longer than 18 octets.
    pub(super) fn new(
        ports: &[PortConfig<PortGroups>],
        clock: HostClock,
        options: Options,
    ) -> Result<Self, ConfigError> {
        let router = Router::new(boxed_storage(), ports, options, clock.now())?;
        Ok(Self {
            router: Mutex::new(router),
            clock,
            published: Published::default(),
        })
    }

    /// A port on `network` with the router's `mac` on it, carrying the
    /// NPDUs its transport reports ([`Self::limits`]).
    pub(super) fn port(
        network: u16,
        mac: bacnet_npdu::Mac,
        max_npdu: NpduLimits,
        groups: PortGroups,
    ) -> PortConfig<PortGroups> {
        PortConfig::new(network, mac, Self::limits(max_npdu), groups)
    }

    /// The engine's limits for a port whose transport reports `reported`
    /// (`TransportPort::max_npdu`, #1665), at start or later (#1696). A
    /// report under [`MIN_PORT_LIMIT`] counts as that: the engine refuses a
    /// smaller limit, and the transport still refuses what it can't carry.
    /// None is over the scratch, which holds more than any `u16`.
    pub(super) fn limits(reported: NpduLimits) -> NpduLimits {
        NpduLimits {
            unicast: reported.unicast.max(MIN_PORT_LIMIT),
            broadcast: reported.broadcast.max(MIN_PORT_LIMIT),
        }
    }

    /// The engine, locked. Hold the guard for one call and its drain, never
    /// across an await. The lock isn't reentrant: a thread that holds the
    /// guard and asks for it again waits for itself forever.
    ///
    /// Under the guard, the engine's own `counters()` are the counts since
    /// the lock was taken: the last guard took the rest.
    pub(super) fn lock(&self) -> EngineGuard<'_> {
        let router = self.router.lock().unwrap_or_else(PoisonError::into_inner);
        EngineGuard {
            router,
            published: &self.published,
        }
    }

    /// The engine's totals since start, as of the last lock to be let go.
    /// Reading them takes no lock, so a thread that holds the guard, or a
    /// hook running under it, may read them too.
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
    pub(super) fn clock(&self) -> HostClock {
        self.clock
    }

    /// The engine's time now.
    pub(super) fn now(&self) -> Instant {
        self.clock.now()
    }
}

/// The engine, locked. Dropping it takes the engine's counts, adds them to
/// the published totals when the call under it counted anything, and then
/// lets the lock go.
pub(super) struct EngineGuard<'a> {
    router: MutexGuard<'a, HostedRouter>,
    published: &'a Published,
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
        // Still under the lock, so additions come in the engine's order
        // and a total never goes back.
        let counts = self.router.take_counters();
        if counts != EngineCounters::default() {
            self.published.add(&counts);
        }
    }
}

totals!(
    /// The engine's totals since start, one atomic each. Only a guard
    /// being dropped changes them, under the engine's lock, each after its
    /// call's sends are queued.
    Published: EngineCounters => Counters {
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
    busy_announced,
    available_announced,
    duplicate_drops,
    not_next_hop,
    originated,
    }
);

/// Zeroed storage on the heap. Built here, outside any async function, so
/// the large table and scratch never sit in a future's state.
#[inline(never)]
fn boxed_storage() -> Box<HostedStorage> {
    Box::new(Storage::new())
}

#[cfg(test)]
mod tests {
    use bacnet_npdu::Mac;
    use bacnet_npdu::NpduLimits;
    use bacnet_router_core::router::{Frame, Permissive};
    use bacnet_router_core::PortId;

    use super::*;
    use std::sync::atomic::Ordering;

    /// An engine on networks 1 and 2.
    fn engine() -> Engine {
        let port = |network, mac| {
            Engine::port(
                network,
                Mac::from_array([mac]),
                NpduLimits::default(),
                PortGroups::default(),
            )
        };
        Engine::new(&[port(1, 1), port(2, 2)], tokio_clock(), OPTIONS).unwrap()
    }

    /// Each guard takes the engine's `u32` counts as it is dropped and adds
    /// them to the totals, so the totals count from the router's start while
    /// the engine counts only since the last lock (decision D3).
    #[tokio::test]
    async fn totals_count_from_start_as_each_guard_drains_the_engine() {
        let engine = engine();
        let short = [0x01];
        let malformed = Frame {
            port: PortId(0),
            source: &[9],
            link_group: false,
            npdu: &short,
        };
        for frames in 1..=3u64 {
            let mut router = engine.lock();
            assert_eq!(router.counters().malformed, 0, "the last guard took them");
            let _ = router.receive(malformed, engine.now(), &mut Permissive);
            assert_eq!(router.counters().malformed, 1);
            drop(router);
            assert_eq!(engine.counters().malformed, frames);
        }
    }

    /// A total saturates at `u64::MAX`, however much a take adds.
    #[test]
    fn totals_saturate() {
        let published = Published::default();
        published.malformed.store(u64::MAX - 1, Ordering::Relaxed);
        let mut counts = EngineCounters::default();
        counts.malformed = 5;
        counts.rejects = 2;
        published.add(&counts);
        let totals = published.load();
        assert_eq!((totals.malformed, totals.rejects), (u64::MAX, 2));
    }
}
