use super::*;
use bacnet_types::enums::ObjectType;

const MAC: [u8; 6] = [192, 168, 1, 100, 0xBA, 0xC0];

fn make_device(instance: u32) -> DiscoveredDevice {
    DiscoveredDevice {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap(),
        mac_address: MacAddr::from_slice(&MAC),
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::NO_SEGMENTATION,
        max_segments_accepted: None,
        vendor_id: 42,
        last_seen: Instant::now(),
        source_network: None,
        source_address: None,
    }
}

fn local_at(instance: u32, mac: &[u8]) -> DiscoveredDevice {
    let mut device = make_device(instance);
    device.mac_address = MacAddr::from_slice(mac);
    device
}

fn make_routed_device(instance: u32, network: u16, address: &[u8]) -> DiscoveredDevice {
    let mut device = make_device(instance);
    device.source_network = Some(network);
    device.source_address = Some(MacAddr::from_slice(address));
    device
}

fn instance_of(device: Option<&DiscoveredDevice>) -> Option<u32> {
    device.map(|d| d.object_identifier.instance_number())
}

#[test]
fn upsert_and_find() {
    let mut table = DeviceTable::new();
    table.upsert(make_device(1234));
    assert_eq!(table.len(), 1);
    let dev = table.find(1234).unwrap().unwrap();
    assert_eq!(dev.vendor_id, 42);
    assert!(table.find(4321).unwrap().is_none());
}

/// Hearing an address again refreshes its row in place: one row, the new
/// values.
#[test]
fn upsert_refreshes_the_row_at_its_address() {
    let mut table = DeviceTable::new();
    table.upsert(make_device(1234));
    let mut updated = make_device(1234);
    updated.vendor_id = 99;
    table.upsert(updated);
    assert_eq!(table.len(), 1);
    assert_eq!(table.get(0, &MAC).unwrap().vendor_id, 99);
}

#[test]
fn heard_reports_insert_and_update() {
    let mut table = DeviceTable::new();
    assert_eq!(table.heard(make_device(1234)).change, Change::Inserted);
    assert_eq!(table.heard(make_device(1234)).change, Change::Updated);
}

#[test]
fn all_lists_rows_in_address_order() {
    let mut table = DeviceTable::new();
    table.upsert(make_routed_device(1, 7, &[0x02]));
    table.upsert(local_at(2, &[0x09]));
    table.upsert(make_routed_device(3, 7, &[0x01]));
    let order: Vec<u32> = table
        .all()
        .iter()
        .map(|d| d.object_identifier.instance_number())
        .collect();
    assert_eq!(order, [2, 3, 1]);
}

#[test]
fn clear_empties_table_and_its_index() {
    let mut table = DeviceTable::new();
    table.upsert(make_device(1));
    table.clear();
    assert!(table.is_empty());
    assert!(table.find(1).unwrap().is_none());
    assert_eq!(table.claiming(1).count(), 0);
}

#[test]
fn get_finds_a_local_device_by_mac() {
    let mut table = DeviceTable::new();
    table.upsert(make_device(1234));
    assert_eq!(instance_of(table.get(0, &MAC)), Some(1234));
    assert!(table.get(0, &[10, 0, 0, 1, 0xBA, 0xC0]).is_none());
}

#[test]
fn get_finds_a_routed_device_by_network_and_address() {
    let mut table = DeviceTable::new();
    table.upsert(make_routed_device(1, 100, &[0x03]));
    table.upsert(make_routed_device(2, 200, &[0x03]));
    assert_eq!(instance_of(table.get(200, &[0x03])), Some(2));
    assert!(table.get(100, &[0x04]).is_none());
    assert!(table.get(101, &[0x03]).is_none());
}

/// A local device whose MAC happens to equal the queried remote address
/// lives in a different address space and must not match.
#[test]
fn a_routed_lookup_ignores_a_local_namesake() {
    let mut table = DeviceTable::new();
    table.upsert(local_at(1, &[0x03]));
    assert!(table.get(100, &[0x03]).is_none());
    table.upsert(make_routed_device(3003, 100, &[0x03]));
    assert_eq!(instance_of(table.get(0, &[0x03])), Some(1));
    assert_eq!(instance_of(table.get(100, &[0x03])), Some(3003));
}

