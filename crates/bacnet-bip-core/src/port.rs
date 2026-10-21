//! The B/IP port driver: a node on one socket, with everything its
//! [`Node`](crate::node::Node) asks of a host carried out.
//!
//! [`node`](crate::node) decides and [`bbmd`](crate::bbmd) keeps the
//! tables; a port joins them to a socket, one type per mode, since a link
//! is never both a BBMD and a foreign device:
//!
//! - [`NodePort`]: a node in normal or foreign-device mode. It sends the
//!   NAKs a node owes, a foreign device's registrations, and the
//!   broadcasts it holds until the first registration attempt resolves
//!   (Annex J.7.4; decision D17), in [`HeldBroadcast`] slots the caller
//!   lends.
//! - [`BbmdPort`]: a BBMD's link over [`Tables`](crate::bbmd::Tables) the
//!   caller lends. It forwards each broadcast where the tables say (Annex
//!   J.4.5), and answers the management requests from them
//!   ([`Tables::serve`](crate::bbmd::Tables::serve)), so the caller never
//!   sees one. A BBMD registers nowhere and holds nothing, so it needs no
//!   node.
//!
//! Every datagram leaves through the caller's [`Datagrams`], which encodes
//! the message straight into its socket's buffer, so a port keeps no
//! datagram buffer of its own. What a received datagram brought comes
//! back as an [`Ingress`]: an NPDU for the network layer, or what the port
//! did instead.
//!
//! Wiring a port to a device or a router:
//!
//! - **Start:** call [`NodePort::start`] once, before anything else: a
//!   foreign device's first registration is due at once, and its response
//!   timeout is the port's first deadline. A [`BbmdPort`] registers
//!   nowhere, and needs no start.
//! - **Receive:** hand each datagram to the port's `receive`, and each
//!   [`Ingress::Npdu`] to the network layer. Its source is the B/IP address
//!   the node delivered from, which for a Forwarded-NPDU is the origin
//!   (Annex J.2.5).
//! - **Send:** `unicast` an NPDU to one node, which refuses a group
//!   address (#1677), or `broadcast` it: held while a foreign device's
//!   first registration is outstanding, and also forwarded by a BBMD.
//! - **Timers:** sleep until the earliest of a [`NodePort`]'s
//!   `next_deadline` and anything else the device runs, and call its
//!   `on_deadline`. A [`BbmdPort`] has no timers of its own: lapsed
//!   registrations are left out of every decision, and purged at the next
//!   registration or through `tables_mut`.
//!
//! ```
//! use core::net::{Ipv4Addr, SocketAddrV4};
//! use bacnet_bip_core::codec::Bvll;
//! use bacnet_bip_core::node::{Addressed, Datagram, Link, Mode, Node, Permissive, SendRefusal};
//! use bacnet_bip_core::port::{Datagrams, Ingress, NodePort};
//! use bacnet_bip_core::{Instant, LocalSubnet};
//!
//! /// A socket that keeps the last datagram it sent.
//! struct Socket([u8; 64], usize);
//!
//! impl Datagrams for Socket {
//!     fn send(&mut self, _to: SocketAddrV4, message: &Bvll<'_>) -> Result<(), SendRefusal> {
//!         self.1 = message.encode_into(&mut self.0).map_err(|_| SendRefusal::SocketError)?;
//!         Ok(())
//!     }
//! }
//!
//! let own = SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 10), 47808);
//! let subnet = LocalSubnet::new(SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 255), 47808));
//! let node: Node = Node::new(Link::new(own, subnet, Mode::Normal)?, Instant::ZERO)?;
//! // A normal node holds no broadcasts, so it needs no slots.
//! let mut port: NodePort<'_> = NodePort::new(node, &mut [])?;
//! let mut socket = Socket([0; 64], 0);
//!
//! // A peer's Read-Broadcast-Distribution-Table draws its NAK.
//! let mut wire = [0u8; 4];
//! Bvll::ReadBroadcastDistributionTable.encode_into(&mut wire)?;
//! let peer = SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 20), 47808);
//! let to = Addressed { node: true, broadcast: false };
//! let ingress = port.receive(&Datagram::new(&wire, peer, Some(to)), Instant::ZERO, &mut Permissive, &mut socket);
//! assert_eq!(ingress, Ingress::Handled);
//! assert_eq!(&socket.0[..socket.1], [0x81, 0x00, 0x00, 0x06, 0x00, 0x20]);
//!
//! // A broadcast goes to the subnet's broadcast address.
//! port.broadcast(&[0x01, 0x00, 0x10, 0x08], Instant::ZERO, &mut socket)?;
//! assert_eq!(&socket.0[..2], [0x81, 0x0B]);
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```
//!
//! # Local rules
//!
//! A datagram the node drops, a malformed one included, goes no further.
//! On a foreign device's port, a refused first registration drops the held
//! broadcasts, a registration the socket refuses ends its attempt, and a
//! full hold refuses the next broadcast as [`SendRefusal::HoldFull`]. Each
//! datagram of a port's own that its socket refuses (a NAK, a released
//! broadcast, a BBMD's forwarded copy or management answer) is counted
//! once in its [`PortCounters`]; a refused registration is the node's
//! [`local_errors`](crate::node::ExchangeCounters::local_errors). A BBMD
//! serves no management request from a source a reply can't reach
//! ([`Link::is_answerable`](crate::node::Link::is_answerable)). Tests:
//! `tests/port_node.rs` and `tests/port_bbmd.rs`.

mod bbmd;
mod node;

use core::fmt;
use core::net::SocketAddrV4;

use bacnet_bip_codec::Bvll;

use crate::bbmd::{BdtError, ForwardRefusal, Served};
use bacnet_npdu::MAX_NPDU;

use crate::node::{ConfigError, Delivery, Discard, SendRefusal};

pub use bbmd::BbmdPort;
pub use node::NodePort;

/// Where a port's datagrams go: the host's UDP socket.
pub trait Datagrams {
    /// Send `message` to `to` from the port's own socket.
    ///
    /// Encode it with [`Bvll::encode_into`] or [`Bvll::encode_to_sink`]
    /// straight into the socket's buffer, sized by [`Bvll::encoded_len`],
    /// and keep no borrow of it. Refuse it with [`SendRefusal::SocketFull`]
    /// when the socket can't take it now, and with
    /// [`SendRefusal::SocketError`] when it can't be encoded there
    /// (`BufferExhausted`, say) or the send fails.
    fn send(&mut self, to: SocketAddrV4, message: &Bvll<'_>) -> Result<(), SendRefusal>;
}

/// What a received datagram brought.
///
/// It is exhaustive, as the node's [`Inbound`](crate::node::Inbound) is:
/// the caller serves each case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ingress<'d> {
    /// An NPDU for the network layer, from its source B/IP address, and
    /// whether it reached more than this node.
    Npdu(Delivery<'d>),
    /// Nothing for the network layer: the port answered it with a NAK, or
    /// it answered one of the node's own requests.
    Handled,
    /// The node dropped it, for this reason.
    Discarded(Discard),
    /// A BBMD's tables refused a Forwarded-NPDU: [`BbmdPort`] only.
    NotForwarded(ForwardRefusal),
    /// A request for a BBMD's tables, which the port answered: what it came
    /// to. [`BbmdPort`] only.
    Served(Served),
}

/// Why a port refused its node and storage.
///
/// It is `#[non_exhaustive]`, so a new refusal isn't a breaking change: a
/// `match` needs a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PortError {
    /// A [`NodePort`] was offered a BBMD's node, or a [`BbmdPort`] another
    /// mode's link.
    Mode,
    /// A [`BbmdPort`]'s link failed [`Link::validate`](crate::node::Link::validate).
    Link(ConfigError),
    /// A foreign device holds more broadcasts than the port has slots for.
    HeldStorage {
        /// The broadcasts the node holds ([`Foreign::hold`](crate::node::Foreign::hold)).
        needed: usize,
        /// The slots lent.
        available: usize,
    },
    /// A foreign device holds broadcasts longer than a slot: its link's
    /// broadcast limit ([`Link::max_npdu`](crate::node::Link::max_npdu) for
    /// [`Cast::Broadcast`](crate::node::Cast::Broadcast)) is over the
    /// slots' `N`. A link with no send budget has no limit short of the
    /// largest BVLL message.
    HeldLength {
        /// The longest broadcast the link sends, in octets.
        needed: usize,
        /// A slot's length, `N`.
        available: usize,
    },
    /// A [`BbmdPort`]'s target storage is shorter than
    /// [`Tables::MAX_TARGETS`](crate::bbmd::Tables::MAX_TARGETS).
    TargetStorage {
        /// [`Tables::MAX_TARGETS`](crate::bbmd::Tables::MAX_TARGETS).
        needed: usize,
        /// The targets lent.
        available: usize,
    },
    /// A [`BbmdPort`]'s row storage is shorter than its foreign device
    /// table, `F`.
    RowStorage {
        /// `F`.
        needed: usize,
        /// The rows lent.
        available: usize,
    },
    /// A [`BbmdPort`] whose link serves a B/IP-M group was lent tables
    /// whose own BDT row isn't two-hop: the error is always
    /// [`BdtError::OneHopForGroup`].
    Bdt(BdtError),
}

