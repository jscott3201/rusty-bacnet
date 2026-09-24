//! Target route resolution must never reenter caller transport code after startup.
use super::*;

#[tokio::test]
async fn audit_target_routes_are_callback_free_after_startup() {
    for ordinary in [false, true] {
        let mut reporter = reporter();
        let mut operations = AuditOperationFlags::empty();
        operations.insert(AuditOperation::READ);
        operations.insert(AuditOperation::WRITE);
        reporter.set_auditable_operations(operations).unwrap();
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
            audit_reporters: Some(AuditReportersConfig {
                reporters: vec![oid(ObjectType::AUDIT_REPORTER, 1)],
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

#[tokio::test]
async fn audit_target_routes_revalidate_generic_next_hops_after_startup() {
    use bacnet_objects::{
        binary::BinaryValueObject,
        device::{DeviceConfig, DeviceObject},
    };
    for routed in [false, true] {
        for initially_unresolved in [true, false] {
            let bad = oid(ObjectType::DEVICE, 20);
            let good = oid(ObjectType::DEVICE, 21);
            let target = oid(ObjectType::DEVICE, 10);
            let initial = BACnetRecipient::Device(if initially_unresolved { bad } else { good });
            let mut device = DeviceObject::new(DeviceConfig {
                instance: 10,
                ..Default::default()
            })
            .unwrap();
            device.provision_audit_recipient(initial.clone()).unwrap();
            let mut db = ObjectDatabase::new();
            db.add(Box::new(device)).unwrap();
            db.add(Box::new(reporter())).unwrap();
            db.add(Box::new(BinaryValueObject::new(1, "value").unwrap()))
                .unwrap();
            let mut transport = CaptureTransport::default();
            transport.learned_broadcast = Some(MacAddr::from_slice(&[0x42]));
            assert!(!transport.is_broadcast_mac(&[0x42]));
            assert_eq!(transport.bip_broadcast_endpoint(), None);
            let mut server = BACnetServer::start_with_clock_mode_and_bindings(
                ServerConfig {
                    audit_reporters: Some(AuditReportersConfig {
                        reporters: vec![oid(ObjectType::AUDIT_REPORTER, 1)],
                    }),
                    ..Default::default()
                },
                db,
                transport.clone(),
                None,
                vec![
                    if routed {
                        DeviceBinding::routed(bad, 200, [9], [0x42]).unwrap()
                    } else {
                        DeviceBinding::local(bad, [0x42]).unwrap()
                    },
                    DeviceBinding::local(good, NEW_LOGGER).unwrap(),
                ],
            )
            .await
            .unwrap();
            assert!(transport.is_broadcast_mac(&[0x42]));
            let callbacks = transport.route_callbacks.load(Ordering::Acquire);
            transport
                .reject_route_callbacks
                .store(true, Ordering::Release);
            assert_eq!(
                health(&server).await,
                if initially_unresolved {
                    Reliability::CONFIGURATION_ERROR
                } else {
                    Reliability::NO_FAULT_DETECTED
                }
            );
            assert!(matches!(
                write_value(&server, None).await,
                Apdu::SimpleAck(_)
            ));
            settle().await;
            assert_eq!(
                notifications(&transport.sent).len(),
                usize::from(!initially_unresolved)
            );
            assert!(transport
                .destinations
                .lock()
                .unwrap()
                .iter()
                .all(|mac| mac.as_slice() == NEW_LOGGER));
            let sequence = server
                .db
                .read()
                .await
                .reserve_event_sequence_number()
                .number();
            let mut bytes = BytesMut::new();
            bacnet_encoding::constructed::encode_recipient(
                &mut bytes,
                &BACnetRecipient::Device(if initially_unresolved { good } else { bad }),
            );
            if initially_unresolved {
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
            } else {
                assert!(matches!(
                    dispatch(
                        &server,
                        ConfirmedServiceChoice::WRITE_PROPERTY,
                        wp(
                            target,
                            PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
                            bytes.to_vec(),
                            None
                        )
                    )
                    .await,
                    Apdu::Error(_)
                ));
            }
            let mut original = BytesMut::new();
            bacnet_encoding::constructed::encode_recipient(&mut original, &initial);
            assert_eq!(
                server
                    .db
                    .read()
                    .await
                    .get(&target)
                    .unwrap()
                    .read_property(PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT, None)
                    .unwrap(),
                PropertyValue::ApplicationData(original.to_vec())
            );
            assert_eq!(
                server
                    .db
                    .read()
                    .await
                    .reserve_event_sequence_number()
                    .number(),
                sequence
            );
            settle().await;
            assert_eq!(
                notifications(&transport.sent).len(),
                usize::from(!initially_unresolved)
            );
            assert_eq!(transport.route_callbacks.load(Ordering::Acquire), callbacks);
            server.stop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn audit_source_correlation_uses_post_start_generic_route_eligibility() {
    use bacnet_objects::{
        binary::BinaryValueObject,
        device::{DeviceConfig, DeviceObject},
    };
    for routed in [false, true] {
        let logger = oid(ObjectType::DEVICE, 20);
        let bad = oid(ObjectType::DEVICE, 30);
        let healthy = oid(ObjectType::DEVICE, 31);
        let observed = oid(ObjectType::DEVICE, 32);
        let mut db = ObjectDatabase::new();
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
        db.add(Box::new(BinaryValueObject::new(1, "value").unwrap()))
            .unwrap();
        let mut transport = CaptureTransport::default();
        transport.learned_broadcast = Some(MacAddr::from_slice(&[0x42]));
        let mut server = BACnetServer::start_with_clock_mode_and_bindings(
            ServerConfig {
                audit_reporters: Some(AuditReportersConfig {
                    reporters: vec![oid(ObjectType::AUDIT_REPORTER, 1)],
                }),
                ..Default::default()
            },
            db,
            transport.clone(),
            None,
            vec![
                DeviceBinding::local(logger, LOGGER).unwrap(),
                if routed {
                    DeviceBinding::routed(bad, 200, [9], [0x42]).unwrap()
                } else {
                    DeviceBinding::local(bad, [0x42]).unwrap()
                },
                if routed {
                    DeviceBinding::routed(healthy, 200, [10], SOURCE).unwrap()
                } else {
                    DeviceBinding::local(healthy, SOURCE).unwrap()
                },
            ],
        )
        .await
        .unwrap();
        let remote = |mac| {
            routed.then(|| NpduAddress {
                network: 200,
                mac_address: MacAddr::from_slice(&[mac]),
            })
        };
        let observed_remote = remote(11);
        server.device_bindings.write().await.observe_i_am_at(
            observed,
            &[0x44],
            observed_remote.as_ref(),
            Instant::now(),
            |mac| transport.is_broadcast_mac(mac),
        );
        let callbacks = transport.route_callbacks.load(Ordering::Acquire);
        transport
            .reject_route_callbacks
            .store(true, Ordering::Release);
        for (index, (immediate, source, expected)) in [
            (
                &[0x42][..],
                remote(9),
                BACnetRecipient::Address(BACnetAddress {
                    network_number: if routed { 200 } else { 0 },
                    mac_address: MacAddr::from_slice(if routed { &[9] } else { &[0x42] }),
                }),
            ),
            (SOURCE, remote(10), BACnetRecipient::Device(healthy)),
            (
                &[0x44][..],
                observed_remote,
                BACnetRecipient::Device(observed),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let response = dispatch_from(
                &server,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                wp(
                    oid(ObjectType::BINARY_VALUE, 1),
                    PropertyIdentifier::PRESENT_VALUE,
                    vec![0x91, 1],
                    None,
                ),
                immediate,
                source,
            )
            .await
            .unwrap();
            assert!(matches!(response, Apdu::SimpleAck(_)));
            settle().await;
            let records = notifications(&transport.sent);
            assert_eq!(records.len(), index + 1);
            assert_eq!(
                records[index].notifications[0].source_device, expected,
                "routed={routed}, source case={index}"
            );
        }
        assert_eq!(transport.route_callbacks.load(Ordering::Acquire), callbacks);
        server.stop().await.unwrap();
    }
}
