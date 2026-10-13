//! Received rejects and their hold-down records.

use core::fmt;

use super::{Reachability, Table, Timing};
use crate::message::RejectReason;
use crate::{swap_apart, Instant, PortId};

/// What [`Table::apply_reject`] did.
///
/// It is `#[non_exhaustive]`, so a new effect isn't a breaking change: a
/// `match` needs a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RejectEffect {
    /// It changed a learned route's state.
    Applied,
    /// Nothing to change: no route, a direct one, or the state already in
    /// effect.
    SameState,
    /// It would change the route, but came within the hold-down of the last
    /// change from the same arrival port.
    HeldDown,
    /// For a learned route, with a reason other than 1 or 2, which changes
    /// no route (Clause 6.6.3.5).
    Kept,
}

/// When a reject from one arrival port last changed one network's route.
/// The crate's own: outside it, the hold-down shows only in what
/// [`Table::apply_reject`] returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Hold {
    /// The network.
    pub(crate) network: u16,
    /// The port the reject arrived on.
    pub(crate) port: PortId,
    /// When it was applied.
    pub(crate) since: Instant,
}

/// Unused pool storage: all zero bits.
const NO_HOLD: Hold = Hold {
    network: 0,
    port: PortId(0),
    since: Instant::ZERO,
};

/// Up to `H` hold-down records, unordered, at most one per network and port.
#[derive(Clone)]
pub(super) struct Holds<const H: usize> {
    records: [Hold; H],
    len: usize,
}

/// The records kept, not the unused storage.
impl<const H: usize> fmt::Debug for Holds<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.stored()).finish()
    }
}

impl<const H: usize> Holds<H> {
    pub(super) const fn new() -> Self {
        Self {
            records: [NO_HOLD; H],
            len: 0,
        }
    }

    fn stored(&self) -> &[Hold] {
        &self.records[..self.len]
    }

    fn since(&self, network: u16, port: PortId) -> Option<Instant> {
        self.stored()
            .iter()
            .find(|hold| hold.network == network && hold.port == port)
            .map(|hold| hold.since)
    }

    /// Record a change at `now`, in the existing record or a new one. A full
    /// pool gives up its oldest record; `true` when one was evicted.
    fn record(&mut self, network: u16, port: PortId, now: Instant) -> bool {
        let hold = Hold {
            network,
            port,
            since: now,
        };
        let stored = &mut self.records[..self.len];
        if let Some(existing) = stored
            .iter_mut()
            .find(|hold| hold.network == network && hold.port == port)
        {
            *existing = hold;
            return false;
        }
        if self.len < H {
            self.records[self.len] = hold;
            self.len += 1;
            return false;
        }
        if let Some(oldest) = self.records.iter_mut().min_by_key(|hold| hold.since) {
            *oldest = hold;
        }
        true
    }

    /// Drop every record.
    pub(super) fn clear(&mut self) {
        self.records[..self.len].fill(NO_HOLD);
        self.len = 0;
    }

    /// Drop every record for `network`: fresh learning or removal re-arms it.
    pub(super) fn forget(&mut self, network: u16) {
        // Each kept record is swapped forward, never copied over another:
        // that copy linked a memmove call on ARMv6-M (`swap_apart`).
        let mut kept = 0;
        for index in 0..self.len {
            if self.records[index].network != network {
                swap_apart(&mut self.records, kept, index);
                kept += 1;
            }
        }
        self.records[kept..self.len].fill(NO_HOLD);
        self.len = kept;
    }
}

impl<const R: usize, const H: usize> Table<R, H> {
    /// Apply what a received reject says about the route to `network`,
    /// arriving on `port` with `reason` (Clause 6.6.3.5). Relaying the
    /// reject is the router's business, not the table's.
    ///
    /// Reason 1 marks a learned route unreachable. Reason 2 marks it busy
    /// for [`Timing::busy_for`], unless it is unreachable or still busy: a
    /// repeat never extends the deadline. Any other reason leaves a learned
    /// route as it is ([`RejectEffect::Kept`], counted in
    /// [`ClaimCounters::reject_kept`](super::ClaimCounters::reject_kept)),
    /// since the clause changes the table for those two only. A reject that
    /// would change nothing, or one within [`Timing::hold_down`] of the last
    /// change applied from the same port, is ignored and counted, and moves
    /// neither the window nor the deadline. Direct routes never change.
    pub fn apply_reject(
        &mut self,
        network: u16,
        port: PortId,
        reason: RejectReason,
        now: Instant,
        timing: &Timing,
    ) -> RejectEffect {
        let learned = self.lookup(network).is_some_and(|route| !route.direct);
        let other = !matches!(
            reason,
            RejectReason::NOT_DIRECTLY_CONNECTED | RejectReason::ROUTER_BUSY
        );
        if learned && other {
            let kept = &mut self.counters.reject_kept;
            *kept = kept.saturating_add(1);
            return RejectEffect::Kept;
        }
        let changes = self.lookup(network).is_some_and(|route| {
            // The effective state counts: a busy deadline may have passed
            // before the sweep cleared it.
            let lapsed = route.busy_until.is_some_and(|deadline| now >= deadline);
            !route.direct
                && match reason {
                    RejectReason::NOT_DIRECTLY_CONNECTED => {
                        route.reachability != Reachability::Unreachable
                    }
                    RejectReason::ROUTER_BUSY => {
                        route.reachability != Reachability::Unreachable
                            && (route.reachability != Reachability::Busy || lapsed)
                    }
                    _ => false,
                }
        });
        let held_down = self
            .holds
            .since(network, port)
            .is_some_and(|since| now.saturating_duration_since(since) < timing.hold_down);
        let counters = &mut self.counters;
        if !changes || held_down {
            counters.reject_dampened = counters.reject_dampened.saturating_add(1);
            return if changes {
                counters.reject_dampened_hold_down =
                    counters.reject_dampened_hold_down.saturating_add(1);
                RejectEffect::HeldDown
            } else {
                counters.reject_dampened_same_state =
                    counters.reject_dampened_same_state.saturating_add(1);
                RejectEffect::SameState
            };
        }
        counters.reject_applied = counters.reject_applied.saturating_add(1);
        if reason == RejectReason::NOT_DIRECTLY_CONNECTED {
            self.mark_unreachable(network);
        } else {
            // Only reasons 1 and 2 change a route.
            self.mark_busy(network, now + timing.busy_for);
        }
        if self.holds.record(network, port, now) {
            self.counters.hold_evicted = self.counters.hold_evicted.saturating_add(1);
        }
        RejectEffect::Applied
    }

    /// When a reject from `port` last changed the route to `network`, while
    /// that record is kept.
    #[cfg(test)]
    pub(crate) fn held_since(&self, network: u16, port: PortId) -> Option<Instant> {
        self.holds.since(network, port)
    }

    /// Every hold-down record, in no particular order. Records for a network
    /// go when it is learned afresh, made direct, removed or aged out.
    #[cfg(test)]
    pub(crate) fn holds(&self) -> impl Iterator<Item = Hold> + '_ {
        self.holds.stored().iter().copied()
    }
}
