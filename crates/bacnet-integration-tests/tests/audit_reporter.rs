//! RB-21a/b/c/d/e: mutation outcomes -> selected target Reporter -> Audit Log over UDP.

use std::net::Ipv4Addr;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use bacnet_client::client::BACnetClient;
use bacnet_objects::{
    audit::{AuditLogObject, AuditLogPersistence, AuditLogSnapshot, AuditReporterObject},
    binary::BinaryValueObject,
    database::ObjectDatabase,
    device::{DeviceConfig, DeviceObject},
};
use bacnet_server::server::{AuditReporterConfig, BACnetServer, DeviceBinding};
use bacnet_services::audit::AuditLogQueryRequest;
use bacnet_types::{
    bitstring::AuditOperationFlags,
    constructed::{
        BACnetAuditLogDatum, BACnetAuditLogQueryParameters, BACnetObjectSelector, BACnetRecipient,
    },
    enums::{
        AuditLevel, AuditOperation, BACnetSuccessFilter, ErrorClass, ErrorCode, ObjectType,
        PropertyIdentifier, Reliability,
    },
    error::Error,
    primitives::ObjectIdentifier,
};

#[derive(Default)]
struct MemoryPersistence(Mutex<Option<AuditLogSnapshot>>);

impl AuditLogPersistence for MemoryPersistence {
    fn load(&self, _: ObjectIdentifier) -> Result<Option<AuditLogSnapshot>, Error> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn commit(&self, snapshot: &AuditLogSnapshot) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(snapshot.clone());
        Ok(())
    }
}

fn oid(kind: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(kind, instance).unwrap()
}

fn database(instance: u32) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            instance,
            name: format!("Device-{instance}"),
            ..Default::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db
}

