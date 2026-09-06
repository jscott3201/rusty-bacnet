use super::*;

// ---------------------------------------------------------------------------
// Local-write COV/event integration tests (#117)
//
// `write_local` is the server-owned local-mutation entry point. These tests
// prove it fires the same post-write COV notifications as a network
// WriteProperty, and that OBJECT_NAME local writes still enforce uniqueness
// and refresh the database name index.
// ---------------------------------------------------------------------------

/// A local `write_local` of PRESENT_VALUE fires a COV notification that a
/// subscribed client receives — matching the network WriteProperty path.
#[tokio::test]
async fn local_write_fires_cov_notification() {
    use bacnet_objects::analog::AnalogOutputObject;
    use bacnet_types::primitives::PropertyValue;
    use tokio::time::Duration;

    // Server with a writable AnalogOutput + Device.
    let mut db = ObjectDatabase::new();
    let ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    let ao_oid = ao.object_identifier();
    db.add(Box::new(ao)).unwrap();
    let dev = DeviceObject::new(DeviceConfig {
        instance: 1234,
        name: "Local-COV-Dev".into(),
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

    // 1. Subscribe to COV on AO:1 (unconfirmed, 300s lifetime).
    client
        .subscribe_cov(&server_mac, 42, ao_oid, false, Some(300))
        .await
        .unwrap();

    // Drain the initial COV notification emitted at subscription time.
    let initial = tokio::time::timeout(Duration::from_secs(2), cov_rx.recv())
        .await
        .expect("timed out waiting for initial COV notification")
        .expect("COV channel closed");
    assert_eq!(initial.notification.monitored_object_identifier, ao_oid);

    // 2. Mutate PRESENT_VALUE through the LOCAL write path — the path that
    //    previously bypassed COV. A notification must now arrive.
    server
        .write_local(
            &ao_oid,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(50.0),
            Some(16),
        )
        .await
        .unwrap();

    let received = tokio::time::timeout(Duration::from_secs(2), cov_rx.recv())
        .await
        .expect("timed out waiting for COV notification after local write")
        .expect("COV channel closed");
    assert_eq!(received.notification.monitored_object_identifier, ao_oid);

    // 3. Cancel the subscription; a further local write must NOT notify.
    client
        .unsubscribe_cov(&server_mac, 42, ao_oid)
        .await
        .unwrap();

    server
        .write_local(
            &ao_oid,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(99.0),
            Some(16),
        )
        .await
        .unwrap();

    let timeout_result = tokio::time::timeout(Duration::from_millis(500), cov_rx.recv()).await;
    assert!(
        timeout_result.is_err(),
        "should NOT receive a COV notification after unsubscribe, but got one"
    );

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

/// A local `write_local` of OBJECT_NAME renames the object and refreshes the
/// database name index, so a subsequent lookup by the new name resolves to the
/// renamed object and a duplicate rename is rejected.
#[tokio::test]
async fn local_object_name_write_refreshes_name_index() {
    use bacnet_objects::analog::AnalogOutputObject;
    use bacnet_objects::database::ObjectDatabase;
    use bacnet_types::primitives::PropertyValue;
    use tokio::time::Duration;

    let mut db = ObjectDatabase::new();
    let ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    let ao_oid = ao.object_identifier();
    db.add(Box::new(ao)).unwrap();
    let dev = DeviceObject::new(DeviceConfig {
        instance: 1234,
        name: "Local-Name-Dev".into(),
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

    // Rename AO:1 to "AO-Renamed" via the local path.
    server
        .write_local(
            &ao_oid,
            PropertyIdentifier::OBJECT_NAME,
            None,
            PropertyValue::CharacterString("AO-Renamed".into()),
            None,
        )
        .await
        .unwrap();

    // The name index must now resolve the new name to AO:1, and the old name
    // must be freed.
    {
        let db = server.database().read().await;
        assert_eq!(
            db.find_by_name("AO-Renamed").map(|o| o.object_identifier()),
            Some(ao_oid)
        );
        assert!(db.find_by_name("AO-1").is_none());
    }

    // "Local-Name-Dev" is owned by the Device object; renaming AO:1 to it via
    // the local path must be rejected as a duplicate.
    let dup = server
        .write_local(
            &ao_oid,
            PropertyIdentifier::OBJECT_NAME,
            None,
            PropertyValue::CharacterString("Local-Name-Dev".into()),
            None,
        )
        .await;
    assert!(dup.is_err(), "duplicate object name must be rejected");

    // The failed rename must not have corrupted the index: AO:1 is still
    // reachable as "AO-Renamed" and the Device still owns its name.
    {
        let db = server.database().read().await;
        assert_eq!(
            db.find_by_name("AO-Renamed").map(|o| o.object_identifier()),
            Some(ao_oid)
        );
        assert!(db.find_by_name("Local-Name-Dev").is_some());
    }

    // Let any background dispatch settle, then stop.
    tokio::time::sleep(Duration::from_millis(50)).await;
    server.stop().await.unwrap();
}
