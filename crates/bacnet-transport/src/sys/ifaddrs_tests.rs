use super::*;

/// A BSD-layout IPv4 socket address with `length` and `family`, carrying
/// `mask`, cut to the octets the walker would read for it.
fn bsd_mask(length: u8, family: i32, mask: [u8; 4]) -> Vec<u8> {
    let mut raw = vec![length, u8::try_from(family).unwrap(), 0, 0];
    raw.extend(mask);
    raw.truncate(usize::from(length).clamp(2, 8));
    raw
}

#[test]
fn a_whole_bsd_netmask_decodes_as_is() {
    let raw = bsd_mask(16, libc::AF_INET, [255, 255, 255, 0]);
    assert_eq!(truncated_netmask(&raw), Some(Ipv4Addr::new(255, 255, 255, 0)));
    // Octets past the address, as a caller holding the whole sockaddr has,
    // change nothing.
    let mut whole = raw.clone();
    whole.extend([0xaa; 8]);
    assert_eq!(truncated_netmask(&whole), Some(Ipv4Addr::new(255, 255, 255, 0)));
}

#[test]
fn a_truncated_bsd_netmask_without_a_family_recovers_its_zero_octets() {
    for (length, mask) in [
        (8, Ipv4Addr::new(255, 255, 255, 252)),
        (7, Ipv4Addr::new(255, 255, 255, 0)),
        (6, Ipv4Addr::new(255, 255, 0, 0)),
        (5, Ipv4Addr::new(255, 0, 0, 0)),
    ] {
        let raw = bsd_mask(length, libc::AF_UNSPEC, mask.octets());
        assert_eq!(raw.len(), usize::from(length));
        assert_eq!(truncated_netmask(&raw), Some(mask), "length {length}");
    }
    // A length that stops short of the address leaves octets the caller
    // holds out of the mask.
    let mut raw = bsd_mask(6, libc::AF_UNSPEC, [255, 255, 0, 0]);
    raw.extend([255, 255]);
    assert_eq!(truncated_netmask(&raw), Some(Ipv4Addr::new(255, 255, 0, 0)));
}

#[test]
fn a_zero_length_bsd_netmask_is_all_zero_and_a_foreign_family_is_none() {
    for length in [0, 1, 2, 3, 4] {
        let raw = bsd_mask(length, libc::AF_UNSPEC, [255; 4]);
        assert_eq!(
            truncated_netmask(&raw),
            Some(Ipv4Addr::UNSPECIFIED),
            "length {length}"
        );
    }
    let raw = bsd_mask(16, libc::AF_INET6, [255; 4]);
    assert_eq!(truncated_netmask(&raw), None);
    assert_eq!(truncated_netmask(&[]), None);
    assert_eq!(truncated_netmask(&[8]), None);
}

#[test]
fn flags_decode_from_32_and_64_bit_words() {
    let bit = |flag: libc::c_int| u32::try_from(flag).unwrap();
    assert_eq!(Flags::from_raw(0u32), Flags::default());
    let each = [
        (libc::IFF_UP, Flags { up: true, ..Flags::default() }),
        (
            libc::IFF_BROADCAST,
            Flags {
                broadcast: true,
                ..Flags::default()
            },
        ),
        (
            libc::IFF_MULTICAST,
            Flags {
                multicast: true,
                ..Flags::default()
            },
        ),
        (
            libc::IFF_LOOPBACK,
            Flags {
                loopback: true,
                ..Flags::default()
            },
        ),
    ];
    for (flag, expected) in each {
        assert_eq!(Flags::from_raw(bit(flag)), expected, "{flag:#x}");
        // illumos and Solaris report 64 bits, some above the 32 here.
        assert_eq!(
            Flags::from_raw(u64::from(bit(flag)) | 1 << 40),
            expected,
            "{flag:#x}"
        );
    }
    let all = bit(libc::IFF_UP) | bit(libc::IFF_MULTICAST) | bit(libc::IFF_LOOPBACK);
    assert_eq!(
        Flags::from_raw(all),
        Flags {
            up: true,
            broadcast: false,
            multicast: true,
            loopback: true,
        }
    );
}

