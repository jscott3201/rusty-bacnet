//! The router a known routed row names (#2823): an I-Am through another
//! station leaves the row as it was and marks its path for a check; only
//! the check's own `reroute` moves it, and an explicit registration sets
//! it. The paths waiting are ranked, bounded and paced here.

use super::*;
use bacnet_types::enums::ObjectType;

const ROUTER: [u8; 1] = [0x10];
const OTHER: [u8; 1] = [0x20];

fn routed(instance: u32, network: u16, station: &[u8], router: &[u8]) -> DiscoveredDevice {
    DiscoveredDevice {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap(),
        mac_address: MacAddr::from_slice(router),
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::SEGMENTED_BOTH,
        max_segments_accepted: None,
        vendor_id: 42,
        last_seen: now(),
        last_announced: Some(now()),
        source_network: Some(network),
        source_address: Some(MacAddr::from_slice(station)),
    }
}

fn router_at(table: &DeviceTable, network: u16, station: &[u8]) -> Vec<u8> {
    table.get(network, station).unwrap().mac_address.to_vec()
}

fn path(network: u16, router: &[u8]) -> PathKey {
    (network, MacAddr::from_slice(router))
}

/// The whole row stays: its router, and what the held I-Am said about the
/// instance, its limits and the vendor too. Its path, the network and the
/// row's router, waits for a check until one is recorded.
#[test]
fn an_i_am_through_another_station_is_held_and_its_path_marked() {
    let mut table = DeviceTable::new();
    let first = routed(5, 2, &[5], &ROUTER);
    table.upsert(first.clone());
    let mut claim = routed(6, 2, &[5], &OTHER);
    claim.max_apdu_length = 50;
    claim.segmentation_supported = Segmentation::NO_SEGMENTATION;
    claim.vendor_id = 7;
    let heard = table.heard(claim);
    assert_eq!(heard.change, Change::Held);
    assert!(heard.replaced.is_none() && heard.contested.is_empty());
    let row = table.get(2, &[5]).unwrap();
    assert_eq!(row.mac_address.as_slice(), ROUTER);
    assert_eq!(row.object_identifier, first.object_identifier);
    assert_eq!(row.max_apdu_length, 1476);
    assert_eq!(row.segmentation_supported, Segmentation::SEGMENTED_BOTH);
    assert_eq!(row.vendor_id, 42);
    assert_eq!(row.last_seen, first.last_seen);
    assert!(table.find(6).unwrap().is_none());
    let at = now();
    assert_eq!(table.due_paths(at, 8), [path(2, &ROUTER)]);
    assert!(table.next_path_due().is_some_and(|due| due <= at));
    table.path_checked(&path(2, &ROUTER), Checked::Kept, at);
    assert!(
        table.due_paths(at, 8).is_empty(),
        "checked, and no new mark"
    );
    assert_eq!(table.next_path_due(), None);
    // A new mark waits for the hold.
    assert_eq!(table.heard(routed(5, 2, &[5], &OTHER)).change, Change::Held);
    assert!(table.due_paths(at, 8).is_empty());
    assert_eq!(table.next_path_due(), Some(at + FIRST_HOLD));
    assert_eq!(table.due_paths(at + FIRST_HOLD, 8), [path(2, &ROUTER)]);
}

/// First sight makes the row through whatever station the I-Am came
/// through, and the row's own router refreshes it: neither marks a path.
#[test]
fn first_sight_and_the_rows_own_router_are_not_held() {
    let mut table = DeviceTable::new();
    assert_eq!(
        table.heard(routed(5, 2, &[5], &OTHER)).change,
        Change::Inserted
    );
    assert_eq!(router_at(&table, 2, &[5]), OTHER);
    assert_eq!(
        table.heard(routed(5, 2, &[5], &OTHER)).change,
        Change::Updated
    );
    assert_eq!(
        table.heard(routed(9, 2, &[5], &OTHER)).change,
        Change::Replaced
    );
    // A local row's address is the MAC its I-Am came from: never held.
    let mut local = routed(3, 2, &[5], &[0x05]);
    local.source_network = None;
    local.source_address = None;
    assert_eq!(table.heard(local).change, Change::Inserted);
    assert!(table.due_paths(now(), 8).is_empty());
}