impl fmt::Display for PortError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mode => f.write_str("the link's mode doesn't suit this port"),
            Self::Link(why) => write!(f, "invalid link: {why}"),
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
            Self::RowStorage { needed, available } => write!(
                f,
                "a BBMD reports up to {needed} registrations, with rows for {available}"
            ),
            Self::Bdt(why) => write!(f, "invalid BDT: {why}"),
        }
    }
}

impl core::error::Error for PortError {}

/// What a port counted since its counters were last taken. Its node keeps
/// counts of its own ([`Node::take_counters`](crate::node::Node::take_counters)).
///
/// It is `#[non_exhaustive]`, so a new count isn't a breaking change.
/// Outside this crate, build one from `PortCounters::default()` and set its
/// fields.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct PortCounters {
    /// Datagrams of the port's own that the socket refused: a NAK, a
    /// released broadcast, or a BBMD's forwarded copy or management answer.
    /// A refused registration is the node's
    /// [`local_errors`](crate::node::ExchangeCounters::local_errors).
    pub refused: u32,
}

/// Room for one broadcast that a foreign device's port holds until its
/// first registration attempt resolves (Annex J.7.4; decision D17). It is
/// all zero bits, so firmware lends the port a static's.
///
/// A slot holds `N` octets, at most 65,535: by default [`MAX_NPDU`], B/IP's
/// largest NPDU. A link with a smaller send budget sends shorter
/// broadcasts, and its port can hold them in shorter slots;
/// [`NodePort::new`] checks that the link's broadcast limit fits `N`.
///
/// A default doesn't drive inference, so `HeldBroadcast::EMPTY` alone, in
/// an expression, leaves `N` open. Name the type instead: `<HeldBroadcast>`
/// is a type, so it takes the default.
///
/// ```
/// use bacnet_bip_core::port::HeldBroadcast;
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

/// Send each `(to, message)`, returning how many the socket refused.
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
