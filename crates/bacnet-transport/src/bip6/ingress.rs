//! BACnet/IPv6 ingress provenance checks.

use std::net::{IpAddr, Ipv6Addr, SocketAddr, SocketAddrV6};

use super::receive_counters::DropReason;
use super::{
    Bip6Vmac, Bvlc6Function, Bvll6, BACNET_IPV6_MULTICAST_LINK_LOCAL,
    BACNET_IPV6_MULTICAST_ORG_LOCAL, BACNET_IPV6_MULTICAST_SITE_LOCAL,
};

/// Whether a frame whose sender's VMAC is `source`, from `from`, is this
/// node's own: [`DropReason::OwnEcho`] when it comes from this node's own
/// address `local` (its own broadcast looped back, say), and
/// [`DropReason::OwnVmacSource`] when another address uses this node's
/// VMAC `vmac`. The network layer must never see either as a frame from a
/// peer, and the neighbour table never learns this node's VMAC.
pub(super) fn own_frame(
    source: Bip6Vmac,
    from: SocketAddrV6,
    local: SocketAddrV6,
    vmac: Bip6Vmac,
) -> Option<DropReason> {
    if from.ip() == local.ip() && from.port() == local.port() {
        Some(DropReason::OwnEcho)
    } else if source == vmac {
        Some(DropReason::OwnVmacSource)
    } else {
        None
    }
}

/// Why a datagram from `peer` is dropped for its source, before anything
/// learns from it, if it is:
///
/// - [`own_frame`] for an Original-Unicast-NPDU or Original-Broadcast-NPDU
///   from this node's own address or VMAC, or an Address-Resolution-ACK or
///   Virtual-Address-Resolution-ACK from its VMAC, which only another node
///   with the same VMAC would send;
/// - [`DropReason::UnanswerableSource`] for any other Original-Unicast-NPDU
///   or Original-Broadcast-NPDU from a source [`is_answerable_source`]
///   refuses. The table never learns that source, so the NPDU's source MAC
///   would name a VMAC this node can't reach at the address it came from.
///
/// A Forwarded-NPDU's origin is judged where it is decoded.
/// Address-Resolution and Virtual-Address-Resolution pass: another node
/// probing for this node's VMAC sends an Address-Resolution from it, and the
/// answer is how that node finds the collision.
pub(super) fn source_drop(
    message: &Bvll6<'_>,
    peer: SocketAddrV6,
    local: SocketAddrV6,
    vmac: Bip6Vmac,
) -> Option<DropReason> {
    match *message {
        Bvll6::OriginalUnicastNpdu { source, .. } | Bvll6::OriginalBroadcastNpdu { source, .. } => {
            own_frame(source, peer, local, vmac).or_else(|| {
                (!is_answerable_source(peer.into())).then_some(DropReason::UnanswerableSource)
            })
        }
        Bvll6::AddressResolutionAck { source, .. }
        | Bvll6::VirtualAddressResolutionAck { source, .. } => {
            (source == vmac).then_some(DropReason::OwnVmacSource)
        }
        _ => None,
    }
}

/// Whether a message that names a recipient's VMAC names this node's: an
/// Original-Unicast-NPDU or either ACK for another VMAC is discarded (U.3).
/// Address-Resolution's target is not a destination: it goes to a group,
/// and its handler decides whether the target is local.
pub(super) fn destination_vmac_matches(message: &Bvll6<'_>, local_vmac: Bip6Vmac) -> bool {
    message
        .destination_vmac()
        .is_none_or(|destination| destination == local_vmac)
}

fn is_bacnet_ipv6_multicast(destination: Ipv6Addr) -> bool {
    matches!(
        destination,
        BACNET_IPV6_MULTICAST_LINK_LOCAL
            | BACNET_IPV6_MULTICAST_SITE_LOCAL
            | BACNET_IPV6_MULTICAST_ORG_LOCAL
    )
}

/// Whether an address a message carries (a Forwarded-NPDU's origin, a
/// Forwarded-Address-Resolution's asker) is usable without a scope ID: a
/// unicast address that is not unspecified, loopback, link-local, or
/// IPv4-mapped or -compatible, with a non-zero port. Stricter than
/// [`is_answerable_source`], since the 18 octets carry no scope and name an
/// address this node never saw a datagram from.
pub(super) fn forwarded_source_is_usable(source: SocketAddrV6) -> bool {
    let ip = source.ip();
    source.port() != 0
        && !ip.is_unspecified()
        && !ip.is_multicast()
        && !ip.is_loopback()
        && ip.to_ipv4().is_none()
        // A Forwarded-NPDU carries no IPv6 scope ID. Synthesizing the BBMD's
        // ingress scope for a link-local origin can target the wrong link.
        && !ip.is_unicast_link_local()
}

