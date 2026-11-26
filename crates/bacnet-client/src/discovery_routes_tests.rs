//! The router a routed row names (#2823): an I-Am through another station
//! leaves the row as it was and waits as a cue; only the check's own
//! `reroute` moves it, and an explicit registration sets it.

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

/// The whole row stays: its router, and what the held I-Am said about the
/// instance, its limits and the vendor too. The cue names the path, the
/// network and the row's router, and the station.
#[test]
fn an_i_am_through_another_station_is_held_and_cued() {
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
    assert_eq!(table.find(6).unwrap().map(|d| d.address()), None);
    assert_eq!(
        table.take_route_cues(),
        [((2, MacAddr::from_slice(&ROUTER)), MacAddr::from_slice(&[5]))]
    );
    assert!(table.take_route_cues().is_empty(), "taken once");
}

/// First sight makes the row through whatever station the I-Am came
/// through, and the row's own router refreshes it: neither is a cue.
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
    assert!(table.take_route_cues().is_empty());
}

/// Cues coalesce per path, the latest station kept, and at most 256 paths
/// wait at once; a held I-Am past that is still held.
#[test]
fn cues_coalesce_per_path_and_are_bounded() {
    let mut table = DeviceTable::new();
    table.upsert(routed(1, 2, &[5], &ROUTER));
    table.upsert(routed(2, 2, &[6], &ROUTER));
    assert_eq!(table.heard(routed(1, 2, &[5], &OTHER)).change, Change::Held);
    assert_eq!(table.heard(routed(2, 2, &[6], &OTHER)).change, Change::Held);
    assert_eq!(
        table.take_route_cues(),
        [((2, MacAddr::from_slice(&ROUTER)), MacAddr::from_slice(&[6]))]
    );
    for network in 1..=300u16 {
        table.upsert(routed(u32::from(network), network, &[5], &ROUTER));
    }
    for network in 1..=300u16 {
        let claim = routed(u32::from(network), network, &[5], &OTHER);
        assert_eq!(table.heard(claim).change, Change::Held);
    }
    assert_eq!(table.take_route_cues().len(), 256);
}

/// Only rows through the router count as heard through it: an I-Am it
/// carried or an answer to a request, at the instant given or later.
#[test]
fn heard_through_since_counts_the_router_alone() {
    let mut table = DeviceTable::new();
    let mut quiet = routed(1, 2, &[5], &ROUTER);
    let since = now();
    quiet.last_seen = since - Duration::from_secs(1);
    table.upsert(quiet);
    table.upsert(routed(2, 2, &[6], &OTHER));
    table.upsert(routed(3, 3, &[5], &ROUTER));
    assert!(!table.heard_through_since(2, &ROUTER, since));
    assert!(table.routes_through(2, &ROUTER));
    assert!(!table.routes_through(4, &ROUTER));
    table.answered(2, &[5]);
    assert!(table.heard_through_since(2, &ROUTER, since));
}

/// The check's move takes every row on the network that went through the
/// old router, and nothing else changes; an explicit registration still
/// sets the router it names.
#[test]
fn reroute_moves_the_networks_rows_through_the_old_router_only() {
    let mut table = DeviceTable::new();
    table.upsert(routed(1, 2, &[5], &ROUTER));
    table.upsert(routed(2, 2, &[6], &ROUTER));
    table.upsert(routed(3, 2, &[7], &[0x30]));
    table.upsert(routed(4, 3, &[5], &ROUTER));
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
    table.upsert(routed(1, 2, &[5], &[0x40]));
    assert_eq!(router_at(&table, 2, &[5]), [0x40]);
}

#[test]
fn clear_drops_waiting_cues() {
    let mut table = DeviceTable::new();
    table.upsert(routed(1, 2, &[5], &ROUTER));
    assert_eq!(table.heard(routed(1, 2, &[5], &OTHER)).change, Change::Held);
    table.clear();
    assert!(table.take_route_cues().is_empty());
}
