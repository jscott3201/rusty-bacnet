//! A BDT written to the Network Port, checked and activated (#1578).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use bacnet_types::bip_port::BbmdChanges;
use bacnet_types::constructed::{BACnetBDTEntry, BACnetHostAddress, BACnetHostNPort};

use super::*;

fn entry(host: BACnetHostAddress, mask: Option<[u8; 4]>) -> BACnetBDTEntry {
    BACnetBDTEntry {
        bbmd_address: BACnetHostNPort { host, port: 0xBAC0 },
        broadcast_mask: mask,
    }
}

fn ipv4(ip: [u8; 4]) -> BACnetHostAddress {
    BACnetHostAddress::Ip(IpAddr::V4(Ipv4Addr::from(ip)))
}

/// No host (Clause 12.56.34), a host name, an IPv6 host, a missing mask
/// and rows the BDT policy refuses are all refused, changing nothing.
#[test]
fn a_bdt_the_bbmd_cannot_take_is_refused() {
    let mut state = BbmdState::new([192, 168, 1, 1], 0xBAC0);
    let before = state.bdt();
    let ipv6 = BACnetHostAddress::Ip(IpAddr::V6(Ipv6Addr::LOCALHOST));
    for table in [
        vec![entry(BACnetHostAddress::None, Some([0xFF; 4]))],
        vec![entry(
            BACnetHostAddress::Name("bbmd.example".into()),
            Some([0xFF; 4]),
        )],
        vec![entry(ipv6, Some([0xFF; 4]))],
        vec![entry(ipv4([10, 0, 0, 2]), None)],
        vec![entry(ipv4([224, 0, 0, 1]), Some([0xFF; 4]))],
        vec![
            entry(ipv4([10, 0, 0, 2]), Some([0xFF; 4])),
            entry(ipv4([10, 0, 0, 2]), Some([255, 255, 255, 0])),
        ],
    ] {
        assert!(state.check_written_bdt(&table).is_err(), "{table:?}");
        let mut changes = BbmdChanges::default();
        changes.broadcast_distribution_table = Some(table);
        changes.accept_foreign_device_registrations = Some(true);
        assert!(state.activate(&changes).is_err());
        assert_eq!(state.bdt(), before);
        assert!(!state.accepts_foreign_device_registrations(), "all or none");
    }
}

#[test]
fn an_activated_bdt_replaces_the_table_and_keeps_the_own_row() {
    let mut state = BbmdState::new([192, 168, 1, 1], 0xBAC0);
    let table = vec![entry(ipv4([10, 0, 0, 2]), Some([0xFF; 4]))];
    state.check_written_bdt(&table).unwrap();
    assert!(state.bdt().is_empty(), "a check changes nothing");
    let mut changes = BbmdChanges::default();
    changes.broadcast_distribution_table = Some(table);
    state.activate(&changes).unwrap();
    let rows: Vec<_> = state.bdt().iter().map(|row| row.ip).collect();
    assert_eq!(rows, [[10, 0, 0, 2], [192, 168, 1, 1]]);
}

/// Turning registrations off keeps the policy for when they are turned on
/// again; with none kept, the default policy applies.
#[test]
fn accepting_registrations_parks_and_restores_the_policy() {
    let mut state = BbmdState::new([192, 168, 1, 1], 0xBAC0);
    state.set_accepts_foreign_device_registrations(true);
    assert_eq!(
        state.foreign_device_policy(),
        Some(&ForeignDevicePolicy::default())
    );
    let policy = ForeignDevicePolicy {
        max_ttl: 600,
        ..ForeignDevicePolicy::default()
    };
    state.set_foreign_device_policy(Some(policy.clone()));
    state.set_accepts_foreign_device_registrations(false);
    assert!(!state.accepts_foreign_device_registrations());
    assert_eq!(
        state.register_foreign_device([10, 0, 0, 5], 0xBAC0, 60),
        BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK
    );
    state.set_accepts_foreign_device_registrations(true);
    assert_eq!(state.foreign_device_policy(), Some(&policy));
}
