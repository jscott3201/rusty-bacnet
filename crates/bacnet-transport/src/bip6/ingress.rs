//! BACnet/IPv6 ingress provenance checks.

use std::net::{IpAddr, Ipv6Addr, SocketAddr, SocketAddrV6};

use super::{
    Bip6Vmac, Bvlc6Function, BACNET_IPV6_MULTICAST_LINK_LOCAL, BACNET_IPV6_MULTICAST_ORG_LOCAL,
    BACNET_IPV6_MULTICAST_SITE_LOCAL,
};

fn is_bacnet_ipv6_multicast(destination: Ipv6Addr) -> bool {
    matches!(
        destination,
        BACNET_IPV6_MULTICAST_LINK_LOCAL
            | BACNET_IPV6_MULTICAST_SITE_LOCAL
            | BACNET_IPV6_MULTICAST_ORG_LOCAL
    )
}

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

pub(super) fn is_local_unicast_delivery(
    destination: IpAddr,
    destination_vmac: Option<Bip6Vmac>,
    local_ip: Ipv6Addr,
    local_vmac: Bip6Vmac,
    local_unicast_ips: &[Ipv6Addr],
    wildcard_bind: bool,
    os_group_delivery: Option<bool>,
) -> bool {
    let ip_matches = match destination {
        IpAddr::V6(ip) if wildcard_bind => {
            !ip.is_multicast()
                && (local_unicast_ips.contains(&ip)
                    || (cfg!(windows) && os_group_delivery == Some(false)))
        }
        IpAddr::V6(ip) => ip == local_ip,
        IpAddr::V4(_) => false,
    };
    ip_matches && destination_vmac == Some(local_vmac) && os_group_delivery != Some(true)
}

pub(super) fn original_destination_matches(
    function: Bvlc6Function,
    destination: IpAddr,
    destination_vmac: Option<Bip6Vmac>,
    local_ip: Ipv6Addr,
    local_vmac: Bip6Vmac,
    local_unicast_ips: &[Ipv6Addr],
    wildcard_bind: bool,
    os_group_delivery: Option<bool>,
) -> bool {
    match function {
        Bvlc6Function::OriginalUnicast
        | Bvlc6Function::AddressResolutionAck
        | Bvlc6Function::VirtualAddressResolutionAck => is_local_unicast_delivery(
            destination,
            destination_vmac,
            local_ip,
            local_vmac,
            local_unicast_ips,
            wildcard_bind,
            os_group_delivery,
        ),
        Bvlc6Function::VirtualAddressResolution => {
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
        Bvlc6Function::OriginalBroadcast | Bvlc6Function::AddressResolution => {
            matches!(destination, IpAddr::V6(ip) if is_bacnet_ipv6_multicast(ip))
                && os_group_delivery != Some(false)
        }
        // Forwarded-Address-Resolution is not implemented. Drop it before
        // learning or any other state mutation.
        Bvlc6Function::ForwardedAddressResolution => false,
        _ => true,
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
    if let Some((bbmd_ip, bbmd_port)) = foreign_bbmd {
        let peer_matches = !bbmd_ip.is_unicast_link_local()
            && matches!(peer, SocketAddr::V6(peer) if *peer.ip() == bbmd_ip && peer.port() == bbmd_port);
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
        matches!(destination, IpAddr::V6(ip) if is_bacnet_ipv6_multicast(ip))
            && os_group_delivery != Some(false)
    }
}
