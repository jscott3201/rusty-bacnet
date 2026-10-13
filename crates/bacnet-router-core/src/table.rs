//! The routing table (Clause 6.6.1), in caller-sized storage.
//!
//! [`Table`] maps each network the router can reach to a [`Route`]: the
//! port it is reached through, the next router's MAC (none for a directly
//! attached network), and a [`Reachability`] that keeps a lasting failure
//! apart from congestion. Up to `R` routes are kept sorted by network
//! number, so a lookup is a binary search and every listing comes out in
//! ascending order with no sort.
//!
//! The rules are those of the hosted router's table:
//!
//! - **Direct routes are fixed.** No learning claim, reject, flow-control
//!   mark or management write changes one.
//! - **Learning** ([`Table::apply_claim`]): under [`Convergence::Standard`]
//!   each claim applies at once, so the last one wins (Clause 6.6.3.2), and a
//!   route that changes port three times within 60 s is reported as
//!   flapping. [`Convergence::Hardened`], an opt-in, holds a move to another
//!   port until a second claim for the same network and port arrives within
//!   the corroboration window (60 s, inclusive). A new network, or a refresh
//!   on the same port, applies at once in both modes.
//! - **Freshness is split.** `last_seen` is set by learning and management
//!   only, and aging keys off it; forwarding sets `last_used`, which never
//!   extends a route's life. Busy and Available marks touch neither.
//! - **Rejects** ([`Table::apply_reject`]): reason 1 marks a learned route
//!   unreachable and reason 2 busy for 30 s; any other reason leaves it as
//!   it is, and is counted apart (Clause 6.6.3.5).
//!   Each change starts a 30 s hold-down for its network and arrival port,
//!   during which further reject-driven changes from that port are ignored.
//!   Hold-down records live in their own pool of `H`; a full pool evicts the
//!   oldest and counts it ([`ClaimCounters::hold_evicted`]).
//! - **Busy marks** lapse at their deadline, which
//!   [`Table::effective_reachability`] reads inline, so the sweep's cadence
//!   never changes a forwarding decision. Only Router-Available or fresh
//!   learning lifts Unreachable.
//!
//! [`ClaimCounters`] counts the outcomes. The counts saturate and never
//! change a decision.
//!
//! The table stores no timers: the calls that need the busy period, the
//! hold-down or the corroboration window take a [`Timing`], which the
//! router's options own. So [`Table::new`] is all zero bits, and a table in
//! a `static` costs firmware no flash for an initial image.

mod claims;
mod holds;

use core::fmt;

use crate::{swap_apart, Duration, Instant, Mac, PortId};

pub use claims::{LearnRefusal, Learned};
pub use holds::RejectEffect;

/// How far a route can be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reachability {
    /// Usable.
    Reachable,
    /// Congested until the route's busy deadline (Router-Busy, or reject
    /// reason 2).
    Busy,
    /// Failed until Router-Available or fresh learning (reject reason 1).
    Unreachable,
}

/// How a learning claim that moves a route to another port is taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Convergence {
    /// At once: the last claim wins (Clause 6.6.3.2).
    #[default]
    Standard,
    /// After a second claim for the same network and port within the
    /// corroboration window. It authenticates nothing, and it slows
    /// legitimate moves: alternating claims never converge, and a single
    /// claim waits for the window to lapse and a fresh one.
    Hardened,
}

/// The way to one network: a copy of the table's row, which
/// [`Table::lookup`] and [`Table::routes`] hand out.
///
/// Its fields are the table's bookkeeping, so they are read through its
/// getters, and only the table's own calls change the row: a direct route
/// stays direct, and the flap count and timestamps stay the table's. Build
/// one with [`Route::direct`] or [`Route::learned`], to compare with what a
/// lookup returns. It is `#[non_exhaustive]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Route {
    port: PortId,
    direct: bool,
    next_hop: Mac,
    last_seen: Option<Instant>,
    last_used: Option<Instant>,
    reachability: Reachability,
    busy_until: Option<Instant>,
    flap_count: u8,
    last_port_change: Option<Instant>,
}

