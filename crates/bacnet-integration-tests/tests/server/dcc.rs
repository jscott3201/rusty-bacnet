use super::*;

// ---------------------------------------------------------------------------
// DeviceCommunicationControl enforcement tests (Clause 16.4.3)
// ---------------------------------------------------------------------------

/// DCC DISABLE (deprecated in 2020 spec) is rejected with SERVICE_REQUEST_DENIED.
#[tokio::test]
async fn dcc_disable_sets_comm_state() {
    use bacnet_types::enums::EnableDisable;

    let mut server = make_server().await;
    let mut client = make_client().await;
    let server_mac = server.local_mac().to_vec();

    // Clause 16.1: DISABLE sets comm_state to 1 (per spec, all three values are supported)
    let result = client
        .device_communication_control(&server_mac, EnableDisable::DISABLE, None, None)
        .await;
    assert!(result.is_ok(), "DCC DISABLE should succeed per Clause 16.1");

    // Server should be in DISABLE state (1)
    assert_eq!(server.comm_state(), 1);

    // Re-enable should work (DCC is allowed even when disabled)
    let result = client
        .device_communication_control(&server_mac, EnableDisable::ENABLE, None, None)
        .await;
    assert!(result.is_ok(), "DCC ENABLE should succeed");
    assert_eq!(server.comm_state(), 0);

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

/// DCC DISABLE_INITIATION still allows DCC re-enable.
#[tokio::test]
async fn dcc_disable_initiation_allows_re_enable() {
    use bacnet_types::enums::EnableDisable;

    let mut server = make_server().await;
    let mut client = make_client().await;
    let server_mac = server.local_mac().to_vec();
    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    // DCC DISABLE_INITIATION
    client
        .device_communication_control(&server_mac, EnableDisable::DISABLE_INITIATION, None, None)
        .await
        .unwrap();
    assert_eq!(server.comm_state(), 2);

    // DCC ENABLE while disable-initiation — should still work
    client
        .device_communication_control(&server_mac, EnableDisable::ENABLE, None, None)
        .await
        .unwrap();
    assert_eq!(server.comm_state(), 0);

    // ReadProperty should work
    let ack = client
        .read_property(&server_mac, ai_oid, PropertyIdentifier::PRESENT_VALUE, None)
        .await;
    assert!(
        ack.is_ok(),
        "ReadProperty should succeed after DCC re-enable"
    );

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

/// DCC DISABLE_INITIATION allows ReadProperty but blocks COV notifications.
#[tokio::test]
async fn dcc_disable_initiation_allows_rp_blocks_cov() {
    use bacnet_objects::analog::AnalogOutputObject;
    use bacnet_types::enums::EnableDisable;
    use tokio::time::Duration;

    // Build a server with an AnalogOutput (writable present-value)
    let mut db = ObjectDatabase::new();
    let ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    db.add(Box::new(ao)).unwrap();
    let dev = DeviceObject::new(DeviceConfig {
        instance: 1234,
        name: "DCC-Test-Dev".into(),
        ..DeviceConfig::default()
    })
    .unwrap();
    db.add(Box::new(dev)).unwrap();

    let mut server = BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .database(db)
        .build()
        .await
        .unwrap();
    let server_mac = server.local_mac().to_vec();

    let mut client = make_client().await;
    let mut cov_rx = client.cov_notifications();
    let ao_oid = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap();

    // Subscribe to COV
    client
        .subscribe_cov(&server_mac, 42, ao_oid, false, Some(300))
        .await
        .unwrap();

    // Write to trigger a COV notification (should fire normally)
    let mut value_buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_real(&mut value_buf, 50.0);
    client
        .write_property(
            &server_mac,
            ao_oid,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            value_buf.to_vec(),
            Some(16),
        )
        .await
        .unwrap();

    // Should receive COV notification
    let notification = tokio::time::timeout(Duration::from_secs(2), cov_rx.recv())
        .await
        .expect("Timed out waiting for COV notification")
        .expect("COV channel closed");
    assert_eq!(notification.monitored_object_identifier, ao_oid);

    // Now set DCC DISABLE_INITIATION
    client
        .device_communication_control(&server_mac, EnableDisable::DISABLE_INITIATION, None, None)
        .await
        .unwrap();
    assert_eq!(server.comm_state(), 2);

    // ReadProperty should still work
    let ack = client
        .read_property(&server_mac, ao_oid, PropertyIdentifier::PRESENT_VALUE, None)
        .await;
    assert!(
        ack.is_ok(),
        "ReadProperty should succeed under DISABLE_INITIATION"
    );

    // Write again — server should NOT send COV notification
    let mut value_buf2 = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_real(&mut value_buf2, 99.0);
    client
        .write_property(
            &server_mac,
            ao_oid,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            value_buf2.to_vec(),
            Some(16),
        )
        .await
        .unwrap();

    // Wait briefly — no notification should arrive
    let timeout_result = tokio::time::timeout(Duration::from_millis(500), cov_rx.recv()).await;
    assert!(
        timeout_result.is_err(),
        "Should NOT receive a COV notification under DISABLE_INITIATION"
    );

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

/// DCC Enable after DISABLE_INITIATION restores normal COV notifications.
#[tokio::test]
async fn dcc_enable_restores_normal_operation() {
    use bacnet_objects::analog::AnalogOutputObject;
    use bacnet_types::enums::EnableDisable;
    use tokio::time::Duration;

    // Build a server with an AnalogOutput
    let mut db = ObjectDatabase::new();
    let ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    db.add(Box::new(ao)).unwrap();
    let dev = DeviceObject::new(DeviceConfig {
        instance: 1234,
        name: "DCC-Restore-Dev".into(),
        ..DeviceConfig::default()
    })
    .unwrap();
    db.add(Box::new(dev)).unwrap();

    let mut server = BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .database(db)
        .build()
        .await
        .unwrap();
    let server_mac = server.local_mac().to_vec();

    let mut client = make_client().await;
    let mut cov_rx = client.cov_notifications();
    let ao_oid = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap();

    // Subscribe to COV
    client
        .subscribe_cov(&server_mac, 42, ao_oid, false, Some(300))
        .await
        .unwrap();

    // Disable initiation
    client
        .device_communication_control(&server_mac, EnableDisable::DISABLE_INITIATION, None, None)
        .await
        .unwrap();
    assert_eq!(server.comm_state(), 2);

    // Write — no COV (suppressed)
    let mut value_buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_real(&mut value_buf, 50.0);
    client
        .write_property(
            &server_mac,
            ao_oid,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            value_buf.to_vec(),
            Some(16),
        )
        .await
        .unwrap();

    let timeout_result = tokio::time::timeout(Duration::from_millis(500), cov_rx.recv()).await;
    assert!(
        timeout_result.is_err(),
        "Should NOT receive COV under DISABLE_INITIATION"
    );

    // Re-enable
    client
        .device_communication_control(&server_mac, EnableDisable::ENABLE, None, None)
        .await
        .unwrap();
    assert_eq!(server.comm_state(), 0);

    // Write again — COV should fire now
    let mut value_buf2 = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_real(&mut value_buf2, 99.0);
    client
        .write_property(
            &server_mac,
            ao_oid,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            value_buf2.to_vec(),
            Some(16),
        )
        .await
        .unwrap();

    let notification = tokio::time::timeout(Duration::from_secs(2), cov_rx.recv())
        .await
        .expect("Timed out waiting for COV notification after re-enable")
        .expect("COV channel closed");
    assert_eq!(notification.monitored_object_identifier, ao_oid);

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}
