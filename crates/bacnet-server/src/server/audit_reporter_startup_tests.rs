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
    start_profiles(vec![selected], recipient, transport).await
}

async fn start_profiles(
    selected: Vec<ObjectIdentifier>,
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
    if let Some(recipient) = recipient {
        db.get_mut(&oid(ObjectType::DEVICE, 10))
            .unwrap()
            .device_authority_internal()
            .unwrap()
            .provision_audit_recipient(BACnetRecipient::Device(recipient))
            .unwrap();
    }
    db.add(Box::new(BinaryValueObject::new(1, "value").unwrap()))
        .unwrap();
    // An available Reporter must not silently substitute for the selected one.
    db.add(Box::new(reporter())).unwrap();
    BACnetServer::start_with_clock_mode_and_bindings(
        ServerConfig {
            audit_reporters: Some(AuditReportersConfig {
                reporters: selected,
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
    assert!(matches!(error, Error::Encoding(_)), "{error:?}");
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

#[tokio::test]
async fn audit_recipient_non_bip_six_byte_mac_is_not_an_address_capability() {
    let mut transport = CaptureTransport::default();
    transport.six_byte_mac = true;
    // Six octets on a generic link do not establish the B/IP address grammar.
    let mut db = ObjectDatabase::new();
    let mut device = DeviceObject::new(DeviceConfig {
        instance: 10,
        ..Default::default()
    })
    .unwrap();
    device
        .provision_audit_recipient(BACnetRecipient::Address(BACnetAddress {
            network_number: 0,
            mac_address: MacAddr::from_slice(&[127, 0, 0, 1, 0xba, 0xc1]),
        }))
        .unwrap();
    db.add(Box::new(device)).unwrap();
    db.add(Box::new(reporter())).unwrap();
    let result = BACnetServer::start_with_clock_mode_and_bindings(
        ServerConfig {
            audit_reporters: Some(AuditReportersConfig {
                reporters: vec![oid(ObjectType::AUDIT_REPORTER, 1)],
            }),
            ..Default::default()
        },
        db,
        transport.clone(),
        None,
        vec![],
    )
    .await;
    assert!(result.is_err());
    assert!(!transport.started.load(Ordering::Acquire));
    let mut server = start_profile(
        oid(ObjectType::AUDIT_REPORTER, 1),
        Some(oid(ObjectType::DEVICE, 20)),
        transport.clone(),
    )
    .await
    .unwrap();
    assert!(
        transport.started.load(Ordering::Acquire),
        "configured Device routes remain link independent"
    );
    server.stop().await.unwrap();
}

#[tokio::test]
async fn target_reporter_set_validation_precedes_transport_start() {
    let first = oid(ObjectType::AUDIT_REPORTER, 1);
    for selected in [
        vec![],
        vec![first, first],
        vec![first; 65],
        vec![oid(ObjectType::BINARY_VALUE, 1)],
        vec![oid(
            ObjectType::AUDIT_REPORTER,
            ObjectIdentifier::MAX_INSTANCE,
        )],
        vec![first, oid(ObjectType::AUDIT_REPORTER, 2)],
    ] {
        let transport = CaptureTransport::default();
        assert!(start_profiles(
            selected,
            Some(oid(ObjectType::DEVICE, 20)),
            transport.clone()
        )
        .await
        .is_err());
        assert!(!transport.started.load(Ordering::Acquire));
        assert!(transport.sent.lock().unwrap().is_empty());
    }
}
