//! The checks a routed confirmed request takes before anything is sent:
//! the router it goes through must be one station on this link (#2722), and
//! the station's MAC on its own network must not spell that network's
//! broadcast (#2709). Both sit beside the local check (#1479, #1493) on the
//! one request path every typed call takes, and the Python binding and
//! bacnet-ops make the second before they look a station up
//! ([`Station::check_confirmed`]).
//!
//! An unconfirmed request is left alone by both: a remote broadcast may
//! carry one (Clause 6.3), and has its own calls
//! ([`BACnetClient::broadcast_network_unconfirmed`], `Broadcast::Network`).

use std::fmt;
use std::net::Ipv4Addr;

use bacnet_types::data_link::DataLink;

use super::*;

/// A MAC that names every station of a remote network rather than one, read
/// from its length and octets alone, since this client can't see the far
/// network's data link (#2709). A router delivers a DADR equal to its
/// network's broadcast MAC as a broadcast there (Clause 6.5.4).
///
/// Only an all-ones MAC of one, three or six octets is such a form, and
/// only on ARCNET (Clause 8) is the one-octet `0xFF` a station. Two- and
/// seven-octet (LonTalk) and other lengths are taken as one station.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RemoteBroadcast {
    /// The one octet `0xFF`: the broadcast on an MS/TP network (Clause 9),
    /// whose stations are 0 to 254. On an ARCNET network it is a station:
    /// [`Station::arcnet`] says so.
    MstpBroadcast,
    /// The one octet `0x00` of a station marked ARCNET
    /// ([`Station::arcnet`]): that network's broadcast (Clause 8).
    ArcnetBroadcast,
    /// Three octets of `0xFF`: no device's VMAC, since a device instance's
    /// VMAC leaves the top two bits clear (Clause H.7.2), on B/IPv6
    /// (Annex U) and ZigBee (Annex O) alike.
    NoVmac,
    /// Six octets of `0xFF`: the broadcast MAC on Ethernet (Clause 6.3),
    /// the local broadcast VMAC on BACnet/SC (Annex AB.1.5.2), and on
    /// BACnet/IP an address whose host bits are all set, a broadcast
    /// (Annex J.1.2).
    SixOctetBroadcast,
}

impl RemoteBroadcast {
    /// The form `mac`, a station's MAC on a remote network, takes, if it
    /// is one; `arcnet` when the station is marked as on an ARCNET network
    /// ([`Station::arcnet`]). A handful of comparisons, no allocation.
    pub fn of(mac: &[u8], arcnet: bool) -> Option<Self> {
        match mac {
            [0xFF] if !arcnet => Some(Self::MstpBroadcast),
            [0x00] if arcnet => Some(Self::ArcnetBroadcast),
            [0xFF, 0xFF, 0xFF] => Some(Self::NoVmac),
            [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF] => Some(Self::SixOctetBroadcast),
            _ => None,
        }
    }
}

impl fmt::Display for RemoteBroadcast {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::MstpBroadcast => {
                "station 255, the broadcast of an MS/TP network (on an ARCNET network, where \
                 255 is a station, mark the station ARCNET)"
            }
            Self::ArcnetBroadcast => "station 0, the broadcast of an ARCNET network",
            Self::NoVmac => {
                "VMAC 0xFFFFFF, which no device has (a device's VMAC keeps its top two bits \
                 clear)"
            }
            Self::SixOctetBroadcast => {
                "FF:FF:FF:FF:FF:FF, a broadcast on every six-octet link: Ethernet's broadcast \
                 MAC, BACnet/SC's broadcast VMAC, and 255.255.255.255:65535 on BACnet/IP"
            }
        })
    }
}

/// Refuse a confirmed request to `mac` on `network` when the MAC is a
/// [`RemoteBroadcast`] form (#2709). Free unless the MAC is one octet,
/// three or six.
#[inline]
pub(super) fn check_remote_station(network: u16, mac: &[u8], arcnet: bool) -> Result<(), Error> {
    match RemoteBroadcast::of(mac, arcnet) {
        None => Ok(()),
        Some(form) => Err(remote_broadcast_refused(network, mac, form)),
    }
}

