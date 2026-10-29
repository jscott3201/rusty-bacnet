use super::*;

fn candidate(address: &str, index: u32) -> Candidate {
    Candidate {
        link: SelectedLink {
            address: address.parse().unwrap(),
            index,
        },
        up: true,
        multicast: true,
        loopback: address == "::1",
    }
}

#[test]
fn auto_selects_unique_physical_link_and_prefers_unique_non_link_local() {
    let ula = candidate("fd12::1", 3);
    let entries = [candidate("::1", 1), candidate("fe80::1", 3), ula, ula];
    assert_eq!(select(Ipv6Addr::UNSPECIFIED, &entries).unwrap(), ula.link);
    assert_eq!(
        select("fe80::1".parse().unwrap(), &entries).unwrap().index,
        3
    );
}

#[test]
fn auto_and_explicit_reject_different_ambiguities_without_default_route_guessing() {
    let entries = [candidate("fd12::1", 3), candidate("fd12::2", 4)];
    assert!(select(Ipv6Addr::UNSPECIFIED, &entries).is_err());
    assert_eq!(
        select("fd12::2".parse().unwrap(), &entries).unwrap().index,
        4
    );
    assert!(
        select(
            "fd12::1".parse().unwrap(),
            &[entries[0], candidate("fd12::1", 4)]
        )
        .is_err()
    );
    assert!(
        select(
            Ipv6Addr::UNSPECIFIED,
            &[entries[0], candidate("fd12::2", 3)]
        )
        .is_err()
    );
    assert!(select("fd12::9".parse().unwrap(), &entries).is_err());
}

#[test]
fn only_link_local_retains_zone_and_loopback_is_an_explicit_or_last_choice() {
    let local = candidate("fe80::1", 3);
    assert_eq!(select(Ipv6Addr::UNSPECIFIED, &[local]).unwrap(), local.link);
    let mut loopback = candidate("::1", 1);
    loopback.multicast = false;
    assert_eq!(
        select(Ipv6Addr::UNSPECIFIED, &[loopback]).unwrap(),
        loopback.link
    );
    assert_eq!(
        select(Ipv6Addr::LOCALHOST, &[loopback, local]).unwrap(),
        loopback.link
    );
}

#[test]
fn unavailable_and_non_unicast_candidates_never_become_selected_identity() {
    let mut down = candidate("fd12::1", 3);
    down.up = false;
    let mut no_multicast = candidate("fd12::2", 4);
    no_multicast.multicast = false;
    for entry in [
        down,
        no_multicast,
        candidate("fd12::3", 0),
        candidate("::", 3),
        candidate("ff05::bac0", 3),
        candidate("::ffff:192.0.2.1", 3),
    ] {
        assert!(select(Ipv6Addr::UNSPECIFIED, &[entry]).is_err());
        assert!(select(entry.link.address, &[entry]).is_err());
    }
}
