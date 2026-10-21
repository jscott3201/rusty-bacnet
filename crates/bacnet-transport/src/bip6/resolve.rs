//! The B/IPv6 neighbour table: the [`VmacTable`] and the resolutions in
//! flight for it, as plain state driven with explicit instants (U.5).
//!
//! A send to a VMAC the table lacks starts a resolution: an
//! Address-Resolution goes out at once and again each
//! [`RESOLUTION_INTERVAL`], [`RESOLUTION_ATTEMPTS`] times in all, and the
//! resolution fails one interval after the last. Any learn of the VMAC
//! completes it, an Address-Resolution-ACK or any other frame from the peer.
//! While it runs, an application's send waits for it and a router port's send
//! is queued ([`UnresolvedSend`]). A failed VMAC fails sends at once for a
//! while ([`UNREACHABLE_FOR`](super::vmac_table::UNREACHABLE_FOR)).
//!
//! [`Bip6Transport::resolve_virtual_address`](super::Bip6Transport::resolve_virtual_address)
//! runs the reverse lookup the same way, with Virtual-Address-Resolutions
//! sent to the peer itself. So does the check on a peer that has been quiet
//! for a while: a Virtual-Address-Resolution to the address on file, which
//! reaches the peer however it was learned, multicast or not.
//!
//! Everything here is bounded, so a flood of sends to VMACs nobody answers,
//! which any node on any network a router serves can cause, costs a fixed
//! amount of memory and Address-Resolutions.

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddrV6;
use std::time::Duration;

use bacnet_time::earliest;
use bytes::Bytes;
use tokio::sync::oneshot;
use tokio::time::Instant;

use super::resolve_counters::ResolutionCounts;
use super::vmac_table::{Heard, VmacTable};
use super::{Bip6Vmac, UnresolvedSend};

/// Address-Resolutions sent for one resolution.
pub(super) const RESOLUTION_ATTEMPTS: u32 = 3;
/// Time between two attempts, and from the last attempt to failure.
pub(super) const RESOLUTION_INTERVAL: Duration = Duration::from_secs(1);
/// How long a resolution runs before it fails.
pub(super) const RESOLUTION_TIMEOUT: Duration =
    Duration::from_secs(RESOLUTION_INTERVAL.as_secs() * RESOLUTION_ATTEMPTS as u64);
/// Most VMACs resolving at once.
pub(super) const MAX_RESOLVING: usize = 64;
/// Most sends waiting on one VMAC.
pub(super) const MAX_WAITERS_PER_VMAC: usize = 64;
/// Most sends waiting in all.
pub(super) const MAX_WAITERS: usize = 512;
/// Most NPDUs queued for one VMAC.
pub(super) const MAX_QUEUED_PER_VMAC: usize = 8;
/// Most NPDUs queued in all: about 190 KB of the largest NPDUs.
pub(super) const MAX_QUEUED: usize = 128;
/// Most Virtual-Address-Resolutions running at once.
pub(super) const MAX_VIRTUAL: usize = 16;

/// Why a resolution failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Failure {
    /// Nothing answered in time.
    TimedOut,
    /// The configured BBMD refused to pass an Address-Resolution on, or the
    /// peer refused a Virtual-Address-Resolution.
    Refused,
}

/// A send waiting on a resolution.
pub(super) type Waiter = oneshot::Sender<Result<SocketAddrV6, Failure>>;
/// A [`Bip6Transport::resolve_virtual_address`](super::Bip6Transport::resolve_virtual_address)
/// call waiting on a Virtual-Address-Resolution.
pub(super) type VirtualWaiter = oneshot::Sender<Result<Bip6Vmac, Failure>>;

/// Which bound a refused send hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Limit {
    /// [`MAX_RESOLVING`] or [`MAX_VIRTUAL`].
    Resolving,
    /// [`MAX_WAITERS_PER_VMAC`] or [`MAX_QUEUED_PER_VMAC`].
    PerVmac,
    /// [`MAX_WAITERS`] or [`MAX_QUEUED`].
    Total,
}

