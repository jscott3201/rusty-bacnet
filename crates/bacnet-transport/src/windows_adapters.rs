//! The host's unicast addresses from `GetAdaptersAddresses`, copied out of the
//! OS result buffer so that no pointer into it escapes this module.
//!
//! B/IP lists its local IPv4 addresses with this (`local_addresses`), and
//! B/IPv6 picks its link from the IPv6 ones (`bip6::link`).
#![allow(unsafe_code)]

use std::io;
use std::mem::{size_of, MaybeUninit};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::ptr::{addr_of, read_unaligned};

use windows_sys::Win32::{
    Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_NO_DATA, NO_ERROR},
    NetworkManagement::{
        IpHelper::{
            GetAdaptersAddresses, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER,
            GAA_FLAG_SKIP_FRIENDLY_NAME, GAA_FLAG_SKIP_MULTICAST, IF_TYPE_SOFTWARE_LOOPBACK,
            IP_ADAPTER_ADDRESSES_LH, IP_ADAPTER_NO_MULTICAST,
        },
        Ndis::IfOperStatusUp,
    },
    Networking::WinSock::{
        ADDRESS_FAMILY, AF_INET, AF_INET6, NL_DAD_STATE, SOCKADDR, SOCKADDR_IN, SOCKADDR_IN6,
        SOCKET_ADDRESS,
    },
};

/// The documented starting size for the adapter buffer.
const INITIAL_BUFFER_BYTES: u32 = 15_000;
/// Refuse an inventory larger than this rather than allocate without bound.
const MAX_BUFFER_BYTES: u32 = 8 * 1024 * 1024;
/// Adapters can appear between the size query and the call that fills the
/// buffer, so the call is retried a few times with the size the OS asks for.
const ATTEMPTS: usize = 3;

/// One unicast address and what its adapter reports about itself.
#[derive(Clone, Copy, Debug)]
// The IPv4 listing and interface lookup read `ip`, `dad_state`, `loopback`
// and `prefix_length`; B/IPv6 reads the rest.
#[cfg_attr(not(feature = "ipv6"), allow(dead_code))]
pub(crate) struct UnicastAddress {
    pub(crate) ip: IpAddr,
    /// Where duplicate address detection stands for the address.
    pub(crate) dad_state: NL_DAD_STATE,
    /// The adapter's IPv6 interface index; 0 without IPv6.
    pub(crate) ipv6_index: u32,
    /// The adapter is operationally up.
    pub(crate) up: bool,
    /// The adapter can send and receive multicast.
    pub(crate) multicast: bool,
    /// The adapter is the software loopback interface.
    pub(crate) loopback: bool,
    /// The length of the address's on-link prefix, in bits.
    pub(crate) prefix_length: u8,
}

/// Every unicast address of `family` (`AF_INET`, `AF_INET6` or `AF_UNSPEC`)
/// on every adapter, whatever its state. `ERROR_NO_DATA` is an empty list.
pub(crate) fn unicast_addresses(family: ADDRESS_FAMILY) -> io::Result<Vec<UnicastAddress>> {
    let mut bytes = INITIAL_BUFFER_BYTES;
    for _ in 0..ATTEMPTS {
        if bytes > MAX_BUFFER_BYTES {
            return Err(io::Error::other(
                "the adapter inventory exceeds the bounded allocation",
            ));
        }
        // Whole records, so the buffer has the alignment of the head record.
        let records = (bytes as usize).div_ceil(size_of::<IP_ADAPTER_ADDRESSES_LH>());
        let mut buffer = vec![MaybeUninit::<IP_ADAPTER_ADDRESSES_LH>::uninit(); records];
        let head = buffer.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        // Report the buffer's real size, which rounding up may have grown.
        bytes = u32::try_from(records * size_of::<IP_ADAPTER_ADDRESSES_LH>())
            .map_err(io::Error::other)?;
        // SAFETY: `head` points to a writable, suitably aligned allocation of
        // `bytes` bytes that outlives this synchronous call, and `bytes` is a
        // live `u32` the call may overwrite with the size it needs. The
        // reserved pointer must be null.
        let status = unsafe {
            GetAdaptersAddresses(
                u32::from(family),
                GAA_FLAG_SKIP_ANYCAST
                    | GAA_FLAG_SKIP_MULTICAST
                    | GAA_FLAG_SKIP_DNS_SERVER
                    | GAA_FLAG_SKIP_FRIENDLY_NAME,
                std::ptr::null(),
                head,
                &mut bytes,
            )
        };
        match status {
            ERROR_BUFFER_OVERFLOW => continue,
            ERROR_NO_DATA => return Ok(Vec::new()),
            NO_ERROR => {}
            other => return Err(io::Error::from_raw_os_error(other as i32)),
        }
        // SAFETY: the call succeeded, so `head` starts the adapter list it
        // wrote into `buffer`, which stays allocated and untouched until the
        // walk returns. The buffer is ours: nothing in it is freed separately.
        return Ok(unsafe { copy_addresses(head) });
    }
    Err(io::Error::other(
        "the adapter inventory kept changing during discovery",
    ))
}