impl Route {
    /// All zero bits: what an unused slot holds.
    const ZERO: Self = Self {
        port: PortId(0),
        direct: false,
        next_hop: Mac::EMPTY,
        last_seen: None,
        last_used: None,
        reachability: Reachability::Reachable,
        busy_until: None,
        flap_count: 0,
        last_port_change: None,
    };

    /// A directly attached network on `port`.
    #[must_use]
    pub const fn direct(port: PortId) -> Self {
        let mut route = Self::ZERO;
        route.port = port;
        route.direct = true;
        route
    }

    /// A network reached through `next_hop` on `port`, confirmed `now`.
    #[must_use]
    pub const fn learned(port: PortId, next_hop: Mac, now: Instant) -> Self {
        let mut route = Self::ZERO;
        route.port = port;
        route.next_hop = next_hop;
        route.last_seen = Some(now);
        route
    }

    /// The port the network is reached through.
    #[must_use]
    pub const fn port(&self) -> PortId {
        self.port
    }

    /// Whether the network is attached to that port, rather than reached
    /// through another router.
    #[must_use]
    pub const fn is_direct(&self) -> bool {
        self.direct
    }

    /// The next router's MAC on that port; empty for a direct route.
    #[must_use]
    pub const fn next_hop(&self) -> &Mac {
        &self.next_hop
    }

    /// When learning or management last confirmed the route; `None` for a
    /// direct route. Aging keys off it.
    #[must_use]
    pub const fn last_seen(&self) -> Option<Instant> {
        self.last_seen
    }

    /// When the route last carried traffic; `None` until it does, and again
    /// after a move to another port. It never extends the route's life.
    #[must_use]
    pub const fn last_used(&self) -> Option<Instant> {
        self.last_used
    }

    /// The stored state. A lapsed busy mark still reads `Busy` here until
    /// the sweep; [`Table::effective_reachability`] reads it at a time.
    #[must_use]
    pub const fn reachability(&self) -> Reachability {
        self.reachability
    }

    /// When a busy mark lapses.
    #[must_use]
    pub const fn busy_until(&self) -> Option<Instant> {
        self.busy_until
    }

    /// Moves to another port within 60 s of the one before.
    #[must_use]
    pub const fn flap_count(&self) -> u8 {
        self.flap_count
    }

    /// When the route last moved to another port.
    #[must_use]
    pub const fn last_port_change(&self) -> Option<Instant> {
        self.last_port_change
    }

    /// Whether flow control from the router at `peer` on `port` covers it:
    /// learned, through that port, with that next router.
    fn served_via(&self, port: PortId, peer: &[u8]) -> bool {
        !self.direct && self.port == port && self.next_hop == *peer
    }
}

/// A hardened-mode claim waiting for its second (see [`Table::pending`]).
///
/// The table builds it. It is `#[non_exhaustive]`, so a new field isn't a
/// breaking change: a pattern needs `..`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Pending {
    /// The port the claim would move the route to.
    pub port: PortId,
    /// When the first claim came.
    pub first_seen: Instant,
}

/// Counts of how routing claims fared. They saturate, never affect a
/// decision, and name no peer.
///
/// Learning counts cover I-Am-Router and Initialize-Routing-Table-Ack
/// entries the router inspects, not management writes or calls on the
/// table itself. The router records some of them itself.
///
/// It is `#[non_exhaustive]`, so a new count isn't a breaking change.
/// Outside this crate, build one from `ClaimCounters::default()` and set
/// its fields.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ClaimCounters {
    /// Rejects that changed a learned route's state.
    pub reject_applied: u64,
    /// Rejects ignored, for either reason below; each counts once here.
    pub reject_dampened: u64,
    /// Rejects that would change nothing: no route, a direct one, or the
    /// state already in effect. A repeat Busy never extends its deadline.
    pub reject_dampened_same_state: u64,
    /// State-changing rejects ignored within the hold-down.
    pub reject_dampened_hold_down: u64,
    /// Rejects for a learned route with a reason other than 1 or 2, which
    /// leave it as it is (Clause 6.6.3.5).
    pub reject_kept: u64,
    /// Learning claims that added or refreshed a learned route.
    pub learned_ok: u64,
    /// Learning claims that met the router's route cap and ended the
    /// message's learning, and I-Could-Be-Router offers the cap turned
    /// away.
    pub learned_cap_ignored: u64,
    /// Moves that reached the flap warning.
    pub flap_warned: u64,
    /// Hardened-mode claims held pending, new or restarted after expiry.
    pub pending_started: u64,
    /// Moves applied by a second claim within the window.
    pub corroborated_applied: u64,
    /// Pending claims found past the window and dropped, by the next claim
    /// or by the sweep. Other cleanup doesn't count.
    pub pending_expired: u64,
    /// Disconnect-Connection removals ignored, since there are no PTP
    /// connections.
    pub disconnect_removal_ignored: u64,
    /// Hold-down records evicted, oldest first, from a full pool.
    pub hold_evicted: u64,
}

