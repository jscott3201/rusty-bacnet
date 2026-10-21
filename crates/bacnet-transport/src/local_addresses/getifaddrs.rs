//! The host's IPv4 addresses from the one `getifaddrs` walk
//! ([`crate::sys::ifaddrs`]).

use std::io;
use std::net::Ipv4Addr;

use super::{LocalInterface, ReportedAddress};
use crate::sys::ifaddrs::{interface_addresses, Address, Flags};

/// One IPv4 address `getifaddrs` reports, with its interface.
struct Entry {
    ip: Ipv4Addr,
    netmask: Option<Ipv4Addr>,
    flags: Flags,
    /// The interface's index, 0 when the entry has no name or its name none.
    index: u32,
}

/// Every IPv4 address `getifaddrs` reports, on any interface.
fn entries() -> io::Result<Vec<Entry>> {
    Ok(interface_addresses()?
        .into_iter()
        .filter_map(|entry| match entry.address {
            Address::V4 { ip, netmask } => Some(Entry {
                ip,
                netmask,
                flags: entry.flags,
                index: entry.index.unwrap_or(0),
            }),
            Address::V6(_) => None,
        })
        .collect())
}

/// Every IPv4 address `getifaddrs` reports, on any interface.
pub(super) fn reported_ipv4() -> io::Result<Vec<ReportedAddress>> {
    Ok(entries()?
        .into_iter()
        .map(|entry| ReportedAddress {
            ip: entry.ip,
            // getifaddrs lists the addresses assigned to interfaces and
            // reports no duplicate address detection state.
            assigned: true,
        })
        .collect())
}

/// The interface that has `ip`, or whose subnet holds it, as loopback's
/// 127.0.0.0/8 holds 127.0.0.2.
pub(crate) fn interface_of(ip: Ipv4Addr) -> io::Result<Option<LocalInterface>> {
    let entries = entries()?;
    let on_subnet = |entry: &&Entry| {
        entry
            .netmask
            .is_some_and(|mask| entry.ip & mask == ip & mask && !mask.is_unspecified())
    };
    let found = entries
        .iter()
        .find(|entry| entry.ip == ip)
        .or_else(|| entries.iter().find(on_subnet));
    Ok(found.map(|entry| LocalInterface {
        index: (entry.index != 0).then_some(entry.index),
        netmask: entry.netmask,
        broadcast: entry.flags.broadcast,
    }))
}