/// A routed row's `mac_address` is the router it was heard through, not
/// the remote device's identity (Clause 6.2.2: SNET/SADR identify the
/// original source; the router MAC is only the local next hop). A local
/// lookup of the router's own address never selects a peer behind it,
/// however many share the router (#372).
#[test]
fn a_local_lookup_of_a_router_mac_finds_no_routed_peer() {
    let mut table = DeviceTable::new();
    let router_mac = [192, 168, 1, 1, 0xBA, 0xC0];
    for instance in [1u32, 2, 3] {
        let mut peer = make_routed_device(instance, 100, &[instance as u8]);
        peer.mac_address = MacAddr::from_slice(&router_mac);
        table.upsert(peer);
    }
    assert!(table.get(0, &router_mac).is_none());
    assert_eq!(table.len(), 3);
}

/// One MAC on two networks is two devices, and one instance claimed by both
/// is ambiguous (#2560).
#[test]
fn one_mac_on_two_networks_is_two_devices() {
    let mut table = DeviceTable::new();
    table.upsert(make_routed_device(5, 2, &[0x05]));
    table.upsert(make_routed_device(5, 3, &[0x05]));
    assert_eq!(table.len(), 2);
    assert!(table.get(2, &[0x05]).is_some() && table.get(3, &[0x05]).is_some());
    assert!(matches!(
        table.find(5),
        Err(Error::AmbiguousDevice { instance: 5, ref candidates }) if candidates.len() == 2
    ));
}

/// The router a device is heard through is not part of its identity: a
/// device heard through another router keeps its row.
#[test]
fn a_new_router_keeps_the_row() {
    let mut table = DeviceTable::new();
    table.upsert(make_routed_device(5, 2, &[0x05]));
    let mut moved = make_routed_device(5, 2, &[0x05]);
    moved.mac_address = MacAddr::from_slice(&[0x20]);
    assert_eq!(table.heard(moved).change, Change::Updated);
    assert_eq!(table.len(), 1);
    assert_eq!(
        table.find(5).unwrap().unwrap().mac_address.as_slice(),
        [0x20]
    );
}

/// Routing fields that name no one station (one half of SNET/SADR, no
/// network, the global broadcast network, a remote broadcast) give no
/// address, so the row isn't stored at all; network 0 in particular would
/// alias a local row.
#[test]
fn routing_fields_naming_no_station_are_not_stored() {
    let mut table = DeviceTable::new();
    let mut partial = make_device(7);
    partial.source_network = Some(100);
    let mut unrouted = make_device(7);
    unrouted.source_address = Some(MacAddr::from_slice(&[0x03]));
    for device in [
        partial,
        unrouted,
        make_routed_device(7, 0, &MAC),
        make_routed_device(7, u16::MAX, &[0x03]),
        make_routed_device(7, 100, &[]),
    ] {
        assert!(device.address().is_none(), "{device:?}");
        assert_eq!(table.heard(device).change, Change::Dropped);
    }
    assert!(table.is_empty());
    assert!(table.find(7).unwrap().is_none());
}

/// Provenance transitions (#371): a legacy placeholder row becomes
/// authoritative when refreshed through the I-Am insertion path, and
/// provenance never outlives its row across purge/clear.
#[test]
fn segmentation_provenance_tracks_row_lifecycle() {
    let mut table = DeviceTable::new();
    let provenance = |table: &DeviceTable| table.peer(0, &MAC).map(|(_, s)| s);
    table.upsert_placeholder(make_device(1));
    assert_eq!(provenance(&table), Some(PeerSegmentation::Placeholder));

    // I-Am refresh of the same address (the dispatch path).
    table.heard(make_device(1));
    assert_eq!(
        provenance(&table),
        Some(PeerSegmentation::Authoritative(
            Segmentation::NO_SEGMENTATION
        ))
    );

    // Purge removes the row and its provenance with it.
    let now = Instant::now();
    let mut stale = make_device(1);
    stale.last_seen = now;
    table.upsert_placeholder(stale);
    table.purge_stale_at(now + Duration::from_secs(120), Duration::from_secs(60));
    assert_eq!(provenance(&table), None);

    // Clear leaves no orphan provenance either.
    table.upsert(make_device(2));
    table.clear();
    assert_eq!(provenance(&table), None);
}

