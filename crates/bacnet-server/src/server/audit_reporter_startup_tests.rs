use super::*;
use bacnet_objects::{
    binary::BinaryValueObject,
    device::{DeviceConfig, DeviceObject},
};

async fn start_profile(
    selected: ObjectIdentifier,
    recipient: Option<ObjectIdentifier>,
    transport: CaptureTransport,
) -> Result<BACnetServer<CaptureTransport>, Error> {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            instance: 10,
            ..Default::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db.add(Box::new(BinaryValueObject::new(1, "value").unwrap()))
        .unwrap();
    // An available Reporter must not silently substitute for the selected one.
    db.add(Box::new(reporter())).unwrap();
    BACnetServer::start_with_clock_mode_and_bindings(
        ServerConfig {
            audit_reporter: Some(AuditReporterConfig {
                reporter: selected,
                recipient,
            }),
            ..Default::default()
        },
        db,
        transport,
        None,
        vec![DeviceBinding::local(oid(ObjectType::DEVICE, 20), LOGGER).unwrap()],
    )
    .await
}

async fn assert_invalid_selection(selected: ObjectIdentifier) {
    let transport = CaptureTransport::default();
    let error = match start_profile(
        selected,
        Some(oid(ObjectType::DEVICE, 20)),
        transport.clone(),
    )
    .await
    {
        Err(error) => error,
        Ok(mut server) => {
            server.stop().await.unwrap();
            panic!("startup accepted invalid selected Reporter {selected:?}");
        }
    };
    assert!(
        matches!(error, Error::Encoding(ref message)
        if message.starts_with("invalid audit reporter:")
            && message.contains("Audit Reporter capability")),
        "{error:?}"
    );
    assert!(!transport.started.load(Ordering::Acquire));
    assert!(transport.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn audit_reporter_startup_rejects_absent_selected_object() {
    assert_invalid_selection(oid(ObjectType::AUDIT_REPORTER, 99)).await;
}

#[tokio::test]
async fn audit_reporter_startup_rejects_wrong_type_selected_object() {
    assert_invalid_selection(oid(ObjectType::BINARY_VALUE, 1)).await;
}

#[tokio::test]
async fn audit_reporter_startup_preserves_recipient_configuration_health() {
    for (recipient, expected) in [
        (None, Reliability::CONFIGURATION_ERROR),
        (
            Some(oid(ObjectType::DEVICE, 999)),
            Reliability::CONFIGURATION_ERROR,
        ),
        (
            Some(oid(ObjectType::DEVICE, 20)),
            Reliability::NO_FAULT_DETECTED,
        ),
    ] {
        let transport = CaptureTransport::default();
        let mut server = start_profile(
            oid(ObjectType::AUDIT_REPORTER, 1),
            recipient,
            transport.clone(),
        )
        .await
        .unwrap();
        assert!(transport.started.load(Ordering::Acquire));
        assert_eq!(health(&server).await, expected);
        assert!(transport.sent.lock().unwrap().is_empty());
        assert_eq!(server.notification_transactions.active_count(), 0);
        server.stop().await.unwrap();
    }
}
