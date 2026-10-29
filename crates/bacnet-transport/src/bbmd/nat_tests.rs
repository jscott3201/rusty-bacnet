//! A hosted BBMD behind a NAT router (#940): its setting, and the
//! BACnet_IP_NAT_Traversal and BACnet_IP_Global_Address values written to
//! the Network Port, checked and activated with the BDT.

use std::net::{IpAddr, Ipv4Addr, SocketAddrV4};
use std::time::Instant;

use bacnet_types::bip_port::BbmdChanges;
use bacnet_types::constructed::{BACnetBDTEntry, BACnetHostAddress, BACnetHostNPort};

use super::*;

const LOCAL: [u8; 4] = [192, 168, 1, 10];
const PORT: u16 = 47809;
const GLOBAL: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 1), PORT);
const OTHER_SITE: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(198, 51, 100, 2), PORT);

fn row(address: SocketAddrV4, mask: [u8; 4]) -> BdtEntry {
    BdtEntry {
        ip: address.ip().octets(),
        port: address.port(),
        broadcast_mask: mask,
    }
}

fn two_hop(address: SocketAddrV4) -> BdtEntry {
    row(address, [255; 4])
}

fn host(address: SocketAddrV4) -> BACnetHostNPort {
    BACnetHostNPort {
        host: BACnetHostAddress::Ip(IpAddr::V4(*address.ip())),
        port: address.port(),
    }
}

const UNSET: BACnetHostNPort = BACnetHostNPort {
    host: BACnetHostAddress::None,
    port: 0,
};

fn written(rows: &[BdtEntry]) -> Vec<BACnetBDTEntry> {
    rows.iter()
        .map(|row| BACnetBDTEntry {
            bbmd_address: host(SocketAddrV4::new(Ipv4Addr::from(row.ip), row.port)),
            broadcast_mask: Some(row.broadcast_mask),
        })
        .collect()
}

/// A BBMD behind NAT at [`GLOBAL`], with both sites in its BDT.
fn behind_nat() -> BbmdState {
    let mut state = BbmdState::new(LOCAL, PORT);
    state
        .set_nat_traversal(Some(NatTraversal::new(GLOBAL)))
        .unwrap();
    state
        .set_bdt(vec![two_hop(GLOBAL), two_hop(OTHER_SITE)])
        .unwrap();
    state
}

#[test]
fn behind_nat_the_own_row_is_global_and_the_socket_stays_local() {
    let state = behind_nat();
    assert!(state.nat_traversal());
    assert_eq!(state.global_address(), Some(GLOBAL));
    assert_eq!(state.own_address(), GLOBAL);
    assert_eq!(state.local_pair(), (LOCAL, PORT));
    assert_eq!(state.bdt(), [two_hop(GLOBAL), two_hop(OTHER_SITE)]);
    assert_eq!(state.global_address_value(), host(GLOBAL));
    // Without NAT the value is the unset sentinel (Clause 12.56.33).
    let plain = BbmdState::new(LOCAL, PORT);
    assert!(!plain.nat_traversal());
    assert_eq!(plain.global_address_value(), UNSET);
}

#[test]
fn behind_nat_the_bdt_rules_refuse_with_reasons() {
    let mut state = behind_nat();
    let missing = state.set_bdt(vec![two_hop(OTHER_SITE)]).unwrap_err();
    assert!(missing.to_string().contains("global address"), "{missing}");
    let one_hop = state
        .set_bdt(vec![two_hop(GLOBAL), row(OTHER_SITE, [255, 255, 255, 0])])
        .unwrap_err();
    assert!(one_hop.to_string().contains("all ones"), "{one_hop}");
    assert_eq!(state.bdt(), [two_hop(GLOBAL), two_hop(OTHER_SITE)]);
}

#[test]
fn a_global_address_no_peer_could_reach_is_refused() {
    let mut state = BbmdState::new(LOCAL, PORT);
    let multicast = SocketAddrV4::new(Ipv4Addr::new(239, 1, 1, 1), PORT);
    let error = state
        .set_nat_traversal(Some(NatTraversal::new(multicast)))
        .unwrap_err();
    assert!(error.to_string().contains("NAT global address"), "{error}");
    assert!(!state.nat_traversal());
    let mut nat = NatTraversal::new(GLOBAL);
    for octet in 0..=NatTraversal::MAX_PRIVATE_RANGES {
        let octet = u8::try_from(octet).unwrap();
        nat.private_ranges
            .push(PrivateRange::new(Ipv4Addr::new(100, 64, octet, 0), 24).unwrap());
    }
    assert!(state.set_nat_traversal(Some(nat)).is_err());
    assert_eq!(state.global_address(), None);
}

