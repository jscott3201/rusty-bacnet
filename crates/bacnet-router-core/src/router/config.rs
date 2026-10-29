//! How a router is set up: its ports and its options.

use core::fmt;

use super::{Congestion, Duplicates, GroupClassifier, Groups};

use crate::npdu::NpduLimits;
use crate::table::{Convergence, Timing};
use crate::{Duration, Mac, PortId};

/// The least NPDU limit a port may have, either way, in octets: room for
/// any reject the router passes on, so its own limit check never refuses
/// one.
pub const MIN_PORT_LIMIT: u16 = 64;

/// Whether a port may have `limits` on a router whose scratch holds
/// `scratch` octets: at least [`MIN_PORT_LIMIT`] and at most the scratch,
/// each way.
pub(super) fn limits_fit(limits: NpduLimits, scratch: usize) -> bool {
    [limits.unicast, limits.broadcast]
        .iter()
        .all(|&limit| limit >= MIN_PORT_LIMIT && usize::from(limit) <= scratch)
}

/// One port as the router sees it: data, with no link behind it. The
/// caller drives the link and carries out the router's sends on it.
///
/// Construct it with [`Self::new`] or the B/IP/MS/TP glue. The default
/// [`Groups`] rule is copyable; a custom [`GroupClassifier`] may own its
/// state. `#[non_exhaustive]` keeps future port facts out of caller literals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct PortConfig<G = Groups> {
    /// The network number, 1 to 65534: 0 is never a valid network number,
    /// and 0xFFFF, the global broadcast DNET, can't be an SNET (Clause
    /// 6.2.2.1), which the router puts in what it relays from the port.
    /// [`Router::new`](super::Router::new) refuses either.
    pub network: u16,
    /// The router's own MAC on the port.
    pub mac: Mac,
    /// The largest NPDU the port carries. A link whose limit changes while
    /// the router runs has it replaced with
    /// [`Router::set_port_max_npdu`](super::Router::set_port_max_npdu).
    pub max_npdu: NpduLimits,
    /// The link's destination classifier, supplied by its configuration.
    pub groups: G,
}

impl<G> PortConfig<G> {
    /// A port with its network, own MAC, limits and link group rule.
    #[must_use]
    pub const fn new(network: u16, mac: Mac, max_npdu: NpduLimits, groups: G) -> Self {
        Self {
            network,
            mac,
            max_npdu,
            groups,
        }
    }
}

impl<G: GroupClassifier> PortConfig<G> {
    /// An unused slot; all zero bits with the default group rule.
    pub(super) const ZERO: Self = Self {
        network: 0,
        mac: Mac::EMPTY,
        max_npdu: NpduLimits::both(0),
        groups: G::NONE,
    };
}

/// A router's policy values and timers.
///
/// It is `#[non_exhaustive]`, so a new option isn't a breaking change:
/// outside this crate, start from [`Options::STANDARD`] or
/// [`Options::HOSTED`] and set the fields to change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Options {
    /// How a claim that moves a route to another port is taken.
    pub convergence: Convergence,
    /// The most routes, direct ones included, that learning, management
    /// and I-Could-Be-Router offers may fill the table to, at most the
    /// table's capacity; `None` lets them fill the whole table.
    pub route_cap: Option<u16>,
    /// How long a learned route lives without confirmation.
    pub learned_age: Duration,
    /// How often the sweep runs.
    pub sweep: Duration,
    /// The table's timers: the busy period, the reject hold-down and the
    /// corroboration window.
    pub timing: Timing,
    /// The least time between two searches for one network.
    pub discovery_cooldown: Duration,
    /// How long a search is remembered.
    pub discovery_age: Duration,
    /// The router's own Router-Busy for a clogged egress port (#1697):
    /// `None`, the default, sends none.
    pub congestion: Option<Congestion>,
    /// Apply a received reject to a learned route only when it came from
    /// that route's next router (#1698). Off, the default, any sender's
    /// reject updates the route, as Clause 6.6.3.5 reads: it names no
    /// sender. Router-Busy and Router-Available are always scoped to the
    /// routes through their sender (Clauses 6.6.3.6, 6.6.3.7; RB-04). Either
    /// way, what a check leaves alone is counted in
    /// [`Counters::not_next_hop`](super::Counters::not_next_hop), and the
    /// message is passed on as before.
    pub next_hop_rejects: bool,
    /// Duplicate suppression for global broadcasts (#1698), for the
    /// network messages the router passes on with no origin, I-Am-Router,
    /// Router-Busy and Router-Available (#1810), and for the Who-Is-Router
    /// queries it passes on (#2020): `None`, the default, suppresses
    /// nothing. A global broadcast or Who-Is-Router naming the router
    /// itself as its source is dropped either way (#2021).
    ///
    /// Entries are keyed by origin, so in a looped site the router's own
    /// global broadcast sent with
    /// [`SourceSelection::EgressPort`](super::SourceSelection::EgressPort),
    /// a bare copy per port, is a different origin per port to the other
    /// routers, and may reach their applications once per port; send it
    /// with [`SourceSelection::Port`](super::SourceSelection::Port) there.
    pub duplicates: Option<Duplicates>,
}

