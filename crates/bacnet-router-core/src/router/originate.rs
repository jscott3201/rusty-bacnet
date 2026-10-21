//! Requests the router's own application sends to other networks (#1699).
//!
//! A router's application entity is reachable at the router's address on
//! any of its networks (Clause 6.1), and what it sends to a remote network
//! is routed like any other message (Clause 6.5): by the routing table, to
//! the next router or onto a directly attached network. Unlike a relay,
//! nothing arrived to take a source from, so the request names its own
//! source address ([`SourceSelection`]). Replies come back to that address,
//! which the router delivers to its application as its own.
//!
//! [`Router::originate`](super::Router::originate) writes the request as the
//! call's output, polled as any send is. It doesn't touch local delivery:
//! APDUs for the router's application are still handed over in place, from
//! the frame that carried them.

use core::fmt;

use super::counters::bump;
use super::ingress::is_unconfirmed;
use super::output::{Cause, Current, Kind, Target};
use super::tasks::Task;
use super::{Group, Storage};
use crate::npdu::{Address, NetworkPriority, Npdu, GLOBAL_NETWORK};
use crate::table::Reachability;
use crate::{Instant, PortId};

/// Which of the router's addresses a request it originates names as its
/// source.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SourceSelection {
    /// The router's address on the port the request leaves by: no SNET or
    /// SADR, so the next router adds that port's network and the router's
    /// MAC there, as it does for any node on that network (Clause 6.5.4).
    /// The default.
    #[default]
    EgressPort,
    /// The router's address on this port's network: SNET and SADR naming
    /// it, whichever port the request leaves by, or none when it leaves by
    /// this port, where the link source already says so. For an
    /// application that answers to one address.
    Port(PortId),
}

/// A request the router's own application sends.
///
/// It is `#[non_exhaustive]`: build one with [`Request::new`] and set the
/// fields to change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Request<'a> {
    /// DNET and DADR: one node, or with an empty DADR every node on that
    /// network. Never 0 or 0xFFFF.
    pub destination: Address<'a>,
    /// The APDU.
    pub apdu: &'a [u8],
    /// Whether a reply is expected.
    pub expecting_reply: bool,
    /// The network priority.
    pub priority: NetworkPriority,
    /// Which address of the router it comes from.
    pub source: SourceSelection,
}

impl<'a> Request<'a> {
    /// `apdu` for `destination`, at Normal priority, expecting no reply,
    /// from the egress port's address.
    #[must_use]
    pub const fn new(destination: Address<'a>, apdu: &'a [u8]) -> Self {
        Self {
            destination,
            apdu,
            expecting_reply: false,
            priority: NetworkPriority::Normal,
            source: SourceSelection::EgressPort,
        }
    }
}

/// Why [`Router::originate`](super::Router::originate) sent nothing.
///
/// It is `#[non_exhaustive]`, so a new refusal isn't a breaking change: a
/// `match` needs a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OriginateError {
    /// DNET 0 or 0xFFFF: no network, or every one. Neither is routed
    /// here.
    ReservedNetwork,
    /// The router's own address: its application, not a wire.
    OwnAddress,
    /// A broadcast destination for an APDU other than an
    /// Unconfirmed-Request (Clause 6.3).
    BroadcastNotUnconfirmed,
    /// The source selection names a port the router doesn't have, or the
    /// route does.
    UnknownPort,
    /// The table has no route to the network. A search for one was
    /// started, unless one was made within the cooldown (Clause 6.6.3.1);
    /// poll for it.
    NoRoute,
    /// The route is marked busy by flow control (Clause 6.6.3.6).
    Busy,
    /// The route has failed for good (Clause 6.6.3.5).
    Unreachable,
    /// The DADR is a group address on its directly attached port that the
    /// router may not send this to ([`Group`]).
    GroupDestination,
    /// Longer than the egress port carries.
    TooLong {
        /// The NPDU's length.
        octets: usize,
        /// The port's limit for that kind of send.
        limit: usize,
    },
    /// Longer than the scratch, or with a field no NPDU may carry.
    Unwritable,
}

