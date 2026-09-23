//! Audit lifecycle operations over actual UDP.
use super::*;

#[tokio::test]
async fn audit_reporter_create_delete_reach_real_log_over_udp() {
    use bacnet_services::{common::BACnetPropertyValue, object_mgmt::ObjectSpecifier};
    use bacnet_types::primitives::PropertyValue;

    let persistence = Arc::new(MemoryPersistence::default());
    let mut logger = start_logger(persistence.clone(), Arc::new(AtomicBool::new(true))).await;

    let mut target_db = database(10);
    target_db
        .get_mut(&oid(ObjectType::DEVICE, 10))
        .unwrap()
        .device_authority_internal()
        .unwrap()
        .provision_audit_recipient(BACnetRecipient::Device(oid(ObjectType::DEVICE, 20)))
        .unwrap();
    let mut reporter = AuditReporterObject::new(1, "reporter").unwrap();
    reporter.set_audit_level(AuditLevel::AUDIT_CONFIG).unwrap();
    let mut operations = AuditOperationFlags::empty();
    for operation in [
        AuditOperation::CREATE,
        AuditOperation::DELETE,
        AuditOperation::WRITE,
    ] {
        operations.insert(operation);
    }
    reporter.set_auditable_operations(operations);
    reporter.set_issue_confirmed_notifications(true);
    reporter.set_monitored_objects(Some(vec![BACnetObjectSelector::ObjectType(
        ObjectType::BINARY_VALUE,
    )]));
    target_db.add(Box::new(reporter)).unwrap();
    let mut target = BACnetServer::builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .database(target_db)
        .audit_reporter(AuditReporterConfig {
            reporter: oid(ObjectType::AUDIT_REPORTER, 1),
        })
        .device_binding(
            DeviceBinding::local(oid(ObjectType::DEVICE, 20), logger.local_mac()).unwrap(),
        )
        .unwrap()
        .build()
        .await
        .unwrap();
    let mut client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .build()
        .await
        .unwrap();
    let created = oid(ObjectType::BINARY_VALUE, 1);
    for step in 0..4 {
        match step {
            0 => {
                let ack = client
                    .create_object(
                        target.local_mac(),
                        ObjectSpecifier::Type(ObjectType::BINARY_VALUE),
                        vec![BACnetPropertyValue {
                            property_identifier: PropertyIdentifier::PRESENT_VALUE,
                            property_array_index: None,
                            value: vec![0x91, 1],
                            priority: Some(8),
                        }],
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    bacnet_encoding::primitives::decode_application_value(&ack, 0)
                        .unwrap()
                        .0,
                    PropertyValue::ObjectIdentifier(created)
                );
                assert_eq!(
                    target
                        .database()
                        .read()
                        .await
                        .get(&created)
                        .unwrap()
                        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
                        .unwrap(),
                    PropertyValue::Enumerated(1)
                );
            }
            1 => {
                let error = client
                    .create_object(
                        target.local_mac(),
                        ObjectSpecifier::Identifier(created),
                        vec![],
                    )
                    .await
                    .unwrap_err();
                assert!(matches!(error, Error::Protocol { class, code }
                    if class == ErrorClass::OBJECT.to_raw() as u32 && code == ErrorCode::OBJECT_IDENTIFIER_ALREADY_EXISTS.to_raw() as u32));
            }
            2 => {
                client
                    .delete_object(target.local_mac(), created)
                    .await
                    .unwrap();
                assert!(target.database().read().await.get(&created).is_none());
            }
            _ => {
                let error = client
                    .delete_object(target.local_mac(), created)
                    .await
                    .unwrap_err();
                assert!(matches!(error, Error::Protocol { class, code }
                    if class == ErrorClass::OBJECT.to_raw() as u32 && code == ErrorCode::UNKNOWN_OBJECT.to_raw() as u32));
            }
        }
        wait_for_records(&persistence, step + 1).await;
    }
    let snapshot = persistence.0.lock().unwrap().clone().unwrap();
    assert_eq!(snapshot.records.len(), 4);
    let mut invokes = std::collections::HashSet::new();
    for (index, stored) in snapshot.records.iter().enumerate() {
        let BACnetAuditLogDatum::AuditNotification(record) = &stored.record.datum else {
            panic!("expected lifecycle")
        };
        assert_eq!(
            record.operation,
            if index < 2 {
                AuditOperation::CREATE
            } else {
                AuditOperation::DELETE
            }
        );
        assert_eq!(record.target_object, Some(created));
        assert_eq!(
            record.target_device,
            BACnetRecipient::Device(oid(ObjectType::DEVICE, 10))
        );
        assert_eq!(
            record.source_device,
            BACnetRecipient::Address(bacnet_types::constructed::BACnetAddress {
                network_number: 0,
                mac_address: bacnet_types::MacAddr::from_slice(client.local_mac()),
            })
        );
        assert!(invokes.insert(record.invoke_id.unwrap()));
        assert!(record.source_timestamp.is_none());
        assert!(record.target_timestamp.is_some());
        assert!(record.target_property.is_none());
        assert!(record.target_priority.is_none());
        assert!(record.target_value.is_none());
        assert!(record.current_value.is_none());
        assert_eq!(
            record.result,
            match index {
                1 => Some((
                    ErrorClass::OBJECT,
                    ErrorCode::OBJECT_IDENTIFIER_ALREADY_EXISTS
                )),
                3 => Some((ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT)),
                _ => None,
            }
        );
    }
    client.stop().await.unwrap();
    target.stop().await.unwrap();
    logger.stop().await.unwrap();
    assert_eq!(persistence.record_count(), 4);
}