/// What a send to a VMAC does now.
#[derive(Debug)]
pub(super) enum Route {
    /// Send to this address. `wake` asks the driver to send the first
    /// query of a check that the peer is still there.
    Send { addr: SocketAddrV6, wake: bool },
    /// The VMAC failed to resolve a moment ago.
    Unreachable,
    /// Refused: a bound is reached.
    Full(Limit),
    /// Wait for the resolution. `wake` when it is a new one.
    Wait {
        rx: oneshot::Receiver<Result<SocketAddrV6, Failure>>,
        wake: bool,
    },
    /// The NPDU is queued until the resolution ends.
    Queued { wake: bool },
}

/// What a Virtual-Address-Resolution call does now.
#[derive(Debug)]
pub(super) enum VirtualRoute {
    /// The table already maps the address to one VMAC.
    Known(Bip6Vmac),
    /// Refused: a bound is reached.
    Full(Limit),
    /// Wait for the answer. `wake` when it is a new lookup.
    Wait {
        rx: oneshot::Receiver<Result<Bip6Vmac, Failure>>,
        wake: bool,
    },
}

/// One VMAC being resolved, or a quiet peer being checked.
#[derive(Debug)]
struct Pending {
    started: Instant,
    /// Queries sent so far.
    sent: u32,
    /// Whether the configured BBMD refused an Address-Resolution.
    refused: bool,
    /// For a check on a known peer that has been quiet: the address on
    /// file, which each query goes to as a Virtual-Address-Resolution.
    revalidating: Option<SocketAddrV6>,
    /// Whether a send waits on it or queued an NPDU for it, which makes it
    /// a resolution, counted as one, whose queries are Address-Resolutions.
    /// A send joins a check this way only when the peer's entry has gone
    /// meanwhile.
    resolving: bool,
    waiters: Vec<Waiter>,
    queued: VecDeque<Bytes>,
}

/// One address being asked for its VMAC.
#[derive(Debug)]
struct Virtual {
    peer: SocketAddrV6,
    started: Instant,
    sent: u32,
    waiters: Vec<VirtualWaiter>,
}

/// Whether a lookup that started at `started` and sent `sent` attempts is
/// due another, has failed, or is waiting until the instant returned.
fn schedule(started: Instant, sent: u32, now: Instant) -> Step {
    if now >= started + RESOLUTION_TIMEOUT {
        Step::Failed
    } else if sent < RESOLUTION_ATTEMPTS && now >= started + RESOLUTION_INTERVAL * sent {
        Step::Send
    } else if sent < RESOLUTION_ATTEMPTS {
        Step::Until(started + RESOLUTION_INTERVAL * sent)
    } else {
        Step::Until(started + RESOLUTION_TIMEOUT)
    }
}

enum Step {
    Send,
    Failed,
    Until(Instant),
}

/// What the driver has to do after [`Neighbours::poll`].
#[derive(Debug, Default)]
pub(super) struct Due {
    /// VMACs to send an Address-Resolution for.
    pub(super) resolutions: Vec<Bip6Vmac>,
    /// Addresses to send a Virtual-Address-Resolution to.
    pub(super) virtual_resolutions: Vec<SocketAddrV6>,
    /// Quiet peers' addresses on file, each to be checked with a
    /// Virtual-Address-Resolution.
    pub(super) revalidations: Vec<SocketAddrV6>,
    /// Queued NPDUs whose VMAC resolved: the address, the VMAC, the NPDU.
    pub(super) flush: Vec<(SocketAddrV6, Bip6Vmac, Bytes)>,
    /// When to poll again, unless woken first.
    pub(super) next: Option<Instant>,
}

