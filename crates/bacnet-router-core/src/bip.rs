//! B/IP ports: the glue between a [`Router`](crate::Router) and
//! `bacnet-bip-core`'s node engine (feature `bip`).
//!
//! To the router a B/IP port is data, the [`PortConfig`] that
//! [`port_config`] reads from the node's [`Link`]. The rest goes through a
//! port that owns the port's [`Node`], one type per mode, since a link is
//! never both a BBMD and a foreign device:
//!
//! - [`BipPort`]: a node in normal or foreign-device mode.
//! - [`BbmdPort`]: a BBMD, which runs the BBMD's share of each message
//!   through its [`Tables`].
//!
//! Each port does what its node asks: the NAKs, a foreign device's
//! registrations, and the broadcasts it holds until the first one resolves.
//! Every datagram leaves through the caller's [`Datagrams`], which encodes
//! the message straight into its socket's buffer, so a port keeps no
//! datagram buffer of its own.
//!
//! Wiring a port to the router:
//!
//! - **Receive:** hand each datagram to the port's `receive`. For
//!   [`Ingress::Npdu`], pass its [`Arrived::frame`] to
//!   [`Router::receive`](crate::Router::receive). Its source is the B/IP
//!   address the node delivered from, which for a Forwarded-NPDU is the
//!   origin (Annex J.2.5), so the SADR the router adds names the origin
//!   (#1678). A datagram the node drops never reaches the router.
//! - **Send:** hand each [`Send`] for the port to its `send`, and an error
//!   to [`Router::refused`](crate::Router::refused). A unicast goes through
//!   [`Link::unicast`], which refuses a group address (#1677); a broadcast
//!   through [`Link::broadcast`], held while a foreign device's first
//!   registration is outstanding, and also forwarded by a BBMD.
//! - **Start:** call [`BipPort::start`] once, before anything else: a
//!   foreign device's first registration is due at once, and its response
//!   timeout is the port's first deadline. A [`BbmdPort`] registers
//!   nowhere, and needs no start.
//! - **Timers:** sleep until the earliest of the router's and the ports'
//!   `next_deadline`, and call each one's `on_deadline`.
//! - **Groups:** answer [`Host::group`](crate::router::Host::group) for
//!   the port with [`group`].
//!
//! # Annex J and Clause 6, for a router's B/IP port
//!
//! | Obligation, in our words | Clause | Test |
//! | --- | --- | --- |
//! | A Forwarded-NPDU's originating address is the source of its NPDU, so a router names it in the SADR it adds, and the network priority travels with the message | J.2.5, 6.2.2, 6.5.4 | `tests/glue_bip_mstp.rs`: `forwarded_origin_and_priority_survive_bip_to_mstp_composition` |
//! | A port that is a foreign-device registration registers before the router announces its networks there | J.7.2, J.7.4 | `tests/glue_bip_foreign.rs`: `foreign_port_announces_after_registration`, `foreign_port_announces_after_the_response_timeout`, `held_broadcasts_leave_before_a_later_one` |
//! | A BBMD port hands the router the broadcasts its subnet, its peers and its foreign devices send, and forwards them, and forwards the router's own broadcasts too | J.4.5, J.7.1 | `tests/glue_bip.rs`: `bbmd_port_delivers_and_forwards` |
//! | A directed message goes to one node's own address, never to a group address | J.2.11, J.3 | `tests/glue_bip.rs`: `unicast_group_destinations_are_rejected_before_encoding_or_send` |
//!
//! Local rules: a datagram the node drops, a malformed one included,
//! changes no router or queue state (`tests/glue_bip_mstp.rs`:
//! `strict_bip_failure_precedes_router_or_queue_mutation`); each refusal of
//! a router send maps to one [`SendRefusal`] and is counted once
//! (`real_bip_success_and_admission_failures_are_observed_once`). On a
//! foreign-device port, a refused first registration drops the held
//! broadcasts, a registration the socket refuses ends its attempt, and a
//! full hold refuses the next broadcast as [`SendRefusal::Full`]; each
//! datagram of a port's own that its socket refuses is counted once in its
//! [`PortCounters`] (`tests/glue_bip_foreign.rs`, `tests/glue_bip.rs`).

use core::fmt;
use core::net::SocketAddrV4;