/// Local addressing state that a received destination is judged against.
pub(super) struct LocalBinding<'a> {
    /// IP address the socket is bound to.
    pub(super) ip: Ipv6Addr,
    /// This node's virtual MAC.
    pub(super) vmac: Bip6Vmac,
    /// Unicast addresses of the local interfaces (used for wildcard binds).
    pub(super) unicast_ips: &'a [Ipv6Addr],
    /// Whether the socket is bound to the wildcard address.
    pub(super) wildcard_bind: bool,
}

/// Whether a datagram was sent to this node's own unicast address, not to a
/// group.
fn is_local_unicast_address(
    destination: IpAddr,
    local: &LocalBinding<'_>,
    os_group_delivery: Option<bool>,
) -> bool {
    let LocalBinding {
        ip: local_ip,
        unicast_ips: local_unicast_ips,
        wildcard_bind,
        ..
    } = *local;
    let ip_matches = match destination {
        IpAddr::V6(ip) if wildcard_bind => {
            !ip.is_multicast()
                && (local_unicast_ips.contains(&ip)
                    || (cfg!(windows) && os_group_delivery == Some(false)))
        }
        IpAddr::V6(ip) => ip == local_ip,
        IpAddr::V4(_) => false,
    };
    ip_matches && os_group_delivery != Some(true)
}

pub(super) fn is_local_unicast_delivery(
    destination: IpAddr,
    destination_vmac: Option<Bip6Vmac>,
    local: &LocalBinding<'_>,
    os_group_delivery: Option<bool>,
) -> bool {
    is_local_unicast_address(destination, local, os_group_delivery)
        && destination_vmac == Some(local.vmac)
}

/// Whether a datagram was sent to a group: an IPv6 multicast address, or a
/// delivery the OS reports as multicast or broadcast.
pub(super) fn group_destination(destination: IpAddr, os_group_delivery: Option<bool>) -> bool {
    matches!(destination, IpAddr::V6(ip) if ip.is_multicast()) || os_group_delivery == Some(true)
}

/// Whether a frame's BVLC function fits the address it was sent to.
///
/// Annex U keeps directed and broadcast NPDUs apart by addressing: a directed
/// NPDU goes to the recipient's unicast B/IPv6 address and VMAC in an
/// Original-Unicast-NPDU (U.2.2, U.3), and a broadcast goes to a B/IPv6
/// multicast group in an Original-Broadcast-NPDU (U.4). A frame that mixes the
/// two is dropped before its NPDU reaches the network layer.
///
/// The unicast requests this node answers (Virtual-Address-Resolution and
/// the three only a BBMD serves) must come to its own address too. One sent
/// to a group is dropped, so a single forged multicast can't draw an answer
/// from every node in the group. Forwarded-Address-Resolution's addressing
/// depends on the mode: [`forwarded_address_resolution_is_trusted`].
pub(super) fn original_destination_matches(
    function: Bvlc6Function,
    destination: IpAddr,
    destination_vmac: Option<Bip6Vmac>,
    local: &LocalBinding<'_>,
    os_group_delivery: Option<bool>,
) -> bool {
    match function {
        Bvlc6Function::ORIGINAL_UNICAST_NPDU
        | Bvlc6Function::ADDRESS_RESOLUTION_ACK
        | Bvlc6Function::VIRTUAL_ADDRESS_RESOLUTION_ACK => {
            is_local_unicast_delivery(destination, destination_vmac, local, os_group_delivery)
        }
        Bvlc6Function::VIRTUAL_ADDRESS_RESOLUTION
        | Bvlc6Function::REGISTER_FOREIGN_DEVICE
        | Bvlc6Function::DELETE_FOREIGN_DEVICE_TABLE_ENTRY
        | Bvlc6Function::DISTRIBUTE_BROADCAST_TO_NETWORK => {
            is_local_unicast_address(destination, local, os_group_delivery)
        }
        Bvlc6Function::ORIGINAL_BROADCAST_NPDU | Bvlc6Function::ADDRESS_RESOLUTION => {
            is_bacnet_group_delivery(destination, os_group_delivery)
        }
        _ => true,
    }
}

