//! Which B/IP destinations reach a group of nodes (#1479), and the counts of
//! datagrams the receive path drops for naming one as their sender: a UDP
//! source (#1504) or a Forwarded-NPDU's originating address (#1493).

use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use bacnet_bip_codec::address_from_slice;
use bacnet_bip_core::node::Groups;

use super::BipTransport;

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
    /// The groups of a link bound to `interface` at `port` whose configured
    /// broadcast IP is `broadcast`.
    pub(super) fn new(broadcast: Ipv4Addr, port: u16, interface: Ipv4Addr) -> Self {
        Self(Groups::new(SocketAddrV4::new(broadcast, port), interface))
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
        BipGroups::new(self.broadcast_address, self.port, self.interface)
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
