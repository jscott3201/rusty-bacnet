//! A received frame, from its link source to the decision on its APDU.

use super::config::Options;
use super::counters::bump;
use super::host::{Group, Host};
use super::tasks::Task;
use super::Storage;
use crate::message::RejectReason;
use crate::npdu::{Address, AddressField, Malformed, Npdu, ParseError, GLOBAL_NETWORK};
use crate::table::{Reachability, Route};
use crate::{Instant, Mac, PortId};

/// A frame a port received.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame<'f> {
    /// The port.
    pub port: PortId,
    /// The link-layer source address. For a B/IP Forwarded-NPDU it is the
    /// origin the BBMD names, so the SADR the router adds names the origin.
    pub source: &'f [u8],
    /// Whether it arrived as a link-layer broadcast or multicast.
    pub link_group: bool,
    /// The NPDU.
    pub npdu: &'f [u8],
}

/// What a frame means for the router itself. It borrows the frame, not the
/// router, so the caller can poll the actions the frame also prompted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "a frame may be for the router's own application"]
pub enum Received<'f> {
    /// An APDU for the router's own application entity, if it has one. It
    /// may come with sends: a global broadcast, or a DLEN 0 broadcast on
    /// the arrival network, is also passed on.
    Local(Local<'f>),
    /// A reject addressed to the router: for its network-control consumer,
    /// if it has one. The table has already taken it.
    ToRouter(Npdu<'f>),
    /// Routed, answered, relayed or taken into the table: see the actions.
    Handled,
    /// Discarded, and counted, for this reason.
    Discarded(Discard),
}

/// An APDU for the router's own application entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Local<'f> {
    /// The APDU.
    pub apdu: &'f [u8],
    /// Where it starts in the frame's NPDU, so a host can slice its own
    /// buffer.
    pub apdu_offset: usize,
    /// SNET and SADR, when a router relayed it.
    pub source: Option<Address<'f>>,
    /// Whether it was sent to a group: a link broadcast with no DNET, a
    /// DLEN 0 broadcast, or a global broadcast.
    pub group: bool,
    /// Whether it was a global broadcast.
    pub global: bool,
    /// Whether its sender expects a reply.
    pub expecting_reply: bool,
    /// Its network priority: the control octet's two low bits, 0 (normal)
    /// to 3 (life safety), so never more than 3.
    pub priority: u8,
}

/// Why a frame was discarded. Each has its count in
/// [`Counters`](super::Counters).
///
/// It is `#[non_exhaustive]`, so a new reason isn't a breaking change: a
/// `match` needs a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Discard {
    /// It came on a port the router doesn't have, or its route names one
    /// (a route written through [`Router::table_mut`](super::Router::table_mut)).
    UnknownPort,
    /// Its link source is longer than [`MAX_MAC_LEN`](crate::MAX_MAC_LEN)
    /// (#1198). Nothing is sent back to such a source.
    SourceTooLong {
        /// The source's length.
        length: usize,
    },
    /// A DLEN or SLEN past [`MAX_MAC_LEN`](crate::MAX_MAC_LEN) (#1141).
    /// With a DNET other than 0xFFFF, reject reason 6 is queued.
    AddressTooLong {
        /// Which address.
        field: AddressField,
        /// Its length octet.
        length: u8,
    },
    /// It isn't an NPDU.
    Malformed(Malformed),
    /// A DADR beside DNET 0xFFFF (#1379).
    GlobalWithDadr,
    /// A broadcast whose APDU isn't an Unconfirmed-Request (#1491).
    BroadcastNotUnconfirmed,
    /// For a DADR that is a group address on its delivery port: an APDU
    /// other than an Unconfirmed-Request, a network message, or anything
    /// for a [`Group::Refuse`] address ([`Group`]; #1504, #1677).
    GroupDadr,
    /// Its hop count had run out.
    HopCount,
    /// [`Host::authorize`] refused it.
    Denied,
    /// A network message whose data, or whose addressing, its type doesn't
    /// allow.
    BadMessage,
    /// A received reject that isn't for this router and has nowhere to go:
    /// its DNET is 0xFFFF or the arrival network, or has no route.
    Unroutable,
    /// Its rewrite can't be written: longer than the scratch, or with a
    /// field no NPDU may carry, such as an SNET of 0.
    Unwritable,
}

