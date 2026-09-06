use super::*;
use bacnet_types::enums::ObjectType;

fn local_device(instance: u32, mac: &[u8], last_seen: Instant) -> DiscoveredDevice {
    DiscoveredDevice {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap(),
        mac_address: MacAddr::from_slice(mac),
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::NONE,
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

fn retained_from_collision(result: DeviceUpsertResult) -> DiscoveredDevice {
    match result {
        DeviceUpsertResult::Collision { retained } => retained,
        other => panic!("expected collision, got {other:?}"),
    }
}

#[test]
fn first_i_am_inserts_and_same_local_endpoint_refreshes_metadata() {
    let now = Instant::now();
    let mut table = DeviceTable::new();
    let original = local_device(1001, &[0x01], now);

    assert!(matches!(
        table.upsert_with_result(original.clone()),
        DeviceUpsertResult::Inserted
    ));

    let mut refreshed = original;
    refreshed.max_apdu_length = 480;
    refreshed.segmentation_supported = Segmentation::BOTH;
    refreshed.max_segments_accepted = Some(8);
    refreshed.vendor_id = 84;
    refreshed.last_seen = now + Duration::from_secs(10);
    assert!(matches!(
        table.upsert_with_result(refreshed.clone()),
        DeviceUpsertResult::Updated
    ));
    assert_same_device(table.get(1001).unwrap(), &refreshed);
}

#[test]
fn different_local_endpoint_collides_and_retains_exact_row() {
    let now = Instant::now();
    let mut table = DeviceTable::new();
    let retained = local_device(1001, &[0x01], now);
    table.upsert_with_result(retained.clone());

    let mut incoming = local_device(1001, &[0x02], now + Duration::from_secs(10));
    incoming.vendor_id = 84;
    let collision = retained_from_collision(table.upsert_with_result(incoming));

    assert_same_device(&collision, &retained);
    assert_same_device(table.get(1001).unwrap(), &retained);
}

#[test]
fn same_routed_endpoint_refreshes_router_and_metadata() {
    let now = Instant::now();
    let mut table = DeviceTable::new();
    let original = routed_device(1001, &[0x10], 100, &[0x03], now);
    table.upsert_with_result(original.clone());

    let mut refreshed = routed_device(1001, &[0x20], 100, &[0x03], now + Duration::from_secs(10));
    refreshed.max_apdu_length = 480;
    refreshed.segmentation_supported = Segmentation::BOTH;
    refreshed.vendor_id = 84;
    assert!(matches!(
        table.upsert_with_result(refreshed.clone()),
        DeviceUpsertResult::Updated
    ));
    assert_same_device(table.get(1001).unwrap(), &refreshed);
}

#[test]
fn differing_complete_endpoint_identities_collide() {
    let now = Instant::now();
    let routed = routed_device(1001, &[0x10], 100, &[0x03], now);
    let mut table = DeviceTable::new();
    table.upsert_with_result(routed.clone());

    for incoming in [
        routed_device(1001, &[0x20], 200, &[0x03], now),
        routed_device(1001, &[0x20], 100, &[0x04], now),
    ] {
        let collision = retained_from_collision(table.upsert_with_result(incoming));
        assert_same_device(&collision, &routed);
        assert_same_device(table.get(1001).unwrap(), &routed);
    }

    let local = local_device(2002, &[0x01], now);
    let mut local_table = DeviceTable::new();
    local_table.upsert_with_result(local.clone());
    let collision = retained_from_collision(local_table.upsert_with_result(routed_device(
        2002,
        &[0x10],
        100,
        &[0x03],
        now,
    )));
    assert_same_device(&collision, &local);

    let routed = routed_device(3003, &[0x10], 100, &[0x03], now);
    let mut routed_table = DeviceTable::new();
    routed_table.upsert_with_result(routed.clone());
    let collision =
        retained_from_collision(routed_table.upsert_with_result(local_device(3003, &[0x01], now)));
    assert_same_device(&collision, &routed);
}

#[test]
fn placeholder_and_partial_rows_remain_repairable_by_complete_i_am() {
    let now = Instant::now();
    let mut placeholder_table = DeviceTable::new();
    placeholder_table.upsert_placeholder(local_device(1001, &[0x01], now));
    let replacement = local_device(1001, &[0x02], now + Duration::from_secs(10));
    assert!(matches!(
        placeholder_table.upsert_with_result(replacement.clone()),
        DeviceUpsertResult::Updated
    ));
    assert_same_device(placeholder_table.get(1001).unwrap(), &replacement);

    let mut partial = local_device(2002, &[0x01], now);
    partial.source_network = Some(100);
    let mut partial_table = DeviceTable::new();
    partial_table.upsert(partial);
    let replacement = routed_device(2002, &[0x10], 200, &[0x03], now);
    assert!(matches!(
        partial_table.upsert_with_result(replacement.clone()),
        DeviceUpsertResult::Updated
    ));
    assert_same_device(partial_table.get(2002).unwrap(), &replacement);
}

#[test]
fn partial_incoming_row_cannot_displace_stable_authority() {
    let now = Instant::now();
    let retained = local_device(1001, &[0x01], now);
    let mut table = DeviceTable::new();
    table.upsert_with_result(retained.clone());

    let mut partial = local_device(1001, &[0x01], now + Duration::from_secs(10));
    partial.source_address = Some(MacAddr::from_slice(&[0x03]));
    let collision = retained_from_collision(table.upsert_with_result(partial));

    assert_same_device(&collision, &retained);
    assert_same_device(table.get(1001).unwrap(), &retained);
}

#[test]
fn explicit_manual_upsert_overwrites_different_endpoint() {
    let now = Instant::now();
    let mut table = DeviceTable::new();
    table.upsert_with_result(local_device(1001, &[0x01], now));
    let manual = routed_device(1001, &[0x10], 100, &[0x03], now);

    table.upsert(manual.clone());

    assert_same_device(table.get(1001).unwrap(), &manual);
}

#[test]
fn repeated_collisions_do_not_refresh_and_claimant_can_insert_after_purge() {
    let now = Instant::now();
    let retained = local_device(1001, &[0x01], now);
    let mut table = DeviceTable::new();
    table.upsert_with_result(retained.clone());

    let mut claimant = local_device(1001, &[0x02], now + Duration::from_secs(10));
    for vendor_id in [84, 85] {
        claimant.vendor_id = vendor_id;
        claimant.last_seen += Duration::from_secs(10);
        let collision = retained_from_collision(table.upsert_with_result(claimant.clone()));
        assert_same_device(&collision, &retained);
        assert_same_device(table.get(1001).unwrap(), &retained);
    }

    let removed = table.purge_stale_at(now + Duration::from_secs(61), Duration::from_secs(60));
    assert_eq!(removed.len(), 1);
    assert!(matches!(
        table.upsert_with_result(claimant.clone()),
        DeviceUpsertResult::Inserted
    ));
    assert_same_device(table.get(1001).unwrap(), &claimant);
}