/// Marks coalesce per path, and the due paths come highest ranked first:
/// the longest known, then the most rows, up to the limit.
#[test]
fn due_paths_come_highest_ranked_first() {
    let mut table = DeviceTable::new();
    for station in [5u8, 6, 7] {
        table.upsert(routed(u32::from(station), 3, &[station], &ROUTER));
    }
    for station in [5u8, 6] {
        table.upsert(routed(u32::from(station) + 10, 2, &[station], &ROUTER));
    }
    table.upsert(routed(1, 1, &[5], &ROUTER));
    for (network, station) in [(1, 5u8), (2, 5), (2, 6), (3, 5)] {
        let claim = routed(1, network, &[station], &OTHER);
        assert_eq!(table.heard(claim).change, Change::Held);
    }
    let at = now();
    assert_eq!(
        table.due_paths(at, 8),
        [path(3, &ROUTER), path(2, &ROUTER), path(1, &ROUTER)]
    );
    assert_eq!(table.due_paths(at, 2), [path(3, &ROUTER), path(2, &ROUTER)]);
}

/// A full store keeps the paths more rows depend on: a new path with more
/// rows takes the place of the lowest ranked; one that ranks no higher is
/// dropped, and its row still stays as it is.
#[test]
fn a_full_store_keeps_the_paths_with_more_rows() {
    let mut table = DeviceTable::new();
    for station in [5u8, 6, 7] {
        table.upsert(routed(u32::from(station), 5000, &[station], &ROUTER));
    }
    let networks = 1..=MAX_PATHS as u16;
    for network in networks.clone() {
        table.upsert(routed(u32::from(network), network, &[5], &ROUTER));
    }
    for network in networks {
        let claim = routed(u32::from(network), network, &[5], &OTHER);
        assert_eq!(table.heard(claim).change, Change::Held);
    }
    // A newer single-row path ranks below every single-row path there.
    table.upsert(routed(1, 4000, &[5], &ROUTER));
    assert_eq!(
        table.heard(routed(1, 4000, &[5], &OTHER)).change,
        Change::Held
    );
    assert_eq!(router_at(&table, 4000, &[5]), ROUTER);
    // The three-row path outranks them all.
    assert_eq!(
        table.heard(routed(5, 5000, &[5], &OTHER)).change,
        Change::Held
    );
    let due = table.due_paths(now(), usize::MAX);
    assert_eq!(due.len(), MAX_PATHS);
    assert_eq!(due[0], path(5000, &ROUTER));
    assert!(!due.contains(&path(4000, &ROUTER)));
}

/// A path a forger made in numbers ranks below a long-known one with fewer
/// rows: age comes first.
#[test]
fn a_long_known_path_outranks_a_newer_one_with_more_rows() {
    let mut table = DeviceTable::new();
    table.upsert(routed(1, 1, &[5], &ROUTER));
    for station in 1..=20u8 {
        table.upsert(routed(u32::from(station) + 100, 2, &[station], &ROUTER));
    }
    assert_eq!(table.heard(routed(9, 2, &[1], &OTHER)).change, Change::Held);
    assert_eq!(table.heard(routed(1, 1, &[5], &OTHER)).change, Change::Held);
    assert_eq!(table.due_paths(now(), 1), [path(1, &ROUTER)]);
}

/// A strike is cleared by an explicit registration that gives a row on the
/// path another router, and lost with the path when the store evicts it.
#[test]
fn a_strike_ends_with_a_registration_or_an_eviction() {
    let strike = || Strike {
        to: MacAddr::from_slice(&OTHER),
        since: now(),
    };
    let mut table = DeviceTable::new();
    table.upsert(routed(1, 2, &[5], &ROUTER));
    table.upsert(routed(2, 2, &[6], &ROUTER));
    assert_eq!(table.heard(routed(1, 2, &[5], &OTHER)).change, Change::Held);
    table.path_checked(&path(2, &ROUTER), Checked::Struck(strike()), now());
    assert!(table.path_strike(&path(2, &ROUTER)).is_some());
    table.upsert(routed(1, 2, &[5], &[0x40]));
    assert_eq!(table.path_strike(&path(2, &ROUTER)), None);

    // 256 paths known longer than the struck one push it out of the store.
    let mut crowded = DeviceTable::new();
    for network in 100..100 + MAX_PATHS as u16 {
        crowded.upsert(routed(u32::from(network), network, &[5], &ROUTER));
    }
    crowded.upsert(routed(1, 2, &[5], &ROUTER));
    assert_eq!(
        crowded.heard(routed(1, 2, &[5], &OTHER)).change,
        Change::Held
    );
    crowded.path_checked(&path(2, &ROUTER), Checked::Struck(strike()), now());
    for network in 100..100 + MAX_PATHS as u16 {
        let claim = routed(u32::from(network), network, &[5], &OTHER);
        assert_eq!(crowded.heard(claim).change, Change::Held);
    }
    assert_eq!(crowded.path_strike(&path(2, &ROUTER)), None, "evicted");
}