async fn exercise(confirmed: bool, selected: bool, lists: bool) {
    let allowed = Arc::new(AtomicBool::new(!confirmed));
    let policy = Arc::clone(&allowed);
    let persistence = Arc::new(MemoryPersistence::default());
    let mut logger_db = database(20);
    logger_db
        .add(Box::new(
            AuditLogObject::new(1, "log", 16, persistence.clone()).unwrap(),
        ))
        .unwrap();
    let mut logger = BACnetServer::builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .database(logger_db)
        .audit_notification_sink(oid(ObjectType::AUDIT_LOG, 1))
        .audit_notification_authorizer(move |_| policy.load(Ordering::Acquire))
        .unconfirmed_audit_notification_authorizer(|_| true)
        .build()
        .await
        .unwrap();

    let mut target_db = database(10);
    if lists {
        let mut list =
            bacnet_objects::multistate::MultiStateInputObject::new(1, "list", 3).unwrap();
        list.set_alarm_values(vec![1]);
        target_db.add(Box::new(list)).unwrap();
    }
    target_db
        .add(Box::new(BinaryValueObject::new(1, "value").unwrap()))
        .unwrap();
    target_db
        .add(Box::new(
            BinaryValueObject::new(2, "unselected-value").unwrap(),
        ))
        .unwrap();
    let mut reporter = AuditReporterObject::new(1, "reporter").unwrap();
    reporter.set_audit_level(AuditLevel::AUDIT_ALL).unwrap();
    let mut operations = AuditOperationFlags::empty();
    operations.insert(AuditOperation::WRITE);
    reporter.set_auditable_operations(operations);
    reporter.set_issue_confirmed_notifications(confirmed);
    if selected {
        reporter.set_monitored_objects(Some(vec![
            BACnetObjectSelector::None,
            BACnetObjectSelector::Object(oid(ObjectType::BINARY_VALUE, 1)),
            BACnetObjectSelector::Object(oid(ObjectType::BINARY_VALUE, 1)),
        ]));
    }
    target_db.add(Box::new(reporter)).unwrap();
    let mut target = BACnetServer::builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .database(target_db)
        .audit_reporter(AuditReporterConfig {
            reporter: oid(ObjectType::AUDIT_REPORTER, 1),
            recipient: Some(oid(ObjectType::DEVICE, 20)),
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

    if selected {
        // Both an unmatched commit and an unmatched execution failure remain
        // silent; the matching outcomes below still reach the real recipient.
        client
            .write_property(
                target.local_mac(),
                oid(ObjectType::BINARY_VALUE, 2),
                PropertyIdentifier::PRESENT_VALUE,
                None,
                vec![0x91, 1],
                None,
            )
            .await
            .unwrap();
        let error = client
            .write_property(
                target.local_mac(),
                oid(ObjectType::BINARY_VALUE, 2),
                PropertyIdentifier::PRESENT_VALUE,
                None,
                vec![0x91, 9],
                None,
            )
            .await
            .unwrap_err();
        assert!(matches!(error, Error::Protocol { class, code }
            if class == ErrorClass::PROPERTY.to_raw() as u32
                && code == ErrorCode::VALUE_OUT_OF_RANGE.to_raw() as u32));
        assert_eq!(
            target
                .database()
                .read()
                .await
                .get(&oid(ObjectType::BINARY_VALUE, 2))
                .unwrap()
                .read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            bacnet_types::primitives::PropertyValue::Enumerated(1)
        );
    }

    if confirmed {
        // A real recipient's Error response must reach the shared coordinator,
        // not merely a test adapter, and leave the target write committed.
        client
            .write_property(
                target.local_mac(),
                oid(ObjectType::BINARY_VALUE, 1),
                PropertyIdentifier::PRESENT_VALUE,
                None,
                vec![0x91, 1],
                None,
            )
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let db = target.database().read().await;
                let health = db
                    .get(&oid(ObjectType::AUDIT_REPORTER, 1))
                    .unwrap()
                    .read_property(PropertyIdentifier::RELIABILITY, None)
                    .unwrap();
                if health
                    == bacnet_types::primitives::PropertyValue::Enumerated(
                        Reliability::COMMUNICATION_FAILURE.to_raw(),
                    )
                {
                    break;
                }
                drop(db);
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        allowed.store(true, Ordering::Release);
    }
    let new_value = u8::from(!confirmed);
    client
        .write_property(
            target.local_mac(),
            oid(ObjectType::BINARY_VALUE, 1),
            PropertyIdentifier::PRESENT_VALUE,
            None,
            vec![0x91, new_value],
            None,
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let count = persistence
                .0
                .lock()
                .unwrap()
                .as_ref()
                .map_or(0, |snapshot| snapshot.records.len());
            let db = target.database().read().await;
            let healthy = db
                .get(&oid(ObjectType::AUDIT_REPORTER, 1))
                .unwrap()
                .read_property(PropertyIdentifier::RELIABILITY, None)
                .unwrap()
                == bacnet_types::primitives::PropertyValue::Enumerated(
                    Reliability::NO_FAULT_DETECTED.to_raw(),
                );
            if count == 1 && healthy {
                break;
            }
            drop(db);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();

    // A decoded, authorized semantic execution error is returned unchanged to
    // the client and separately delivered to the real Audit Log with Result.
    let error = client
        .write_property(
            target.local_mac(),
            oid(ObjectType::BINARY_VALUE, 1),
            PropertyIdentifier::PRESENT_VALUE,
            None,
            vec![0x91, 9],
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, Error::Protocol { class, code }
        if class == ErrorClass::PROPERTY.to_raw() as u32
            && code == ErrorCode::VALUE_OUT_OF_RANGE.to_raw() as u32));
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if persistence
                .0
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .records
                .len()
                == 2
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        target
            .database()
            .read()
            .await
            .get(&oid(ObjectType::BINARY_VALUE, 1))
            .unwrap()
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        bacnet_types::primitives::PropertyValue::Enumerated(u32::from(new_value))
    );

    let mut query = AuditLogQueryRequest {
        audit_log: oid(ObjectType::AUDIT_LOG, 1),
        query_parameters: BACnetAuditLogQueryParameters::ByTarget {
            target_device_identifier: oid(ObjectType::DEVICE, 10),
            target_device_address: None,
            target_object_identifier: None,
            target_property_identifier: None,
            target_array_index: None,
            target_priority: None,
            operations: None,
            successful_actions_only: BACnetSuccessFilter::SUCCESSES_ONLY,
        },
        start_at_sequence_number: None,
        requested_count: 10,
    };
    let ack = client
        .audit_log_query(logger.local_mac(), &query)
        .await
        .unwrap();
    assert_eq!(ack.records.len(), 1);
    let BACnetAuditLogDatum::AuditNotification(record) = &ack.records[0].record.datum else {
        panic!("expected operation")
    };
    assert_eq!(record.operation, AuditOperation::WRITE);
    assert_eq!(
        record.target_device,
        BACnetRecipient::Device(oid(ObjectType::DEVICE, 10))
    );
    assert_eq!(record.target_object, Some(oid(ObjectType::BINARY_VALUE, 1)));
    assert_eq!(record.target_priority, Some(16));
    assert_eq!(record.target_value, Some(vec![0x91, new_value]));
    assert_eq!(record.current_value, Some(vec![0x91, u8::from(confirmed)]));
    assert!(record.source_timestamp.is_none());
    assert!(record.target_timestamp.is_some());
    assert!(record.invoke_id.is_some());
    assert!(record.result.is_none());

    let BACnetAuditLogQueryParameters::ByTarget {
        successful_actions_only,
        ..
    } = &mut query.query_parameters
    else {
        unreachable!()
    };
    *successful_actions_only = BACnetSuccessFilter::FAILURES_ONLY;
    let ack = client
        .audit_log_query(logger.local_mac(), &query)
        .await
        .unwrap();
    assert_eq!(ack.records.len(), 1);
    let BACnetAuditLogDatum::AuditNotification(record) = &ack.records[0].record.datum else {
        panic!("expected failed operation");
    };
    assert_eq!(record.operation, AuditOperation::WRITE);
    assert_eq!(
        record.target_device,
        BACnetRecipient::Device(oid(ObjectType::DEVICE, 10))
    );
    assert_eq!(record.target_object, Some(oid(ObjectType::BINARY_VALUE, 1)));
    assert_eq!(
        record.target_property.as_ref().unwrap().property_identifier,
        PropertyIdentifier::PRESENT_VALUE
    );
    assert_eq!(record.target_priority, Some(16));
    assert_eq!(record.target_value, Some(vec![0x91, 9]));
    assert_eq!(record.current_value, Some(vec![0x91, new_value]));
    assert_eq!(
        record.result,
        Some((ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE))
    );
    assert!(record.source_timestamp.is_none());
    assert!(record.target_timestamp.is_some());
    assert!(record.invoke_id.is_some());
    if lists {
        let object = oid(ObjectType::MULTI_STATE_INPUT, 1);
        for step in 0..3 {
            if step == 0 {
                client
                    .add_list_element(
                        target.local_mac(),
                        object,
                        PropertyIdentifier::ALARM_VALUES,
                        None,
                        vec![0x21, 2, 0x21, 3],
                    )
                    .await
                    .unwrap();
            } else {
                client
                    .remove_list_element(
                        target.local_mac(),
                        object,
                        PropertyIdentifier::ALARM_VALUES,
                        None,
                        vec![0x21, 2, 0x21, 3],
                    )
                    .await
                    .unwrap();
            }
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    if persistence
                        .0
                        .lock()
                        .unwrap()
                        .as_ref()
                        .unwrap()
                        .records
                        .len()
                        == 3 + step
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
            let snapshot = persistence.0.lock().unwrap().clone().unwrap();
            let BACnetAuditLogDatum::AuditNotification(record) =
                &snapshot.records[2 + step].record.datum
            else {
                panic!("expected list operation")
            };
            assert_eq!(record.operation, AuditOperation::WRITE);
            assert_eq!(record.target_object, Some(object));
            assert_eq!(
                record.target_property,
                Some(bacnet_types::constructed::AuditPropertyReference {
                    property_identifier: PropertyIdentifier::ALARM_VALUES,
                    property_array_index: None,
                })
            );
            assert_eq!(record.target_priority, None);
            assert_eq!(record.target_value, Some(vec![0x21, 2, 0x21, 3]));
            assert_eq!(
                record.current_value,
                Some(if step == 1 {
                    vec![0x21, 1, 0x21, 2, 0x21, 3]
                } else {
                    vec![0x21, 1]
                })
            );
            assert_eq!(record.result, None);
            assert!(record.source_timestamp.is_none());
            assert!(record.target_timestamp.is_some());
            assert_eq!(
                target
                    .database()
                    .read()
                    .await
                    .get(&object)
                    .unwrap()
                    .read_property(PropertyIdentifier::ALARM_VALUES, None)
                    .unwrap(),
                bacnet_types::primitives::PropertyValue::List(
                    if step == 0 { vec![1, 2, 3] } else { vec![1] }
                        .into_iter()
                        .map(bacnet_types::primitives::PropertyValue::Unsigned)
                        .collect()
                )
            );
        }
    }
    client.stop().await.unwrap();
    target.stop().await.unwrap();
    logger.stop().await.unwrap();
}

#[tokio::test]
async fn audit_reporter_unconfirmed_write_reaches_real_log() {
    exercise(false, false, false).await;
}

#[tokio::test]
async fn audit_reporter_confirmed_error_then_ack_updates_health_over_udp() {
    exercise(true, false, false).await;
}

#[tokio::test]
async fn audit_reporter_selected_writes_and_failures_reach_real_log_once_over_udp() {
    exercise(false, true, false).await;
    exercise(true, true, false).await;
}

#[tokio::test]
async fn audit_reporter_list_add_remove_and_noop_reach_real_log_over_udp() {
    exercise(false, false, true).await;
    exercise(true, false, true).await;
}

#[tokio::test]
async fn audit_reporter_create_delete_reach_real_log_over_udp() {
    use bacnet_services::{common::BACnetPropertyValue, object_mgmt::ObjectSpecifier};
    use bacnet_types::primitives::PropertyValue;

    let persistence = Arc::new(MemoryPersistence::default());
    let mut logger_db = database(20);
    logger_db
        .add(Box::new(
            AuditLogObject::new(1, "log", 16, persistence.clone()).unwrap(),
        ))
        .unwrap();
    let mut logger = BACnetServer::builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .database(logger_db)
        .audit_notification_sink(oid(ObjectType::AUDIT_LOG, 1))
        .audit_notification_authorizer(|_| true)
        .build()
        .await
        .unwrap();

    let mut target_db = database(10);
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
            recipient: Some(oid(ObjectType::DEVICE, 20)),
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
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if persistence
                    .0
                    .lock()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .records
                    .len()
                    == step + 1
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
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
    assert_eq!(
        persistence
            .0
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .records
            .len(),
        4
    );
}
