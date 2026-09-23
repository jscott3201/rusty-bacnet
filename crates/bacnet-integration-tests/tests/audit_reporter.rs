//! RB-21a/b/c/d/e/f: mutation outcomes -> selected target Reporter -> Audit Log over UDP.

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
    file::FileObject,
};
use bacnet_server::server::{AuditReporterConfig, BACnetServer, DeviceBinding};
use bacnet_services::audit::AuditLogQueryRequest;
use bacnet_services::file::FileWriteAccessMethod;
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

impl MemoryPersistence {
    fn record_count(&self) -> usize {
        self.0
            .lock()
            .unwrap()
            .as_ref()
            .map_or(0, |s| s.records.len())
    }
}

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

async fn wait_for_records(persistence: &MemoryPersistence, count: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if persistence.record_count() == count {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

async fn start_logger(
    persistence: Arc<MemoryPersistence>,
    policy: Arc<AtomicBool>,
) -> BACnetServer<bacnet_transport::bip::BipTransport> {
    let mut logger_db = database(20);
    logger_db
        .add(Box::new(
            AuditLogObject::new(1, "log", 16, persistence.clone()).unwrap(),
        ))
        .unwrap();
    BACnetServer::builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .database(logger_db)
        .audit_notification_sink(oid(ObjectType::AUDIT_LOG, 1))
        .audit_notification_authorizer(move |_| policy.load(Ordering::Acquire))
        .unconfirmed_audit_notification_authorizer(|_| true)
        .build()
        .await
        .unwrap()
}

async fn exercise(confirmed: bool, selected: bool, lists: bool, files: bool) {
    let allowed = Arc::new(AtomicBool::new(!confirmed));
    let persistence = Arc::new(MemoryPersistence::default());
    let mut logger = start_logger(persistence.clone(), allowed.clone()).await;

    let mut target_db = database(10);
    target_db
        .get_mut(&oid(ObjectType::DEVICE, 10))
        .unwrap()
        .device_authority_internal()
        .unwrap()
        .provision_audit_recipient(BACnetRecipient::Device(oid(ObjectType::DEVICE, 20)))
        .unwrap();
    if files {
        for instance in 1..=2 {
            let mut file = FileObject::new(instance, format!("file-{instance}"), "binary").unwrap();
            if instance == 2 {
                file.set_file_access_method(
                    bacnet_types::enums::FileAccessMethod::RECORD_ACCESS.to_raw(),
                );
                file.set_records(vec![vec![1]]);
            } else {
                file.set_data(vec![1]);
            }
            target_db.add(Box::new(file)).unwrap();
        }
    }
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
            let count = persistence.record_count();
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
    wait_for_records(&persistence, 2).await;
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
            wait_for_records(&persistence, 3 + step).await;
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
    if files {
        for step in 0..4 {
            let record_access = step >= 2;
            let object = oid(ObjectType::FILE, if record_access { 2 } else { 1 });
            let failure = step % 2 == 1;
            let start = if failure { -2 } else { -1 };
            let access = if record_access {
                FileWriteAccessMethod::Record {
                    file_start_record: start,
                    record_count: 1,
                    file_record_data: vec![vec![42]],
                }
            } else {
                FileWriteAccessMethod::Stream {
                    file_start_position: start,
                    file_data: vec![42],
                }
            };
            let result = client
                .atomic_write_file(target.local_mac(), object, access)
                .await;
            if failure {
                assert!(
                    matches!(result, Err(Error::Protocol { class, code }) if class == ErrorClass::SERVICES.to_raw() as u32 && code == ErrorCode::INVALID_FILE_START_POSITION.to_raw() as u32)
                );
            } else {
                assert_eq!(
                    result.unwrap().as_ref(),
                    &[if record_access { 0x19 } else { 0x09 }, 1]
                );
            }
            wait_for_records(&persistence, 3 + step).await;
            let snapshot = persistence.0.lock().unwrap().clone().unwrap();
            let BACnetAuditLogDatum::AuditNotification(record) =
                &snapshot.records[2 + step].record.datum
            else {
                panic!("expected file WRITE")
            };
            assert_eq!(record.operation, AuditOperation::WRITE);
            assert_eq!(record.target_object, Some(object));
            assert_eq!(
                record.target_device,
                BACnetRecipient::Device(oid(ObjectType::DEVICE, 10))
            );
            assert!(record.invoke_id.is_some());
            assert!(record.source_timestamp.is_none());
            assert!(record.target_timestamp.is_some());
            assert!(record.target_property.is_none());
            assert!(record.target_priority.is_none());
            assert!(record.target_value.is_none());
            assert!(record.current_value.is_none());
            assert_eq!(
                record.result,
                failure.then_some((ErrorClass::SERVICES, ErrorCode::INVALID_FILE_START_POSITION))
            );
            let db = target.database().read().await;
            let storage = db.get(&object).unwrap().file_storage_internal().unwrap();
            if record_access {
                assert_eq!(
                    storage.read_records(0, 10).unwrap().records,
                    vec![vec![1], vec![42]]
                );
            } else {
                assert_eq!(storage.read_stream(0, 10).unwrap().data, vec![1, 42]);
            }
        }
    }
    client.stop().await.unwrap();
    target.stop().await.unwrap();
    logger.stop().await.unwrap();
    if files {
        assert_eq!(persistence.record_count(), 6, "no duplicates or recursion");
    }
}

#[tokio::test]
async fn audit_reporter_unconfirmed_write_reaches_real_log() {
    exercise(false, false, false, false).await;
}

#[tokio::test]
async fn audit_reporter_confirmed_error_then_ack_updates_health_over_udp() {
    exercise(true, false, false, false).await;
}

#[tokio::test]
async fn audit_reporter_selected_writes_and_failures_reach_real_log_once_over_udp() {
    exercise(false, true, false, false).await;
    exercise(true, true, false, false).await;
}

#[tokio::test]
async fn audit_reporter_list_add_remove_and_noop_reach_real_log_over_udp() {
    exercise(false, false, true, false).await;
    exercise(true, false, true, false).await;
}

#[tokio::test]
async fn audit_reporter_atomic_write_file_confirmed_and_unconfirmed_reach_real_log() {
    exercise(false, false, false, true).await;
    exercise(true, false, false, true).await;
}

#[path = "audit_reporter/device_recipient.rs"]
mod device_recipient;

#[path = "audit_reporter/lifecycle.rs"]
mod lifecycle;
