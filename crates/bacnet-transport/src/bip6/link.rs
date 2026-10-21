//! One normal-mode local IPv6 address and its OS interface owner.

use std::io;
use std::net::Ipv6Addr;

with_getifaddrs! {
    mod getifaddrs;
}
#[cfg(windows)]
mod windows;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SelectedLink {
    pub address: Ipv6Addr,
    pub index: u32,
}

#[derive(Clone, Copy, Debug)]
struct Candidate {
    link: SelectedLink,
    up: bool,
    multicast: bool,
    loopback: bool,
}

impl SelectedLink {
    pub async fn resolve(requested: Ipv6Addr) -> io::Result<Self> {
        let candidates = tokio::task::spawn_blocking(enumerate)
            .await
            .map_err(io::Error::other)??;
        select(requested, &candidates)
    }
}

fn selection_error(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::AddrNotAvailable, message)
}

fn select(requested: Ipv6Addr, candidates: &[Candidate]) -> io::Result<SelectedLink> {
    let mut viable: Vec<_> = candidates
        .iter()
        .filter(|candidate| {
            candidate.up
                && (candidate.multicast || candidate.loopback)
                && candidate.link.index != 0
                && !candidate.link.address.is_unspecified()
                && !candidate.link.address.is_multicast()
                && candidate.link.address.to_ipv4_mapped().is_none()
        })
        .copied()
        .collect();
    viable.sort_unstable_by_key(|candidate| (candidate.link.index, candidate.link.address));
    viable.dedup_by_key(|candidate| (candidate.link.index, candidate.link.address));
    if !requested.is_unspecified() {
        viable.retain(|candidate| candidate.link.address == requested);
    } else {
        if viable.iter().any(|candidate| !candidate.loopback) {
            viable.retain(|candidate| !candidate.loopback);
        }
        if let Some(first) = viable.first() {
            if viable
                .iter()
                .any(|candidate| candidate.link.index != first.link.index)
            {
                return Err(selection_error(
                    "multiple usable IPv6 interfaces; configure a concrete local IPv6 address",
                ));
            }
        }
        if viable
            .iter()
            .any(|candidate| !candidate.link.address.is_unicast_link_local())
        {
            viable.retain(|candidate| !candidate.link.address.is_unicast_link_local());
        }
    }
    match viable.as_slice() {
        [candidate] => Ok(candidate.link),
        [] => Err(selection_error(
            "no usable local IPv6 address/interface matches selection",
        )),
        _ => Err(selection_error(
            "ambiguous local IPv6 address selection; configure one uniquely owned concrete address",
        )),
    }
}

with_getifaddrs! {
    fn enumerate() -> io::Result<Vec<Candidate>> {
        getifaddrs::enumerate()
    }
}

#[cfg(windows)]
fn enumerate() -> io::Result<Vec<Candidate>> {
    windows::enumerate()
}

without_interface_list! {
    fn enumerate() -> io::Result<Vec<Candidate>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "selected IPv6 interfaces are unsupported",
        ))
    }
}

#[cfg(test)]
mod tests;