use bacnet_bip_codec::{address_from_slice, address_to_octets, Bvll};
use bacnet_bip_core::bbmd::{Context, ForwardRefusal, Tables, Target};
use bacnet_bip_core::node::{
    self, Action, Admission, Cast, Datagram, Delivery, Discard, ForBbmd, Inbound, Link,
    LocalFailure, Mode, Node, Policy,
};

use crate::router::{Dest, Group, Limits, PortConfig, Send, SendRefusal};
use crate::{Arrived, Instant, Mac, PortId, MAX_NPDU};

/// Where a B/IP port's datagrams go: the host's UDP socket.
pub trait Datagrams {
    /// Send `message` to `to` from the port's own socket.
    ///
    /// Encode it with [`Bvll::encode_into`] or [`Bvll::encode_to_sink`]
    /// straight into the socket's buffer, sized by [`Bvll::encoded_len`],
    /// and keep no borrow of it. Refuse it with [`SendRefusal::Full`] when
    /// the socket can't take it now, and with [`SendRefusal::Other`] when it
    /// can't be encoded there (`BufferExhausted`, say).
    fn send(&mut self, to: SocketAddrV4, message: &Bvll<'_>) -> Result<(), SendRefusal>;
}

/// A B/IP port's [`PortConfig`] on `network`: the router's MAC is the
/// link's own B/IP address, and its limits are [`Link::max_npdu`] for each
/// cast, at most [`MAX_NPDU`] (Table 6-1).
#[must_use]
pub fn port_config(link: &Link, network: u16) -> PortConfig {
    // At most MAX_NPDU, so the casts are exact.
    let limit = |cast| link.max_npdu(cast).min(MAX_NPDU) as u16;
    PortConfig {
        network,
        mac: Mac::from_array(address_to_octets(link.address)),
        max_npdu: Limits {
            unicast: limit(Cast::Unicast),
            broadcast: limit(Cast::Broadcast),
        },
    }
}

/// What `mac` is on a B/IP port, for [`Host::group`](crate::router::Host::group):
/// one of the link's group addresses ([`Link::groups`]) at the link's own
/// UDP port is its local broadcast; one at another port belongs to another
/// B/IP network, and is refused (#1677, decision L24).
#[must_use]
pub fn group(link: &Link, mac: &[u8]) -> Group {
    match address_from_slice(mac) {
        Ok(address) if link.groups().contains(address) => {
            if address.port() == link.address.port() {
                Group::Broadcast
            } else {
                Group::Refuse
            }
        }
        _ => Group::No,
    }
}

/// What a received datagram brought.
///
/// It is exhaustive, as the router's [`Received`](crate::router::Received)
/// and the node's [`Inbound`] are: the caller serves each case, a BBMD's
/// [`Management`](Self::Management) requests included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ingress<'d> {
    /// An NPDU for the router: pass [`Arrived::frame`] to
    /// [`Router::receive`](crate::Router::receive).
    Npdu(Arrived<'d>),
    /// Nothing for the router: the port answered it with a NAK, or it
    /// answered one of the node's own requests.
    Handled,
    /// The node dropped it, for this reason.
    Discarded(Discard),
    /// A BBMD's tables refused a Forwarded-NPDU: [`BbmdPort`] only.
    NotForwarded(ForwardRefusal),
    /// A request for a BBMD's tables (Register-Foreign-Device, the table
    /// reads, Delete-Foreign-Device-Table-Entry), for the caller to serve
    /// through [`BbmdPort::tables_mut`]: [`BbmdPort`] only.
    Management(ForBbmd<'d>),
}

/// Why a port refused its node and storage.
///
/// It is `#[non_exhaustive]`, so a new refusal isn't a breaking change: a
/// `match` needs a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PortError {
    /// A [`BipPort`] was offered a BBMD's node, or a [`BbmdPort`] another
    /// mode's.
    Mode,
    /// A foreign device holds more broadcasts than the port has slots for.
    HeldStorage {
        /// The broadcasts the node holds ([`Foreign::hold`](node::Foreign::hold)).
        needed: usize,
        /// The slots lent.
        available: usize,
    },
    /// A foreign device holds broadcasts longer than a slot: its link's
    /// broadcast limit ([`Link::max_npdu`] for [`Cast::Broadcast`]) is
    /// over the slots' `N`. A link with no send budget has no limit short
    /// of the largest BVLL message.
    HeldLength {
        /// The longest broadcast the link sends, in octets.
        needed: usize,
        /// A slot's length, `N`.
        available: usize,
    },
    /// A [`BbmdPort`]'s target storage is shorter than
    /// [`Tables::MAX_TARGETS`].
    TargetStorage {
        /// [`Tables::MAX_TARGETS`].
        needed: usize,
        /// The targets lent.
        available: usize,
    },
}

impl fmt::Display for PortError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mode => f.write_str("the node's mode doesn't suit this port"),
            Self::HeldStorage { needed, available } => write!(
                f,
                "the node holds {needed} broadcasts, with slots for {available}"
            ),
            Self::HeldLength { needed, available } => write!(
                f,
                "a held broadcast may be {needed} octets, with slots of {available}"
            ),
            Self::TargetStorage { needed, available } => write!(
                f,
                "a BBMD forwards to up to {needed} targets, with room for {available}"
            ),
        }
    }
}