/// The neighbour table and the resolutions in flight.
#[derive(Debug, Default)]
pub(super) struct Neighbours {
    pub(super) table: VmacTable,
    pending: HashMap<Bip6Vmac, Pending>,
    virtual_pending: Vec<Virtual>,
    /// Waiters across `pending`, closed ones included until pruned.
    waiters: usize,
    /// NPDUs queued across `pending` and `flush`.
    queued: usize,
    /// Queued NPDUs whose VMAC resolved, for the driver to send.
    flush: Vec<(SocketAddrV6, Bip6Vmac, Bytes)>,
}

impl Neighbours {
    /// Decide what a send of `npdu` to `vmac` at `now` does, holding it as
    /// `hold` says when the VMAC needs resolving.
    pub(super) fn route(
        &mut self,
        vmac: Bip6Vmac,
        npdu: &[u8],
        hold: UnresolvedSend,
        now: Instant,
        counts: &ResolutionCounts,
    ) -> Route {
        if let Some(addr) = self.table.lookup(vmac) {
            // A check the bound leaves no room for isn't recorded, so the
            // next send tries again.
            let wake = !self.pending.contains_key(&vmac)
                && self.pending.len() < MAX_RESOLVING
                && self.table.revalidation_due(vmac, now);
            if wake {
                self.pending.insert(vmac, Pending::new(now, Some(addr)));
            }
            return Route::Send { addr, wake };
        }
        if self.table.is_unreachable(vmac, now) {
            counts.unreachable();
            return Route::Unreachable;
        }
        let new = !self.pending.contains_key(&vmac);
        // A check on a peer whose entry has gone becomes this send's
        // resolution.
        let joins_check = self
            .pending
            .get(&vmac)
            .is_some_and(|pending| !pending.resolving);
        if new && self.pending.len() >= MAX_RESOLVING {
            counts.over_limit();
            return Route::Full(Limit::Resolving);
        }
        let held = self.pending.get_mut(&vmac).map_or(0, |pending| {
            let before = pending.waiters.len();
            pending.waiters.retain(|waiter| !waiter.is_closed());
            self.waiters -= before - pending.waiters.len();
            match hold {
                UnresolvedSend::Wait => pending.waiters.len(),
                UnresolvedSend::Queue => pending.queued.len(),
            }
        });
        let limit = match hold {
            UnresolvedSend::Wait if held >= MAX_WAITERS_PER_VMAC => Some(Limit::PerVmac),
            UnresolvedSend::Wait if self.waiters >= MAX_WAITERS => Some(Limit::Total),
            UnresolvedSend::Queue if held >= MAX_QUEUED_PER_VMAC => Some(Limit::PerVmac),
            UnresolvedSend::Queue if self.queued >= MAX_QUEUED => Some(Limit::Total),
            _ => None,
        };
        if let Some(limit) = limit {
            counts.over_limit();
            return Route::Full(limit);
        }
        if new || joins_check {
            counts.started();
        }
        let pending = self
            .pending
            .entry(vmac)
            .or_insert_with(|| Pending::new(now, None));
        pending.resolving = true;
        match hold {
            UnresolvedSend::Wait => {
                let (tx, rx) = oneshot::channel();
                pending.waiters.push(tx);
                self.waiters += 1;
                Route::Wait { rx, wake: new }
            }
            UnresolvedSend::Queue => {
                pending.queued.push_back(Bytes::copy_from_slice(npdu));
                self.queued += 1;
                Route::Queued { wake: new }
            }
        }
    }

