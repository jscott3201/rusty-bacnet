//! A port on a node in normal or foreign-device mode.

use core::net::SocketAddrV4;

use bacnet_bip_codec::Bvll;
use bacnet_time::Instant;

use super::{Datagrams, HeldBroadcast, Ingress, PortCounters, PortError, count};
use crate::node::{
    Action, Admission, Cast, Datagram, Foreign, Inbound, Link, LocalFailure, Mode, Node, Policy,
    SendRefusal,
};
use bacnet_npdu::MAX_NPDU;

/// A port on a B/IP node in normal or foreign-device mode, which it owns,
/// with `X` exchange slots and held-broadcast slots of `N` octets.
///
/// It carries out every action the node queues: the registrations, and the
/// broadcasts held until the first registration attempt resolves, which it
/// sends or drops as the node says. It drops the rest: the registration
/// events, and a manual exchange's end, whose response it also swallows
/// (the node matches it, and [`receive`](Self::receive) says only
/// [`Ingress::Handled`]). What the events carried stays readable in
/// [`Node::registration`] and [`Node::take_counters`]; a caller that runs
/// manual exchanges drives a node of its own.
#[derive(Debug)]
pub struct NodePort<'h, const X: usize = 4, const N: usize = MAX_NPDU> {
    node: Node<X>,
    held: &'h mut [HeldBroadcast<N>],
    held_len: usize,
    counters: PortCounters,
}

impl<'h, const X: usize, const N: usize> NodePort<'h, X, N> {
    /// A port on `node`, holding a foreign device's broadcasts in `held`:
    /// at least [`Foreign::hold`](crate::node::Foreign::hold) slots, each
    /// as long as the link's broadcast limit ([`Link::max_npdu`] for
    /// [`Cast::Broadcast`]) when it holds any. A normal node holds none.
    ///
    /// [`Link::max_npdu`]: crate::node::Link::max_npdu
    pub fn new(node: Node<X>, held: &'h mut [HeldBroadcast<N>]) -> Result<Self, PortError> {
        check_hold::<N>(node.link(), held.len())?;
        Ok(Self {
            node,
            held,
            held_len: 0,
            counters: PortCounters::default(),
        })
    }

    /// The node.
    #[must_use]
    pub const fn node(&self) -> &Node<X> {
        &self.node
    }

    /// The node, to take its counters. Leave its actions to the port: one
    /// polled here is one the port never carries out.
    pub fn node_mut(&mut self) -> &mut Node<X> {
        &mut self.node
    }

    /// The counts since the counters were last taken, which then start
    /// again from zero: add them to your own totals, as wide as you need.
    /// Each is a `u32` that saturates between takes.
    #[must_use = "the counts are gone once taken; add them to your totals"]
    pub fn take_counters(&mut self) -> PortCounters {
        core::mem::take(&mut self.counters)
    }

    /// The counts since the counters were last taken, as
    /// [`take_counters`](Self::take_counters) would return them, without
    /// taking them.
    #[must_use]
    pub const fn counters(&self) -> PortCounters {
        self.counters
    }