#[test]
fn a_restart_moves_the_socket_but_not_the_global_row() {
    let mut state = behind_nat();
    state.set_local_address([192, 168, 1, 11], PORT).unwrap();
    assert_eq!(state.local_pair(), ([192, 168, 1, 11], PORT));
    assert_eq!(state.own_address(), GLOBAL);
    assert_eq!(state.bdt(), [two_hop(GLOBAL), two_hop(OTHER_SITE)]);
}

/// The DBTN rule reaches the host: a private foreign device's broadcast
/// goes on from the global address, a public one's from its own.
#[test]
fn the_hosts_forwarding_uses_the_cores_origin_rule() {
    let mut state = behind_nat();
    state.set_foreign_device_policy(Some(ForeignDevicePolicy::default()));
    let now = Instant::now();
    for device in [[192, 168, 1, 40], [198, 51, 100, 7]] {
        assert_eq!(
            state.register_foreign_device_at(device, 47808, 60, now),
            BvlcResultCode::SUCCESSFUL_COMPLETION
        );
    }
    let broadcast = SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 255), PORT);
    let inside = SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 40), 47808);
    let roaming = SocketAddrV4::new(Ipv4Addr::new(198, 51, 100, 7), 47808);
    let sent = state
        .forward_distribute_broadcast(inside, &[1, 0], broadcast, now)
        .unwrap();
    assert_eq!(sent.origin, GLOBAL);
    let sent = state
        .forward_distribute_broadcast(roaming, &[1, 0], broadcast, now)
        .unwrap();
    assert_eq!(sent.origin, roaming);
    let local_device = SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 30), PORT);
    let sent = state.forward_original_broadcast(local_device, &[1, 0], now);
    assert_eq!(sent.origin, GLOBAL);
    assert_eq!(state.forward_own_broadcast(&[1, 0], now).origin, GLOBAL);
}

// --- Network Port writes and activation ------------------------------------

#[test]
fn nat_and_its_bdt_activate_together() {
    let mut state = BbmdState::new(LOCAL, PORT);
    state.set_bdt(vec![two_hop(OTHER_SITE)]).unwrap();
    let mut changes = BbmdChanges::default();
    changes.nat_traversal = Some(true);
    changes.global_address = Some(host(GLOBAL));
    // Without the global row the pair is refused, and nothing changes.
    assert!(state.check_changes(&changes).is_err());
    assert!(state.activate(&changes).is_err());
    assert!(!state.nat_traversal());
    assert_eq!(state.global_address(), None);
    // With it, checked first on a copy, then taken.
    changes.broadcast_distribution_table = Some(written(&[two_hop(GLOBAL), two_hop(OTHER_SITE)]));
    state.check_changes(&changes).unwrap();
    assert!(!state.nat_traversal(), "a check changes nothing");
    state.activate(&changes).unwrap();
    assert!(state.nat_traversal());
    assert_eq!(state.own_address(), GLOBAL);
    assert_eq!(state.bdt(), [two_hop(GLOBAL), two_hop(OTHER_SITE)]);
}

#[test]
fn nat_without_a_global_address_is_refused() {
    let mut state = BbmdState::new(LOCAL, PORT);
    let mut changes = BbmdChanges::default();
    changes.nat_traversal = Some(true);
    assert!(state.check_changes(&changes).is_err());
    assert!(state.activate(&changes).is_err());
    // Clearing the address of a BBMD behind NAT is refused too.
    let mut state = behind_nat();
    let mut clear = BbmdChanges::default();
    clear.global_address = Some(UNSET);
    assert!(state.activate(&clear).is_err());
    assert_eq!(state.global_address(), Some(GLOBAL));
}

#[test]
fn a_global_address_is_an_ipv4_host_and_port_or_the_unset_sentinel() {
    let state = BbmdState::new(LOCAL, PORT);
    for refused in [
        BACnetHostNPort {
            host: BACnetHostAddress::Name("nat.example".into()),
            port: PORT,
        },
        BACnetHostNPort {
            host: BACnetHostAddress::Ip(IpAddr::V6(std::net::Ipv6Addr::LOCALHOST)),
            port: PORT,
        },
        BACnetHostNPort {
            host: BACnetHostAddress::Ip(IpAddr::V4(*GLOBAL.ip())),
            port: 0,
        },
        BACnetHostNPort {
            host: BACnetHostAddress::None,
            port: PORT,
        },
    ] {
        let mut changes = BbmdChanges::default();
        changes.global_address = Some(refused.clone());
        assert!(state.check_changes(&changes).is_err(), "{refused:?}");
    }
    // With NAT off, a global address is kept for later, and cleared again.
    let mut state = BbmdState::new(LOCAL, PORT);
    let mut set = BbmdChanges::default();
    set.global_address = Some(host(GLOBAL));
    state.activate(&set).unwrap();
    assert_eq!(state.global_address(), Some(GLOBAL));
    assert!(!state.nat_traversal());
    assert_eq!(state.own_address(), SocketAddrV4::new(LOCAL.into(), PORT));
    let mut clear = BbmdChanges::default();
    clear.global_address = Some(UNSET);
    state.activate(&clear).unwrap();
    assert_eq!(state.global_address(), None);
}

