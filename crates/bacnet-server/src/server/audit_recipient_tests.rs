//! Observable target recipient changes through the same wire and local owners.
use super::*;
use bacnet_objects::audit::AuditReporterObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};

fn device(instance: u32) -> BACnetRecipient {
    BACnetRecipient::Device(oid(ObjectType::DEVICE, instance))
}
fn value(recipient: &BACnetRecipient) -> Vec<u8> {
    let mut data = BytesMut::new();
    bacnet_encoding::constructed::encode_recipient(&mut data, recipient);
    data.to_vec()
}
async fn fixture(reporter: AuditReporterObject) -> Fixture {
    try_server(
        reporter,
        &[10],
        Some(device(20)),
        vec![
            DeviceBinding::local(oid(ObjectType::DEVICE, 20), LOGGER).unwrap(),
            DeviceBinding::local(oid(ObjectType::DEVICE, 21), NEW_LOGGER).unwrap(),
            DeviceBinding::local(oid(ObjectType::DEVICE, 22), LOGGER).unwrap(),
        ],
    )
    .await
    .unwrap()
}
async fn read_recipient(fixture: &Fixture) -> PropertyValue {
    fixture
        .server
        .db
        .read()
        .await
        .get(&oid(ObjectType::DEVICE, 10))
        .unwrap()
        .read_property(PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT, None)
        .unwrap()
}
async fn change(fixture: &Fixture, recipient: &BACnetRecipient) -> Apdu {
    dispatch(
        &fixture.server,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        wp(
            oid(ObjectType::DEVICE, 10),
            PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
            value(recipient),
            None,
        ),
    )
    .await
}
fn assert_pair(
    fixture: &Fixture,
    source: BACnetRecipient,
    invoke: Option<u8>,
    sequence: u16,
    old: BACnetRecipient,
    new: BACnetRecipient,
) {
    let records = notifications(&fixture.transport.sent);
    assert_eq!(records.len(), 2);
    let a = &records[0].notifications[0];
    assert_eq!(*a, records[1].notifications[0]);
    assert_eq!(a.source_device, source);
    assert_eq!(a.invoke_id, invoke);
    assert_eq!(a.target_device, device(10));
    assert_eq!(a.target_object, Some(oid(ObjectType::DEVICE, 10)));
    assert_eq!(
        a.target_property,
        Some(AuditPropertyReference {
            property_identifier: PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
            property_array_index: None
        })
    );
    assert_eq!(
        a.target_timestamp,
        Some(BACnetTimeStamp::SequenceNumber(sequence))
    );
    assert_eq!(a.source_timestamp, None);
    assert_eq!(a.operation, AuditOperation::WRITE);
    assert_eq!(a.target_priority, None);
    assert_eq!(a.target_value, Some(value(&new)));
    assert_eq!(a.current_value, Some(value(&old)));
    assert_eq!(a.result, None);
}

