//! An Audit Log sink and its forwarder speak for the identity's Device, not
//! a lower-instance Device the application adds once the server runs
//! (#2227, #2228).
use super::*;
use crate::handlers::handle_audit_notification;
use bacnet_services::audit::AuditNotificationRequest;

/// Device `instance` whose APDU_Timeout is `apdu_timeout` (0 is invalid for
/// a sink, which then refuses every batch).
fn device(instance: u32, apdu_timeout: u32) -> Box<DeviceObject> {
    Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::new(instance, 0)
                .unwrap()
                .with_name(format!("Device {instance}"))
                .with_apdu_segment_timeout_ms(u64::from(apdu_timeout.min(2000))),
            apdu_timeout,
            ..Default::default()
        })
        .unwrap(),
    )
}

/// The sink reads APDU_Timeout from the Device it's told is this one: a
/// lower Device's invalid timeout doesn't refuse the batch, and a missing
/// identity Device still does (DEVICE / OPERATIONAL_PROBLEM).
#[test]
fn the_audit_helper_reads_apdu_timeout_from_the_named_device() {
    let sink = oid(ObjectType::AUDIT_LOG, 7);
    let request = AuditNotificationRequest {
        notifications: vec![notification(AuditOperation::WRITE)],
    };
    let store = Arc::new(MemoryPersistence::default());
    let db = super::super::audit_notification_tests::database_with_device(store, 7, None);
    let mut db = Arc::into_inner(db).unwrap().into_inner();
    db.add(device(10, 3000)).unwrap();
    db.add(device(5, 0)).unwrap();
    let refused = |result: Result<(), Error>| {
        matches!(
            result,
            Err(Error::Remote(bacnet_types::error::RemoteError::Error { class, code }))
                if class == ErrorClass::DEVICE && code == ErrorCode::OPERATIONAL_PROBLEM
        )
    };
    handle_audit_notification(&mut db, oid(ObjectType::DEVICE, 10), sink, &request).unwrap();
    assert!(refused(handle_audit_notification(
        &mut db,
        oid(ObjectType::DEVICE, 5),
        sink,
        &request
    )));
    assert!(refused(handle_audit_notification(
        &mut db,
        oid(ObjectType::DEVICE, 11),
        sink,
        &request
    )));
}

/// A running server's sink takes the identity Device's APDU_Timeout, so a
/// lower Device added after start with an invalid one changes nothing; and a
/// parent naming the identity's Device is this device, so no copy goes to
/// the binding a peer holds for it.
#[tokio::test(start_paused = true)]
async fn the_sink_and_forwarder_follow_the_identity_device_beside_a_lower_one() {
    let store = Arc::new(MemoryPersistence::default());
    let mut log = AuditLogObject::new(7, "forwarder", 16, store.clone()).unwrap();
    log.set_member_of(Some(BACnetDeviceObjectReference {
        device_identifier: Some(oid(ObjectType::DEVICE, 10)),
        ..parent()
    }));
    let binding = DeviceBinding::local(oid(ObjectType::DEVICE, 10), [2]).unwrap();
    let (server, wire) = start_on(&[10], log, Some(binding), 1476).await;
    server.db.write().await.add(device(5, 0)).unwrap();
    let mut f = Fixture {
        server,
        store,
        wire,
    };
    assert!(matches!(
        f.confirmed(201, &[3], payload(false)).await,
        Some(Apdu::SimpleAck(_))
    ));
    settle().await;
    assert!(f.requests().is_empty(), "no copy to this device's own log");
    f.server.stop().await.unwrap();
}
