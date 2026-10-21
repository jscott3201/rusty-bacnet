//! The IP subnet a node is on.

use core::net::{Ipv4Addr, SocketAddrV4};

/// A node's own IP subnet as its B/IP network uses it: the broadcast
/// address that reaches the network's devices there and, when the caller
/// knows it, the subnet mask.
///
/// A BBMD passes it to the decisions that can send to its subnet,
/// [`Tables::forwarded_npdu`](crate::bbmd::Tables::forwarded_npdu) and
/// [`Tables::distribute_broadcast`](crate::bbmd::Tables::distribute_broadcast).
///
/// It is `#[non_exhaustive]`, so a new detail of the subnet isn't a
/// breaking change: outside this crate, build it with
/// [`LocalSubnet::new`] and set [`mask`](Self::mask) when it is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct LocalSubnet {
    /// Where a broadcast to the network's devices goes: the subnet's
    /// directed broadcast address (or 255.255.255.255) at the B/IP
    /// network's UDP port (Annex J.1.2). For a B/IP-M node it is the
    /// multicast group and its port instead (Annex J.8.1): the group acts
    /// as the node's subnet, and broadcasts go to it and come from it
    /// alone ([`Addressed::classify`](crate::node::Addressed::classify)).
    pub broadcast: SocketAddrV4,
    /// The mask of the subnet the node's own B/IP address is on, when the
    /// caller knows it. With it, a BBMD doesn't broadcast a peer's
    /// Forwarded-NPDU again when the peer sent it from this subnet at the
    /// BBMD's own port: the network's devices there heard it first hand.
    /// Leave it `None` when it isn't known, or when the own address isn't
    /// on the subnet, as behind NAT; such a forward is then broadcast. A
    /// BBMD that serves a multicast group doesn't use it: a peer's own
    /// broadcasts reach its IP subnet, not the group.
    pub mask: Option<Ipv4Addr>,
    /// Whether the node is the only device of its B/IP network on the
    /// subnet, as a BBMD behind a NAT router often is (Annex J.7.5). A BBMD
    /// then skips the local broadcast of a peer's Forwarded-NPDU, as Annex
    /// J.4.5 permits (leaving how the BBMD knows to the installation), and,
    /// extending that permission, of a foreign device's
    /// Distribute-Broadcast-To-Network: nothing there would receive either,
    /// and the BBMD's own application still gets the message. `false` unless the caller sets it; set it only when
    /// no other device of the network shares the subnet, or they stop
    /// receiving remote broadcasts.
    pub alone: bool,
    /// For a B/IP-M node, whose [`broadcast`](Self::broadcast) is its
    /// multicast group, the IP subnet's directed broadcast address, when
    /// the caller knows it. The node neither sends there nor takes
    /// broadcasts from there, but the address still reaches more than one
    /// node, so a datagram from it, or a Forwarded-NPDU naming it as the
    /// origin, is dropped ([`Groups`](crate::node::Groups)). `None` for a
    /// plain node, whose broadcast address already is it.
    pub directed: Option<Ipv4Addr>,
}

impl LocalSubnet {
    /// The subnet whose broadcasts go to `broadcast`, its mask not known,
    /// shared with other devices.
    #[must_use]
    pub const fn new(broadcast: SocketAddrV4) -> Self {
        Self {
            broadcast,
            mask: None,
            alone: false,
            directed: None,
        }
    }

    /// A B/IP-M node's subnet (Annex J.8.1): broadcasts go to `group`, a
    /// multicast address at the network's port, and `directed`, the IP
    /// subnet's own broadcast address when known, stays a group address.
    /// A BBMD that serves the group (Annex J.8.3) has the same subnet.
    #[must_use]
    pub const fn member(group: SocketAddrV4, directed: Option<Ipv4Addr>) -> Self {
        Self {
            broadcast: group,
            mask: None,
            alone: false,
            directed,
        }
    }

    /// Whether the broadcast address is a multicast group: the subnet of a
    /// B/IP-M node, or of the BBMD that serves its group (Annex J.8).
    #[must_use]
    pub const fn is_group(&self) -> bool {
        self.broadcast.ip().is_multicast()
    }
}