impl Options {
    /// The standard values, for a router over storage of any size:
    /// standard convergence, no route cap of its own, so learning may fill
    /// whatever table the [`Storage`](super::Storage) holds, 300 s learned
    /// age, a 60 s sweep, [`Timing::DEFAULT`], and a 5 s discovery cooldown
    /// and 30 s age, with every opt-in policy off. Firmware sizes its
    /// table with the storage's `R` and starts here.
    pub const STANDARD: Self = Self {
        convergence: Convergence::Standard,
        route_cap: None,
        learned_age: Duration::from_secs(300),
        sweep: Duration::from_secs(60),
        timing: Timing::DEFAULT,
        discovery_cooldown: Duration::from_secs(5),
        discovery_age: Duration::from_secs(30),
        congestion: None,
        next_hop_rejects: false,
        duplicates: None,
    };

    /// The hosted router's values: [`STANDARD`](Self::STANDARD)'s, with
    /// `bacnet-network`'s cap of 256 routes. The cap is checked against the
    /// table: [`Router::new`](super::Router::new) refuses it over a table
    /// of fewer routes, where `STANDARD` takes the table's size.
    pub const HOSTED: Self = Self {
        route_cap: Some(256),
        ..Self::STANDARD
    };

    /// The route cap on a table of `capacity` routes.
    pub(super) const fn cap(&self, capacity: usize) -> usize {
        match self.route_cap {
            Some(cap) => cap as usize,
            None => capacity,
        }
    }
}

impl Default for Options {
    /// [`Options::STANDARD`].
    fn default() -> Self {
        Self::STANDARD
    }
}

/// Why [`Router::new`](super::Router::new) refused its configuration.
///
/// It is `#[non_exhaustive]`, so a new refusal isn't a breaking change: a
/// `match` needs a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConfigError {
    /// No port at all.
    NoPorts,
    /// More ports than the storage holds, or than 255, the most a wire
    /// Port ID can name.
    TooManyPorts {
        /// Ports offered.
        count: usize,
        /// Ports the storage holds.
        capacity: usize,
    },
    /// Two ports share a network number.
    DuplicateNetwork(u16),
    /// A port's network number is 0 or 0xFFFF, which no port can use
    /// ([`PortConfig::network`]).
    ReservedNetwork {
        /// The port.
        port: PortId,
        /// Its network number.
        network: u16,
    },
    /// A port's limit is under [`MIN_PORT_LIMIT`] or over the scratch.
    Limit(PortId),
    /// The route cap is over the table's capacity.
    RouteCap {
        /// The cap asked for.
        cap: u16,
        /// The routes the table holds.
        capacity: usize,
    },
    /// The table can't hold a route for every port.
    TableTooSmall,
    /// The sweep period is zero.
    ZeroSweep,
    /// [`Options::duplicates`] is on, but the storage records no
    /// broadcasts (its `B` is 0), so nothing would be suppressed.
    NoDuplicateRecord,
    /// [`Options::congestion`] is a policy
    /// [`Congestion::is_valid`](super::Congestion::is_valid) refuses.
    Congestion,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoPorts => f.write_str("a router needs at least one port"),
            Self::TooManyPorts { count, capacity } => {
                write!(f, "{count} ports offered, room for {capacity}")
            }
            Self::DuplicateNetwork(network) => {
                write!(f, "duplicate network number {network} in router ports")
            }
            Self::ReservedNetwork { port, network } => write!(
                f,
                "port {} is on network {network}, which no port can use: 1 to 65534 can",
                port.0
            ),
            Self::Limit(port) => write!(
                f,
                "port {} has an NPDU limit under 64 octets or over the scratch",
                port.0
            ),
            Self::RouteCap { cap, capacity } => {
                write!(f, "a route cap of {cap} is over the table's {capacity}")
            }
            Self::TableTooSmall => f.write_str("the table can't hold every port's network"),
            Self::ZeroSweep => f.write_str("the sweep period is zero"),
            Self::NoDuplicateRecord => {
                f.write_str("duplicate suppression is on, but the storage records no broadcasts")
            }
            Self::Congestion => f.write_str(
                "a congestion policy needs a nonzero clog time and repeat, \
                 and a low-water mark of 1 to 99 percent",
            ),
        }
    }
}

impl core::error::Error for ConfigError {}

/// Why [`Router::set_port_max_npdu`](super::Router::set_port_max_npdu)
/// refused new limits. It changed nothing.
///
/// It is `#[non_exhaustive]`, so a new refusal isn't a breaking change: a
/// `match` needs a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LimitError {
    /// The router has no such port.
    UnknownPort,
    /// A limit is under [`MIN_PORT_LIMIT`] or over the scratch, which
    /// [`Router::new`](super::Router::new) refuses too.
    OutOfRange,
}

impl fmt::Display for LimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnknownPort => "the router has no such port",
            Self::OutOfRange => "an NPDU limit under 64 octets or over the scratch",
        })
    }
}

impl core::error::Error for LimitError {}
