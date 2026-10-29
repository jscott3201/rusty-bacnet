//! Learning: claims, convergence, flap detection and management writes.

use core::fmt;

use super::{Convergence, Pending, Route, Table, Timing, is_reserved};
use crate::{Duration, Instant, Mac, PortId};

/// A route that moves port again within this long of its last move counts
/// toward the flap warning.
const FLAP_WINDOW: Duration = Duration::from_secs(60);

/// Moves within the window at which a route is reported as flapping.
const FLAP_WARNING: u8 = 3;

/// Why a learning claim changed nothing.
///
/// It is `#[non_exhaustive]`, so a new refusal isn't a breaking change: a
/// `match` needs a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LearnRefusal {
    /// Network 0 or 0xFFFF, which name no single network.
    Reserved,
    /// The network is directly attached, and learning never changes that.
    Direct,
    /// The network is new and the table is full.
    Full,
}

impl fmt::Display for LearnRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Reserved => "network 0 and 0xFFFF name no single network",
            Self::Direct => "the network is directly attached",
            Self::Full => "the routing table is full",
        })
    }
}

impl core::error::Error for LearnRefusal {}

/// What a learning claim did.
///
/// It is `#[non_exhaustive]`, so a new outcome isn't a breaking change: a
/// `match` needs a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
#[must_use]
pub enum Learned {
    /// Nothing, for the reason given.
    Refused(LearnRefusal),
    /// Hardened convergence holds the move until a second claim.
    Held,
    /// A new route, or a refresh through the same port.
    Applied,
    /// The route moved to another port.
    Moved {
        /// The port it was on.
        from: PortId,
        /// Moves within 60 s of the one before, this one included.
        flaps: u8,
    },
}

impl Learned {
    /// Whether the route was installed or refreshed.
    #[must_use]
    pub const fn applied(self) -> bool {
        matches!(self, Self::Applied | Self::Moved { .. })
    }

    /// Whether this move reached the flap warning.
    #[must_use]
    pub const fn flapping(self) -> bool {
        matches!(self, Self::Moved { flaps, .. } if flaps >= FLAP_WARNING)
    }
}

impl<const R: usize, const H: usize> Table<R, H> {
    /// Apply a learning claim from an I-Am-Router or an Init-RT-Ack entry:
    /// `network` is reachable through the router at `next_hop` on `port`.
    ///
    /// Under [`Convergence::Standard`] it is [`learn`](Self::learn), and any
    /// pending claim for the network is dropped without counting. Under
    /// [`Convergence::Hardened`], a claim that would move a learned route to
    /// another port is held until a second claim for the same network and
    /// port arrives within [`Timing::corroboration`], inclusive; the second
    /// supplies the next hop. A claim for another port takes the pending
    /// slot over, and one past the window starts it again. The held route
    /// keeps forwarding as it was.
    pub fn apply_claim(
        &mut self,
        network: u16,
        port: PortId,
        next_hop: Mac,
        now: Instant,
        timing: &Timing,
    ) -> Learned {
        // Only a stored route can have a pending claim.
        let Ok(index) = self.find(network) else {
            return self.learn(network, port, next_hop, now);
        };
        let pending = self.slots[index].pending.take();
        if self.convergence == Convergence::Standard {
            return self.learn(network, port, next_hop, now);
        }
        let counters = &mut self.counters;
        let pending = match pending {
            Some(pending)
                if now.saturating_duration_since(pending.first_seen) > timing.corroboration =>
            {
                counters.pending_expired = counters.pending_expired.saturating_add(1);
                None
            }
            live => live,
        };
        let route = self.slots[index].route;
        if !route.direct && route.port != port {
            if pending.is_some_and(|pending| pending.port == port) {
                counters.corroborated_applied = counters.corroborated_applied.saturating_add(1);
            } else {
                self.slots[index].pending = Some(Pending {
                    port,
                    first_seen: now,
                });
                counters.pending_started = counters.pending_started.saturating_add(1);
                return Learned::Held;
            }
        }
        self.learn(network, port, next_hop, now)
    }