#[test]
fn the_walk_reports_both_loopback_addresses_on_one_interface() {
    let entries = interface_addresses().unwrap();
    let v4 = entries
        .iter()
        .find(|entry| matches!(entry.address, Address::V4 { ip, .. } if ip.is_loopback()))
        .unwrap_or_else(|| panic!("no IPv4 loopback in {entries:?}"));
    assert_eq!(
        v4.address,
        Address::V4 {
            ip: Ipv4Addr::LOCALHOST,
            netmask: Some(Ipv4Addr::new(255, 0, 0, 0)),
        }
    );
    assert!(v4.flags.up && v4.flags.loopback && !v4.flags.broadcast, "{v4:?}");
    // `::1` is required, not skipped when absent: the crate's other tests
    // (udp_metadata, bip6) already bind it unconditionally, so a host with
    // IPv6 disabled fails those first.
    let v6 = entries
        .iter()
        .find(|entry| entry.address == Address::V6(Ipv6Addr::LOCALHOST))
        .unwrap_or_else(|| panic!("no ::1 in {entries:?}"));
    assert!(v6.flags.up && v6.flags.loopback, "{v6:?}");
    assert!(v4.index.is_some_and(|index| index != 0), "{v4:?}");
    assert_eq!(v4.index, v6.index);
}

/// nix walks the same list. Where it decodes a field, the walk agrees; the
/// only field it may miss is a netmask Apple or the BSDs cut short.
///
/// On the BSDs this call makes nix's over-read of a short IPv4 netmask (see
/// the module doc). It is test-only, and the extra octets it reads stay
/// within the one buffer libc's getifaddrs allocates for the list.
#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_vendor = "apple",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd",
))]
#[test]
fn the_walk_agrees_with_nix() {
    use nix::net::if_::{if_nametoindex, InterfaceFlags};

    let truncating = cfg!(any(
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd",
    ));
    let theirs: Vec<_> = nix::ifaddrs::getifaddrs()
        .unwrap()
        .filter_map(|entry| {
            let address = entry.address.as_ref()?;
            let address = match (address.as_sockaddr_in(), address.as_sockaddr_in6()) {
                (Some(v4), _) => Address::V4 {
                    ip: v4.ip(),
                    netmask: entry
                        .netmask
                        .as_ref()
                        .and_then(|mask| mask.as_sockaddr_in())
                        .map(|mask| mask.ip()),
                },
                (None, Some(v6)) => Address::V6(v6.ip()),
                (None, None) => return None,
            };
            let flags = Flags {
                up: entry.flags.contains(InterfaceFlags::IFF_UP),
                broadcast: entry.flags.contains(InterfaceFlags::IFF_BROADCAST),
                multicast: entry.flags.contains(InterfaceFlags::IFF_MULTICAST),
                loopback: entry.flags.contains(InterfaceFlags::IFF_LOOPBACK),
            };
            let index = Some(if_nametoindex(entry.interface_name.as_str()).unwrap_or(0));
            Some(InterfaceAddress {
                address,
                flags,
                index,
            })
        })
        .collect();
    let ours = interface_addresses().unwrap();
    assert_eq!(ours.len(), theirs.len(), "{ours:?}\n{theirs:?}");
    for (ours, theirs) in ours.iter().zip(&theirs) {
        let mut ours = *ours;
        if let (
            Address::V4 { netmask, .. },
            Address::V4 {
                netmask: None, ..
            },
        ) = (&mut ours.address, theirs.address)
        {
            assert!(truncating || netmask.is_none(), "{ours:?}");
            *netmask = None;
        }
        assert_eq!(ours, *theirs);
    }
}
