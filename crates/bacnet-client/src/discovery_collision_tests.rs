//! Two or more addresses claiming one Device instance (#2560): every claim
//! keeps its row, a lookup by the instance names them all, and the contest
//! ends when only one claimant is left.

use super::*;
use bacnet_types::enums::ObjectType;

fn local_device(instance: u32, mac: &[u8], last_seen: Instant) -> DiscoveredDevice {
    DiscoveredDevice {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap(),
        mac_address: MacAddr::from_slice(mac),
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::NO_SEGMENTATION,
        max_segments_accepted: None,
        vendor_id: 42,
        last_seen,
        source_network: None,
        source_address: None,
    }
}

fn routed_device(
    instance: u32,
    router_mac: &[u8],
    network: u16,
    address: &[u8],
    last_seen: Instant,
) -> DiscoveredDevice {
    let mut device = local_device(instance, router_mac, last_seen);
    device.source_network = Some(network);
    device.source_address = Some(MacAddr::from_slice(address));
    device
}

fn assert_same_device(actual: &DiscoveredDevice, expected: &DiscoveredDevice) {
    assert_eq!(actual.object_identifier, expected.object_identifier);
    assert_eq!(actual.mac_address, expected.mac_address);
    assert_eq!(actual.max_apdu_length, expected.max_apdu_length);
    assert_eq!(
        actual.segmentation_supported,
        expected.segmentation_supported
    );
    assert_eq!(actual.max_segments_accepted, expected.max_segments_accepted);
    assert_eq!(actual.vendor_id, expected.vendor_id);
    assert_eq!(actual.last_seen, expected.last_seen);
    assert_eq!(actual.source_network, expected.source_network);
    assert_eq!(actual.source_address, expected.source_address);
}

fn address(network_number: u16, mac: &[u8]) -> BACnetAddress {
    BACnetAddress {
        network_number,
        mac_address: MacAddr::from_slice(mac),
    }
}

/// The addresses a lookup of `instance` refuses with.
fn candidates(table: &DeviceTable, instance: u32) -> Vec<BACnetAddress> {
    match table.find(instance) {
        Err(Error::AmbiguousDevice {
            instance: named,
            candidates,
        }) => {
            assert_eq!(named, instance);
            candidates.into_vec()
        }
        other => panic!("expected an ambiguous instance, got {other:?}"),
    }
}

#[test]
fn first_i_am_inserts_and_same_local_endpoint_refreshes_metadata() {
    let now = Instant::now();
    let mut table = DeviceTable::new();
    let original = local_device(1001, &[0x01], now);

    let heard = table.heard(original.clone());
    assert_eq!(heard.change, Change::Inserted);
    assert!(heard.contested.is_empty());

    let mut refreshed = original;
    refreshed.max_apdu_length = 480;
    refreshed.segmentation_supported = Segmentation::SEGMENTED_BOTH;
    refreshed.max_segments_accepted = Some(8);
    refreshed.vendor_id = 84;
    refreshed.last_seen = now + Duration::from_secs(10);
    let heard = table.heard(refreshed.clone());
    assert_eq!(heard.change, Change::Updated);
    assert!(heard.contested.is_empty());
    assert_same_device(table.find(1001).unwrap().unwrap(), &refreshed);
}

/// A second local address claiming the instance gets a row of its own; the
/// first row is untouched, and a lookup by instance names both.
#[test]
fn a_second_local_claimant_keeps_both_rows() {
    let now = Instant::now();
    let mut table = DeviceTable::new();
    let first = local_device(1001, &[0x01], now);
    table.heard(first.clone());

    let mut second = local_device(1001, &[0x02], now + Duration::from_secs(10));
    second.vendor_id = 84;
    let heard = table.heard(second.clone());

    assert_eq!(heard.change, Change::Inserted);
    assert_eq!(heard.contested, [1001]);
    assert_same_device(table.get(0, &[0x01]).unwrap(), &first);
    assert_same_device(table.get(0, &[0x02]).unwrap(), &second);
    assert_eq!(table.claiming(1001).count(), 2);
    assert_eq!(
        candidates(&table, 1001),
        [address(0, &[0x01]), address(0, &[0x02])]
    );
}

#[test]
fn same_routed_endpoint_refreshes_router_and_metadata() {
    let now = Instant::now();
    let mut table = DeviceTable::new();
    let original = routed_device(1001, &[0x10], 100, &[0x03], now);
    table.heard(original);

    let mut refreshed = routed_device(1001, &[0x20], 100, &[0x03], now + Duration::from_secs(10));
    refreshed.max_apdu_length = 480;
    refreshed.segmentation_supported = Segmentation::SEGMENTED_BOTH;
    refreshed.vendor_id = 84;
    assert_eq!(table.heard(refreshed.clone()).change, Change::Updated);
    assert_eq!(table.len(), 1);
    assert_same_device(table.find(1001).unwrap().unwrap(), &refreshed);
}

/// Another network, another station on the same network, or the local
/// network: each is another address, so another claimant.
#[test]
fn every_other_address_is_another_claimant() {
    let now = Instant::now();
    let mut table = DeviceTable::new();
    table.heard(routed_device(1001, &[0x10], 100, &[0x03], now));
    table.heard(routed_device(1001, &[0x20], 200, &[0x03], now));
    table.heard(routed_device(1001, &[0x20], 100, &[0x04], now));
    table.heard(local_device(1001, &[0x03], now));
    assert_eq!(
        candidates(&table, 1001),
        [
            address(0, &[0x03]),
            address(100, &[0x03]),
            address(100, &[0x04]),
            address(200, &[0x03]),
        ]
    );
    assert_eq!(table.len(), 4);
}

