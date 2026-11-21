//! Which B/IP destinations reach a group of nodes (#1479), and the counts of
//! datagrams the receive path drops for naming one as their sender: a UDP
//! source (#1504) or a Forwarded-NPDU's originating address (#1493).

use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bacnet_bip_codec::address_from_slice;
use bacnet_bip_core::LocalSubnet;
use bacnet_bip_core::node::Groups;

use super::BipTransport;

/// Whether `mac` names one B/IP station by unicast: a six-octet IPv4
/// address and nonzero UDP port that is not 0.0.0.0, a multicast group, the
/// limited broadcast, or the subnet broadcast of a local interface whose
/// subnet holds it (at any UDP port). It says nothing about what the station
/// is. For an off-link peer the subnet mask is unknown, so its address is
/// taken as one station. The interface lookup asks the operating system and
/// sends nothing; with overlapping local subnets, the first interface whose
/// subnet holds the address decides. This transport's own configured broadcast is
/// [`TransportPort::is_group_destination`](crate::port::TransportPort::is_group_destination)'s
/// to refuse.
pub async fn is_unicast_station(mac: &[u8]) -> std::io::Result<bool> {
    let Ok(address) = address_from_slice(mac) else {
        return Ok(false);
    };
    let ip = *address.ip();
    if address.port() == 0 || ip.is_unspecified() || ip.is_multicast() || ip.is_broadcast() {
        return Ok(false);
    }
    // A subnet with a broadcast has at least two host bits, all ones in
    // the broadcast address. Other addresses need no interface lookup.
    if ip.to_bits() & 3 != 3 {
        return Ok(true);
    }
    tokio::task::spawn_blocking(move || {
        let interface = crate::local_addresses::interface_of(ip)?;
        Ok(interface.and_then(|local| local.subnet_broadcast(ip)) != Some(ip))
    })
    .await?
}

/// A B/IP link's group destinations, as [`TransportPort::is_group_destination`](crate::port::TransportPort::is_group_destination)
/// reads them (#1479): this link's broadcast MAC, the limited broadcast
/// 255.255.255.255 or the configured broadcast IP at any UDP port, and any
/// IPv4 multicast address (224.0.0.0/4). The socket may send to each, since
/// it sets SO_BROADCAST, and each reaches more than one node. A configured
/// broadcast IP that is a loopback address or this link's own interface,
/// as loopback tests set it up, counts only at this link's port, as
/// [`TransportPort::is_broadcast_mac`](crate::port::TransportPort::is_broadcast_mac) has it.
///
/// It is the node engine's [`Groups`] with the configured interface, which
/// on a wildcard bind is 0.0.0.0; the receive path judges senders by the
/// node's own address instead (`Link::groups`).
#[derive(Debug, Clone, Copy)]
pub(super) struct BipGroups(Groups);

impl BipGroups {
    /// The groups of a link bound to `interface` whose broadcasts go to
    /// `subnet`: for a B/IP-M node its group, with the configured broadcast
    /// IP as the directed broadcast beside it, as its node's link has them.
    pub(super) fn new(subnet: LocalSubnet, interface: Ipv4Addr) -> Self {
        let mut groups = Groups::new(subnet.broadcast, interface);
        groups.directed = subnet.directed.unwrap_or(Ipv4Addr::UNSPECIFIED);
        Self(groups)
    }

    pub(super) fn contains(self, mac: &[u8]) -> bool {
        address_from_slice(mac).is_ok_and(|address| self.0.contains(address))
    }
}

/// The datagrams the receive path dropped for a group sender, shared with
/// the transport, whose totals survive a restart.
#[derive(Clone, Debug, Default)]
pub(super) struct GroupDrops {
    /// See [`BipTransport::group_source_drops`].
    pub(super) sources: Arc<AtomicU64>,
    /// See [`BipTransport::forwarded_group_origin_drops`].
    pub(super) origins: Arc<AtomicU64>,
}

impl GroupDrops {
    pub(super) fn source(&self) {
        self.sources.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn origin(&self) {
        self.origins.fetch_add(1, Ordering::Relaxed);
    }
}

impl BipTransport {
    pub(super) fn groups(&self) -> BipGroups {
        let subnet = SocketAddrV4::new(self.broadcast_address, self.port);
        let subnet = super::multicast::subnet(self.multicast.config(), subnet);
        BipGroups::new(subnet, self.interface)
    }

    /// Datagrams dropped since this transport was created because their UDP
    /// source address is a group address of this link: the limited
    /// broadcast, a multicast address, or the configured broadcast IP
    /// (#1504). No node sends from one, and an answer to it would reach
    /// every node in the group, so the datagram is dropped before any BVLC
    /// function is handled: it never reaches the network layer, and a BBMD
    /// neither registers nor forwards from it. Linux already discards most
    /// such datagrams; other systems may not. The total survives a restart.
    pub fn group_source_drops(&self) -> u64 {
        self.group_drops.sources.load(Ordering::Relaxed)
    }

    /// Forwarded-NPDUs dropped since this transport was created because
    /// their originating address is one of this link's group destinations
    /// ([`TransportPort::is_group_destination`](crate::port::TransportPort::is_group_destination)),
    /// such as the limited broadcast or a multicast address (#1493). No node
    /// sends from one, so such a frame is malformed: it never reaches the
    /// network layer, and a BBMD forwards it nowhere. The total survives a
    /// restart.
    pub fn forwarded_group_origin_drops(&self) -> u64 {
        self.group_drops.origins.load(Ordering::Relaxed)
    }
}
