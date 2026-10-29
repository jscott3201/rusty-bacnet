//! What the router asks of its caller during a call: the [`Host`] hooks.

use crate::PortId;
use crate::message::{Networks, RoutingEntries};
use crate::npdu::{NetworkMessageType, RejectMessageReason};
use crate::table::RejectEffect;

/// The caller's side of a [`receive`](super::Router::receive) or
/// [`on_deadline`](super::Router::on_deadline) call.
///
/// Every method has a default, so `impl Host for MyHost {}` gives a router
/// that admits every protected message and
/// reports nothing: what [`Permissive`] is.
///
/// The router calls these with its state borrowed, and a hosted router
/// calls them under the lock it keeps the router in: an implementation
/// must not block, and must never call into the router or take that lock.
pub trait Host {
    /// Whether to act on a protected network message (RB-09). The router
    /// asks after the message's data has been checked and before anything
    /// changes. `false` drops it silently: no table change, relay, answer
    /// or reject.
    fn authorize(&mut self, claim: &Claim<'_>) -> bool {
        let _ = claim;
        true
    }

    /// Something the hosted router logs today happened.
    fn observe(&mut self, event: Event) {
        let _ = event;
    }
}

/// A [`Host`] with every default: every protected message admitted,
/// nothing observed. Port group rules still apply. For firmware and tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Permissive;

impl Host for Permissive {}

/// The protected classes of network message, one per kind of change a
/// message can make to the routing table.
///
/// It is `#[non_exhaustive]`, so a new class isn't a breaking change: a
/// `match` needs a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ControlClass {
    /// I-Am-Router learning.
    IAm,
    /// Initialize-Routing-Table-Ack learning.
    InitAck,
    /// Initialize-Routing-Table updates and removals (not queries).
    InitMgmt,
    /// Router-Busy marks.
    Busy,
    /// Router-Available marks.
    Available,
    /// Changes a received reject makes.
    Reject,
    /// Routes stored from I-Could-Be-Router.
    ICouldBe,
}

/// The networks a protected message names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Targets<'c> {
    /// Networks by number, as I-Am-Router, Router-Busy and
    /// Router-Available carry them, or the one network a reject or an
    /// I-Could-Be names.
    Networks(Networks<'c>),
    /// The routing entries an Initialize-Routing-Table, or an Ack to one,
    /// carries.
    Entries(RoutingEntries<'c>),
}

impl<'c> Targets<'c> {
    /// The networks, in the order the message lists them.
    pub fn iter(&self) -> impl Iterator<Item = u16> + 'c + use<'c> {
        let (list, entries) = match *self {
            Self::Networks(list) => (Some(list.iter()), None),
            Self::Entries(entries) => (None, Some(entries.iter().map(|entry| entry.network))),
        };
        list.into_iter()
            .flatten()
            .chain(entries.into_iter().flatten())
    }

    /// Whether it names none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.iter().next().is_none()
    }
}

/// What [`Host::authorize`] is told about a protected message. It carries
/// what the message claims, none of it verified.
///
/// The router builds it. It is `#[non_exhaustive]`, so a new field isn't a
/// breaking change: a pattern needs `..`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Claim<'c> {
    /// The class of change it would make.
    pub class: ControlClass,
    /// The port it arrived on.
    pub port: PortId,
    /// That port's network number.
    pub port_network: u16,
    /// The link-layer address of the node that sent it.
    pub source: &'c [u8],
    /// Its message type.
    pub message_type: NetworkMessageType,
    /// Its vendor ID, for a proprietary type.
    pub vendor_id: Option<u16>,
    /// The SNET it claims, if any.
    pub snet: Option<u16>,
    /// The DNET it names, if any.
    pub dnet: Option<u16>,
    /// The networks it names.
    pub targets: Targets<'c>,
    /// A Router-Busy or Router-Available that lists no network: it covers
    /// every network the sender serves.
    pub targets_omitted: bool,
}