#[cold]
#[inline(never)]
fn remote_broadcast_refused(network: u16, mac: &[u8], form: RemoteBroadcast) -> Error {
    Error::Encoding(format!(
        "a confirmed request goes to one station, and {} on network {network} is {form}; \
         a remote broadcast carries only an unconfirmed request",
        station::station_text(network, mac)
    ))
}

/// Refuse a named router with no MAC: it names no router, and the client
/// marks the local broadcast form that way
/// ([`VIA_LOCAL_BROADCAST`](station::VIA_LOCAL_BROADCAST)).
#[inline]
pub(super) fn check_router_named(router: &[u8]) -> Result<(), Error> {
    if router.is_empty() {
        return Err(Error::Encoding(
            "an empty router MAC names no router: name the router, or send by one local \
             broadcast with Station::RoutedViaLocalBroadcast"
                .into(),
        ));
    }
    Ok(())
}

/// Refuse a routed confirmed request whose router, `router` on this link,
/// is the link's broadcast or a group address (#2722): such a request would
/// reach every router there at once, a write included. The form that
/// reaches a network by one local broadcast names no router and isn't
/// checked here.
#[inline]
pub(super) fn check_router<T: TransportPort>(
    transport: &T,
    network: u16,
    router: &[u8],
) -> Result<(), Error> {
    if router.is_empty() || !transport.is_group_destination(router) {
        return Ok(());
    }
    Err(group_router_refused(transport.data_link(), network, router))
}

#[cold]
#[inline(never)]
fn group_router_refused(link: Option<DataLink>, network: u16, router: &[u8]) -> Error {
    let on = match link {
        Some(link) => format!("this {link} link"),
        None => "this link".to_owned(),
    };
    Error::Encoding(format!(
        "a routed confirmed request goes to one router by unicast, and the router {} is a \
         broadcast or group address on {on}: name the router's own address, or reach \
         network {network} by one local broadcast with no router named",
        link_text(link, router)
    ))
}

/// `mac` as its link writes it: an IPv4 address and port on BACnet/IP, a
/// station number on MS/TP, a VMAC on BACnet/IPv6, and colon-separated hex
/// otherwise (an Ethernet MAC, a BACnet/SC VMAC, or a link this client
/// can't name).
pub(super) fn link_text(link: Option<DataLink>, mac: &[u8]) -> String {
    match (link, mac) {
        (Some(DataLink::Bip), &[a, b, c, d, hi, lo]) => {
            format!(
                "{}:{}",
                Ipv4Addr::new(a, b, c, d),
                u16::from_be_bytes([hi, lo])
            )
        }
        (Some(DataLink::Mstp), &[station]) => format!("MS/TP station {station}"),
        (Some(DataLink::Bip6), &[a, b, c]) => format!("VMAC 0x{a:02X}{b:02X}{c:02X}"),
        _ => mac
            .iter()
            .map(|octet| format!("{octet:02X}"))
            .collect::<Vec<_>>()
            .join(":"),
    }
}

impl ClientOptions {
    /// Send every frame of this client's confirmed requests (the first, each
    /// retry and each segment, to any station form) with `attributes`, the
    /// data attributes of the link: on BACnet/SC its header data options
    /// (Annex AB), which the transport checks when it sends. A link that
    /// carries none ignores them. Empty by default, which costs a request
    /// nothing. Unconfirmed requests and the client's answers carry none.
    ///
    /// For different attributes per request, start a client for each set:
    /// on an endpoint session each `EndpointSession::client_with` takes its
    /// own options and shares the session's link (#2723).
    pub fn with_data_attributes(mut self, attributes: Vec<DataAttribute>) -> Self {
        self.data_attributes = attributes;
        self
    }
}

#[cfg(test)]
#[path = "send_checks_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "routed_send_checks_tests.rs"]
mod routed_tests;
