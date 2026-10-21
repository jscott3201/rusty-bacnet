//! What a datagram's addresses mean to this node: where it was sent, whether
//! its sender is a group address, and whether a reply could reach it.

use core::net::{Ipv4Addr, SocketAddrV4};

use bacnet_bip_codec::BvlcFunction;

use super::config::Link;
use crate::Arrival;

/// What a datagram's destination address was, for this node. A host
/// builds it with [`Addressed::classify`]. Both can hold at once, as where
/// a loopback test's broadcast address is its own; neither, for a
/// destination that is neither (another host's unicast, a multicast group).
///
/// It is a plain record, and closed on purpose: the receive rules ask a
/// destination two questions, whether it was this node alone and whether
/// it reached every node on the subnet (Annex J.2.11, J.2.12, J.4.1). Any
/// other kind of destination answers the same two: for a B/IP-M node
/// (Annex J.8, #941) the group address stands in for the broadcast one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Addressed {
    /// Sent to this node's own unicast address.
    pub node: bool,
    /// Sent to the subnet's broadcast address or 255.255.255.255, or for a
    /// B/IP-M node to its multicast group.
    pub broadcast: bool,
}

/// The addresses a destination is judged against.
///
/// It is `#[non_exhaustive]`, so a new kind of address isn't a breaking
/// change: outside this crate, build it with [`LocalAddresses::new`] and
/// set [`wildcard`](Self::wildcard) for a socket bound to the wildcard
/// address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct LocalAddresses<'a> {
    /// The IP address of the node's own B/IP address.
    pub own: Ipv4Addr,
    /// The subnet's broadcast IP address, or a B/IP-M node's multicast
    /// group (Annex J.8): the link's [`LocalSubnet::broadcast`](crate::LocalSubnet::broadcast).
    /// A group here makes the group the only broadcast destination: see
    /// [`Addressed::classify`].
    pub broadcast: Ipv4Addr,
    /// For a socket bound to the wildcard address, the host's own unicast
    /// addresses: unicast to any of them is the node's. `None` for a socket
    /// bound to `own`.
    pub wildcard: Option<&'a [Ipv4Addr]>,
}

impl LocalAddresses<'_> {
    /// The addresses of a socket bound to `own`, on the subnet whose
    /// broadcast IP is `broadcast`.
    #[must_use]
    pub const fn new(own: Ipv4Addr, broadcast: Ipv4Addr) -> Self {
        Self {
            own,
            broadcast,
            wildcard: None,
        }
    }
}

impl Addressed {
    /// Classify a datagram by its destination IP address and, when the host
    /// reports it, whether the OS delivered it to a group (`os_group`). The
    /// OS's word overrides the address: a group delivery is never unicast
    /// to the node, and a reported unicast never a broadcast.
    ///
    /// When the broadcast address is a multicast group, the node is a
    /// B/IP-M member (Annex J.8): the group is its only broadcast
    /// destination, and 255.255.255.255 or a subnet's directed broadcast
    /// is neither broadcast nor unicast to it. Members and plain B/IP
    /// devices don't share a subnet (Annex J.8.1), so a subnet broadcast
    /// isn't this network's, and taking it as well could deliver one
    /// broadcast twice.
    #[must_use]
    pub fn classify(
        destination: Ipv4Addr,
        os_group: Option<bool>,
        local: &LocalAddresses<'_>,
    ) -> Self {
        let to_broadcast = destination == local.broadcast
            || (destination.is_broadcast() && !local.broadcast.is_multicast());
        let node = match local.wildcard {
            Some(own_ips) => {
                !to_broadcast && !destination.is_multicast() && own_ips.contains(&destination)
            }
            None => destination == local.own,
        } && os_group != Some(true);
        Self {
            node,
            broadcast: to_broadcast && os_group != Some(false),
        }
    }

    /// How a message of `function` arrived, or `None` when the function
    /// doesn't fit the destination (Annex J.2.11, J.2.12, J.3, J.4.1).
    ///
    /// An Original-Unicast-NPDU must be sent to the node, an
    /// Original-Broadcast-NPDU to the broadcast address, and a
    /// Forwarded-NPDU to either (a broadcast when both fit). Every other
    /// function is point-to-point, so only to the node.
    pub(crate) fn arrival(self, function: BvlcFunction) -> Option<Arrival> {
        let unicast = self.node.then_some(Arrival::Unicast);
        let broadcast = self.broadcast.then_some(Arrival::Broadcast);
        match function {
            BvlcFunction::ORIGINAL_UNICAST_NPDU => unicast,
            BvlcFunction::ORIGINAL_BROADCAST_NPDU => broadcast,
            BvlcFunction::FORWARDED_NPDU => broadcast.or(unicast),
            _ => unicast,
        }
    }
}

