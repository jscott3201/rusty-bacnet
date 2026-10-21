//! The router's own congestion control for a clogged egress port (#1697).
//!
//! Clause 6.6.4 leaves the decision of when to curtail traffic to the
//! implementation, suggesting buffer occupancy as the input. Here it is
//! each port's egress backlog, which the caller reports with
//! [`Router::egress_backlog`](super::Router::egress_backlog), and the rule
//! is [`Congestion`]: a port whose backlog has stayed at capacity for
//! `clog_after`, without falling under the low-water mark in between, is
//! clogged. The router then broadcasts Router-Busy-To-Network on every
//! other port, naming the networks it reaches through the clogged one
//! (Clause 6.6.3.6), answers each send that port's full queue refuses with
//! reject reason 2, and, once the backlog falls under the low-water mark,
//! broadcasts Router-Available-To-Network for the same networks (Clause
//! 6.6.3.7). A port still clogged `repeat` after its last Router-Busy
//! announces it again, so peers' 30-second busy marks don't lapse while it
//! lasts.
//!
//! The announcements wait in a per-port flag, not the task queue, so any
//! number of ports can change state within one drain. They are written
//! when [`poll`](super::Storage::poll) reaches them, after every queued
//! task, from the table as it is then.

use super::counters::bump;
use super::output::{Cause, Current, Kind, Target};
use super::Storage;
use crate::message::Networks;
use crate::npdu::{NetworkMessageType, NetworkPriority, Npdu};
use crate::table::Reachability;
use crate::{Duration, Instant, PortId};

/// When a port's egress counts as clogged, and when as drained: the
/// router's own Router-Busy policy, off unless
/// [`Options::congestion`](super::Options::congestion) sets one.
///
/// It is `#[non_exhaustive]`: start from [`Congestion::DEFAULT`] and set
/// the fields to change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Congestion {
    /// How long a port's backlog must stay at capacity, never falling
    /// under the low-water mark, before the port counts as clogged.
    pub clog_after: Duration,
    /// The low-water mark, in percent of the queue's capacity: a backlog
    /// under it ends a clog, and the wait for one.
    pub drain_below_percent: u8,
    /// How long after its last Router-Busy a still-clogged port announces
    /// again, when its backlog is reported at capacity.
    pub repeat: Duration,
}

impl Congestion {
    /// A port clogs after 2 s at capacity and drains under half of it;
    /// Router-Busy repeats every 20 s while the clog lasts, inside the
    /// 30 s a peer holds its busy mark (Clause 6.6.3.6).
    pub const DEFAULT: Self = Self {
        clog_after: Duration::from_secs(2),
        drain_below_percent: 50,
        repeat: Duration::from_secs(20),
    };
}

impl Congestion {
    /// Whether [`Router::new`](super::Router::new) takes it: a nonzero
    /// `clog_after`, since with none a queue full for a moment would
    /// announce busy, which is not one that stays full; a nonzero
    /// `repeat`; and a low-water mark of 1 to 99 percent, so a port can
    /// drain (0 never) and need not be empty to have drained below a
    /// full queue (100 or more would clear it at once).
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        !self.clog_after.is_zero()
            && !self.repeat.is_zero()
            && self.drain_below_percent >= 1
            && self.drain_below_percent <= 99
    }
}

impl Default for Congestion {
    /// [`Congestion::DEFAULT`].
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// How many sends wait in one port's egress queue, out of how many it
/// holds: what [`Router::egress_backlog`](super::Router::egress_backlog)
/// is told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backlog {
    /// Sends waiting.
    pub queued: u32,
    /// The most the queue holds.
    pub capacity: u32,
}

impl Backlog {
    /// `queued` sends waiting in a queue of `capacity`.
    #[must_use]
    pub const fn new(queued: u32, capacity: u32) -> Self {
        Self { queued, capacity }
    }

    fn full(self) -> bool {
        self.queued >= self.capacity
    }

    /// Under `percent` of the capacity, in 32-bit multiplication only.
    fn under(self, percent: u8) -> bool {
        self.queued.saturating_mul(100) < self.capacity.saturating_mul(u32::from(percent))
    }
}

/// Where a port stands. `Clear` is zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Not at capacity since it last drained.
    Clear,
    /// At capacity since `since`, not yet for `clog_after`.
    Filling,
    /// Clogged; the last Router-Busy went at `since`.
    Clogged,
}

/// Which announcement waits for a port. `None` is zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    None,
    Busy,
    Available,
}

/// One port's congestion state: all zero bits when clear.
#[derive(Debug, Clone, Copy)]
pub(super) struct Flow {
    state: State,
    since: Instant,
    pending: Pending,
    /// The first network the announcement's next message may list.
    cursor: u32,
}