impl core::error::Error for PortError {}

/// What a B/IP port counted since its counters were last taken. Its node
/// keeps counts of its own ([`Node::take_counters`]).
///
/// It is `#[non_exhaustive]`, so a new count isn't a breaking change.
/// Outside this crate, build one from `PortCounters::default()` and set its
/// fields.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct PortCounters {
    /// Datagrams of the port's own that the sink refused: a NAK, a
    /// released broadcast, or a BBMD's forwarded copy. A refused
    /// registration is the node's
    /// [`local_errors`](node::ExchangeCounters::local_errors).
    pub refused: u32,
}

/// Room for one broadcast that a foreign device's port holds until its
/// first registration attempt resolves (Annex J.7.4; decision D17). It is
/// all zero bits, so firmware lends the port a static's.
///
/// A slot holds `N` octets, at most 65,535: by default [`MAX_NPDU`], B/IP's
/// largest NPDU. A link with a smaller send budget sends shorter
/// broadcasts, and its port can hold them in shorter slots;
/// [`BipPort::new`] checks that the link's broadcast limit fits `N`.
///
/// A default doesn't drive inference, so `HeldBroadcast::EMPTY` alone, in
/// an expression, leaves `N` open. Name the type instead: `<HeldBroadcast>`
/// is a type, so it takes the default.
///
/// ```
/// use bacnet_router_core::bip::HeldBroadcast;
///
/// // Two slots of MAX_NPDU octets, the default.
/// let full = [<HeldBroadcast>::EMPTY; 2];
/// // Two slots of 501 octets, for a link whose broadcasts are no longer.
/// let short = [HeldBroadcast::<501>::EMPTY; 2];
/// # let _ = (full, short);
/// ```
#[derive(Clone)]
pub struct HeldBroadcast<const N: usize = MAX_NPDU> {
    len: u16,
    npdu: [u8; N],
}

impl<const N: usize> HeldBroadcast<N> {
    /// An empty slot.
    pub const EMPTY: Self = {
        assert!(
            N <= u16::MAX as usize,
            "a held broadcast is at most 65,535 octets"
        );
        Self {
            len: 0,
            npdu: [0; N],
        }
    };
}

impl<const N: usize> Default for HeldBroadcast<N> {
    fn default() -> Self {
        Self::EMPTY
    }
}

/// The length held, not the slot.
impl<const N: usize> fmt::Debug for HeldBroadcast<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HeldBroadcast")
            .field("len", &self.len)
            .finish_non_exhaustive()
    }
}

/// A router port on a B/IP node in normal or foreign-device mode, which it
/// owns, with `X` exchange slots and held-broadcast slots of `N` octets.
#[derive(Debug)]
pub struct BipPort<'h, const X: usize = 4, const N: usize = MAX_NPDU> {
    port: PortId,
    node: Node<X>,
    held: &'h mut [HeldBroadcast<N>],
    held_len: usize,
    counters: PortCounters,
}

impl<'h, const X: usize, const N: usize> BipPort<'h, X, N> {
    /// Port `port` on `node`, holding a foreign device's broadcasts in
    /// `held`: at least [`Foreign::hold`](node::Foreign::hold) slots, each
    /// as long as the link's broadcast limit ([`Link::max_npdu`] for
    /// [`Cast::Broadcast`]) when it holds any. A normal node holds none.
    pub fn new(
        port: PortId,
        node: Node<X>,
        held: &'h mut [HeldBroadcast<N>],
    ) -> Result<Self, PortError> {
        let link = node.link();
        let hold = match link.mode {
            Mode::Normal => 0,
            Mode::Foreign(foreign) => usize::from(foreign.hold),
            Mode::Bbmd => return Err(PortError::Mode),
        };
        if hold > held.len() {
            return Err(PortError::HeldStorage {
                needed: hold,
                available: held.len(),
            });
        }
        let longest = link.max_npdu(Cast::Broadcast);
        if hold > 0 && longest > N {
            return Err(PortError::HeldLength {
                needed: longest,
                available: N,
            });
        }
        Ok(Self {
            port,
            node,
            held,
            held_len: 0,
            counters: PortCounters::default(),
        })
    }