#[tokio::test]
async fn recipient_wire_change_notifies_both_even_when_ordinary_reporting_is_disabled() {
    for level in [AuditLevel::NONE, AuditLevel::AUDIT_ALL] {
        let mut reporter = reporter();
        reporter.set_audit_level(level).unwrap();
        reporter.set_auditable_operations(AuditOperationFlags::empty());
        let mut fixture = fixture(reporter).await;
        assert!(matches!(
            change(&fixture, &device(21)).await,
            Apdu::SimpleAck(_)
        ));
        settle().await;
        assert_pair(
            &fixture,
            BACnetRecipient::Address(BACnetAddress {
                network_number: 0,
                mac_address: MacAddr::from_slice(SOURCE),
            }),
            Some(77),
            0,
            device(20),
            device(21),
        );
        assert_eq!(
            *fixture.transport.destinations.lock().unwrap(),
            vec![LOGGER.to_vec(), NEW_LOGGER.to_vec()]
        );
        assert_eq!(
            read_recipient(&fixture).await,
            PropertyValue::ApplicationData(value(&device(21)))
        );
        assert!(matches!(
            write_value(&fixture.server, None).await,
            Apdu::SimpleAck(_)
        ));
        settle().await;
        assert_eq!(
            notifications(&fixture.transport.sent).len(),
            2,
            "ordinary WRITE filter remains disabled"
        );
        fixture.server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn recipient_direct_and_server_local_writes_use_the_same_owner() {
    for direct in [false, true] {
        let mut fixture = fixture(reporter()).await;
        let target = oid(ObjectType::DEVICE, 10);
        let new = PropertyValue::ApplicationData(value(&device(21)));
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
                    new,
                    Some(16),
                )
                .unwrap();
        } else {
            fixture
                .server
                .write_local(
                    &target,
                    PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
                    None,
                    new,
                    None,
                )
                .await
                .unwrap();
        }
        settle().await;
        assert_pair(&fixture, device(10), None, 0, device(20), device(21));
        fixture.server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn recipient_second_reservation_failure_and_noops_are_atomic() {
    let mut fixture = fixture(reporter()).await;
    let permits = (0..63)
        .map(|_| {
            fixture
                .server
                .notification_transactions
                .try_admit_audit()
                .unwrap()
        })
        .collect::<Vec<_>>();
    let status = fixture
        .server
        .db
        .read()
        .await
        .get(&oid(ObjectType::AUDIT_REPORTER, 1))
        .unwrap()
        .audit_reporter_internal()
        .unwrap()
        .status_internal();
    let before = status.begin_delivery();
    assert!(matches!(
        change(&fixture, &device(21)).await,
        Apdu::Error(_)
    ));
    assert_eq!(status.begin_delivery(), before);
    assert_eq!(
        read_recipient(&fixture).await,
        PropertyValue::ApplicationData(value(&device(20)))
    );
    assert_eq!(
        fixture.server.notification_transactions.audit_resources(),
        (false, 0, 1)
    );
    for bytes in [vec![0], value(&device(20))] {
        assert!(matches!(
            dispatch(
                &fixture.server,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                wp(
                    oid(ObjectType::DEVICE, 10),
                    PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
                    bytes,
                    Some(1)
                )
            )
            .await,
            Apdu::SimpleAck(_)
        ));
    }
    assert_eq!(status.begin_delivery(), before);
    assert_eq!(
        fixture
            .server
            .db
            .read()
            .await
            .reserve_event_sequence_number()
            .number(),
        0
    );
    assert!(fixture.transport.sent.lock().unwrap().is_empty());
    drop(permits);
    assert!(matches!(
        change(&fixture, &device(21)).await,
        Apdu::SimpleAck(_)
    ));
    settle().await;
    assert_eq!(notifications(&fixture.transport.sent).len(), 2);
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn recipient_different_values_on_same_route_still_admit_two_attempts() {
    let mut fixture = fixture(reporter()).await;
    assert!(matches!(
        change(&fixture, &device(22)).await,
        Apdu::SimpleAck(_)
    ));
    settle().await;
    assert_eq!(
        *fixture.transport.destinations.lock().unwrap(),
        vec![LOGGER.to_vec(), LOGGER.to_vec()]
    );
    assert_eq!(notifications(&fixture.transport.sent).len(), 2);
    assert_eq!(
        fixture
            .server
            .db
            .read()
            .await
            .reserve_event_sequence_number()
            .number(),
        1
    );
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn recipient_wpm_commits_change_once_and_preserves_prefix_on_unavailable_suffix() {
    let mut fixture = fixture(reporter()).await;
    let mut bytes = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid(ObjectType::DEVICE, 10),
            list_of_properties: [21, 999, 20]
                .into_iter()
                .map(|instance| BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
                    property_array_index: None,
                    value: value(&device(instance)),
                    priority: None,
                })
                .collect(),
        }],
    }
    .encode(&mut bytes)
    .unwrap();
    assert!(matches!(
        dispatch(
            &fixture.server,
            ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
            bytes.freeze()
        )
        .await,
        Apdu::Error(_)
    ));
    settle().await;
    assert_pair(
        &fixture,
        BACnetRecipient::Address(BACnetAddress {
            network_number: 0,
            mac_address: MacAddr::from_slice(SOURCE),
        }),
        Some(77),
        0,
        device(20),
        device(21),
    );
    assert_eq!(
        read_recipient(&fixture).await,
        PropertyValue::ApplicationData(value(&device(21)))
    );
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn recipient_invalid_and_unauthorized_requests_preserve_value_epoch_and_sequence() {
    let mut fixture = fixture(reporter()).await;
    let target = oid(ObjectType::DEVICE, 10);
    let status = fixture
        .server
        .db
        .read()
        .await
        .get(&oid(ObjectType::AUDIT_REPORTER, 1))
        .unwrap()
        .audit_reporter_internal()
        .unwrap()
        .status_internal();
    let before = status.begin_delivery();
    for (bytes, index, priority) in [
        (vec![0, 0], None, None),
        (vec![0x21, 1], None, None),
        (value(&device(21)), Some(0), None),
        (value(&device(21)), None, Some(0)),
        (value(&device(21)), None, Some(17)),
        (value(&device(ObjectIdentifier::MAX_INSTANCE)), None, None),
        (
            value(&BACnetRecipient::Device(oid(ObjectType::ANALOG_VALUE, 21))),
            None,
            None,
        ),
    ] {
        let mut data = BytesMut::new();
        WritePropertyRequest {
            object_identifier: target,
            property_identifier: PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
            property_array_index: index,
            property_value: bytes,
            priority: None,
        }
        .encode(&mut data)
        .unwrap();
        // Preserve external malformed-priority requests independently of typed output.
        if let Some(priority) = priority {
            bacnet_encoding::primitives::encode_ctx_unsigned(&mut data, 4, priority);
        }
        assert!(matches!(
            dispatch(
                &fixture.server,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                data.freeze()
            )
            .await,
            Apdu::Error(_)
        ));
    }
    for panic in [false, true] {
        fixture.server.config.mutation_authorizer = Some(Arc::new(move |_| {
            assert!(!panic, "authorization panic");
            false
        }));
        assert!(matches!(
            change(&fixture, &device(21)).await,
            Apdu::Error(_)
        ));
    }
    assert_eq!(status.begin_delivery(), before);
    assert_eq!(
        read_recipient(&fixture).await,
        PropertyValue::ApplicationData(value(&device(20)))
    );
    assert_eq!(
        fixture
            .server
            .db
            .read()
            .await
            .reserve_event_sequence_number()
            .number(),
        0
    );
    assert_eq!(
        fixture.server.notification_transactions.audit_resources(),
        (false, 0, 64)
    );
    assert!(fixture.transport.sent.lock().unwrap().is_empty());
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn recipient_unavailable_old_route_cannot_be_bypassed_by_a_valid_new_route() {
    let mut fixture = try_server(
        reporter(),
        &[10],
        Some(device(999)),
        vec![DeviceBinding::local(oid(ObjectType::DEVICE, 21), NEW_LOGGER).unwrap()],
    )
    .await
    .unwrap();
    assert!(matches!(
        change(&fixture, &device(21)).await,
        Apdu::Error(_)
    ));
    assert_eq!(
        health(&fixture.server).await,
        Reliability::CONFIGURATION_ERROR
    );
    assert_eq!(
        read_recipient(&fixture).await,
        PropertyValue::ApplicationData(value(&device(999)))
    );
    assert_eq!(
        fixture
            .server
            .db
            .read()
            .await
            .reserve_event_sequence_number()
            .number(),
        0
    );
    assert_eq!(
        fixture.server.notification_transactions.audit_resources(),
        (false, 0, 64)
    );
    assert!(fixture.transport.sent.lock().unwrap().is_empty());
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn recipient_active_metadata_rpm_and_property_list_follow_runtime_presence() {
    use bacnet_objects::property_metadata::{PropertyConformance, PropertyPresenceCondition};
    use bacnet_services::{
        common::PropertyReference,
        rpm::{ReadAccessSpecification, ReadPropertyMultipleACK, ReadPropertyMultipleRequest},
    };
    let mut fixture = fixture(reporter()).await;
    let target = oid(ObjectType::DEVICE, 10);
    let property = PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT;
    for active in [true, false] {
        if !active {
            fixture.server.stop().await.unwrap();
        }
        let db = fixture.server.db.read().await;
        let object = db.get(&target).unwrap();
        assert_eq!(object.property_list().contains(&property), active);
        assert_eq!(object.required_properties().contains(&property), active);
        assert_eq!(object.is_writable_property(property), active);
        if active {
            let metadata = object.property_metadata();
            let row = metadata
                .iter()
                .find(|row| row.property_identifier == property)
                .unwrap();
            assert_eq!(row.conformance, PropertyConformance::Optional);
            assert_eq!(
                row.presence_condition,
                Some(PropertyPresenceCondition::AuditReporting)
            );
            assert!(row.is_required());
        } else {
            assert!(object.read_property(property, None).is_err());
        }
        for (selector, expected) in [
            (PropertyIdentifier::REQUIRED, active),
            (PropertyIdentifier::OPTIONAL, false),
            (PropertyIdentifier::ALL, active),
        ] {
            let mut data = BytesMut::new();
            ReadPropertyMultipleRequest {
                list_of_read_access_specs: vec![ReadAccessSpecification {
                    object_identifier: target,
                    list_of_property_references: vec![PropertyReference {
                        property_identifier: selector,
                        property_array_index: None,
                    }],
                }],
            }
            .encode(&mut data)
            .unwrap();
            let mut response = BytesMut::new();
            crate::handlers::handle_read_property_multiple(&db, &data, &mut response).unwrap();
            let ack = ReadPropertyMultipleACK::decode(&response).unwrap();
            assert_eq!(
                ack.list_of_read_access_results[0]
                    .list_of_results
                    .iter()
                    .any(|entry| entry.property_identifier == property
                        && entry.error.is_none()
                        && entry.property_value.as_ref() == Some(&value(&device(20)))),
                expected
            );
        }
    }
}

#[path = "audit_recipient_lifecycle_tests.rs"]
mod lifecycle;

#[tokio::test]
async fn recipient_wpm_authorizer_denies_after_committed_prefix_without_suffix_attempt() {
    let mut fixture = fixture(reporter()).await;
    let decisions = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let captured = Arc::clone(&decisions);
    fixture.server.config.mutation_authorizer = Some(Arc::new(move |context| {
        assert_eq!(context.invoke_id, 77);
        assert_eq!(context.source_mac.as_slice(), SOURCE);
        captured.fetch_add(1, Ordering::AcqRel) == 0
    }));
    let mut bytes = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid(ObjectType::DEVICE, 10),
            list_of_properties: [21, 20, 22]
                .into_iter()
                .map(|instance| BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
                    property_array_index: None,
                    value: value(&device(instance)),
                    priority: None,
                })
                .collect(),
        }],
    }
    .encode(&mut bytes)
    .unwrap();
    assert!(matches!(
        dispatch(
            &fixture.server,
            ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
            bytes.freeze()
        )
        .await,
        Apdu::Error(_)
    ));
    settle().await;
    assert_eq!(decisions.load(Ordering::Acquire), 2);
    assert_eq!(
        read_recipient(&fixture).await,
        PropertyValue::ApplicationData(value(&device(21)))
    );
    assert_pair(
        &fixture,
        BACnetRecipient::Address(BACnetAddress {
            network_number: 0,
            mac_address: MacAddr::from_slice(SOURCE),
        }),
        Some(77),
        0,
        device(20),
        device(21),
    );
    assert_eq!(
        fixture
            .server
            .db
            .read()
            .await
            .reserve_event_sequence_number()
            .number(),
        1
    );
    assert_eq!(
        fixture.server.notification_transactions.audit_resources(),
        (false, 0, 64)
    );
    fixture.server.stop().await.unwrap();
}