/// Copy every unicast address out of the adapter list that starts at `head`.
///
/// # Safety
///
/// `head` is null or the first record of a list that a successful
/// `GetAdaptersAddresses` call wrote, and the buffer holding that list stays
/// alive and unmodified for the whole call.
unsafe fn copy_addresses(head: *const IP_ADAPTER_ADDRESSES_LH) -> Vec<UnicastAddress> {
    let mut addresses = Vec::new();
    let mut adapter_ptr = head;
    while !adapter_ptr.is_null() {
        // SAFETY: a non-null record pointer is the head or a `Next` link of
        // the OS-written list, which lies in the live buffer (the contract).
        let adapter = unsafe { &*adapter_ptr };
        // SAFETY: both members of this union are plain `u32` views of the
        // same flags word, so either one is initialized.
        let flags = unsafe { adapter.Anonymous2.Flags };
        let mut unicast_ptr = adapter.FirstUnicastAddress.cast_const();
        while !unicast_ptr.is_null() {
            // SAFETY: a non-null unicast pointer is the adapter's first entry
            // or a `Next` link, all in the same live buffer.
            let unicast = unsafe { &*unicast_ptr };
            // SAFETY: the OS wrote this socket address with the length it
            // reports, inside the live buffer.
            if let Some(ip) = unsafe { decode_socket_address(&unicast.Address) } {
                addresses.push(UnicastAddress {
                    ip,
                    dad_state: unicast.DadState,
                    ipv6_index: adapter.Ipv6IfIndex,
                    up: adapter.OperStatus == IfOperStatusUp,
                    multicast: flags & IP_ADAPTER_NO_MULTICAST == 0,
                    loopback: adapter.IfType == IF_TYPE_SOFTWARE_LOOPBACK,
                    prefix_length: unicast.OnLinkPrefixLength,
                });
            }
            unicast_ptr = unicast.Next.cast_const();
        }
        adapter_ptr = adapter.Next.cast_const();
    }
    addresses
}

