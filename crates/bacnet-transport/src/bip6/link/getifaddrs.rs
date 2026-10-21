//! The local IPv6 addresses from the one `getifaddrs` walk
//! ([`crate::sys::ifaddrs`]).

use std::io;

use super::{Candidate, SelectedLink};
use crate::sys::ifaddrs::{interface_addresses, Address};

pub(super) fn enumerate() -> io::Result<Vec<Candidate>> {
    Ok(interface_addresses()?
        .into_iter()
        .filter_map(|entry| match (entry.address, entry.index) {
            // An entry without an interface name has no interface to own it.
            (Address::V6(address), Some(index)) => Some(Candidate {
                link: SelectedLink { address, index },
                up: entry.flags.up,
                multicast: entry.flags.multicast,
                loopback: entry.flags.loopback,
            }),
            _ => None,
        })
        .collect())
}