/// The table's timers, which the router's options own and pass to each
/// call that needs one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// How long reject reason 2 marks a route busy.
    pub busy_for: Duration,
    /// How long a reject-driven change holds off the next one for its
    /// network and arrival port.
    pub hold_down: Duration,
    /// How long a hardened-mode claim waits for its second.
    pub corroboration: Duration,
}

impl Timing {
    /// 30 s busy, 30 s hold-down, 60 s corroboration: the hosted router's.
    pub const DEFAULT: Self = Self {
        busy_for: Duration::from_secs(30),
        hold_down: Duration::from_secs(30),
        corroboration: Duration::from_secs(60),
    };
}

impl Default for Timing {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// One stored route and its pending claim.
#[derive(Debug, Clone, Copy)]
struct Slot {
    network: u16,
    route: Route,
    pending: Option<Pending>,
}

/// An unused slot: all zero bits.
const VACANT: Slot = Slot {
    network: 0,
    route: Route::ZERO,
    pending: None,
};

/// Whether `network` names no single network (0, or the global 0xFFFF).
const fn is_reserved(network: u16) -> bool {
    network == 0 || network == 0xFFFF
}

/// A routing table of up to `R` routes and `H` hold-down records.
///
/// A full table refuses a new network: [`add_direct`](Self::add_direct)
/// and [`add_learned`](Self::add_learned) return `false`, and
/// [`learn`](Self::learn) and [`apply_claim`](Self::apply_claim) return
/// [`LearnRefusal::Full`]. Routes it holds can still be refreshed, moved and
/// removed. A router keeps its route cap at or below `R`, so the cap, which
/// it counts, is met first.
///
/// A [`Route`] is a copy of a row, so a route changes only through the
/// table's calls, each of which keeps its rules: [`add_direct`](Self::add_direct)
/// and [`add_learned`](Self::add_learned) install one;
/// [`learn`](Self::learn) and [`apply_claim`](Self::apply_claim) learn one,
/// with flap detection and convergence; [`mark_busy`](Self::mark_busy),
/// [`mark_available`](Self::mark_available) and
/// [`mark_unreachable`](Self::mark_unreachable) set its state;
/// [`touch`](Self::touch) notes its use; and [`remove`](Self::remove) and
/// [`purge_stale`](Self::purge_stale) take it out. None of them makes a
/// learned route direct, or a direct one learned, except by replacing it.
///
/// [`new`](Self::new) is a `const fn` whose result is all zero bits, so a
/// table can live in a zero-initialized static (`.bss`).
#[derive(Clone)]
pub struct Table<const R: usize, const H: usize> {
    slots: [Slot; R],
    len: usize,
    holds: holds::Holds<H>,
    counters: ClaimCounters,
    convergence: Convergence,
}

/// The routes and holds stored, not the unused storage.
impl<const R: usize, const H: usize> fmt::Debug for Table<R, H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Table")
            .field("routes", &self.stored())
            .field("holds", &self.holds)
            .field("counters", &self.counters)
            .field("convergence", &self.convergence)
            .finish()
    }
}