    /// Learn that `vmac` is at `addr`, heard at `now`, completing any
    /// resolution of it. Returns whether queued NPDUs are now ready for
    /// the driver.
    pub(super) fn learn(
        &mut self,
        vmac: Bip6Vmac,
        addr: SocketAddrV6,
        heard: Heard,
        now: Instant,
        counts: &ResolutionCounts,
    ) -> bool {
        if !self.table.learn(vmac, addr, heard, now) {
            return false;
        }
        let Some(pending) = self.pending.remove(&vmac) else {
            return false;
        };
        if pending.resolving {
            counts.resolved();
        }
        self.waiters -= pending.waiters.len();
        for waiter in pending.waiters {
            let _ = waiter.send(Ok(addr));
        }
        // The NPDUs stay counted in `queued` until the driver takes them,
        // so the queue bound holds while they wait to go out.
        let ready = !pending.queued.is_empty();
        self.flush
            .extend(pending.queued.into_iter().map(|npdu| (addr, vmac, npdu)));
        ready
    }

    /// The configured BBMD refused an Address-Resolution: every resolution
    /// now running fails as refused unless it is answered after all.
    pub(super) fn refuse_resolutions(&mut self) {
        for pending in self.pending.values_mut() {
            pending.refused = true;
        }
    }

    /// Decide what a lookup of `peer`'s VMAC at `now` does.
    pub(super) fn resolve_virtual(&mut self, peer: SocketAddrV6, now: Instant) -> VirtualRoute {
        if let Some(vmac) = self.table.by_address(peer) {
            return VirtualRoute::Known(vmac);
        }
        let (tx, rx) = oneshot::channel();
        if let Some(lookup) = self
            .virtual_pending
            .iter_mut()
            .find(|v| same_peer(v.peer, peer))
        {
            lookup.waiters.retain(|waiter| !waiter.is_closed());
            if lookup.waiters.len() >= MAX_WAITERS_PER_VMAC {
                return VirtualRoute::Full(Limit::PerVmac);
            }
            lookup.waiters.push(tx);
            return VirtualRoute::Wait { rx, wake: false };
        }
        if self.virtual_pending.len() >= MAX_VIRTUAL {
            return VirtualRoute::Full(Limit::Resolving);
        }
        self.virtual_pending.push(Virtual {
            peer,
            started: now,
            sent: 0,
            waiters: vec![tx],
        });
        VirtualRoute::Wait { rx, wake: true }
    }

    /// `peer` answered a Virtual-Address-Resolution with `vmac`.
    pub(super) fn virtual_answered(
        &mut self,
        peer: SocketAddrV6,
        vmac: Bip6Vmac,
        counts: &ResolutionCounts,
    ) {
        if let Some(lookup) = self.take_virtual(peer) {
            counts.virtual_resolved();
            for waiter in lookup.waiters {
                let _ = waiter.send(Ok(vmac));
            }
        }
    }

    /// `peer` refused a Virtual-Address-Resolution.
    pub(super) fn virtual_refused(&mut self, peer: SocketAddrV6, counts: &ResolutionCounts) {
        if let Some(lookup) = self.take_virtual(peer) {
            counts.virtual_refused();
            for waiter in lookup.waiters {
                let _ = waiter.send(Err(Failure::Refused));
            }
        }
    }

    fn take_virtual(&mut self, peer: SocketAddrV6) -> Option<Virtual> {
        let index = self
            .virtual_pending
            .iter()
            .position(|lookup| same_peer(lookup.peer, peer))?;
        Some(self.virtual_pending.swap_remove(index))
    }

