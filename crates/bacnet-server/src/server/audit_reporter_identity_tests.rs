//! Active target runtime membership is exact and protected until quiescence.
use super::*;
use bacnet_objects::device::{DeviceConfig, DeviceObject};

fn recipient() -> BACnetRecipient {
    BACnetRecipient::Device(oid(ObjectType::DEVICE, 20))
}
fn bindings() -> Vec<DeviceBinding> {
    vec![DeviceBinding::local(oid(ObjectType::DEVICE, 20), LOGGER).unwrap()]
}
fn device(instance: u32) -> Box<DeviceObject> {
    Box::new(
        DeviceObject::new(DeviceConfig {
            instance,
            name: format!("replacement-{instance}"),
            ..Default::default()
        })
        .unwrap(),
    )
}

#[tokio::test]
async fn audit_reporter_identity_startup_requires_exactly_one_device() {
    for devices in [
        &[][..],
        &[10, 11][..],
        &[ObjectIdentifier::MAX_INSTANCE][..],
    ] {
        assert!(
            try_server(reporter(), devices, Some(recipient()), bindings())
                .await
                .is_err()
        );
    }
    assert!(try_server(reporter(), &[10], None, bindings())
        .await
        .is_err());
    let mut fixture = try_server(reporter(), &[10], Some(recipient()), bindings())
        .await
        .unwrap();
    assert_eq!(
        health(&fixture.server).await,
        Reliability::NO_FAULT_DETECTED
    );
    assert!(matches!(
        write_value(&fixture.server, None).await,
        Apdu::SimpleAck(_)
    ));
    settle().await;
    assert_eq!(
        notifications(&fixture.transport.sent)[0].notifications[0].target_device,
        BACnetRecipient::Device(oid(ObjectType::DEVICE, 10))
    );
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn audit_reporter_identity_active_membership_rejects_mutation_before_callbacks() {
    let mut fixture = server(reporter()).await;
    let mut db = fixture.server.db.write().await;
    for target in [
        oid(ObjectType::DEVICE, 10),
        oid(ObjectType::AUDIT_REPORTER, 1),
    ] {
        assert!(db.remove(&target).is_err());
        assert!(db
            .with_object_adapter(&target, |_| panic!("protected callback ran"))
            .is_err());
    }
    assert!(db.add(device(10)).is_err());
    assert!(db.add(device(11)).is_err());
    assert!(db.add(Box::new(reporter())).is_err());
    assert_eq!(
        db.find_by_type(ObjectType::DEVICE),
        vec![oid(ObjectType::DEVICE, 10)]
    );
    assert_eq!(db.reserve_event_sequence_number().number(), 0);
    drop(db);
    assert!(matches!(
        write_value(&fixture.server, None).await,
        Apdu::SimpleAck(_)
    ));
    settle().await;
    assert_eq!(notifications(&fixture.transport.sent).len(), 1);
    fixture.server.stop().await.unwrap();
    let mut db = fixture.server.db.write().await;
    assert!(db
        .remove(&oid(ObjectType::AUDIT_REPORTER, 1))
        .unwrap()
        .is_some());
    assert!(db.remove(&oid(ObjectType::DEVICE, 10)).unwrap().is_some());
    db.add(device(11)).unwrap();
}

#[tokio::test]
async fn audit_reporter_identity_drop_releases_membership_after_task_frames_end() {
    let fixture = server(reporter()).await;
    let database = Arc::clone(&fixture.server.db);
    // Hold the DB guard so aborted producer frames can be queued for access.
    let mut db = database.write().await;
    drop(fixture);
    assert!(
        db.add(device(11)).is_err(),
        "seal alone cannot release task leases"
    );
    drop(db);
    settle().await;
    let mut db = database.write().await;
    db.add(device(11)).unwrap();
    assert!(db
        .remove(&oid(ObjectType::AUDIT_REPORTER, 1))
        .unwrap()
        .is_some());
    assert!(!db
        .get(&oid(ObjectType::DEVICE, 10))
        .unwrap()
        .property_list()
        .contains(&PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT));
}

#[tokio::test]
async fn audit_reporter_identity_unique_device_preserves_configured_routed_logger() {
    let mut fixture = try_server(
        reporter(),
        &[10],
        Some(recipient()),
        vec![DeviceBinding::routed(oid(ObjectType::DEVICE, 20), 200, [9], LOGGER).unwrap()],
    )
    .await
    .unwrap();
    assert!(matches!(
        write_value(&fixture.server, None).await,
        Apdu::SimpleAck(_)
    ));
    settle().await;
    let records = notifications(&fixture.transport.sent);
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].notifications[0].target_device,
        BACnetRecipient::Device(oid(ObjectType::DEVICE, 10))
    );
    let npdu = decode_npdu(fixture.transport.sent.lock().unwrap()[0].clone()).unwrap();
    assert_eq!(
        npdu.destination.unwrap(),
        NpduAddress {
            network: 200,
            mac_address: MacAddr::from_slice(&[9])
        }
    );
    fixture.server.stop().await.unwrap();
}