impl<const R: usize, const H: usize> Default for Table<R, H> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const R: usize, const H: usize> Table<R, H> {
    /// How many routes it can hold.
    pub const CAPACITY: usize = R;

    /// An empty table with standard convergence: all zero bits.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            slots: [VACANT; R],
            len: 0,
            holds: holds::Holds::new(),
            counters: ClaimCounters {
                reject_applied: 0,
                reject_dampened: 0,
                reject_dampened_same_state: 0,
                reject_dampened_hold_down: 0,
                reject_kept: 0,
                learned_ok: 0,
                learned_cap_ignored: 0,
                flap_warned: 0,
                pending_started: 0,
                corroborated_applied: 0,
                pending_expired: 0,
                disconnect_removal_ignored: 0,
                hold_evicted: 0,
            },
            convergence: Convergence::Standard,
        }
    }

    /// Forget every route, hold-down record and count, keeping the
    /// convergence mode: what [`new`](Self::new) gives, without building a
    /// second table.
    pub(crate) fn clear(&mut self) {
        self.slots[..self.len].fill(VACANT);
        self.len = 0;
        self.holds.clear();
        self.counters = ClaimCounters::default();
    }

    /// Choose the convergence mode for later claims. Routes stay; a pending
    /// claim waits for its next claim or the sweep, as before. It is a
    /// `const fn`, so a table in a `static` can start hardened.
    pub const fn set_convergence(&mut self, convergence: Convergence) {
        self.convergence = convergence;
    }

    /// The convergence mode.
    #[must_use]
    pub const fn convergence(&self) -> Convergence {
        self.convergence
    }

    /// The claim counts so far.
    #[must_use]
    pub const fn claim_counters(&self) -> ClaimCounters {
        self.counters
    }

    /// How many routes it holds.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether it holds none.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether a new network has no room.
    #[must_use]
    pub const fn is_full(&self) -> bool {
        self.len == R
    }

    /// The stored slots, sorted by network.
    fn stored(&self) -> &[Slot] {
        &self.slots[..self.len]
    }

    /// The index of `network`'s slot, or where it would go.
    fn find(&self, network: u16) -> Result<usize, usize> {
        self.stored()
            .binary_search_by_key(&network, |slot| slot.network)
    }

    /// The slot for `network`, if it is stored.
    fn slot_mut(&mut self, network: u16) -> Option<&mut Slot> {
        let index = self.find(network).ok()?;
        Some(&mut self.slots[index])
    }

    /// Store `route` for `network`, replacing any route and pending claim it
    /// had. `false` when the network is new and there is no room.
    fn put(&mut self, network: u16, route: Route) -> bool {
        let slot = Slot {
            network,
            route,
            pending: None,
        };
        match self.find(network) {
            Ok(index) => self.slots[index] = slot,
            Err(_) if self.len == R => return false,
            Err(index) => {
                // Into the free slot at the end, then back to its place one
                // swap at a time, with no memmove (`swap_apart`).
                self.slots[self.len] = slot;
                for at in (index..self.len).rev() {
                    swap_apart(&mut self.slots, at, at + 1);
                }
                self.len += 1;
            }
        }
        true
    }

    /// The route to `network`.
    #[must_use]
    pub fn lookup(&self, network: u16) -> Option<Route> {
        let index = self.find(network).ok()?;
        Some(self.slots[index].route)
    }

    /// Remove the route to `network`, with its pending claim and its
    /// hold-down records.
    pub fn remove(&mut self, network: u16) -> Option<Route> {
        let index = self.find(network).ok()?;
        let route = self.slots[index].route;
        // The emptied slot moves to the end one swap at a time, with no
        // memmove (`swap_apart`).
        for at in index + 1..self.len {
            swap_apart(&mut self.slots, at - 1, at);
        }
        self.len -= 1;
        self.slots[self.len] = VACANT;
        self.holds.forget(network);
        Some(route)
    }

    /// Every route, in ascending network order. This and the other listings
    /// can be cloned, so a writer can count before it writes.
    pub fn routes(&self) -> impl Iterator<Item = (u16, &Route)> + Clone + '_ {
        self.stored().iter().map(|slot| (slot.network, &slot.route))
    }

    /// Every network, in ascending order.
    pub fn networks(&self) -> impl Iterator<Item = u16> + Clone + '_ {
        self.stored().iter().map(|slot| slot.network)
    }

    /// The networks reached through `port`, in ascending order.
    pub fn networks_on_port(&self, port: PortId) -> impl Iterator<Item = u16> + Clone + '_ {
        self.routes()
            .filter(move |(_, route)| route.port == port)
            .map(|(network, _)| network)
    }

    /// The networks reached through any port but `port`, in ascending order.
    pub fn networks_not_on_port(&self, port: PortId) -> impl Iterator<Item = u16> + Clone + '_ {
        self.routes()
            .filter(move |(_, route)| route.port != port)
            .map(|(network, _)| network)
    }

    /// Install a directly attached network on `port`, replacing whatever
    /// route it had and clearing its records. Network 0 and 0xFFFF are
    /// ignored. `false` when nothing was installed.
    pub fn add_direct(&mut self, network: u16, port: PortId) -> bool {
        if is_reserved(network) {
            return false;
        }
        self.holds.forget(network);
        self.put(network, Route::direct(port))
    }

    /// Install or refresh a learned route at once, without corroboration or
    /// flap detection: a management write, or a call on the table itself.
    ///
    /// It sets `last_seen`, keeps `last_used` on a refresh through the same
    /// port and clears it otherwise, clears any mark, and re-arms the
    /// hold-down. A direct route, network 0 and 0xFFFF are left alone.
    /// `false` when nothing was installed.
    pub fn add_learned(&mut self, network: u16, port: PortId, next_hop: Mac, now: Instant) -> bool {
        if is_reserved(network) {
            return false;
        }
        let last_used = match self.lookup(network) {
            Some(existing) if existing.direct => return false,
            Some(existing) if existing.port == port => existing.last_used,
            _ => None,
        };
        self.holds.forget(network);
        let mut route = Route::learned(port, next_hop, now);
        route.last_used = last_used;
        self.put(network, route)
    }

    /// Mark a learned route busy until `deadline`, unless it is unreachable:
    /// congestion never revives a failed route. Direct routes are never
    /// marked, and neither timestamp moves.
    pub fn mark_busy(&mut self, network: u16, deadline: Instant) {
        if let Some(slot) = self.slot_mut(network) {
            busy(&mut slot.route, deadline);
        }
    }

    /// Make a learned route reachable, lifting a busy or unreachable mark.
    /// Direct routes and timestamps are untouched.
    pub fn mark_available(&mut self, network: u16) {
        if let Some(slot) = self.slot_mut(network) {
            available(&mut slot.route);
        }
    }

    /// [`mark_busy`](Self::mark_busy) every route served via the router at
    /// `peer` on `port`, as a Router-Busy from it that lists no network asks,
    /// and return how many were marked. Unreachable routes stay unreachable.
    pub fn mark_busy_via_peer(&mut self, port: PortId, peer: &[u8], deadline: Instant) -> usize {
        let mut marked = 0;
        for slot in &mut self.slots[..self.len] {
            if slot.route.served_via(port, peer) && busy(&mut slot.route, deadline) {
                marked += 1;
            }
        }
        marked
    }

    /// [`mark_available`](Self::mark_available) every route served via the
    /// router at `peer` on `port`, as a Router-Available from it that lists
    /// no network asks, and return how many there were.
    pub fn mark_available_via_peer(&mut self, port: PortId, peer: &[u8]) -> usize {
        let mut marked = 0;
        for slot in &mut self.slots[..self.len] {
            if slot.route.served_via(port, peer) {
                available(&mut slot.route);
                marked += 1;
            }
        }
        marked
    }

    /// Mark a learned route unreachable, keeping it in the table. Direct
    /// routes are untouched.
    pub fn mark_unreachable(&mut self, network: u16) {
        if let Some(slot) = self.slot_mut(network) {
            let route = &mut slot.route;
            if !route.direct {
                route.reachability = Reachability::Unreachable;
                route.busy_until = None;
            }
        }
    }

    /// Whether `network` is served by the router at `peer` on `port`: a
    /// learned route through that port to that next router. This is the
    /// scope of a flow-control message from `peer`, whose own list, when it
    /// has one, is intersected with it.
    #[must_use]
    pub fn is_served_via_peer(&self, network: u16, port: PortId, peer: &[u8]) -> bool {
        self.lookup(network)
            .is_some_and(|route| route.served_via(port, peer))
    }

    /// The networks [`is_served_via_peer`](Self::is_served_via_peer) holds
    /// for, in ascending order: the scope of a flow-control message from
    /// `peer` that lists none.
    pub fn routes_served_via_peer<'t>(
        &'t self,
        port: PortId,
        peer: &'t [u8],
    ) -> impl Iterator<Item = u16> + Clone + 't {
        self.routes()
            .filter(move |(_, route)| route.served_via(port, peer))
            .map(|(network, _)| network)
    }

    /// Clear every busy mark whose deadline has come, returning how many.
    pub fn clear_expired_busy(&mut self, now: Instant) -> usize {
        let mut cleared = 0;
        for slot in &mut self.slots[..self.len] {
            let route = &mut slot.route;
            if route.busy_until.is_some_and(|deadline| now >= deadline) {
                route.reachability = Reachability::Reachable;
                route.busy_until = None;
                cleared += 1;
            }
        }
        cleared
    }

    /// The reachability of `network` at `now`: a busy mark reads
    /// `Reachable` from its deadline on, whether or not the sweep has run.
    #[must_use]
    pub fn effective_reachability(&self, network: u16, now: Instant) -> Option<Reachability> {
        let route = self.lookup(network)?;
        Some(match route.busy_until {
            Some(deadline) if route.reachability == Reachability::Busy && now >= deadline => {
                Reachability::Reachable
            }
            _ => route.reachability,
        })
    }

    /// Note that a learned route carried traffic `now`. Only `last_used`
    /// moves, so traffic never keeps a route alive.
    pub fn touch(&mut self, network: u16, now: Instant) {
        if let Some(slot) = self.slot_mut(network) {
            if !slot.route.direct {
                slot.route.last_used = Some(now);
            }
        }
    }

    /// Remove every learned route last confirmed more than `max_age` before
    /// `now`, calling `purged` with each network in ascending order, and
    /// return how many. Direct routes never age.
    ///
    /// Every call comes before anything is removed, so a `purged` that
    /// panics leaves the table as it was.
    pub fn purge_stale(
        &mut self,
        now: Instant,
        max_age: Duration,
        mut purged: impl FnMut(u16),
    ) -> usize {
        let stale = |slot: &Slot| {
            !slot.route.direct
                && slot
                    .route
                    .last_seen
                    .is_some_and(|seen| now.saturating_duration_since(seen) > max_age)
        };
        for slot in self.stored().iter().filter(|slot| stale(slot)) {
            purged(slot.network);
        }
        // Each kept route is swapped forward, never copied over another,
        // with no memmove (`swap_apart`).
        let mut kept = 0;
        for index in 0..self.len {
            if stale(&self.slots[index]) {
                self.holds.forget(self.slots[index].network);
            } else {
                swap_apart(&mut self.slots, kept, index);
                kept += 1;
            }
        }
        let removed = self.len - kept;
        self.slots[kept..self.len].fill(VACANT);
        self.len = kept;
        removed
    }
}

/// Mark `route` busy until `deadline` unless it is direct or unreachable;
/// `true` when it was marked.
fn busy(route: &mut Route, deadline: Instant) -> bool {
    let marks = !route.direct && route.reachability != Reachability::Unreachable;
    if marks {
        route.reachability = Reachability::Busy;
        route.busy_until = Some(deadline);
    }
    marks
}

/// Make `route` reachable unless it is direct.
fn available(route: &mut Route) {
    if !route.direct {
        route.reachability = Reachability::Reachable;
        route.busy_until = None;
    }
}

#[cfg(test)]
mod tests;