    /// Take a datagram the port's socket received.
    pub fn receive<'d>(
        &mut self,
        datagram: &Datagram<'d>,
        now: Instant,
        policy: &mut impl Policy,
        out: &mut impl Datagrams,
    ) -> Ingress<'d> {
        self.drain(now, out);
        let ingress = match self.node.receive(datagram, now, policy) {
            Inbound::Deliver(delivery) => Ingress::Npdu(delivery),
            Inbound::Nak { to, code, .. } => {
                count(&mut self.counters, out.send(to, &Bvll::BvlcResult(code)));
                Ingress::Handled
            }
            Inbound::Discarded(why) => Ingress::Discarded(why),
            // `new` refuses a BBMD, so the node has no share to hand over.
            Inbound::Bbmd(_)
            | Inbound::Matched { .. }
            | Inbound::Duplicate { .. }
            | Inbound::Unmatched { .. } => Ingress::Handled,
        };
        self.drain(now, out);
        ingress
    }

    /// Start the port at `now`, once, before anything else: a foreign
    /// device's first registration goes out, and its response timeout
    /// becomes [`next_deadline`](Self::next_deadline). Until then the
    /// registration waits in the node's queue, and nothing wakes the port
    /// for it.
    pub fn start(&mut self, now: Instant, out: &mut impl Datagrams) {
        self.on_deadline(now, out);
    }

    /// Move a foreign device to `foreign`'s BBMD and time-to-live at `now`
    /// ([`Node::retarget`]): its first attempt there goes out at once.
    /// Refused, changing nothing, when the node is not a foreign device,
    /// when the link refuses the registration, or when the port's held
    /// slots can't take what `foreign` holds.
    pub fn retarget(
        &mut self,
        foreign: Foreign,
        now: Instant,
        out: &mut impl Datagrams,
    ) -> Result<(), PortError> {
        let mut link = *self.node.link();
        link.mode = Mode::Foreign(foreign);
        check_hold::<N>(&link, self.held.len())?;
        self.drain(now, out);
        self.node.retarget(foreign, now).map_err(PortError::Link)?;
        self.drain(now, out);
        Ok(())
    }

    /// Send `npdu` to the node at `to`, as an Original-Unicast-NPDU (Annex
    /// J.3). A group address of the link is refused before anything is
    /// encoded ([`SendRefusal::GroupDestination`], #1677).
    ///
    /// The node is first brought up to `now`, and what it owes from before
    /// then goes out first.
    pub fn unicast(
        &mut self,
        to: SocketAddrV4,
        npdu: &[u8],
        now: Instant,
        out: &mut impl Datagrams,
    ) -> Result<(), SendRefusal> {
        self.on_deadline(now, out);
        let outgoing = self.node.link().unicast(to, npdu)?;
        out.send(outgoing.to, &outgoing.message)
    }

    /// Broadcast `npdu` on the network: sent at once, or held while a
    /// foreign device's first registration attempt is outstanding, as the
    /// [`Admission`] says.
    ///
    /// The node is first brought up to `now`, and what it owes from before
    /// then goes out first: broadcasts it held for a first registration
    /// whose response timeout has passed unserved leave before this one,
    /// in the order they were held.
    pub fn broadcast(
        &mut self,
        npdu: &[u8],
        now: Instant,
        out: &mut impl Datagrams,
    ) -> Result<Admission, SendRefusal> {
        self.on_deadline(now, out);
        let result = self.admit(npdu, now, out);
        self.drain(now, out);
        result
    }

    /// When the node next has work of its own ([`Node::next_deadline`]).
    #[must_use]
    pub fn next_deadline(&self) -> Option<Instant> {
        self.node.next_deadline()
    }

    /// Do what the node has due at `now`, and carry out what it asks.
    pub fn on_deadline(&mut self, now: Instant, out: &mut impl Datagrams) {
        self.node.on_deadline(now);
        self.drain(now, out);
    }

    /// A broadcast: sent, or held while the first registration attempt is
    /// outstanding.
    fn admit(
        &mut self,
        npdu: &[u8],
        now: Instant,
        out: &mut impl Datagrams,
    ) -> Result<Admission, SendRefusal> {
        let (outgoing, _) = self.node.link().broadcast(npdu)?;
        let admission = self.node.admit_broadcast(now)?;
        match admission {
            Admission::Send => out.send(outgoing.to, &outgoing.message)?,
            Admission::Hold => {
                // `new` lent a slot for each broadcast the node holds, each
                // as long as any the link's limit lets through.
                let slot = self
                    .held
                    .get_mut(self.held_len)
                    .ok_or(SendRefusal::HoldFull)?;
                let room = slot
                    .npdu
                    .get_mut(..npdu.len())
                    .ok_or(SendRefusal::TooLong { max: N })?;
                room.copy_from_slice(npdu);
                // At most N octets, and N is at most 65,535.
                slot.len = npdu.len() as u16;
                self.held_len += 1;
            }
        }
        Ok(admission)
    }

    /// Carry out every action the node has queued.
    fn drain(&mut self, now: Instant, out: &mut impl Datagrams) {
        loop {
            match self.node.poll_action() {
                Action::Idle => return,
                Action::Register { exchange, to, ttl } => {
                    if out.send(to, &Bvll::RegisterForeignDevice { ttl }).is_ok() {
                        self.node.exchange_sent(exchange, now);
                    } else {
                        let failure = LocalFailure::SendFailed;
                        self.node.exchange_failed(exchange, failure, now);
                    }
                }
                Action::ReleaseHeld => {
                    let link = *self.node.link();
                    for slot in &self.held[..self.held_len] {
                        let npdu = &slot.npdu[..usize::from(slot.len)];
                        let sent = link
                            .broadcast(npdu)
                            .and_then(|(outgoing, _)| out.send(outgoing.to, &outgoing.message));
                        count(&mut self.counters, sent);
                    }
                    self.held_len = 0;
                }
                Action::DiscardHeld => self.held_len = 0,
                // The node keeps both in its registration snapshot and
                // counters.
                Action::ExchangeEnded { .. } | Action::Registration(_) => {}
            }
        }
    }
}

/// Whether held slots of `N` octets, `available` of them, take what `link`
/// holds; a BBMD's link has no place on a [`NodePort`].
fn check_hold<const N: usize>(link: &Link, available: usize) -> Result<(), PortError> {
    let hold = match link.mode {
        Mode::Normal => 0,
        Mode::Foreign(foreign) => usize::from(foreign.hold),
        Mode::Bbmd => return Err(PortError::Mode),
    };
    if hold > available {
        return Err(PortError::HeldStorage {
            needed: hold,
            available,
        });
    }
    let longest = link.max_npdu(Cast::Broadcast);
    if hold > 0 && longest > N {
        return Err(PortError::HeldLength {
            needed: longest,
            available: N,
        });
    }
    Ok(())
}