    /// Install or refresh a learned route at once, with flap detection.
    ///
    /// A move to another port sets `last_seen` and the move time to `now`,
    /// clears `last_used`, any mark, the pending claim and the hold-down, and
    /// counts the move toward the flap warning when it comes within 60 s of
    /// the last. A new network or a refresh through the same port is
    /// [`add_learned`](Self::add_learned).
    pub fn learn(&mut self, network: u16, port: PortId, next_hop: Mac, now: Instant) -> Learned {
        if is_reserved(network) {
            return Learned::Refused(LearnRefusal::Reserved);
        }
        if let Some(existing) = self.lookup(network) {
            if existing.direct {
                return Learned::Refused(LearnRefusal::Direct);
            }
            if existing.port != port {
                let flaps = match existing.last_port_change {
                    Some(changed) if now.saturating_duration_since(changed) < FLAP_WINDOW => {
                        existing.flap_count.saturating_add(1)
                    }
                    _ => 1,
                };
                if flaps >= FLAP_WARNING {
                    self.counters.flap_warned = self.counters.flap_warned.saturating_add(1);
                }
                let mut route = Route::learned(port, next_hop, now);
                route.flap_count = flaps;
                route.last_port_change = Some(now);
                self.holds.forget(network);
                self.put(network, route);
                return Learned::Moved {
                    from: existing.port,
                    flaps,
                };
            }
        }
        // Reserved and direct are ruled out above, so only room is left.
        if self.add_learned(network, port, next_hop, now) {
            Learned::Applied
        } else {
            Learned::Refused(LearnRefusal::Full)
        }
    }

    /// Drop every pending claim older than [`Timing::corroboration`] at
    /// `now`, calling `expired` with each network in ascending order, and
    /// return how many. Each counts as expired. A claim is dropped and
    /// counted before its call, so an `expired` that panics leaves the table
    /// consistent.
    pub fn expire_pending(
        &mut self,
        now: Instant,
        timing: &Timing,
        mut expired: impl FnMut(u16),
    ) -> usize {
        let window = timing.corroboration;
        let mut count = 0;
        for slot in &mut self.slots[..self.len] {
            if slot
                .pending
                .is_some_and(|pending| now.saturating_duration_since(pending.first_seen) > window)
            {
                slot.pending = None;
                self.counters.pending_expired = self.counters.pending_expired.saturating_add(1);
                expired(slot.network);
                count += 1;
            }
        }
        count
    }

    /// The claim pending for `network` under hardened convergence.
    #[must_use]
    pub fn pending(&self, network: u16) -> Option<Pending> {
        let index = self.find(network).ok()?;
        self.slots[index].pending
    }

    /// How many claims are pending: at most one per learned route.
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.stored()
            .iter()
            .filter(|slot| slot.pending.is_some())
            .count()
    }

    /// Apply one Initialize-Routing-Table entry with a nonzero Port ID:
    /// install `network` as a learned route through `next_hop` on `port`,
    /// replacing any learned route it had or adding a new one, at once and
    /// without corroboration (Clause 6.4.7). A direct route, network 0 and
    /// 0xFFFF are left alone. `false` when nothing was installed.
    pub fn apply_management_update(
        &mut self,
        network: u16,
        port: PortId,
        next_hop: Mac,
        now: Instant,
    ) -> bool {
        self.add_learned(network, port, next_hop, now)
    }

    /// Apply one Initialize-Routing-Table entry with Port ID 0: delete the
    /// network's learned route. Direct routes stay. `true` only when a route
    /// was removed.
    pub fn apply_management_removal(&mut self, network: u16) -> bool {
        match self.lookup(network) {
            Some(route) if !route.direct => self.remove(network).is_some(),
            _ => false,
        }
    }

    // The router engine counts these three itself. They are the crate's
    // own, so a caller with `Router::table_mut` can't skew the counts.

    /// Count a learning claim the router applied.
    pub(crate) fn record_learned(&mut self) {
        self.counters.learned_ok = self.counters.learned_ok.saturating_add(1);
    }

    /// Count a learning claim that met the router's route cap.
    pub(crate) fn record_learning_cap(&mut self) {
        self.counters.learned_cap_ignored = self.counters.learned_cap_ignored.saturating_add(1);
    }

    /// Count a Disconnect-Connection removal the router ignored.
    pub(crate) fn record_disconnect_removal_ignored(&mut self) {
        self.counters.disconnect_removal_ignored =
            self.counters.disconnect_removal_ignored.saturating_add(1);
    }
}