    /// The port's index.
    #[must_use]
    pub const fn port(&self) -> PortId {
        self.port
    }

    /// The port's [`PortConfig`] on `network` ([`port_config`]).
    #[must_use]
    pub fn config(&self, network: u16) -> PortConfig {
        port_config(self.node.link(), network)
    }

    /// What `mac` is on this port ([`group`]).
    #[must_use]
    pub fn group(&self, mac: &[u8]) -> Group {
        group(self.node.link(), mac)
    }

    /// The node.
    #[must_use]
    pub const fn node(&self) -> &Node<X> {
        &self.node
    }

    /// The node, to take its counters. The port carries out every action
    /// the node queues and drops the rest: the registration events, and a
    /// manual exchange's end, whose response it also swallows (the node
    /// matches it, and [`receive`](Self::receive) says only
    /// [`Ingress::Handled`]). What the events carried stays readable in
    /// [`Node::registration`] and [`Node::take_counters`]; a caller that
    /// runs manual exchanges drives a node of its own.
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
            Inbound::Deliver(delivery) => Ingress::Npdu(arrived(self.port, &delivery)),
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

    /// Carry out one of the router's sends on this port; one for another
    /// port is refused.
    ///
    /// The node is first brought up to `now`, and what it owes from before
    /// then goes out first: broadcasts it held for a first registration
    /// whose response timeout has passed unserved leave before this one,
    /// in the order they were held.
    pub fn send(
        &mut self,
        send: &Send<'_>,
        now: Instant,
        out: &mut impl Datagrams,
    ) -> Result<(), SendRefusal> {
        if send.port != self.port {
            return Err(SendRefusal::Other);
        }
        self.on_deadline(now, out);
        let result = match send.to {
            Dest::Unicast(mac) => unicast(self.node.link(), mac, send.npdu, out),
            Dest::Broadcast => self.broadcast(send.npdu, now, out),
        };
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
    fn broadcast(
        &mut self,
        npdu: &[u8],
        now: Instant,
        out: &mut impl Datagrams,
    ) -> Result<(), SendRefusal> {
        let (outgoing, _) = self.node.link().broadcast(npdu).map_err(refusal)?;
        match self.node.admit_broadcast(now).map_err(refusal)? {
            Admission::Send => out.send(outgoing.to, &outgoing.message),
            Admission::Hold => {
                // `new` lent a slot for each broadcast the node holds, each
                // as long as any the link's limit lets through.
                let slot = self.held.get_mut(self.held_len).ok_or(SendRefusal::Full)?;
                let room = slot
                    .npdu
                    .get_mut(..npdu.len())
                    .ok_or(SendRefusal::TooLong)?;
                room.copy_from_slice(npdu);
                // At most N octets, and N is at most 65,535.
                slot.len = npdu.len() as u16;
                self.held_len += 1;
                Ok(())
            }
        }
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
                            .map_err(refusal)
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

/// A router port on a BBMD, which owns its node and borrows its tables, of
/// `B` BDT rows and `F` foreign devices, and storage for the targets of
/// one forwarding decision.
///
/// The tables keep no clock of their own, so the port keeps the latest
/// time it has been given and hands them that: a `now` earlier than it is
/// taken as it, as the node takes its own, and a registration that has
/// lapsed stays lapsed.
#[derive(Debug)]
pub struct BbmdPort<'t, const B: usize, const F: usize, const X: usize = 4> {
    port: PortId,
    node: Node<X>,
    tables: &'t mut Tables<B, F>,
    targets: &'t mut [Target],
    counters: PortCounters,
    /// The latest time any call has passed in.
    latest: Instant,
}

impl<'t, const B: usize, const F: usize, const X: usize> BbmdPort<'t, B, F, X> {
    /// Port `port` on `node`, a BBMD with `tables`, forwarding through
    /// `targets`: at least [`Tables::MAX_TARGETS`] of them, so no decision
    /// loses one.
    pub fn new(
        port: PortId,
        node: Node<X>,
        tables: &'t mut Tables<B, F>,
        targets: &'t mut [Target],
    ) -> Result<Self, PortError> {
        if node.link().mode != Mode::Bbmd {
            return Err(PortError::Mode);
        }
        if targets.len() < Tables::<B, F>::MAX_TARGETS {
            return Err(PortError::TargetStorage {
                needed: Tables::<B, F>::MAX_TARGETS,
                available: targets.len(),
            });
        }
        Ok(Self {
            port,
            node,
            tables,
            targets,
            counters: PortCounters::default(),
            latest: Instant::ZERO,
        })
    }

    /// `now`, or the latest time passed in when that is later, which it
    /// then stays (the clock rule in the crate docs).
    fn clock(&mut self, now: Instant) -> Instant {
        self.latest = self.latest.max(now);
        self.latest
    }

    /// The port's index.
    #[must_use]
    pub const fn port(&self) -> PortId {
        self.port
    }

    /// The port's [`PortConfig`] on `network` ([`port_config`]).
    #[must_use]
    pub fn config(&self, network: u16) -> PortConfig {
        port_config(self.node.link(), network)
    }

    /// What `mac` is on this port ([`group`]).
    #[must_use]
    pub fn group(&self, mac: &[u8]) -> Group {
        group(self.node.link(), mac)
    }

    /// The node.
    #[must_use]
    pub const fn node(&self) -> &Node<X> {
        &self.node
    }

    /// The node, to take its counters.
    pub fn node_mut(&mut self) -> &mut Node<X> {
        &mut self.node
    }

    /// The tables.
    #[must_use]
    pub fn tables(&self) -> &Tables<B, F> {
        self.tables
    }

    /// The tables, to serve [`Ingress::Management`], set the BDT, or purge
    /// lapsed registrations.
    pub fn tables_mut(&mut self) -> &mut Tables<B, F> {
        self.tables
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

    /// Take a datagram the port's socket received. The BBMD's share of it
    /// goes through the tables: a broadcast is forwarded where they say,
    /// and handed to the router too. A `now` before the latest one passed
    /// in is taken as that one.
    pub fn receive<'d>(
        &mut self,
        datagram: &Datagram<'d>,
        now: Instant,
        policy: &mut impl Policy,
        out: &mut impl Datagrams,
    ) -> Ingress<'d> {
        let now = self.clock(now);
        let link = *self.node.link();
        let ingress = match self.node.receive(datagram, now, policy) {
            Inbound::Deliver(delivery) => Ingress::Npdu(arrived(self.port, &delivery)),
            Inbound::Nak { to, code, .. } => {
                count(&mut self.counters, out.send(to, &Bvll::BvlcResult(code)));
                Ingress::Handled
            }
            Inbound::Discarded(why) => Ingress::Discarded(why),
            Inbound::Bbmd(share) => self.share(share, &link, now, out),
            Inbound::Matched { .. } | Inbound::Duplicate { .. } | Inbound::Unmatched { .. } => {
                Ingress::Handled
            }
        };
        self.drain();
        ingress
    }

    /// The BBMD's share of a message (Annex J.4.5).
    fn share<'d>(
        &mut self,
        share: ForBbmd<'d>,
        link: &Link,
        now: Instant,
        out: &mut impl Datagrams,
    ) -> Ingress<'d> {
        let cx = Context::new(now, F);
        let port = self.port;
        let (source, npdu) = match share {
            ForBbmd::OriginalBroadcast(delivery) => {
                let sent = self.tables.original_broadcast(
                    &cx,
                    delivery.source,
                    delivery.npdu,
                    self.targets,
                );
                add(&mut self.counters, fan_out(out, sent.transmissions()));
                return Ingress::Npdu(arrived(port, &delivery));
            }
            ForBbmd::Forwarded {
                sender,
                arrival,
                origin,
                npdu,
            } => {
                let targets = self.tables.forwarded_npdu(
                    &cx,
                    &link.subnet,
                    sender,
                    arrival,
                    origin,
                    self.targets,
                );
                let fanout = match targets {
                    Ok(fanout) => fanout,
                    Err(why) => return Ingress::NotForwarded(why),
                };
                let message = Bvll::ForwardedNpdu { origin, npdu };
                let copies = fanout
                    .targets
                    .iter()
                    .map(|target| (target.destination, message));
                add(&mut self.counters, fan_out(out, copies));
                (origin, npdu)
            }
            ForBbmd::Distribute { sender, npdu } => {
                let distribution =
                    self.tables
                        .distribute_broadcast(&cx, &link.subnet, sender, npdu, self.targets);
                match distribution {
                    Ok(sent) => add(&mut self.counters, fan_out(out, sent.transmissions())),
                    Err(why) => {
                        let nak = Bvll::BvlcResult(why.result_code());
                        count(&mut self.counters, out.send(sender, &nak));
                        return Ingress::Handled;
                    }
                }
                (sender, npdu)
            }
            management => return Ingress::Management(management),
        };
        Ingress::Npdu(Arrived {
            port,
            source: Mac::from_array(address_to_octets(source)),
            link_group: true,
            npdu,
        })
    }

    /// Carry out one of the router's sends on this port; one for another
    /// port is refused. A broadcast goes out on the subnet and, as the
    /// BBMD's own, to its peers and foreign devices. A `now` before the
    /// latest one passed in is taken as that one.
    pub fn send(
        &mut self,
        send: &Send<'_>,
        now: Instant,
        out: &mut impl Datagrams,
    ) -> Result<(), SendRefusal> {
        if send.port != self.port {
            return Err(SendRefusal::Other);
        }
        let now = self.clock(now);
        let link = self.node.link();
        match send.to {
            Dest::Unicast(mac) => unicast(link, mac, send.npdu, out),
            Dest::Broadcast => {
                let (outgoing, _) = link.broadcast(send.npdu).map_err(refusal)?;
                out.send(outgoing.to, &outgoing.message)?;
                let cx = Context::new(now, F);
                let sent = self.tables.own_broadcast(&cx, send.npdu, self.targets);
                add(&mut self.counters, fan_out(out, sent.transmissions()));
                Ok(())
            }
        }
    }

    /// When the node next has work of its own: a manual exchange's
    /// response timeout.
    #[must_use]
    pub fn next_deadline(&self) -> Option<Instant> {
        self.node.next_deadline()
    }

    /// Do what the node has due at `now`, or at the latest time passed in
    /// when that is later.
    pub fn on_deadline(&mut self, now: Instant) {
        let now = self.clock(now);
        self.node.on_deadline(now);
        self.drain();
    }

    /// Drop what the node queued: a BBMD registers nowhere and holds no
    /// broadcasts, so all it queues is the end of a manual exchange.
    fn drain(&mut self) {
        while self.node.poll_action() != Action::Idle {}
    }
}