/// A manual placeholder is a claim like any other: an I-Am from elsewhere
/// makes the instance ambiguous, and an I-Am from its own address makes it
/// authoritative.
#[test]
fn a_placeholder_is_a_claim_like_any_other() {
    let now = Instant::now();
    let mut table = DeviceTable::new();
    table.upsert_placeholder(local_device(1001, &[0x01], now));
    let elsewhere = local_device(1001, &[0x02], now + Duration::from_secs(10));
    assert_eq!(table.heard(elsewhere).contested, [1001]);
    assert_eq!(candidates(&table, 1001).len(), 2);

    let own = local_device(1001, &[0x01], now + Duration::from_secs(20));
    assert_eq!(table.heard(own.clone()).change, Change::Updated);
    assert_eq!(
        table.peer(0, &[0x01]).map(|(_, s)| s),
        Some(PeerSegmentation::Authoritative(
            Segmentation::NO_SEGMENTATION
        ))
    );
    assert_same_device(table.get(0, &[0x01]).unwrap(), &own);
}

/// A device with half its routing fields names no address: it neither
/// replaces nor joins a row.
#[test]
fn a_partial_incoming_row_changes_nothing() {
    let now = Instant::now();
    let retained = local_device(1001, &[0x01], now);
    let mut table = DeviceTable::new();
    table.heard(retained.clone());

    let mut partial = local_device(1001, &[0x01], now + Duration::from_secs(10));
    partial.source_address = Some(MacAddr::from_slice(&[0x03]));
    assert_eq!(table.heard(partial).change, Change::Dropped);

    assert_eq!(table.len(), 1);
    assert_same_device(table.find(1001).unwrap().unwrap(), &retained);
}

#[test]
fn an_explicit_upsert_at_another_address_adds_a_claimant() {
    let now = Instant::now();
    let mut table = DeviceTable::new();
    let discovered = local_device(1001, &[0x01], now);
    table.heard(discovered.clone());
    let manual = routed_device(1001, &[0x10], 100, &[0x03], now);

    table.upsert(manual.clone());

    assert_same_device(table.get(0, &[0x01]).unwrap(), &discovered);
    assert_same_device(table.get(100, &[0x03]).unwrap(), &manual);
    assert_eq!(candidates(&table, 1001).len(), 2);
}

/// Each claimant's I-Am refreshes only its own row, and reports no change
/// to the contest; the contest ends when the purge takes one claimant.
#[test]
fn repeated_claims_refresh_their_own_rows_until_the_purge_ends_the_contest() {
    let now = Instant::now();
    let first = local_device(1001, &[0x01], now);
    let mut table = DeviceTable::new();
    table.heard(first.clone());

    let mut claimant = local_device(1001, &[0x02], now + Duration::from_secs(10));
    assert_eq!(table.heard(claimant.clone()).contested, [1001]);
    for vendor_id in [84, 85] {
        claimant.vendor_id = vendor_id;
        claimant.last_seen += Duration::from_secs(10);
        let heard = table.heard(claimant.clone());
        assert_eq!(heard.change, Change::Updated);
        assert!(heard.contested.is_empty());
        assert_same_device(table.get(0, &[0x01]).unwrap(), &first);
        assert_same_device(table.get(0, &[0x02]).unwrap(), &claimant);
    }

    let purged = table.purge_stale_at(now + Duration::from_secs(61), Duration::from_secs(60));
    assert_eq!(purged.lost.len(), 1);
    assert_same_device(&purged.lost[0], &first);
    assert_eq!(purged.contested, [1001]);
    assert_same_device(table.find(1001).unwrap().unwrap(), &claimant);
}

/// A purge that takes every claimant reports the contest once.
#[test]
fn a_purge_of_every_claimant_reports_the_contest_once() {
    let now = Instant::now();
    let mut table = DeviceTable::new();
    table.heard(local_device(1001, &[0x01], now));
    table.heard(local_device(1001, &[0x02], now));
    table.heard(local_device(2002, &[0x03], now));
    let purged = table.purge_stale_at(now + Duration::from_secs(61), Duration::from_secs(60));
    assert_eq!(purged.lost.len(), 3);
    assert_eq!(purged.contested, [1001]);
    assert!(table.is_empty());
}

/// A claimant that announces another instance leaves the contest, which
/// ends, and joins the other instance's, which begins.
#[test]
fn a_claimant_announcing_another_instance_moves_between_contests() {
    let now = Instant::now();
    let mut table = DeviceTable::new();
    table.heard(local_device(1001, &[0x01], now));
    table.heard(local_device(1001, &[0x02], now));
    table.heard(local_device(2002, &[0x03], now));

    let heard = table.heard(local_device(2002, &[0x02], now));
    assert_eq!(heard.change, Change::Replaced);
    let replaced = heard
        .replaced
        .as_ref()
        .expect("the row it took the place of");
    assert_eq!(replaced.object_identifier.instance_number(), 1001);
    assert_eq!(heard.contested, [1001, 2002]);
    assert_eq!(
        table.find(1001).unwrap().unwrap().mac_address.as_slice(),
        [0x01]
    );
    assert_eq!(
        candidates(&table, 2002),
        [address(0, &[0x02]), address(0, &[0x03])]
    );

    // Announcing a third, unclaimed instance ends the second contest too.
    let heard = table.heard(local_device(3003, &[0x02], now));
    assert_eq!(heard.contested, [2002]);
    assert!(table.find(2002).unwrap().is_some());
    assert!(table.find(3003).unwrap().is_some());
}