/// The IP address in an IPv4 or IPv6 socket address; `None` for a null
/// pointer, another family, or a length too short for the family's layout.
/// The reads are unaligned, so they assume nothing about where the OS put it.
///
/// # Safety
///
/// A non-null `lpSockaddr` points to at least `iSockaddrLength` readable
/// bytes.
unsafe fn decode_socket_address(address: &SOCKET_ADDRESS) -> Option<IpAddr> {
    let sockaddr = address.lpSockaddr.cast_const();
    let length = usize::try_from(address.iSockaddrLength).ok()?;
    if sockaddr.is_null() || length < size_of::<SOCKADDR>() {
        return None;
    }
    // SAFETY: the pointer is non-null and the `length` readable bytes (the
    // contract) cover a whole `SOCKADDR`, whose first member is the family.
    let family = unsafe { read_unaligned(addr_of!((*sockaddr).sa_family)) };
    match family {
        AF_INET if length >= size_of::<SOCKADDR_IN>() => {
            // SAFETY: the family selects the IPv4 layout, and `length` covers it.
            let v4 = unsafe { read_unaligned(sockaddr.cast::<SOCKADDR_IN>()) };
            // SAFETY: every member of this union is a plain integer view of
            // the same four bytes, which are in network byte order.
            let octets = unsafe { v4.sin_addr.S_un.S_addr }.to_ne_bytes();
            Some(IpAddr::V4(Ipv4Addr::from(octets)))
        }
        AF_INET6 if length >= size_of::<SOCKADDR_IN6>() => {
            // SAFETY: the family selects the IPv6 layout, and `length` covers it.
            let v6 = unsafe { read_unaligned(sockaddr.cast::<SOCKADDR_IN6>()) };
            // SAFETY: both members of this union are plain integer views of the
            // same sixteen bytes; `Byte` is them in network order.
            let octets = unsafe { v6.sin6_addr.u.Byte };
            Some(IpAddr::V6(Ipv6Addr::from(octets)))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Networking::WinSock::AF_UNSPEC;

    fn socket_address(bytes: &mut [u8]) -> SOCKET_ADDRESS {
        SOCKET_ADDRESS {
            lpSockaddr: bytes.as_mut_ptr().cast(),
            iSockaddrLength: i32::try_from(bytes.len()).unwrap(),
        }
    }

    #[test]
    fn decodes_ipv4_and_ipv6_and_rejects_short_or_foreign_addresses() {
        // An IPv4 socket address one byte off alignment.
        let mut v4 = vec![0u8; 1 + size_of::<SOCKADDR_IN>()];
        v4[1..3].copy_from_slice(&AF_INET.to_ne_bytes());
        v4[5..9].copy_from_slice(&[192, 0, 2, 10]);
        let address = socket_address(&mut v4[1..]);
        // SAFETY: the address points into `v4` with its true length.
        let ip = unsafe { decode_socket_address(&address) };
        assert_eq!(ip, Some(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10))));

        let mut v6 = vec![0u8; size_of::<SOCKADDR_IN6>()];
        v6[..2].copy_from_slice(&AF_INET6.to_ne_bytes());
        let documentation: Ipv6Addr = "2001:db8::1".parse().unwrap();
        v6[8..24].copy_from_slice(&documentation.octets());
        // SAFETY: as above.
        let ip = unsafe { decode_socket_address(&socket_address(&mut v6)) };
        assert_eq!(ip, Some(IpAddr::V6(documentation)));

        // Too short for its family, another family, and a null pointer.
        let short = &mut v6[..size_of::<SOCKADDR_IN6>() - 1];
        // SAFETY: as above.
        assert_eq!(
            unsafe { decode_socket_address(&socket_address(short)) },
            None
        );
        let mut unspec = vec![0u8; size_of::<SOCKADDR_IN6>()];
        unspec[..2].copy_from_slice(&AF_UNSPEC.to_ne_bytes());
        // SAFETY: as above.
        assert_eq!(
            unsafe { decode_socket_address(&socket_address(&mut unspec)) },
            None
        );
        let null = SOCKET_ADDRESS {
            lpSockaddr: std::ptr::null_mut(),
            iSockaddrLength: 16,
        };
        // SAFETY: a null pointer is allowed.
        assert_eq!(unsafe { decode_socket_address(&null) }, None);
    }

    #[test]
    fn lists_loopback_and_the_default_route_address_on_their_adapters() {
        use windows_sys::Win32::Networking::WinSock::IpDadStatePreferred;

        let addresses = unicast_addresses(AF_INET).unwrap();
        assert!(
            addresses
                .iter()
                .any(|a| a.ip == IpAddr::V4(Ipv4Addr::LOCALHOST) && a.loopback && a.up),
            "{addresses:?}"
        );
        assert!(addresses.iter().all(|a| a.ip.is_ipv4()), "{addresses:?}");
        // The default-route address sits on a real adapter that is up and has
        // finished duplicate address detection, so a decode or state-mapping
        // regression fails here. A host without a default route skips this.
        if let Some(route) = crate::local_addresses::route_ipv4() {
            assert!(
                addresses.iter().any(|a| a.ip == IpAddr::V4(route)
                    && !a.loopback
                    && a.up
                    && a.dad_state == IpDadStatePreferred),
                "default-route address {route} missing from {addresses:?}"
            );
        }
    }
}