impl Flow {
    pub(super) const CLEAR: Self = Self {
        state: State::Clear,
        since: Instant::ZERO,
        pending: Pending::None,
        cursor: 0,
    };

    /// Whether the port is clogged or filling toward it.
    pub(super) fn congested(&self) -> bool {
        self.state != State::Clear
    }

    /// Whether the port is clogged.
    pub(super) fn clogged(&self) -> bool {
        self.state == State::Clogged
    }

    /// Whether an announcement waits; dropping it counts as undrained.
    pub(super) fn take_pending(&mut self) -> bool {
        let pending = self.pending != Pending::None;
        self.pending = Pending::None;
        pending
    }

    /// Start `pending`, replacing any announcement still waiting.
    fn announce(&mut self, pending: Pending) -> Option<Pending> {
        self.pending = pending;
        self.cursor = 0;
        Some(pending)
    }

    /// Take `backlog` at `now` under `policy`: the announcement it starts,
    /// if any.
    fn report(&mut self, backlog: Backlog, now: Instant, policy: &Congestion) -> Option<Pending> {
        if backlog.under(policy.drain_below_percent) {
            let was = self.state;
            self.state = State::Clear;
            return (was == State::Clogged)
                .then(|| self.announce(Pending::Available))
                .flatten();
        }
        if !backlog.full() {
            return None;
        }
        match self.state {
            State::Clear => {
                self.state = State::Filling;
                self.since = now;
                None
            }
            State::Filling if now >= self.since + policy.clog_after => {
                self.state = State::Clogged;
                self.since = now;
                self.announce(Pending::Busy)
            }
            State::Clogged if now >= self.since + policy.repeat => {
                self.since = now;
                self.announce(Pending::Busy)
            }
            _ => None,
        }
    }
}

impl<
        const P: usize,
        const R: usize,
        const H: usize,
        const D: usize,
        const N: usize,
        G: super::GroupClassifier,
        const B: usize,
    > Storage<P, R, H, D, N, G, B>
{
    /// [`Router::egress_backlog`](super::Router::egress_backlog).
    pub(super) fn egress_backlog(
        &mut self,
        port: PortId,
        backlog: Backlog,
        now: Instant,
        policy: &Congestion,
    ) {
        let Some(flow) = self.flows[..self.port_count].get_mut(usize::from(port.0)) else {
            return;
        };
        match flow.report(backlog, now, policy) {
            Some(Pending::Busy) => bump(&mut self.counters.busy_announced),
            Some(Pending::Available) => bump(&mut self.counters.available_announced),
            _ => {}
        }
    }

    /// Write the next waiting announcement into the scratch as the
    /// current output. `false` when none waits.
    pub(super) fn write_announcement(&mut self) -> bool {
        let count = self.port_count;
        let Some(index) = self.flows[..count]
            .iter()
            .position(|flow| flow.pending != Pending::None)
        else {
            return false;
        };
        // At most 255 ports, so the index fits in a u8.
        let port = PortId(index as u8);
        let flow = self.flows[index];
        let kind = match flow.pending {
            Pending::Available => NetworkMessageType::ROUTER_AVAILABLE_TO_NETWORK,
            _ => NetworkMessageType::ROUTER_BUSY_TO_NETWORK,
        };
        // Sized for the smallest broadcast limit among the ports it goes
        // to, so every one carries it: two octets per network behind a
        // three-octet header. A one-port router has none to tell.
        let limit = self.ports[..count]
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != index)
            .map(|(_, config)| usize::from(config.max_npdu.broadcast))
            .min();
        let Some(limit) = limit else {
            self.flows[index].pending = Pending::None;
            return true;
        };
        let per_message = limit.saturating_sub(3) >> 1;
        let from = flow.cursor;
        let networks = self
            .table
            .routes()
            .filter(move |(network, route)| {
                u32::from(*network) >= from
                    && route.port() == port
                    && route.reachability() != Reachability::Unreachable
            })
            .map(|(network, _)| network)
            .take(per_message);
        let last = networks.clone().last();
        let Some(last) = last else {
            self.flows[index].pending = Pending::None;
            return true;
        };
        let header = Npdu::message(kind, &[]);
        let at = header.encoded_len();
        let written = self
            .scratch
            .split_at_mut_checked(at)
            .and_then(|(head, rest)| {
                let len = Networks::write(networks, rest).ok()?;
                header.encode_into(head).ok()?;
                Some(at + len)
            });
        match written {
            Some(len) => {
                self.out.current = Current::new(
                    len,
                    Kind::Congestion(kind),
                    Cause::Local,
                    NetworkPriority::Normal,
                    Target::AllBut(port),
                );
                self.flows[index].cursor = u32::from(last) + 1;
            }
            None => {
                bump(&mut self.counters.write_failures);
                self.flows[index].pending = Pending::None;
            }
        }
        true
    }
}
