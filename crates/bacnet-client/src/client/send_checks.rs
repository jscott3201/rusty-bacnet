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

use std::net::Ipv4Addr;

use bacnet_types::data_link::DataLink;

use super::*;

/// The DADR forms a confirmed request to a remote station refuses: the
/// network layer's vocabulary, so the server's own confirmed sends can
/// refuse them too. [`Station::one_station`] is this client's `one_station`.
pub use bacnet_npdu::RemoteBroadcast;

/// Refuse a confirmed request to `mac` on `network` when the MAC is a
/// [`RemoteBroadcast`] form (#2709), as `one_station`, the station's mark
/// (#2738), has them. Free unless the MAC is one, two, three or six
/// octets.
#[inline]
pub(super) fn check_remote_station(
    network: u16,
    mac: &[u8],
    one_station: bool,
) -> Result<(), Error> {
    match RemoteBroadcast::of(mac, one_station) {
        None => Ok(()),
        Some(form) => Err(remote_broadcast_refused(network, mac, form, one_station)),
    }
}

/// The refusal, decoded: the station as the target grammar writes it, the
/// form, and, for an unmarked address the mark would let through, the
/// mark as the way to a station there.
#[cold]
#[inline(never)]
fn remote_broadcast_refused(
    network: u16,
    mac: &[u8],
    form: RemoteBroadcast,
    one_station: bool,
) -> Error {
    let station = station::station_text(network, mac);
    let marked = if one_station {
        ", even marked one station"
    } else {
        ""
    };
    // The station the form may be, named for its length.
    let which = match form {
        RemoteBroadcast::MstpBroadcast => "ARCNET's station 255",
        _ => "a station a gateway numbers in two octets",
    };
    let way = if !one_station && form.lifted_by_one_station() {
        format!(
            ". If {station} is one station on its network ({which}), mark it one station \
             (one_station)"
        )
    } else {
        String::new()
    };
    Error::InvalidArgument(Refusal::text(format!(
        "a confirmed request goes to one station, and {station} on network {network} is \
         {form}{marked}; a remote broadcast or multicast carries only an unconfirmed \
         request{way}"
    )))
}

/// Refuse a named router with no MAC: it names no router, and the client
/// marks the local broadcast form that way
/// ([`VIA_LOCAL_BROADCAST`](station::VIA_LOCAL_BROADCAST)).
#[inline]
pub(super) fn check_router_named(router: &[u8]) -> Result<(), Error> {
    if router.is_empty() {
        return Err(Error::invalid_argument(
            "an empty router MAC names no router: name the router, or send by one local \
             broadcast with Station::RoutedViaLocalBroadcast",
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
    Error::InvalidArgument(Refusal::text(format!(
        "a routed confirmed request goes to one router by unicast, and the router {} is a \
         broadcast or group address on {on}: name the router's own address, or reach \
         network {network} by one local broadcast with no router named",
        link_text(link, router)
    )))
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

#[cfg(test)]
#[path = "send_checks_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "routed_send_checks_tests.rs"]
mod routed_tests;
