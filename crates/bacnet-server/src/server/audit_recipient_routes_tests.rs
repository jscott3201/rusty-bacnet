//! Target route resolution must never reenter caller transport code after startup.
use super::*;

#[tokio::test]
async fn audit_target_routes_are_callback_free_after_startup() {
    for ordinary in [false, true] {
        let mut reporter = reporter();
        let mut operations = AuditOperationFlags::empty();
        operations.insert(AuditOperation::READ);
        operations.insert(AuditOperation::WRITE);
        reporter.set_auditable_operations(operations);
        let mut fixture = try_server(
            reporter,
            &[10],
            Some(BACnetRecipient::Device(oid(ObjectType::DEVICE, 20))),
            vec![
                DeviceBinding::local(oid(ObjectType::DEVICE, 20), LOGGER).unwrap(),
                DeviceBinding::routed(oid(ObjectType::DEVICE, 21), 200, [9], NEW_LOGGER).unwrap(),
            ],
        )
        .await
        .unwrap();
        let callbacks = fixture.transport.route_callbacks.load(Ordering::Acquire);
        assert!(callbacks > 0, "startup must validate the concrete link");
        fixture
            .transport
            .reject_route_callbacks
            .store(true, Ordering::Release);
        if ordinary {
            assert!(matches!(
                write_value(&fixture.server, None).await,
                Apdu::SimpleAck(_)
            ));
            let mut bytes = BytesMut::new();
            bacnet_services::read_property::ReadPropertyRequest {
                object_identifier: oid(ObjectType::BINARY_VALUE, 1),
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
            }
            .encode(&mut bytes);
            assert!(matches!(
                dispatch(
                    &fixture.server,
                    ConfirmedServiceChoice::READ_PROPERTY,
                    bytes.freeze()
                )
                .await,
                Apdu::ComplexAck(_)
            ));
        } else {
            for direct in [true, false] {
                let mut bytes = BytesMut::new();
                bacnet_encoding::constructed::encode_recipient(
                    &mut bytes,
                    &BACnetRecipient::Device(oid(ObjectType::DEVICE, if direct { 21 } else { 20 })),
                );
                let value = PropertyValue::ApplicationData(bytes.to_vec());
                let target = oid(ObjectType::DEVICE, 10);
                if direct {
                    fixture
                        .server
                        .db
                        .write()
                        .await
                        .get_mut(&target)
                        .unwrap()
                        .write_property(
                            PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
                            None,
                            value,
                            None,
                        )
                        .unwrap();
                } else {
                    fixture
                        .server
                        .write_local(
                            &target,
                            PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
                            None,
                            value,
                            None,
                        )
                        .await
                        .unwrap();
                }
            }
        }
        settle().await;
        let records = notifications(&fixture.transport.sent);
        assert_eq!(records.len(), if ordinary { 2 } else { 4 });
        assert_eq!(
            fixture.transport.route_callbacks.load(Ordering::Acquire),
            callbacks
        );
        fixture.server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn audit_target_routes_use_actual_bound_bip_broadcast_port() {
    use bacnet_objects::device::{DeviceConfig, DeviceObject};
    use std::net::Ipv4Addr;
    let mut db = ObjectDatabase::new();
    let logger = oid(ObjectType::DEVICE, 20);
    let target = oid(ObjectType::DEVICE, 10);
    let mut device = DeviceObject::new(DeviceConfig {
        instance: 10,
        ..Default::default()
    })
    .unwrap();
    device
        .provision_audit_recipient(BACnetRecipient::Device(logger))
        .unwrap();
    db.add(Box::new(device)).unwrap();
    db.add(Box::new(reporter())).unwrap();
    let broadcast = Ipv4Addr::new(127, 0, 0, 255);
    let transport = bacnet_transport::bip::BipTransport::new(Ipv4Addr::LOCALHOST, 0, broadcast);
    let mut server = BACnetServer::start_with_clock_mode_and_bindings(
        ServerConfig {
            audit_reporter: Some(AuditReporterConfig {
                reporter: oid(ObjectType::AUDIT_REPORTER, 1),
            }),
            ..Default::default()
        },
        db,
        transport,
        None,
        vec![DeviceBinding::local(logger, [127, 0, 0, 1, 0xba, 0xc1]).unwrap()],
    )
    .await
    .unwrap();
    let endpoint = server.network.transport().bip_broadcast_endpoint().unwrap();
    assert_ne!(endpoint.port(), 0);
    let recipient = |port: u16| {
        let mut mac = broadcast.octets().to_vec();
        mac.extend_from_slice(&port.to_be_bytes());
        BACnetRecipient::Address(BACnetAddress {
            network_number: 0,
            mac_address: MacAddr::from_slice(&mac),
        })
    };
    let routes = server.notification_transactions.audit_routes.get().unwrap();
    assert!(routes.resolve(&recipient(endpoint.port())).is_none());
    let other_port = if endpoint.port() == u16::MAX {
        endpoint.port() - 1
    } else {
        endpoint.port() + 1
    };
    assert!(
        routes.resolve(&recipient(other_port)).is_some(),
        "another UDP port is not this link broadcast"
    );
    let mut bytes = BytesMut::new();
    bacnet_encoding::constructed::encode_recipient(&mut bytes, &recipient(endpoint.port()));
    assert!(server
        .write_local(
            &target,
            PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
            None,
            PropertyValue::ApplicationData(bytes.to_vec()),
            None
        )
        .await
        .is_err());
    assert_eq!(
        server
            .db
            .read()
            .await
            .reserve_event_sequence_number()
            .number(),
        0
    );
    server.stop().await.unwrap();
}