#[test]
fn turning_nat_off_returns_to_the_local_own_row_and_drops_the_global_one() {
    let mut state = behind_nat();
    let mut off = BbmdChanges::default();
    off.nat_traversal = Some(false);
    state.activate(&off).unwrap();
    assert!(!state.nat_traversal());
    assert_eq!(state.global_address(), Some(GLOBAL), "kept");
    assert_eq!(state.own_address(), SocketAddrV4::new(LOCAL.into(), PORT));
    let local_row = two_hop(SocketAddrV4::new(LOCAL.into(), PORT));
    // The global row named this BBMD; it would read as a peer now.
    assert_eq!(state.bdt(), [two_hop(OTHER_SITE), local_row.clone()]);
    // The same through set_nat_traversal.
    let mut state = behind_nat();
    state.set_nat_traversal(None).unwrap();
    assert_eq!(state.bdt(), [two_hop(OTHER_SITE), local_row]);
}

/// Written values are checked alone at write time; the rules that tie
/// them together wait for activation (#940).
#[test]
fn a_written_value_is_checked_alone() {
    let state = behind_nat();
    // A BDT without the global row passes alone, and fails at activation.
    let mut table = BbmdChanges::default();
    table.broadcast_distribution_table = Some(written(&[two_hop(OTHER_SITE)]));
    state.check_value(&table).unwrap();
    assert!(state.check_changes(&table).is_err());
    // A one-hop row too.
    let mut one_hop = BbmdChanges::default();
    one_hop.broadcast_distribution_table = Some(written(&[
        two_hop(GLOBAL),
        row(OTHER_SITE, [255, 255, 255, 0]),
    ]));
    state.check_value(&one_hop).unwrap();
    assert!(state.check_changes(&one_hop).is_err());
    // The row policy still applies at once.
    let mut multicast = BbmdChanges::default();
    multicast.broadcast_distribution_table = Some(written(&[two_hop(SocketAddrV4::new(
        Ipv4Addr::new(224, 0, 0, 1),
        PORT,
    ))]));
    assert!(state.check_value(&multicast).is_err());
    // A global address must be an IPv4 unicast host and port.
    for refused in [
        Ipv4Addr::UNSPECIFIED,
        Ipv4Addr::BROADCAST,
        Ipv4Addr::new(239, 1, 1, 1),
        Ipv4Addr::LOCALHOST,
    ] {
        let mut global = BbmdChanges::default();
        global.global_address = Some(host(SocketAddrV4::new(refused, PORT)));
        assert!(state.check_value(&global).is_err(), "{refused}");
    }
    let mut global = BbmdChanges::default();
    global.global_address = Some(host(SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 9), PORT)));
    state.check_value(&global).unwrap();
}

/// Behind NAT, a new global address and the BDT that lists it activate
/// together, whichever was written first.
#[test]
fn the_global_address_moves_while_nat_is_on() {
    let mut state = behind_nat();
    let moved = SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 9), PORT);
    let mut alone = BbmdChanges::default();
    alone.global_address = Some(host(moved));
    assert!(state.check_changes(&alone).is_err(), "the BDT lacks it");
    let mut both = alone.clone();
    both.broadcast_distribution_table = Some(written(&[two_hop(moved), two_hop(OTHER_SITE)]));
    state.check_changes(&both).unwrap();
    state.activate(&both).unwrap();
    assert_eq!(state.own_address(), moved);
    assert_eq!(state.bdt(), [two_hop(moved), two_hop(OTHER_SITE)]);
}

/// Behind NAT an empty BDT is taken: a BBMD with no peers that serves only
/// foreign devices.
#[test]
fn behind_nat_an_empty_bdt_is_taken() {
    let mut state = behind_nat();
    state.set_bdt(Vec::new()).unwrap();
    assert!(state.bdt().is_empty());
    let mut empty = BbmdChanges::default();
    empty.broadcast_distribution_table = Some(Vec::new());
    state.check_changes(&empty).unwrap();
}

#[test]
fn a_written_bdt_is_checked_under_the_nat_rules() {
    let state = behind_nat();
    assert!(
        state
            .check_written_bdt(&written(&[two_hop(OTHER_SITE)]))
            .is_err()
    );
    assert!(
        state
            .check_written_bdt(&written(&[
                two_hop(GLOBAL),
                row(OTHER_SITE, [255, 255, 255, 0])
            ]))
            .is_err()
    );
    state
        .check_written_bdt(&written(&[two_hop(GLOBAL)]))
        .unwrap();
}
