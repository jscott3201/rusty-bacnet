//! Target attribution follows current database membership, independently of service success.
use super::*;
use bacnet_objects::{
    device::{DeviceConfig, DeviceObject},
    multistate::MultiStateInputObject,
};
use bacnet_services::{
    list_manipulation::ListElementRequest,
    object_mgmt::{CreateObjectRequest, DeleteObjectRequest, ObjectSpecifier},
    read_property::{ReadPropertyACK, ReadPropertyRequest},
};

fn all_reporter() -> bacnet_objects::audit::AuditReporterObject {
    let mut reporter = reporter();
    let mut operations = AuditOperationFlags::empty();
    for operation in [
        AuditOperation::READ,
        AuditOperation::WRITE,
        AuditOperation::CREATE,
        AuditOperation::DELETE,
        AuditOperation::AUDITING_FAILURE,
    ] {
        operations.insert(operation);
    }
    reporter.set_auditable_operations(operations);
    reporter
}

async fn add_device(fixture: &Fixture, instance: u32) {
    fixture
        .server
        .db
        .write()
        .await
        .add(Box::new(
            DeviceObject::new(DeviceConfig {
                instance,
                name: format!("Device {instance}"),
                ..Default::default()
            })
            .unwrap(),
        ))
        .unwrap();
}

async fn assert_suppressed(fixture: &Fixture, records: usize, sequence: u16) {
    settle().await;
    assert_eq!(notifications(&fixture.transport.sent).len(), records);
    assert_eq!(
        health(&fixture.server).await,
        Reliability::CONFIGURATION_ERROR
    );
    assert_eq!(fixture.server.notification_transactions.active_count(), 0);
    assert_eq!(
        fixture.server.notification_transactions.audit_resources(),
        (false, 0, 64)
    );
    assert_eq!(
        fixture
            .server
            .db
            .read()
            .await
            .reserve_event_sequence_number()
            .number(),
        sequence
    );
}

async fn read_value(fixture: &Fixture, expected: u8) {
    let mut data = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid(ObjectType::BINARY_VALUE, 1),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    }
    .encode(&mut data);
    let response = dispatch(
        &fixture.server,
        ConfirmedServiceChoice::READ_PROPERTY,
        data.freeze(),
    )
    .await;
    let Apdu::ComplexAck(ack) = response else {
        panic!("{response:?}")
    };
    assert_eq!(
        ReadPropertyACK::decode(&ack.service_ack)
            .unwrap()
            .property_value,
        vec![0x91, expected]
    );
}