#[test]
fn purge_stale_removes_old_entries() {
    let mut table = DeviceTable::new();
    let now = Instant::now();
    let mut old_device = local_at(1, &[0x01]);
    old_device.last_seen = now;
    table.upsert(old_device);
    let mut fresh_device = local_at(2, &[0x02]);
    fresh_device.last_seen = now + Duration::from_secs(120);
    table.upsert(fresh_device);
    assert_eq!(table.len(), 2);

    let purged = table.purge_stale_at(now + Duration::from_secs(120), Duration::from_secs(60));
    assert_eq!(table.len(), 1);
    assert_eq!(purged.lost.len(), 1);
    assert_eq!(purged.lost[0].object_identifier.instance_number(), 1);
    assert!(purged.contested.is_empty());
    assert!(table.find(1).unwrap().is_none());
    assert!(table.find(2).unwrap().is_some());
}

#[test]
fn purge_stale_keeps_all_when_fresh() {
    let mut table = DeviceTable::new();
    table.upsert(local_at(1, &[0x01]));
    table.upsert(local_at(2, &[0x02]));
    table.purge_stale(Duration::from_secs(60));
    assert_eq!(table.len(), 2);
}

#[test]
fn purge_stale_removes_all_when_expired() {
    let mut table = DeviceTable::new();
    let now = Instant::now();
    for (instance, mac) in [(1, 0x01), (2, 0x02)] {
        let mut device = local_at(instance, &[mac]);
        device.last_seen = now;
        table.upsert(device);
    }
    let purged = table.purge_stale_at(now + Duration::from_secs(200), Duration::from_secs(60));
    assert!(table.is_empty());
    assert_eq!(purged.lost.len(), 2);
}

#[test]
fn upsert_refreshes_last_seen() {
    let mut table = DeviceTable::new();
    let now = Instant::now();
    let mut old_device = make_device(1);
    old_device.last_seen = now;
    table.upsert(old_device);

    let mut refreshed = make_device(1);
    refreshed.last_seen = now + Duration::from_secs(120);
    table.upsert(refreshed);
    let purged = table.purge_stale_at(now + Duration::from_secs(120), Duration::from_secs(60));
    assert_eq!(table.len(), 1);
    assert!(purged.lost.is_empty());
}

/// An answer to a confirmed request starts the row's age again; an answer
/// from an address the table doesn't hold adds nothing (#2560).
#[tokio::test(start_paused = true)]
async fn an_answer_refreshes_only_its_own_row() {
    let mut table = DeviceTable::new();
    let long_ago = now();
    tokio::time::advance(Duration::from_secs(3_600)).await;
    for mac in [0x01, 0x02] {
        let mut device = local_at(u32::from(mac), &[mac]);
        device.last_seen = long_ago;
        table.upsert(device);
    }
    table.answered(0, &[0x01]);
    table.answered(0, &[0x03]);
    table.answered(7, &[0x02]);
    assert!(table.get(0, &[0x01]).unwrap().last_seen > long_ago);
    assert_eq!(table.get(0, &[0x02]).unwrap().last_seen, long_ago);
    assert_eq!(table.len(), 2);
}

#[test]
fn purge_stale_handles_max_age_larger_than_instant_history() {
    let mut table = DeviceTable::new();
    table.upsert(make_device(1));

    table.purge_stale(Duration::MAX);

    assert_eq!(table.len(), 1);
}

/// The cap counts addresses: an instance two addresses claim takes two
/// rows, a new address past the cap is dropped, and a known one is still
/// refreshed.
#[test]
fn the_cap_counts_one_row_per_address() {
    let mut table = DeviceTable::new();
    let mac = |n: usize| (n as u32).to_be_bytes();
    // Two addresses claim instance 0.
    table.upsert(local_at(0, &mac(0)));
    for n in 1..MAX_DEVICE_TABLE_ENTRIES {
        table.upsert(local_at((n - 1) as u32, &mac(n)));
    }
    assert_eq!(table.len(), MAX_DEVICE_TABLE_ENTRIES);
    assert_eq!(table.claiming(0).count(), 2);

    let extra = mac(MAX_DEVICE_TABLE_ENTRIES);
    assert_eq!(table.heard(local_at(9_999, &extra)).change, Change::Dropped);
    assert!(table.get(0, &extra).is_none());
    assert!(table.find(9_999).unwrap().is_none());

    let mut refreshed = local_at(9_999, &mac(1));
    refreshed.vendor_id = 7;
    assert_eq!(table.heard(refreshed).change, Change::Updated);
    assert_eq!(table.get(0, &mac(1)).unwrap().vendor_id, 7);
    assert_eq!(table.len(), MAX_DEVICE_TABLE_ENTRIES);
}
