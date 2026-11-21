//! Select an explicitly named interface for a B/IP bind.

use super::{ReportedAddress, host_ipv4};
use std::{io, net::Ipv4Addr};

/// Resolve a B/IP interface supplied as an IPv4 literal or exact OS interface
/// name (the adapter's friendly name on Windows). Literals, including
/// `0.0.0.0`, bypass enumeration. A name must have exactly one usable
/// unicast IPv4 address on an interface that is up; a link-local address
/// (169.254.0.0/16) counts only when the interface has no other. An
/// ambiguity error lists the addresses so the caller can choose a literal.
/// Text that names no interface but looks like a mistyped address
/// (`"10.0.0.300"`, `"127.0.0.1/8"`, `"::1"`) is reported as one.
///
/// Name lookup queries the OS synchronously. Async callers should run it on
/// a blocking thread. The result selects an address for the existing bind
/// policy; it does not bind a socket to a device, and a later address change
/// on the interface is not followed.
pub fn resolve_interface_ipv4(interface: &str) -> io::Result<Ipv4Addr> {
    if let Ok(ip) = interface.parse() {
        return Ok(ip);
    }
    select(interface, named_addresses(interface)?)
}

/// Whether `text`, which is not an IPv4 literal, was likely meant as an
/// address: it starts with a digit, carries a prefix length, or is an IPv6
/// address. Asked only once no interface has that name.
fn looks_like_an_address(text: &str) -> bool {
    text.starts_with(|c: char| c.is_ascii_digit())
        || text.contains('/')
        || text.parse::<std::net::Ipv6Addr>().is_ok()
}

fn select(name: &str, candidates: Option<Vec<ReportedAddress>>) -> io::Result<Ipv4Addr> {
    let records = candidates.ok_or_else(|| {
        if looks_like_an_address(name) {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{name:?} is not a valid IPv4 address, nor a network interface's name"),
            )
        } else {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("unknown network interface {name:?}"),
            )
        }
    })?;
    let mut addresses = host_ipv4(records);
    // A link-local address beside a routable one is usually an
    // autoconfigured alias: the routable one is meant.
    if addresses.iter().any(|ip| !ip.is_link_local()) {
        addresses.retain(|ip| !ip.is_link_local());
    }
    match addresses.as_slice() {
        [ip] => Ok(*ip),
        [] => Err(io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            format!("network interface {name:?} has no usable IPv4 address"),
        )),
        many => {
            let choices = many
                .iter()
                .map(Ipv4Addr::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "network interface {name:?} has multiple IPv4 addresses. Specify one of {choices}"
                ),
            ))
        }
    }
}

with_getifaddrs! {
    fn named_addresses(name: &str) -> io::Result<Option<Vec<ReportedAddress>>> {
        use crate::sys::ifaddrs::{interface_addresses, interface_index};
        let Some(index) = interface_index(name)? else { return Ok(None); };
        Ok(Some(unix_candidates(index, interface_addresses()?)))
    }

    /// The IPv4 addresses of the interface with `index` among `entries`,
    /// usable only while that interface is up.
    fn unix_candidates(
        index: u32,
        entries: Vec<crate::sys::ifaddrs::InterfaceAddress>,
    ) -> Vec<ReportedAddress> {
        use crate::sys::ifaddrs::Address;
        entries.into_iter().filter_map(|entry| match entry.address {
            Address::V4 { ip, .. } if entry.index == Some(index) => Some(ReportedAddress {
                ip, assigned: entry.flags.up,
            }),
            _ => None,
        }).collect()
    }
}

#[cfg(windows)]
fn named_addresses(name: &str) -> io::Result<Option<Vec<ReportedAddress>>> {
    Ok(windows_candidates(
        name,
        crate::sys::win_adapters::named_adapters()?,
    ))
}