#[tokio::test]
async fn audit_reporter_identity_startup_requires_exactly_one_device() {
    for devices in [&[][..], &[10, 11][..], &[10][..]] {
        let mut fixture = server_with_devices(all_reporter(), devices).await;
        let unique = devices.len() == 1;
        assert_eq!(
            health(&fixture.server).await,
            if unique {
                Reliability::NO_FAULT_DETECTED
            } else {
                Reliability::CONFIGURATION_ERROR
            }
        );
        read_value(&fixture, 0).await;
        assert!(matches!(
            write_value(&fixture.server, None).await,
            Apdu::SimpleAck(_)
        ));
        assert_eq!(fixture.writes.load(Ordering::Acquire), 1);
        settle().await;
        if unique {
            let records = notifications(&fixture.transport.sent);
            assert_eq!(records.len(), 2);
            for (index, record) in records.iter().enumerate() {
                assert_eq!(
                    record.notifications[0].target_device,
                    BACnetRecipient::Device(oid(ObjectType::DEVICE, 10))
                );
                assert_eq!(
                    record.notifications[0].target_timestamp,
                    Some(BACnetTimeStamp::SequenceNumber(index as u16))
                );
            }
        } else {
            assert_suppressed(&fixture, 0, 0).await;
        }
        fixture.server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn audit_reporter_identity_live_read_write_recover_without_consuming_sequence() {
    let mut fixture = server(all_reporter()).await;
    read_value(&fixture, 0).await;
    settle().await;
    add_device(&fixture, 11).await;
    // WRITE refreshes health after a live addition; the ordinary commit still succeeds.
    assert!(matches!(
        write_value(&fixture.server, None).await,
        Apdu::SimpleAck(_)
    ));
    assert_eq!(fixture.writes.load(Ordering::Acquire), 1);
    assert_suppressed(&fixture, 1, 1).await;
    read_value(&fixture, 1).await;
    assert_suppressed(&fixture, 1, 1).await;
    fixture
        .server
        .db
        .write()
        .await
        .remove(&oid(ObjectType::DEVICE, 10))
        .unwrap();
    read_value(&fixture, 1).await;
    settle().await;
    assert_eq!(
        health(&fixture.server).await,
        Reliability::NO_FAULT_DETECTED
    );
    let records = notifications(&fixture.transport.sent);
    assert_eq!(records.len(), 2);
    assert_eq!(
        records[1].notifications[0].target_device,
        BACnetRecipient::Device(oid(ObjectType::DEVICE, 11))
    );
    assert_eq!(
        records[1].notifications[0].target_timestamp,
        Some(BACnetTimeStamp::SequenceNumber(1))
    );
    fixture
        .server
        .db
        .write()
        .await
        .remove(&oid(ObjectType::DEVICE, 11))
        .unwrap();
    read_value(&fixture, 1).await;
    assert_suppressed(&fixture, 2, 2).await;
    add_device(&fixture, 12).await;
    assert!(matches!(
        write_value(&fixture.server, None).await,
        Apdu::SimpleAck(_)
    ));
    settle().await;
    let records = notifications(&fixture.transport.sent);
    assert_eq!(records.len(), 3);
    assert_eq!(
        records[2].notifications[0].target_device,
        BACnetRecipient::Device(oid(ObjectType::DEVICE, 12))
    );
    assert_eq!(
        records[2].notifications[0].target_timestamp,
        Some(BACnetTimeStamp::SequenceNumber(2))
    );
    assert_eq!(
        health(&fixture.server).await,
        Reliability::NO_FAULT_DETECTED
    );
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn audit_reporter_identity_list_lifecycle_mutate_without_ambiguous_records() {
    let mut fixture = server_with_devices(all_reporter(), &[10, 11]).await;
    let target = oid(ObjectType::MULTI_STATE_INPUT, 1);
    fixture
        .server
        .db
        .write()
        .await
        .add(Box::new(MultiStateInputObject::new(1, "list", 3).unwrap()))
        .unwrap();
    let mut data = BytesMut::new();
    ListElementRequest {
        object_identifier: target,
        property_identifier: PropertyIdentifier::ALARM_VALUES,
        property_array_index: None,
        list_of_elements: vec![0x21, 2],
    }
    .encode(&mut data);
    assert!(matches!(
        dispatch(
            &fixture.server,
            ConfirmedServiceChoice::ADD_LIST_ELEMENT,
            data.freeze()
        )
        .await,
        Apdu::SimpleAck(_)
    ));
    assert_eq!(
        fixture
            .server
            .db
            .read()
            .await
            .get(&target)
            .unwrap()
            .read_property(PropertyIdentifier::ALARM_VALUES, None)
            .unwrap(),
        PropertyValue::List(vec![PropertyValue::Unsigned(2)])
    );
    assert_suppressed(&fixture, 0, 0).await;
    let created = oid(ObjectType::BINARY_VALUE, 99);
    let mut data = BytesMut::new();
    CreateObjectRequest {
        object_specifier: ObjectSpecifier::Identifier(created),
        list_of_initial_values: vec![],
    }
    .encode(&mut data);
    assert!(matches!(
        dispatch(
            &fixture.server,
            ConfirmedServiceChoice::CREATE_OBJECT,
            data.freeze()
        )
        .await,
        Apdu::ComplexAck(_)
    ));
    assert!(fixture.server.db.read().await.get(&created).is_some());
    assert_suppressed(&fixture, 0, 0).await;
    let mut data = BytesMut::new();
    DeleteObjectRequest {
        object_identifier: created,
    }
    .encode(&mut data);
    assert!(matches!(
        dispatch(
            &fixture.server,
            ConfirmedServiceChoice::DELETE_OBJECT,
            data.freeze()
        )
        .await,
        Apdu::SimpleAck(_)
    ));
    assert!(fixture.server.db.read().await.get(&created).is_none());
    assert_suppressed(&fixture, 0, 0).await;
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn audit_reporter_identity_file_commit_survives_ambiguous_device() {
    let mut fixture = file::file_server(false).await;
    add_device(&fixture, 11).await;
    let target = oid(ObjectType::FILE, 1);
    assert!(matches!(
        dispatch(
            &fixture.server,
            file::SERVICE,
            file::request(target, file::access(false, 0))
        )
        .await,
        Apdu::ComplexAck(_)
    ));
    assert_eq!(
        fixture
            .server
            .db
            .read()
            .await
            .get(&target)
            .unwrap()
            .file_storage_internal()
            .unwrap()
            .read_stream(0, 100)
            .unwrap()
            .data,
        vec![9, 8]
    );
    assert_suppressed(&fixture, 0, 0).await;
    fixture
        .server
        .db
        .write()
        .await
        .remove(&oid(ObjectType::DEVICE, 10))
        .unwrap();
    assert!(matches!(
        dispatch(
            &fixture.server,
            file::SERVICE,
            file::request(target, file::access(false, 0))
        )
        .await,
        Apdu::ComplexAck(_)
    ));
    settle().await;
    let records = notifications(&fixture.transport.sent);
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].notifications[0].target_device,
        BACnetRecipient::Device(oid(ObjectType::DEVICE, 11))
    );
    assert_eq!(
        records[0].notifications[0].target_timestamp,
        Some(BACnetTimeStamp::SequenceNumber(0))
    );
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn audit_reporter_identity_unique_device_preserves_configured_routed_logger() {
    let mut fixture = server(all_reporter()).await;
    fixture
        .server
        .config
        .audit_reporter
        .as_mut()
        .unwrap()
        .recipient = Some(oid(ObjectType::DEVICE, 21));
    fixture
        .server
        .device_bindings
        .write()
        .await
        .insert_configured(
            DeviceBinding::routed(oid(ObjectType::DEVICE, 21), 200, [9], LOGGER).unwrap(),
            |_| false,
        )
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
    assert_eq!(
        health(&fixture.server).await,
        Reliability::NO_FAULT_DETECTED
    );
    fixture.server.stop().await.unwrap();
}