/// What the router knows about the frame in hand.
#[derive(Debug, Clone, Copy)]
pub(super) struct Arrival<'f> {
    /// The arrival port.
    pub(super) port: PortId,
    /// Its network.
    pub(super) network: u16,
    /// The link source.
    pub(super) source: &'f [u8],
    /// The same, stored.
    pub(super) source_mac: Mac,
    /// Whether it was a link broadcast or multicast.
    pub(super) link_group: bool,
    /// The time.
    pub(super) now: Instant,
}

/// The local delivery of `npdu`'s APDU.
fn local<'f>(npdu: &Npdu<'f>, group: bool, global: bool) -> Local<'f> {
    Local {
        apdu: npdu.payload,
        apdu_offset: npdu.payload_offset,
        source: npdu.source,
        group,
        global,
        expecting_reply: npdu.expecting_reply,
        priority: npdu.priority,
    }
}

/// Whether an APDU is an Unconfirmed-Request: the high nibble of its first
/// octet (Clause 20.1). An empty one names no type.
pub(super) fn is_unconfirmed(apdu: &[u8]) -> bool {
    apdu.first().is_some_and(|&first| first >> 4 == 1)
}

impl<const P: usize, const R: usize, const H: usize, const D: usize, const N: usize>
    Storage<P, R, H, D, N>
{
    /// [`Router::receive`](super::Router::receive).
    pub(super) fn receive<'f>(
        &mut self,
        frame: Frame<'f>,
        now: Instant,
        options: &Options,
        host: &mut impl Host,
    ) -> Received<'f> {
        self.drop_undrained();
        let Some(config) = self.ports[..self.port_count].get(usize::from(frame.port.0)) else {
            bump(&mut self.counters.unknown_port);
            return Received::Discarded(Discard::UnknownPort);
        };
        let Ok(source_mac) = Mac::from_slice(frame.source) else {
            bump(&mut self.counters.address_length_drops);
            return Received::Discarded(Discard::SourceTooLong {
                length: frame.source.len(),
            });
        };
        let at = Arrival {
            port: frame.port,
            network: config.network,
            source: frame.source,
            source_mac,
            link_group: frame.link_group,
            now,
        };
        let npdu = match Npdu::parse(frame.npdu) {
            Ok(npdu) => npdu,
            Err(ParseError::AddressTooLong {
                field,
                length,
                dnet,
                source,
            }) => {
                bump(&mut self.counters.address_length_drops);
                // A global broadcast draws no reject, and an NPDU without a
                // DNET has no network to name.
                if let Some(dnet) = dnet.filter(|&dnet| dnet != GLOBAL_NETWORK) {
                    self.reject(&at, source, dnet, RejectReason::ADDRESSING_ERROR, host);
                }
                return Received::Discarded(Discard::AddressTooLong { field, length });
            }
            Err(ParseError::Malformed(fault)) => {
                bump(&mut self.counters.malformed);
                return Received::Discarded(Discard::Malformed(fault));
            }
        };
        if let Some(destination) = npdu.destination {
            let global = destination.network == GLOBAL_NETWORK;
            if global && !destination.mac.is_empty() {
                bump(&mut self.counters.global_broadcast_dadr_drops);
                return Received::Discarded(Discard::GlobalWithDadr);
            }
            let broadcast = global || destination.mac.is_empty();
            if broadcast && !npdu.is_network_message() && !is_unconfirmed(npdu.payload) {
                bump(&mut self.counters.broadcast_pdu_type_drops);
                return Received::Discarded(Discard::BroadcastNotUnconfirmed);
            }
        }
        if let Some(kind) = npdu.message_type {
            return self.network_message(&at, &npdu, kind, options, host);
        }
        self.route_apdu(&at, &npdu, options, host)
    }

    /// Deliver or route an APDU by its DNET.
    fn route_apdu<'f>(
        &mut self,
        at: &Arrival<'f>,
        npdu: &Npdu<'f>,
        options: &Options,
        host: &mut impl Host,
    ) -> Received<'f> {
        let Some(destination) = npdu.destination else {
            return Received::Local(local(npdu, at.link_group, false));
        };
        if destination.network == GLOBAL_NETWORK {
            self.forward_global(at, npdu, host);
            return Received::Local(local(npdu, true, true));
        }
        // The router's own address on any of its networks is its own
        // application's, before any route is looked up (Clause 6.1, Figure
        // 6-12). A DLEN 0 broadcast names no one node, so it is never that.
        if !destination.mac.is_empty() && self.is_own(destination) {
            return Received::Local(local(npdu, false, false));
        }
        let Some(route) = self.find_route(at, npdu, destination.network, options, host) else {
            return Received::Handled;
        };
        let mut delivered = None;
        if route.port() == at.port && route.is_direct() {
            let own = &self.ports[usize::from(at.port.0)].mac;
            if *own == *destination.mac {
                return Received::Local(local(npdu, false, false));
            }
            // A DLEN 0 broadcast on the arrival network is delivered, and
            // also sent back out the arrival port below, as the hosted
            // router does (group_dadr_tests.rs:96 pins that relay). One at
            // the port's limit is delivered and also draws reason 4, since
            // the relay gains SNET and SADR.
            if destination.mac.is_empty() {
                delivered = Some(local(npdu, true, false));
            }
        }
        let mut npdu = *npdu;
        match self.group_of(&route, &npdu, host) {
            Group::No => {}
            // An Unconfirmed-Request for a group of the port's own goes out
            // as the port's local broadcast: no DADR on a directly attached
            // network. It reaches the router's application where a DLEN 0
            // broadcast would: when it came from that network.
            Group::Broadcast if is_unconfirmed(npdu.payload) => {
                npdu.destination = Some(Address {
                    network: destination.network,
                    mac: &[],
                });
                if route.port() == at.port {
                    delivered = Some(local(&npdu, true, false));
                }
            }
            Group::Broadcast | Group::Refuse => return self.group_drop(),
        }
        let forwarded = self.forward(at, &npdu, &route, host);
        delivered.map_or(forwarded, Received::Local)
    }

    /// The usable route to `dnet`, marked as used; or, when the table has
    /// none or can't use it now, `None`, with the search and the reject
    /// that answer the frame queued (Clauses 6.6.3.1, 6.6.3.5, 6.6.3.6).
    /// Nothing is held back for the search: the sender gets reason 1 at
    /// once, and its next message takes whatever route the search finds.
    pub(super) fn find_route(
        &mut self,
        at: &Arrival<'_>,
        npdu: &Npdu<'_>,
        dnet: u16,
        options: &Options,
        host: &mut impl Host,
    ) -> Option<Route> {
        let route = self.table.lookup(dnet);
        let reachability = self.table.effective_reachability(dnet, at.now);
        let Some(route) = route else {
            if self
                .discovery
                .should_solicit(dnet, at.now, options.discovery_cooldown)
            {
                bump(&mut self.counters.solicitations);
                self.out.push(Task::solicit(at.port, dnet));
            }
            self.reject(
                at,
                npdu.source,
                dnet,
                RejectReason::NOT_DIRECTLY_CONNECTED,
                host,
            );
            return None;
        };
        self.table.touch(dnet, at.now);
        let refusal = match reachability {
            Some(Reachability::Busy) => RejectReason::ROUTER_BUSY,
            Some(Reachability::Unreachable) => RejectReason::NOT_DIRECTLY_CONNECTED,
            _ => return Some(route),
        };
        self.reject(at, npdu.source, dnet, refusal, host);
        None
    }

    /// What `npdu`'s DADR is on its delivery port: asked of the host only
    /// when `route` hands it to a node on a directly attached port. A DLEN
    /// 0 broadcast, and a DADR beyond another router, are [`Group::No`].
    pub(super) fn group_of(&self, route: &Route, npdu: &Npdu<'_>, host: &impl Host) -> Group {
        match npdu.destination {
            Some(destination) if route.is_direct() && !destination.mac.is_empty() => {
                host.group(route.port(), destination.mac)
            }
            _ => Group::No,
        }
    }

    /// Count a frame dropped for a group DADR (#1504, #1677).
    pub(super) fn group_drop<'f>(&mut self) -> Received<'f> {
        bump(&mut self.counters.group_dadr_drops);
        Received::Discarded(Discard::GroupDadr)
    }
}