#[cfg(windows)]
fn windows_candidates(
    name: &str,
    adapters: Vec<crate::sys::win_adapters::Adapter>,
) -> Option<Vec<ReportedAddress>> {
    use std::net::IpAddr;
    use windows_sys::Win32::Networking::WinSock::{IpDadStateDeprecated, IpDadStatePreferred};
    let mut found = None;
    for adapter in adapters {
        if adapter.name.as_deref() == Some(name) {
            let records = found.get_or_insert_with(Vec::new);
            for address in adapter.addresses {
                if let IpAddr::V4(ip) = address.ip {
                    records.push(ReportedAddress {
                        ip,
                        assigned: address.up
                            && (address.dad_state == IpDadStatePreferred
                                || address.dad_state == IpDadStateDeprecated),
                    });
                }
            }
        }
    }
    found
}

without_interface_list! {
    fn named_addresses(_: &str) -> io::Result<Option<Vec<ReportedAddress>>> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "interface name lookup is unavailable on this platform"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(target_os = "linux", target_vendor = "apple"))]
    #[test]
    fn resolves_the_loopback_name() {
        let name = if cfg!(target_vendor = "apple") {
            "lo0"
        } else {
            "lo"
        };
        let candidates = named_addresses(name).unwrap().unwrap();
        assert!(
            candidates
                .iter()
                .any(|row| row.assigned && row.ip == Ipv4Addr::LOCALHOST)
        );
        // Hosts may configure more than one address on their loopback.
        assert_eq!(
            resolve_interface_ipv4(name).ok(),
            select(name, Some(candidates)).ok()
        );
        assert_eq!(
            resolve_interface_ipv4("__unknown_nic__")
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn an_address_label_is_not_an_interface_name() {
        assert_eq!(
            resolve_interface_ipv4("lo:zz").unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }

    #[cfg(windows)]
    #[test]
    fn resolves_the_reported_windows_loopback_friendly_name() {
        let name = crate::sys::win_adapters::named_adapters()
            .unwrap()
            .into_iter()
            .find(|adapter| {
                adapter
                    .addresses
                    .iter()
                    .any(|row| row.loopback && row.ip == Ipv4Addr::LOCALHOST)
            })
            .unwrap()
            .name
            .unwrap();
        let rows = named_addresses(&name).unwrap().unwrap();
        assert!(
            rows.iter()
                .any(|row| row.assigned && row.ip == Ipv4Addr::LOCALHOST)
        );
        assert_eq!(
            resolve_interface_ipv4(&name).ok(),
            select(&name, Some(rows)).ok()
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_name_matching_retains_empty_adapters_and_checks_operational_and_dad_state() {
        use crate::sys::win_adapters::{Adapter, UnicastAddress};
        use windows_sys::Win32::Networking::WinSock::*;
        let ip = Ipv4Addr::LOCALHOST;
        let adapter = |name: &str, up: bool, states: &[NL_DAD_STATE]| Adapter {
            name: Some(name.to_owned()),
            addresses: states
                .iter()
                .map(|&dad_state| UnicastAddress {
                    ip: ip.into(),
                    dad_state,
                    up,
                    ipv6_index: 0,
                    multicast: false,
                    loopback: true,
                    prefix_length: 8,
                })
                .collect(),
        };
        assert!(windows_candidates("LAN", vec![adapter("lan", true, &[])]).is_none());
        assert!(
            windows_candidates("LAN", vec![adapter("LAN", true, &[])])
                .unwrap()
                .is_empty()
        );
        let states = [
            IpDadStateTentative,
            IpDadStateInvalid,
            IpDadStateDuplicate,
            IpDadStateDeprecated,
            IpDadStatePreferred,
        ];
        let rows = windows_candidates(
            "LAN",
            vec![
                adapter("LAN", true, &states),
                adapter("LAN", false, &[IpDadStatePreferred]),
            ],
        )
        .unwrap();
        assert_eq!(
            rows.iter().map(|row| row.assigned).collect::<Vec<_>>(),
            [false, false, false, true, true, false]
        );
    }

    #[test]
    fn selection_distinguishes_absence_and_usable_unicast_candidates() {
        let name = "LAN";
        let row = |ip, assigned| ReportedAddress { ip, assigned };
        assert_eq!(
            select(name, None).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        let excluded = vec![
            row(Ipv4Addr::LOCALHOST, false),
            row(Ipv4Addr::UNSPECIFIED, true),
            row(Ipv4Addr::BROADCAST, true),
            row(Ipv4Addr::new(239, 1, 2, 3), true),
        ];
        assert_eq!(
            select(name, Some(excluded)).unwrap_err().kind(),
            io::ErrorKind::AddrNotAvailable
        );
        let local = row(Ipv4Addr::LOCALHOST, true);
        assert_eq!(select(name, Some(vec![local, local])).unwrap(), local.ip);
        let other = row(Ipv4Addr::new(10, 0, 0, 5), true);
        let error = select(name, Some(vec![other, local, other])).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().ends_with("10.0.0.5, 127.0.0.1"));
        assert_eq!(
            resolve_interface_ipv4("0.0.0.0").unwrap(),
            Ipv4Addr::UNSPECIFIED
        );
    }

    /// A link-local address beside a routable one gives way to it; alone,
    /// it is used; several, with no routable one, stay ambiguous.
    #[test]
    fn a_routable_address_is_preferred_to_a_link_local_one() {
        let row = |octets: [u8; 4]| ReportedAddress {
            ip: Ipv4Addr::from(octets),
            assigned: true,
        };
        let (routable, apipa) = (row([192, 168, 1, 20]), row([169, 254, 7, 8]));
        let pick = |rows: Vec<ReportedAddress>| select("eth0", Some(rows));
        assert_eq!(pick(vec![apipa, routable]).unwrap(), routable.ip);
        assert_eq!(pick(vec![apipa]).unwrap(), apipa.ip);
        let error = pick(vec![apipa, row([169, 254, 9, 9])]).unwrap_err();
        assert!(error.to_string().ends_with("169.254.7.8, 169.254.9.9"));
        let error = pick(vec![apipa, routable, row([10, 0, 0, 5])]).unwrap_err();
        assert!(error.to_string().ends_with("10.0.0.5, 192.168.1.20"));
    }

    /// Text that names no interface but looks like an address is reported
    /// as a bad address; an unknown name stays unknown.
    #[test]
    fn a_mistyped_address_is_reported_as_one() {
        for text in ["10.0.0.300", "127.0.0.1/8", "::1", "fe80::1"] {
            let error = select(text, None).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{text}");
            let message = error.to_string();
            assert!(message.contains("not a valid IPv4 address"), "{message}");
        }
        for name in ["eth0", "lo:zz", "Ethernet 2"] {
            let error = select(name, None).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::NotFound, "{name}");
        }
    }

    with_getifaddrs! {
    /// Only the named interface's IPv4 addresses are candidates, and only
    /// while it is up: a test of the selection itself, on made-up entries.
    #[test]
    fn a_unix_interface_counts_only_while_it_is_up() {
        use crate::sys::ifaddrs::{Address, Flags, InterfaceAddress};
        let entry = |octets: [u8; 4], index, up| InterfaceAddress {
            address: Address::V4 {
                ip: Ipv4Addr::from(octets),
                netmask: None,
            },
            flags: Flags {
                up,
                ..Flags::default()
            },
            index: Some(index),
        };
        let v6 = InterfaceAddress {
            address: Address::V6(std::net::Ipv6Addr::LOCALHOST),
            ..entry([0; 4], 2, true)
        };
        let entries = vec![
            entry([192, 0, 2, 10], 2, true),
            entry([198, 51, 100, 7], 3, true),
            v6,
        ];
        let rows = unix_candidates(2, entries);
        assert_eq!(rows.len(), 1);
        let expected = (Ipv4Addr::new(192, 0, 2, 10), true);
        assert_eq!((rows[0].ip, rows[0].assigned), expected);
        let down = unix_candidates(2, vec![entry([192, 0, 2, 10], 2, false)]);
        assert_eq!(
            select("eth1", Some(down)).unwrap_err().kind(),
            io::ErrorKind::AddrNotAvailable
        );
    }
    }
}