/// The station a check asks is the client's choice: the row through the
/// router heard from last, never a broadcast or group form.
#[test]
fn the_probe_is_the_row_heard_last_and_never_a_group_form() {
    let mut table = DeviceTable::new();
    let mut quiet = routed(1, 2, &[5], &ROUTER);
    quiet.last_seen = now() - Duration::from_secs(60);
    table.upsert(quiet);
    let mut recent = routed(2, 2, &[6], &ROUTER);
    recent.last_seen = now() - Duration::from_secs(1);
    table.upsert(recent);
    table.upsert(routed(3, 2, &[0xFF], &ROUTER));
    table.upsert(routed(4, 2, &[7], &OTHER));
    assert_eq!(
        table.probe_target(2, &ROUTER),
        Some(MacAddr::from_slice(&[6]))
    );
    assert_eq!(table.probe_target(2, &[0x30]), None);
}

/// Only rows through the router count as heard through it, and only a
/// frame through a row's own router refreshes it.
#[test]
fn only_the_rows_own_router_refreshes_it() {
    let mut table = DeviceTable::new();
    let mut quiet = routed(1, 2, &[5], &ROUTER);
    let since = now();
    quiet.last_seen = since - Duration::from_secs(1);
    table.upsert(quiet);
    table.upsert(routed(2, 2, &[6], &OTHER));
    table.upsert(routed(3, 3, &[5], &ROUTER));
    assert!(!table.heard_through_since(2, &ROUTER, since));
    table.heard_through(2, &[5], &OTHER);
    assert!(!table.heard_through_since(2, &ROUTER, since));
    table.heard_through(2, &[5], &ROUTER);
    assert!(table.heard_through_since(2, &ROUTER, since));
    assert!(table.routes_through(2, &ROUTER));
    assert!(!table.routes_through(4, &ROUTER));
}

/// The check's move takes every row on the network that went through the
/// old router, and nothing else changes; an explicit registration still
/// sets the router it names. A path no row goes through is forgotten.
#[test]
fn reroute_moves_the_networks_rows_through_the_old_router_only() {
    let mut table = DeviceTable::new();
    table.upsert(routed(1, 2, &[5], &ROUTER));
    table.upsert(routed(2, 2, &[6], &ROUTER));
    table.upsert(routed(3, 2, &[7], &[0x30]));
    table.upsert(routed(4, 3, &[5], &ROUTER));
    assert_eq!(table.heard(routed(1, 2, &[5], &OTHER)).change, Change::Held);
    let moved = table.reroute(2, &ROUTER, &OTHER);
    let moved: Vec<u32> = moved
        .iter()
        .map(|d| d.object_identifier.instance_number())
        .collect();
    assert_eq!(moved, [1, 2]);
    assert_eq!(router_at(&table, 2, &[5]), OTHER);
    assert_eq!(router_at(&table, 2, &[6]), OTHER);
    assert_eq!(router_at(&table, 2, &[7]), [0x30]);
    assert_eq!(router_at(&table, 3, &[5]), ROUTER);
    assert_eq!(table.get(2, &[5]).unwrap().vendor_id, 42);
    assert!(
        table.due_paths(now(), 8).is_empty(),
        "no row left on the path"
    );
    table.upsert(routed(1, 2, &[5], &[0x40]));
    assert_eq!(router_at(&table, 2, &[5]), [0x40]);
}

#[test]
fn clear_drops_waiting_paths() {
    let mut table = DeviceTable::new();
    table.upsert(routed(1, 2, &[5], &ROUTER));
    assert_eq!(table.heard(routed(1, 2, &[5], &OTHER)).change, Change::Held);
    table.clear();
    assert_eq!(table.next_path_due(), None);
}
