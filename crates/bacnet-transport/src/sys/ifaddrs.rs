//! The one `getifaddrs` walk (#1884). The host's IPv4 addresses with their
//! netmasks (`local_addresses`) and its IPv6 addresses (`bip6::link`) both
//! come from [`interface_addresses`], which copies each IPv4 or IPv6 entry
//! out while the OS list is alive.
//!
//! nix's `getifaddrs` is not used, for two reasons. On Apple and the BSDs it
//! drops a netmask the kernel hands back truncated with its family unset,
//! which [`truncated_netmask`] recovers: nix picks the address type by family
//! alone. And with no length to go by, it reads a whole 16-octet
//! `sockaddr_in` for an IPv4 mask whatever `sa_len` says, past the end of a
//! short BSD mask.
//!
//! # The list invariant
//!
//! Every unsafe operation here rests on one argument. A successful
//! `getifaddrs` returns a list that stays valid until the one `freeifaddrs`
//! call, which only [`List`]'s drop makes, after the walk's last read. While
//! it lives, the head and every `ifa_next` link are null or a valid node, and
//! in each node:
//! - `ifa_addr` and `ifa_netmask` are null or point to a socket address whose
//!   two-octet header (length and family on the BSDs, family elsewhere) is
//!   readable, and which is whole for its family: a `sockaddr_in` for
//!   `AF_INET`, a `sockaddr_in6` for `AF_INET6`. Reads go through
//!   `read_unaligned`, so alignment is not assumed.
//! - On Apple and the BSDs a netmask may be cut short after its last nonzero
//!   octet, with its family unset; its length octet still says how many of
//!   its octets are readable.
//! - `ifa_name` is null or a NUL-terminated string, which `if_nametoindex`
//!   only reads.

use std::io;
use std::net::{Ipv4Addr, Ipv6Addr};

/// The address of one `getifaddrs` entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Address {
    /// An IPv4 address and its netmask, when one is reported.
    V4 {
        /// The address.
        ip: Ipv4Addr,
        /// Its netmask.
        netmask: Option<Ipv4Addr>,
    },
    /// An IPv6 address. Only B/IPv6 reads it.
    #[cfg_attr(not(any(feature = "ipv6", test)), allow(dead_code))]
    V6(Ipv6Addr),
}

/// The interface flags the callers read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Flags {
    /// `IFF_UP`.
    #[cfg_attr(not(any(feature = "ipv6", test)), allow(dead_code))]
    pub(crate) up: bool,
    /// `IFF_BROADCAST`.
    pub(crate) broadcast: bool,
    /// `IFF_MULTICAST`.
    #[cfg_attr(not(any(feature = "ipv6", test)), allow(dead_code))]
    pub(crate) multicast: bool,
    /// `IFF_LOOPBACK`.
    #[cfg_attr(not(any(feature = "ipv6", test)), allow(dead_code))]
    pub(crate) loopback: bool,
}

impl Flags {
    /// The flags in `raw`, an `ifa_flags` of any width: it is 32 bits on most
    /// targets and 64 on illumos and Solaris.
    pub(crate) fn from_raw(raw: impl Into<u64>) -> Self {
        let raw = raw.into();
        let has = |flag: libc::c_int| u64::try_from(flag).is_ok_and(|bit| raw & bit != 0);
        Self {
            up: has(libc::IFF_UP),
            broadcast: has(libc::IFF_BROADCAST),
            multicast: has(libc::IFF_MULTICAST),
            loopback: has(libc::IFF_LOOPBACK),
        }
    }
}

/// One IPv4 or IPv6 entry `getifaddrs` reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InterfaceAddress {
    /// The entry's address.
    pub(crate) address: Address,
    /// Its interface's flags.
    pub(crate) flags: Flags,
    /// What `if_nametoindex` says for the interface's name, 0 when it knows
    /// none; `None` when the entry has no name.
    pub(crate) index: Option<u32>,
}

/// The netmask in the leading octets of a BSD socket address, `raw`: a
/// length octet, a family octet, two port octets, then the address. The
/// kernel may cut the mask after its last nonzero octet and leave its family
/// unset, so an `AF_UNSPEC` mask counts, and the octets past the length are
/// zero. `raw` holds the mask's readable octets, up to the address's end.
#[cfg_attr(
    not(any(
        test,
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd",
    )),
    allow(dead_code)
)]
fn truncated_netmask(raw: &[u8]) -> Option<Ipv4Addr> {
    let (&length, rest) = raw.split_first()?;
    let family = i32::from(*rest.first()?);
    if family != libc::AF_INET && family != libc::AF_UNSPEC {
        return None;
    }
    // The address octets the length covers, of those `raw` holds.
    let address = raw.get(4..).unwrap_or_default();
    let present = &address[..address.len().min(usize::from(length).saturating_sub(4)).min(4)];
    let mut octets = [0u8; 4];
    octets[..present.len()].copy_from_slice(present);
    Some(Ipv4Addr::from(octets))
}