/// Whether a datagram's destination is a BACnet group: FF02::BAC0, FF05::BAC0
/// or FF08::BAC0.
fn is_bacnet_group_delivery(destination: IpAddr, os_group_delivery: Option<bool>) -> bool {
    matches!(destination, IpAddr::V6(ip) if is_bacnet_ipv6_multicast(ip))
        && os_group_delivery != Some(false)
}

/// Whether `peer` is an answerable source: an IPv6 source an answer would
/// reach as one node, so neither a group, nor unspecified, nor IPv4-mapped,
/// nor port 0. IPv6 forbids a multicast source address, so a datagram
/// claiming one is forged, and an answer to it would reach every member of
/// the group. The socket is IPv6-only, so an IPv4-mapped source is forged
/// too. Link-local and loopback sources are answerable: the socket reports
/// the scope a link-local one arrived on.
pub(super) fn is_answerable_source(peer: SocketAddr) -> bool {
    matches!(peer, SocketAddr::V6(peer)
        if !peer.ip().is_multicast()
            && !peer.ip().is_unspecified()
            && peer.ip().to_ipv4_mapped().is_none()
            && peer.port() != 0)
}

/// Whether `peer` is the BBMD this node registers with as a foreign device:
/// `false` when it isn't one. A link-local BBMD never matches, since its
/// configured address has no scope ID to tell links apart.
pub(super) fn is_configured_bbmd(peer: SocketAddr, foreign_bbmd: Option<(Ipv6Addr, u16)>) -> bool {
    foreign_bbmd.is_some_and(|(bbmd_ip, bbmd_port)| {
        !bbmd_ip.is_unicast_link_local()
            && matches!(peer, SocketAddr::V6(peer) if *peer.ip() == bbmd_ip && peer.port() == bbmd_port)
    })
}

/// Whether a Forwarded-Address-Resolution came by a path this node answers.
///
/// A BBMD sends one by unicast to each foreign device it serves, and by
/// multicast into its own domain (U.4.4); the node whose VMAC it targets
/// answers at the asker's address the message carries (U.5). Since that
/// address comes from the message, who may send one matters:
///
/// - A foreign device takes one only by unicast from the BBMD it is
///   configured to register with.
/// - Any other node has no configured BBMD to check the sender against. It
///   takes one that arrives on a BACnet multicast group from an answerable
///   source ([`is_answerable_source`]). A forged one draws at most one
///   Address-Resolution-ACK, from the one node whose VMAC it targets, so the
///   reflection is bounded by the per-destination quota the answer counts
///   against.
///
/// Either way the answer also needs the target to be this node's VMAC and
/// the carried address to be usable without a scope ID
/// ([`forwarded_source_is_usable`]); the receive path checks those.
pub(super) fn forwarded_address_resolution_is_trusted(
    peer: SocketAddr,
    destination: IpAddr,
    os_group_delivery: Option<bool>,
    local: &LocalBinding<'_>,
    foreign_bbmd: Option<(Ipv6Addr, u16)>,
) -> bool {
    if foreign_bbmd.is_some() {
        is_configured_bbmd(peer, foreign_bbmd)
            && is_local_unicast_address(destination, local, os_group_delivery)
    } else {
        is_bacnet_group_delivery(destination, os_group_delivery) && is_answerable_source(peer)
    }
}

pub(super) fn forwarded_npdu_is_trusted(
    peer: SocketAddr,
    destination: IpAddr,
    os_group_delivery: Option<bool>,
    local_ip: Ipv6Addr,
    local_unicast_ips: &[Ipv6Addr],
    wildcard_bind: bool,
    foreign_bbmd: Option<(Ipv6Addr, u16)>,
) -> bool {
    if foreign_bbmd.is_some() {
        let peer_matches = is_configured_bbmd(peer, foreign_bbmd);
        let destination_matches = match destination {
            IpAddr::V6(ip) if wildcard_bind => {
                !ip.is_multicast()
                    && (local_unicast_ips.contains(&ip)
                        || (cfg!(windows) && os_group_delivery == Some(false)))
            }
            IpAddr::V6(ip) => ip == local_ip,
            IpAddr::V4(_) => false,
        };
        peer_matches && destination_matches && os_group_delivery != Some(true)
    } else {
        is_bacnet_group_delivery(destination, os_group_delivery)
    }
}