/// A link's group destinations: the B/IP addresses that reach more than one
/// node. No node sends from one, so a datagram from one, or a
/// Forwarded-NPDU naming one as its origin, is dropped: answering it would
/// reach the whole group.
///
/// It is `#[non_exhaustive]`, so a link that comes to have another group
/// destination of its own (a second broadcast address, say) isn't a
/// breaking change: outside this crate, take it from [`Link::groups`] or
/// build it with [`Groups::new`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Groups {
    /// The link's broadcast address: its broadcast IP at the network's port.
    pub broadcast: SocketAddrV4,
    /// The node's own interface IP. A broadcast IP equal to it, or a
    /// loopback one, as loopback tests set up, is a group only at the
    /// network's port.
    pub interface: Ipv4Addr,
    /// For a B/IP-M node, its IP subnet's directed broadcast address
    /// ([`LocalSubnet::directed`](crate::LocalSubnet::directed)): no
    /// longer where its broadcasts go, but still an address that reaches
    /// more than one node. It counts as the broadcast IP does, at any port.
    /// 0.0.0.0 for none: a plain address rather than an `Option`, so a
    /// router's empty group rule stays all zero bits and its storage stays
    /// in `.bss`.
    pub directed: Ipv4Addr,
}

impl Groups {
    /// The group destinations of a link whose broadcast address is
    /// `broadcast`, on the interface whose IP is `interface`, with no
    /// directed broadcast beside it.
    #[must_use]
    pub const fn new(broadcast: SocketAddrV4, interface: Ipv4Addr) -> Self {
        Self {
            broadcast,
            interface,
            directed: Ipv4Addr::UNSPECIFIED,
        }
    }

    /// Whether `address` is a group destination: the link's broadcast
    /// address, or any address whose IP is a group IP ([`Groups::contains_ip`]).
    #[must_use]
    pub const fn contains(&self, address: SocketAddrV4) -> bool {
        let broadcast = &self.broadcast;
        (address.ip().to_bits() == broadcast.ip().to_bits() && address.port() == broadcast.port())
            || self.contains_ip(*address.ip())
    }

    /// Whether `ip` is a group at any port: 255.255.255.255, a multicast
    /// address, or the broadcast IP or a member's directed broadcast when
    /// it is neither loopback nor the node's own interface. This is the
    /// test for a datagram's UDP source.
    #[must_use]
    pub const fn contains_ip(&self, ip: Ipv4Addr) -> bool {
        let ip_bits = ip.to_bits();
        let directed = !self.directed.is_unspecified() && self.directed.to_bits() == ip_bits;
        let subnet = ip_bits == self.broadcast.ip().to_bits() || directed;
        ip.is_broadcast()
            || ip.is_multicast()
            || (subnet && !ip.is_loopback() && ip_bits != self.interface.to_bits())
    }
}

impl Link {
    /// The link's group destinations, with the IP of the node's own B/IP
    /// address as its interface.
    ///
    /// A host bound to the wildcard address may judge a destination by its
    /// bind address (0.0.0.0) instead, as the Tokio transport's
    /// `is_group_destination` does. The two differ only where the configured
    /// broadcast IP is the node's own IP, a misconfiguration: this one then
    /// takes a sender from that IP, which the own-echo rule drops at the
    /// node's port.
    #[must_use]
    pub const fn groups(&self) -> Groups {
        Groups {
            broadcast: self.subnet.broadcast,
            interface: *self.address.ip(),
            directed: match self.subnet.directed {
                Some(directed) => directed,
                None => Ipv4Addr::UNSPECIFIED,
            },
        }
    }

    /// Whether `address` is this node's own B/IP address.
    ///
    /// The one test for the node's own echo, so an address the node is
    /// also reached at (a NAT router's global address) can join it here.
    #[must_use]
    pub fn is_own(&self, address: SocketAddrV4) -> bool {
        address == self.address
    }

    /// Whether a reply to `source` could reach a peer: not the unspecified
    /// address, not port 0, not a group address, and not this node.
    #[must_use]
    pub fn is_answerable(&self, source: SocketAddrV4) -> bool {
        !source.ip().is_unspecified()
            && source.port() != 0
            && !self.groups().contains_ip(*source.ip())
            && !self.is_own(source)
    }
}