/// What [`Host::observe`] hears about: what the hosted router logs today
/// that the call's result and actions don't show.
///
/// It is `#[non_exhaustive]`, so a new event isn't a breaking change: a
/// `match` needs a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Event {
    /// A learning claim installed or refreshed a route.
    Learned {
        /// The network.
        network: u16,
        /// The port it is reached through.
        port: PortId,
        /// I-Am-Router or Initialize-Routing-Table-Ack.
        class: ControlClass,
    },
    /// A route moved port often enough to count as flapping.
    Flapping {
        /// The network.
        network: u16,
        /// Moves within 60 s of the one before, this one included.
        flaps: u8,
    },
    /// The route cap ended a message's learning or update, or turned an
    /// I-Could-Be-Router offer away.
    RouteCap {
        /// The class of the message.
        class: ControlClass,
    },
    /// A reject arrived and was applied to the table.
    RejectReceived {
        /// The network it names.
        network: u16,
        /// Its reason.
        reason: RejectMessageReason,
        /// What it did to the route.
        effect: RejectEffect,
    },
    /// Router-Busy or Router-Available covered a network.
    Marked {
        /// The network.
        network: u16,
        /// Busy, or available.
        busy: bool,
    },
    /// An Initialize-Routing-Table entry installed a route.
    UpdateApplied {
        /// The network.
        network: u16,
        /// The port it now goes through.
        port: PortId,
    },
    /// An Initialize-Routing-Table entry with Port ID 0 removed a route.
    UpdateRemoved {
        /// The network.
        network: u16,
    },
    /// An Initialize-Routing-Table entry named no port of this router, and
    /// was skipped.
    UnknownPortId {
        /// The network.
        network: u16,
        /// The Port ID it named.
        port_id: u8,
    },
    /// An I-Could-Be-Router arrived.
    ICouldBe {
        /// The network on offer.
        network: u16,
        /// The sender's rating of the connection.
        performance_index: u8,
        /// Whether it was stored: only where the table had no route, and
        /// was under the route cap.
        stored: bool,
    },
    /// An Establish-Connection arrived; this router opens no connections.
    EstablishConnection {
        /// The network.
        network: u16,
        /// The termination time it asked for.
        termination_time: u8,
    },
    /// A Disconnect-Connection arrived; with no connections, nothing is
    /// removed.
    DisconnectIgnored {
        /// The network.
        network: u16,
    },
    /// A message of the removed security types arrived; it is not
    /// rejected.
    SecurityMessage {
        /// Its type.
        message_type: NetworkMessageType,
    },
    /// A message of a type this router doesn't handle arrived for it; it
    /// draws reject reason 3.
    UnknownMessage {
        /// Its type.
        message_type: NetworkMessageType,
    },
    /// A Network-Number-Is reported a number other than the port's.
    NumberConflict {
        /// The port it arrived on.
        port: PortId,
        /// The port's configured number.
        local: u16,
        /// The number reported.
        reported: u16,
        /// Whether the sender's number was configured rather than learned.
        configured: bool,
    },
    /// A relay went nowhere because its hop count had run out. It comes
    /// for every such relay, including one whose frame was also delivered
    /// locally (a global broadcast, or a DLEN 0 broadcast on the arrival
    /// network), which [`Received`](super::Received) doesn't show.
    HopCountSpent {
        /// The network the relay was for: 0xFFFF for a global broadcast.
        network: u16,
    },
    /// An NPDU too long for an egress port's limit for its kind of send
    /// (fix R1). One routed toward a network drew reject reason 4 naming
    /// it; a fan-out leaves that port out when it is polled, where the skip
    /// is counted. Both count in
    /// [`Counters::egress_too_long`](super::Counters::egress_too_long).
    TooLong {
        /// The egress port.
        port: PortId,
        /// The NPDU's length, as rewritten for that port.
        octets: usize,
        /// The port's limit for that kind of send.
        limit: usize,
        /// The network the reject named; `None` for a fan-out.
        rejected: Option<u16>,
    },
    /// A refused NPDU named this router as its origin, so no reject went.
    OwnOrigin {
        /// The network the reject would have named.
        network: u16,
    },
    /// The sweep removed a learned route not confirmed for too long.
    Purged {
        /// The network.
        network: u16,
    },
    /// The sweep dropped a hardened-mode claim that waited too long.
    PendingExpired {
        /// The network.
        network: u16,
    },
    /// The sweep forgot a search for an unknown network.
    DiscoveryExpired {
        /// The network.
        network: u16,
    },
}