/// The netmask at `mask`.
///
/// # Safety
///
/// `mask` is a node's `ifa_netmask` in a live list (the module's list
/// invariant).
unsafe fn netmask_of(mask: *const libc::sockaddr) -> Option<Ipv4Addr> {
    if mask.is_null() {
        return None;
    }
    #[cfg(any(
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd",
    ))]
    {
        // SAFETY: the list invariant: the header of a non-null mask is
        // readable, and so are as many octets as its length octet says. The
        // slice covers the header and at most that many more, up to the
        // address's end.
        let raw = unsafe {
            let readable = usize::from((*mask).sa_len).clamp(2, 8);
            std::slice::from_raw_parts(mask.cast::<u8>(), readable)
        };
        truncated_netmask(raw)
    }
    #[cfg(not(any(
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd",
    )))]
    {
        // SAFETY: the list invariant: a non-null mask's family is readable,
        // and an AF_INET one is a whole sockaddr_in.
        unsafe {
            if i32::from((*mask).sa_family) != libc::AF_INET {
                return None;
            }
            let mask = mask.cast::<libc::sockaddr_in>().read_unaligned();
            Some(Ipv4Addr::from(mask.sin_addr.s_addr.to_ne_bytes()))
        }
    }
}

/// The IPv4 or IPv6 entry in `node`, if it is one.
///
/// # Safety
///
/// `node` is a node of a live list (the module's list invariant).
unsafe fn read_node(node: &libc::ifaddrs) -> Option<InterfaceAddress> {
    let address = node.ifa_addr;
    if address.is_null() {
        return None;
    }
    // SAFETY: the list invariant: a non-null address's family is readable.
    let family = i32::from(unsafe { (*address).sa_family });
    let address = match family {
        libc::AF_INET => {
            // SAFETY: the list invariant: an AF_INET address is a whole
            // sockaddr_in.
            let ip = unsafe { address.cast::<libc::sockaddr_in>().read_unaligned() };
            Address::V4 {
                ip: Ipv4Addr::from(ip.sin_addr.s_addr.to_ne_bytes()),
                // SAFETY: the list invariant holds for this node's netmask.
                netmask: unsafe { netmask_of(node.ifa_netmask) },
            }
        }
        libc::AF_INET6 => {
            // SAFETY: the list invariant: an AF_INET6 address is a whole
            // sockaddr_in6.
            let ip = unsafe { address.cast::<libc::sockaddr_in6>().read_unaligned() };
            Address::V6(Ipv6Addr::from(ip.sin6_addr.s6_addr))
        }
        _ => return None,
    };
    let name = node.ifa_name;
    // SAFETY: the list invariant: a non-null name is a NUL-terminated string,
    // which if_nametoindex only reads.
    let index = (!name.is_null()).then(|| unsafe { libc::if_nametoindex(name) });
    Some(InterfaceAddress {
        address,
        flags: Flags::from_raw(node.ifa_flags),
        index,
    })
}

/// The `getifaddrs` list, freed when this is dropped.
struct List(*mut libc::ifaddrs);

impl Drop for List {
    fn drop(&mut self) {
        // SAFETY: the list invariant: a List holds only a list a successful
        // getifaddrs returned, and this is its one freeifaddrs, after the
        // walk's last read.
        unsafe { libc::freeifaddrs(self.0) }
    }
}

/// Every IPv4 and IPv6 entry `getifaddrs` reports, in its order.
pub(crate) fn interface_addresses() -> io::Result<Vec<InterfaceAddress>> {
    let mut head = std::ptr::null_mut();
    // SAFETY: `head` is a writable pointer slot; on success getifaddrs sets
    // it to the list, which the guard below frees.
    if unsafe { libc::getifaddrs(&mut head) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let list = List(head);
    let mut entries = Vec::new();
    let mut current = list.0;
    // SAFETY: the list invariant: `current` is the head or an `ifa_next`
    // link, so null or a valid node of the list `list` keeps alive.
    while let Some(node) = unsafe { current.as_ref() } {
        // SAFETY: as above.
        entries.extend(unsafe { read_node(node) });
        current = node.ifa_next;
    }
    drop(list);
    Ok(entries)
}

#[cfg(test)]
#[path = "ifaddrs_tests.rs"]
mod tests;
