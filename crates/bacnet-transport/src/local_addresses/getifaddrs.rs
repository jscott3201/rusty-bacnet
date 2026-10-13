//! The host's IPv4 addresses from `getifaddrs`, copied out while the OS list
//! is alive.
#![allow(unsafe_code)]

use std::ffi::CStr;
use std::io;
use std::net::Ipv4Addr;

use super::{LocalInterface, ReportedAddress};

/// One IPv4 address `getifaddrs` reports, with its interface.
struct Entry {
    ip: Ipv4Addr,
    netmask: Option<Ipv4Addr>,
    flags: libc::c_uint,
    /// The interface's index, 0 when the name has none.
    index: u32,
}

/// The IPv4 address in `address`, if it is a non-null IPv4 socket address.
///
/// # Safety
///
/// `address` is null or points to a socket address that is valid for its
/// reported family.
unsafe fn ipv4_of(address: *const libc::sockaddr) -> Option<Ipv4Addr> {
    if address.is_null() {
        return None;
    }
    // SAFETY: the non-null sockaddr is valid for its reported family.
    if unsafe { (*address).sa_family as i32 } != libc::AF_INET {
        return None;
    }
    // SAFETY: family AF_INET selects the sockaddr_in layout.
    let address = unsafe { &*(address as *const libc::sockaddr_in) };
    Some(Ipv4Addr::from(address.sin_addr.s_addr.to_ne_bytes()))
}

/// The netmask in `mask`. On Linux it is an IPv4 socket address. The BSDs
/// may hand back a mask truncated after its last nonzero octet, with its
/// family unset, so there the length octet says how much of the address is
/// present and the rest is zero.
///
/// # Safety
///
/// `mask` is null or points to a socket address at least as long as its
/// reported length (on the BSDs) or family's layout (on Linux).
unsafe fn netmask_of(mask: *const libc::sockaddr) -> Option<Ipv4Addr> {
    #[cfg(not(any(
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd",
    )))]
    // SAFETY: as the caller promises.
    return unsafe { ipv4_of(mask) };
    #[cfg(any(
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd",
    ))]
    {
        if mask.is_null() {
            return None;
        }
        // SAFETY: the non-null sockaddr has at least its length and family.
        let (length, family) = unsafe { ((*mask).sa_len as usize, (*mask).sa_family as i32) };
        if family != libc::AF_INET && family != libc::AF_UNSPEC {
            return None;
        }
        // The address octets sit at offsets 4..8 of a sockaddr_in.
        let present = length.clamp(4, 8) - 4;
        // SAFETY: `length` octets are readable, and `present` stops within them.
        let bytes = unsafe { std::slice::from_raw_parts(mask.cast::<u8>().add(4), present) };
        let mut octets = [0u8; 4];
        octets[..present].copy_from_slice(bytes);
        Some(Ipv4Addr::from(octets))
    }
}

/// Every IPv4 address `getifaddrs` reports, on any interface.
fn entries() -> io::Result<Vec<Entry>> {
    struct IfAddrsGuard(*mut libc::ifaddrs);

    impl Drop for IfAddrsGuard {
        fn drop(&mut self) {
            // SAFETY: the pointer was returned by `getifaddrs` and this guard
            // owns the corresponding single `freeifaddrs` call.
            unsafe { libc::freeifaddrs(self.0) }
        }
    }

    let mut head = std::ptr::null_mut();
    // SAFETY: `getifaddrs` initializes `head` on success. The guarded list is
    // walked only through non-null nodes and address-family-checked pointers.
    if unsafe { libc::getifaddrs(&mut head) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let _guard = IfAddrsGuard(head);
    let mut entries = Vec::new();
    let mut current = head;
    while !current.is_null() {
        // SAFETY: `current` is a node in the live guarded list.
        let entry = unsafe { &*current };
        // SAFETY: the node's socket addresses are null or valid for their
        // reported families and lengths while the list is alive.
        if let Some(ip) = unsafe { ipv4_of(entry.ifa_addr) } {
            let index = if entry.ifa_name.is_null() {
                0
            } else {
                // SAFETY: a non-null interface name is a NUL-terminated
                // string in the live list.
                let name = unsafe { CStr::from_ptr(entry.ifa_name) };
                // SAFETY: as above; `if_nametoindex` only reads the name.
                unsafe { libc::if_nametoindex(name.as_ptr()) }
            };
            entries.push(Entry {
                ip,
                // SAFETY: as above.
                netmask: unsafe { netmask_of(entry.ifa_netmask) },
                flags: entry.ifa_flags,
                index,
            });
        }
        current = entry.ifa_next;
    }
    Ok(entries)
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
        broadcast: entry.flags & libc::IFF_BROADCAST as libc::c_uint != 0,
    }))
}