    /// Advance every lookup to `now`: say which requests are due, fail the
    /// lookups whose time is up, and hand over the queued NPDUs ready to go.
    pub(super) fn poll(&mut self, now: Instant, counts: &ResolutionCounts) -> Due {
        let mut due = Due {
            flush: std::mem::take(&mut self.flush),
            ..Due::default()
        };
        self.queued -= due.flush.len();
        let mut failed = Vec::new();
        for (vmac, pending) in &mut self.pending {
            match schedule(pending.started, pending.sent, now) {
                Step::Failed => failed.push(*vmac),
                Step::Send => {
                    pending.sent += 1;
                    match pending.revalidating {
                        Some(addr) if !pending.resolving => due.revalidations.push(addr),
                        _ => due.resolutions.push(*vmac),
                    }
                    due.next = earliest([due.next, schedule_next(pending.started, pending.sent)]);
                }
                Step::Until(at) => due.next = earliest([due.next, Some(at)]),
            }
        }
        for vmac in failed {
            self.fail(vmac, now, counts);
        }
        let mut index = 0;
        while index < self.virtual_pending.len() {
            let lookup = &mut self.virtual_pending[index];
            match schedule(lookup.started, lookup.sent, now) {
                Step::Failed => {
                    counts.virtual_timed_out();
                    for waiter in self.virtual_pending.swap_remove(index).waiters {
                        let _ = waiter.send(Err(Failure::TimedOut));
                    }
                    continue;
                }
                Step::Send => {
                    lookup.sent += 1;
                    due.virtual_resolutions.push(lookup.peer);
                    due.next = earliest([due.next, schedule_next(lookup.started, lookup.sent)]);
                }
                Step::Until(at) => due.next = earliest([due.next, Some(at)]),
            }
            index += 1;
        }
        due
    }

    /// Fail the resolution of `vmac`: its waiters get the reason, its queue
    /// is dropped, and the VMAC is held unreachable for a while. A check on
    /// a quiet peer that nothing answered removes the peer. A check counts
    /// as a failed revalidation, and a resolution, a check a send joined
    /// included, as timed out or refused.
    fn fail(&mut self, vmac: Bip6Vmac, now: Instant, counts: &ResolutionCounts) {
        let Some(pending) = self.pending.remove(&vmac) else {
            return;
        };
        self.waiters -= pending.waiters.len();
        self.queued -= pending.queued.len();
        counts.queued_dropped(pending.queued.len());
        let reason = if pending.refused {
            Failure::Refused
        } else {
            Failure::TimedOut
        };
        if pending.revalidating.is_some() {
            counts.revalidation_failed();
            self.table.forget(vmac);
        }
        if pending.resolving && reason == Failure::Refused {
            counts.refused();
        } else if pending.resolving {
            counts.timed_out();
        }
        self.table.mark_unreachable(vmac, now);
        for waiter in pending.waiters {
            let _ = waiter.send(Err(reason));
        }
    }

    /// Forget everything and fail every waiter, as the transport stops.
    /// Dropping a waiter's sender is how it learns. The queued NPDUs, those
    /// waiting for the driver included, are dropped and counted.
    pub(super) fn clear(&mut self, counts: &ResolutionCounts) {
        counts.queued_dropped(self.queued);
        self.table.clear();
        self.pending.clear();
        self.virtual_pending.clear();
        self.flush.clear();
        self.waiters = 0;
        self.queued = 0;
    }

    /// How many VMACs are resolving.
    #[cfg(test)]
    pub(super) fn resolving(&self) -> usize {
        self.pending.len()
    }
}

/// When a lookup that started at `started` and sent `sent` attempts needs
/// its next poll.
fn schedule_next(started: Instant, sent: u32) -> Option<Instant> {
    Some(if sent < RESOLUTION_ATTEMPTS {
        started + RESOLUTION_INTERVAL * sent
    } else {
        started + RESOLUTION_TIMEOUT
    })
}

/// Whether two addresses name the same peer: the same IP address and port.
/// The scope ID is left out, since the socket gives a link-local peer the
/// link's scope whether or not the caller named it.
fn same_peer(a: SocketAddrV6, b: SocketAddrV6) -> bool {
    a.ip() == b.ip() && a.port() == b.port()
}

impl Pending {
    /// A lookup started at `now`: a check on the quiet peer at
    /// `revalidating`, or, with `None`, a resolution once a send joins it.
    fn new(now: Instant, revalidating: Option<SocketAddrV6>) -> Self {
        Self {
            started: now,
            sent: 0,
            refused: false,
            revalidating,
            resolving: false,
            waiters: Vec::new(),
            queued: VecDeque::new(),
        }
    }
}