impl fmt::Display for OriginateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReservedNetwork => f.write_str("a request can't be for network 0 or 0xFFFF"),
            Self::OwnAddress => f.write_str("a request for the router's own address"),
            Self::BroadcastNotUnconfirmed => {
                f.write_str("only an Unconfirmed-Request may go to a broadcast address")
            }
            Self::UnknownPort => f.write_str("the router has no such port"),
            Self::NoRoute => f.write_str("no route to the network yet; searching for one"),
            Self::Busy => f.write_str("the route to the network is busy"),
            Self::Unreachable => f.write_str("the network is unreachable"),
            Self::GroupDestination => {
                f.write_str("the destination is a group address its port refuses")
            }
            Self::TooLong { octets, limit } => {
                write!(f, "a {octets}-octet NPDU over the egress port's {limit}")
            }
            Self::Unwritable => f.write_str("the request can't be written as an NPDU"),
        }
    }
}

impl core::error::Error for OriginateError {}

/// No port: a fan-out to "every port but this one" reaches every port.
const NO_PORT: PortId = PortId(u8::MAX);

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
    /// [`Router::originate`](super::Router::originate).
    pub(super) fn originate(
        &mut self,
        request: &Request<'_>,
        now: Instant,
        options: &super::Options,
    ) -> Result<(), OriginateError> {
        self.drop_undrained();
        let destination = request.destination;
        if matches!(destination.network, 0 | GLOBAL_NETWORK) {
            return Err(OriginateError::ReservedNetwork);
        }
        let broadcast = destination.mac.is_empty();
        if !broadcast && self.is_own(destination) {
            return Err(OriginateError::OwnAddress);
        }
        if broadcast && !is_unconfirmed(request.apdu) {
            return Err(OriginateError::BroadcastNotUnconfirmed);
        }
        let ports = self.port_count;
        let from = match request.source {
            SourceSelection::EgressPort => None,
            SourceSelection::Port(port) => match self.ports[..ports].get(usize::from(port.0)) {
                Some(_) => Some(port),
                None => return Err(OriginateError::UnknownPort),
            },
        };
        let dnet = destination.network;
        let Some(route) = self.table.lookup(dnet) else {
            if self
                .discovery
                .should_solicit(dnet, now, options.discovery_cooldown)
            {
                bump(&mut self.counters.solicitations);
                self.out.push(Task::solicit(NO_PORT, dnet));
            }
            return Err(OriginateError::NoRoute);
        };
        self.table.touch(dnet, now);
        match self.table.effective_reachability(dnet, now) {
            Some(Reachability::Busy) => return Err(OriginateError::Busy),
            Some(Reachability::Unreachable) => return Err(OriginateError::Unreachable),
            _ => {}
        }
        let Some(egress) = self.ports[..ports].get(usize::from(route.port().0)) else {
            return Err(OriginateError::UnknownPort);
        };
        let mut npdu = Npdu {
            expecting_reply: request.expecting_reply,
            priority: request.priority,
            destination: Some(destination),
            ..Npdu::apdu(request.apdu)
        };
        match self.group_of(&route, &npdu) {
            Group::No => {}
            // As a relay would: a group of the port's own takes an
            // Unconfirmed-Request as the port's broadcast.
            Group::Broadcast if is_unconfirmed(request.apdu) => {
                npdu.destination = Some(Address {
                    network: dnet,
                    mac: &[],
                });
            }
            Group::Broadcast | Group::Refuse => return Err(OriginateError::GroupDestination),
        }
        let source = from.filter(|&port| port != route.port()).map(|port| {
            let config = &self.ports[usize::from(port.0)];
            (config.network, config.mac)
        });
        npdu.source = source.as_ref().map(|(network, mac)| Address {
            network: *network,
            mac: mac.as_slice(),
        });
        let to = if route.is_direct() {
            let dadr = npdu.destination.map_or(&[][..], |address| address.mac);
            npdu.destination = None;
            dadr
        } else {
            route.next_hop().as_slice()
        };
        let limit = egress.max_npdu.get(to.is_empty());
        let octets = npdu.encoded_len();
        if octets > limit {
            bump(&mut self.counters.egress_too_long);
            return Err(OriginateError::TooLong { octets, limit });
        }
        match npdu.encode_into(&mut self.scratch) {
            Ok(len) => {
                let target = Target::Toward(route.port(), to);
                self.out.current =
                    Current::new(len, Kind::Originate, Cause::Local, request.priority, target);
                bump(&mut self.counters.originated);
                Ok(())
            }
            Err(_) => {
                bump(&mut self.counters.write_failures);
                Err(OriginateError::Unwritable)
            }
        }
    }
}
