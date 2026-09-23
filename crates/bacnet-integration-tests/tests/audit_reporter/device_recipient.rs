//! Actual B/IP recipient changes notify the old Device route and new Address.
use super::*;
use bacnet_types::{constructed::BACnetAddress, MacAddr};
use bytes::BytesMut;

#[tokio::test]
async fn device_recipient_bip_address_change_delivers_to_both_real_loggers() {
    for confirmed in [false, true] {
        let old_records = Arc::new(MemoryPersistence::default());
        let new_records = Arc::new(MemoryPersistence::default());
        let mut old_logger =
            start_logger(old_records.clone(), Arc::new(AtomicBool::new(true))).await;
        let mut new_logger =
            start_logger(new_records.clone(), Arc::new(AtomicBool::new(true))).await;
        let mut db = database(10);
        db.get_mut(&oid(ObjectType::DEVICE, 10))
            .unwrap()
            .device_authority_internal()
            .unwrap()
            .provision_audit_recipient(BACnetRecipient::Device(oid(ObjectType::DEVICE, 20)))
            .unwrap();
        let mut reporter = AuditReporterObject::new(1, "reporter").unwrap();
        reporter.set_issue_confirmed_notifications(confirmed);
        // The mandatory property-specific pair applies even at Audit_Level NONE.
        db.add(Box::new(reporter)).unwrap();
        let mut target = BACnetServer::builder()
            .interface(Ipv4Addr::LOCALHOST)
            .port(0)
            .database(db)
            .audit_reporter(AuditReporterConfig {
                reporter: oid(ObjectType::AUDIT_REPORTER, 1),
            })
            .device_binding(
                DeviceBinding::local(oid(ObjectType::DEVICE, 20), old_logger.local_mac()).unwrap(),
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
        // These are valid Recipient grammars outside the runtime route subset.
        for (network_number, mac) in [
            (1, vec![127, 0, 0, 1, 0xba, 0xc1]),
            (0, vec![]),
            (0, vec![1, 2, 3]),
            (0, vec![0, 0, 0, 0, 0xba, 0xc1]),
            (0, vec![255, 255, 255, 255, 0xba, 0xc1]),
            (0, vec![224, 0, 0, 1, 0xba, 0xc1]),
            (0, vec![127, 0, 0, 1, 0, 0]),
        ] {
            let mut bytes = BytesMut::new();
            bacnet_encoding::constructed::encode_recipient(
                &mut bytes,
                &BACnetRecipient::Address(BACnetAddress {
                    network_number,
                    mac_address: MacAddr::from_slice(&mac),
                }),
            );
            let error = client
                .write_property(
                    target.local_mac(),
                    oid(ObjectType::DEVICE, 10),
                    PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
                    None,
                    bytes.to_vec(),
                    None,
                )
                .await
                .unwrap_err();
            assert!(
                matches!(error, Error::Protocol { class, code } if class == ErrorClass::SERVICES.to_raw() as u32 && code == ErrorCode::SERVICE_REQUEST_DENIED.to_raw() as u32)
            );
        }
        assert_eq!(old_records.record_count(), 0);
        assert_eq!(new_records.record_count(), 0);
        let address = BACnetRecipient::Address(BACnetAddress {
            network_number: 0,
            mac_address: MacAddr::from_slice(new_logger.local_mac()),
        });
        let mut bytes = BytesMut::new();
        bacnet_encoding::constructed::encode_recipient(&mut bytes, &address);
        client
            .write_property(
                target.local_mac(),
                oid(ObjectType::DEVICE, 10),
                PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
                None,
                bytes.to_vec(),
                None,
            )
            .await
            .unwrap();
        wait_for_records(&old_records, 1).await;
        wait_for_records(&new_records, 1).await;
        let readback = client
            .read_property(
                target.local_mac(),
                oid(ObjectType::DEVICE, 10),
                PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
                None,
            )
            .await
            .unwrap();
        assert_eq!(readback.property_value, bytes.to_vec());
        let old = old_records.0.lock().unwrap().clone().unwrap();
        let new = new_records.0.lock().unwrap().clone().unwrap();
        assert_eq!(old.records[0].record.datum, new.records[0].record.datum);
        client.stop().await.unwrap();
        target.stop().await.unwrap();
        old_logger.stop().await.unwrap();
        new_logger.stop().await.unwrap();
    }
}