/// The router's view of an NPDU the node delivered.
fn arrived<'d>(port: PortId, delivery: &Delivery<'d>) -> Arrived<'d> {
    Arrived {
        port,
        source: Mac::from_array(address_to_octets(delivery.source)),
        link_group: delivery.group,
        npdu: delivery.npdu,
    }
}

/// A router unicast: an Original-Unicast-NPDU to the B/IP address `mac`
/// spells, which must be six octets and no group address.
fn unicast(
    link: &Link,
    mac: &[u8],
    npdu: &[u8],
    out: &mut impl Datagrams,
) -> Result<(), SendRefusal> {
    let to = address_from_slice(mac).map_err(|_| SendRefusal::Other)?;
    let outgoing = link.unicast(to, npdu).map_err(refusal)?;
    out.send(outgoing.to, &outgoing.message)
}

/// The router's refusal for the node's.
fn refusal(why: node::SendRefusal) -> SendRefusal {
    match why {
        node::SendRefusal::TooLong { .. } => SendRefusal::TooLong,
        node::SendRefusal::HoldFull => SendRefusal::Full,
        _ => SendRefusal::Other,
    }
}

/// Send each `(to, message)`, returning how many the sink refused.
fn fan_out<'n>(
    out: &mut impl Datagrams,
    copies: impl Iterator<Item = (SocketAddrV4, Bvll<'n>)>,
) -> u32 {
    copies.fold(0, |refused: u32, (to, message)| {
        refused.saturating_add(u32::from(out.send(to, &message).is_err()))
    })
}

/// Add `n` refused sends to the counts, saturating.
fn add(counters: &mut PortCounters, n: u32) {
    counters.refused = counters.refused.saturating_add(n);
}

/// Count a refused send.
fn count(counters: &mut PortCounters, sent: Result<(), SendRefusal>) {
    add(counters, u32::from(sent.is_err()));
}
