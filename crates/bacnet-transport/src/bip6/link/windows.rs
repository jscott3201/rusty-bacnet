//! Windows adapter discovery, from the shared `GetAdaptersAddresses` walk.

use std::{io, net::IpAddr};
use windows_sys::Win32::Networking::WinSock::{IpDadStatePreferred, AF_INET6};

use super::{Candidate, SelectedLink};

pub(super) fn enumerate() -> io::Result<Vec<Candidate>> {
    Ok(crate::sys::win_adapters::unicast_addresses(AF_INET6)?
        .into_iter()
        .filter(|address| address.dad_state == IpDadStatePreferred)
        .filter_map(|address| match address.ip {
            IpAddr::V6(ip) => Some(Candidate {
                link: SelectedLink {
                    address: ip,
                    index: address.ipv6_index,
                },
                up: address.up,
                multicast: address.multicast,
                loopback: address.loopback,
            }),
            IpAddr::V4(_) => None,
        })
        .collect())
}
