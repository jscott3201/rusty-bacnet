//! RB-21a/b/c: real WP outcomes -> selected target Reporter -> Audit Log over UDP.

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

async fn exercise(confirmed: bool, selected: bool) {
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
    client.stop().await.unwrap();
    target.stop().await.unwrap();
    logger.stop().await.unwrap();
}

#[tokio::test]
async fn audit_reporter_unconfirmed_write_reaches_real_log() {
    exercise(false, false).await;
}

#[tokio::test]
async fn audit_reporter_confirmed_error_then_ack_updates_health_over_udp() {
    exercise(true, false).await;
}

#[tokio::test]
async fn audit_reporter_selected_writes_and_failures_reach_real_log_once_over_udp() {
    exercise(false, true).await;
    exercise(true, true).await;
}
