use super::*;
use bacnet_objects::audit::{AuditLogPersistence, AuditLogSnapshot, FileAuditLogPersistence};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::constructed::{
    BACnetAuditLogDatum, BACnetAuditLogRecord, BACnetAuditLogRecordResult,
};
use bacnet_types::primitives::{Date, Time};

fn write_v1_fixture(storage: &FileAuditLogPersistence, record: BACnetAuditLogRecord) {
    // Use the existing file encoder for the record and envelope. The only v1
    // layout difference is the absent v2 receipt ledger. Keep this fixture local
    // to the server crate (no source include from another published package).
    storage
        .commit(&AuditLogSnapshot {
            object_identifier: oid(ObjectType::AUDIT_LOG, 7),
            generation: 1,
            capacity: 2,
            log_enable: true,
            total_record_count: 1,
            records: vec![BACnetAuditLogRecordResult {
                sequence_number: 1,
                record,
            }],
            completed_receipts: vec![],
        })
        .unwrap();
    let path = &storage.slot_paths()[1];
    let mut bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[8..10], &2u16.to_be_bytes());
    assert_eq!(&bytes[bytes.len() - 8..bytes.len() - 4], &[0; 4]);
    bytes.truncate(bytes.len() - 8); // empty receipt count + checksum
    bytes[8..10].copy_from_slice(&1u16.to_be_bytes());
    let payload_len = u32::from_be_bytes(bytes[22..26].try_into().unwrap()) - 4;
    bytes[22..26].copy_from_slice(&payload_len.to_be_bytes());
    let mut crc = !0u32;
    for byte in &bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    bytes.extend_from_slice(&(!crc).to_be_bytes());
    std::fs::write(path, bytes).unwrap();
}

#[tokio::test(start_paused = true)]
async fn audit_forwarding_configuration_recovers_only_on_next_changed_batch() {
    for observed in [false, true] {
        let mut f = fixture(Some(parent()), None).await;
        assert_eq!(
            f.reliability().await,
            PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw())
        );
        if observed {
            f.server.device_bindings.write().await.observe_i_am_at(
                oid(ObjectType::DEVICE, 20),
                &[2],
                None,
                Instant::now(),
                |_| false,
            );
        }
        assert!(f.requests().is_empty());
        assert!(matches!(
            f.confirmed(201, &[3], payload(true)).await,
            Some(Apdu::SimpleAck(_))
        ));
        settle().await;
        assert!(f.requests().is_empty());
        assert_eq!(
            f.reliability().await,
            PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw())
        );
        // Use the same validating table construction as startup. This internal
        // test seam does not claim a public dynamic-binding/discovery feature;
        // insert_configured deliberately rejects even an observed duplicate.
        *f.server.device_bindings.write().await = DeviceBindingTable::from_configured(
            vec![DeviceBinding::local(oid(ObjectType::DEVICE, 20), [2]).unwrap()],
            |_| false,
        )
        .unwrap();
        settle().await;
        assert!(
            f.requests().is_empty(),
            "route availability does not replay old work"
        );
        assert_eq!(
            f.reliability().await,
            PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw())
        );
        assert!(matches!(
            f.confirmed(201, &[4], payload(true)).await,
            Some(Apdu::SimpleAck(_))
        ));
        settle().await;
        assert!(
            f.requests().is_empty(),
            "receipt-only acceptance does not attempt delivery"
        );
        let data = request_bytes(vec![notification(AuditOperation::READ)]);
        assert!(matches!(
            f.confirmed(202, &[3], data.clone()).await,
            Some(Apdu::SimpleAck(_))
        ));
        settle().await;
        let requests = f.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].service_request, data);
        assert_eq!(
            f.reliability().await,
            PropertyValue::Enumerated(Reliability::COMMUNICATION_FAILURE.to_raw()),
            "previous failed attempt is not cleared until ACK"
        );
        assert!(f.ack(requests[0].invoke_id, &[2], requests[0].service_choice));
        settle().await;
        assert_eq!(
            f.reliability().await,
            PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
        );
        f.server.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn audit_forwarding_old_instance_completion_and_cancellation_cannot_update_replacement() {
    for terminal in ["ack", "reject", "deadline", "shutdown"] {
        let mut f = ready().await;
        f.unconfirmed(payload(false)).await;
        settle().await;
        if terminal == "ack" {
            // Align both instances' failure epochs. Otherwise the independent
            // stale-success epoch guard could hide a wrong-instance update.
            let first = f.requests()[0].clone();
            assert!(f.server.notification_transactions.admit_terminal(
                &[2],
                None,
                &Apdu::Reject(RejectPdu {
                    invoke_id: first.invoke_id,
                    reject_reason: RejectReason::OTHER,
                })
            ));
            settle().await;
            f.unconfirmed(payload(false)).await;
            settle().await;
        }
        let old_request = f.requests().pop().unwrap();
        let old_sends = f.requests().len();
        assert_eq!(f.server.notification_transactions.active_count(), 1);
        let old = f
            .server
            .db
            .write()
            .await
            .remove(&oid(ObjectType::AUDIT_LOG, 7))
            .unwrap()
            .unwrap();
        let old_profile = old.audit_log_forwarding_internal().unwrap();
        let mut replacement = AuditLogObject::new(7, "replacement", 16, f.store.clone()).unwrap();
        replacement.set_member_of(Some(parent()));
        let replacement_profile = replacement.audit_log_forwarding_internal().unwrap();
        assert!(!Arc::ptr_eq(&old_profile, &replacement_profile));
        f.server
            .db
            .write()
            .await
            .add(Box::new(replacement))
            .unwrap();

        // Give the replacement a real delivery result, opposite to the late
        // old result. Merely preserving CONFIGURATION_ERROR could mask a leak.
        f.unconfirmed(payload(false)).await;
        settle().await;
        let new_request = f.requests().pop().unwrap();
        if terminal == "ack" {
            assert!(f.server.notification_transactions.admit_terminal(
                &[2],
                None,
                &Apdu::Reject(RejectPdu {
                    invoke_id: new_request.invoke_id,
                    reject_reason: RejectReason::OTHER,
                })
            ));
        } else {
            assert!(f.ack(new_request.invoke_id, &[2], new_request.service_choice));
        }
        settle().await;
        let expected = if terminal == "ack" {
            Reliability::COMMUNICATION_FAILURE
        } else {
            Reliability::NO_FAULT_DETECTED
        };
        assert_eq!(
            f.reliability().await,
            PropertyValue::Enumerated(expected.to_raw())
        );
        let durable = f.store.snapshot.lock().unwrap().clone();
        match terminal {
            "ack" => assert!(f.ack(old_request.invoke_id, &[2], old_request.service_choice)),
            "reject" => assert!(f.server.notification_transactions.admit_terminal(
                &[2],
                None,
                &Apdu::Reject(RejectPdu {
                    invoke_id: old_request.invoke_id,
                    reject_reason: RejectReason::OTHER,
                })
            )),
            "deadline" => tokio::time::advance(Duration::from_secs(3)).await,
            "shutdown" => f.server.stop().await.unwrap(),
            _ => unreachable!(),
        }
        settle().await;
        assert_eq!(
            f.reliability().await,
            PropertyValue::Enumerated(expected.to_raw())
        );
        let old_expected = if terminal == "ack" {
            Reliability::NO_FAULT_DETECTED
        } else {
            Reliability::COMMUNICATION_FAILURE
        };
        assert_eq!(
            old.read_property(PropertyIdentifier::RELIABILITY, None)
                .unwrap(),
            PropertyValue::Enumerated(old_expected.to_raw())
        );
        assert_eq!(*f.store.snapshot.lock().unwrap(), durable);
        assert_eq!(f.requests().len(), old_sends + 1);
        assert_eq!(f.server.notification_transactions.active_count(), 0);
        assert_eq!(
            f.server.notification_transactions.audit_resources(),
            (false, 0, 64)
        );
        if terminal != "shutdown" {
            f.server.stop().await.unwrap();
        }
        assert!(f.server.notification_transactions.workers_empty());
    }
}

#[tokio::test(start_paused = true)]
async fn audit_forwarding_file_v1_reopen_has_no_replay_or_historical_receipt() {
    let base = std::env::temp_dir().join(format!(
        "rusty-bacnet-forward-v1-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&base).unwrap();
    let storage = Arc::new(FileAuditLogPersistence::new(base.join("state")).unwrap());
    let data = payload(true);
    let item = bacnet_services::audit::AuditNotificationRequest::decode(&data)
        .unwrap()
        .notifications
        .remove(0);
    let record = BACnetAuditLogRecord {
        timestamp: (
            Date {
                year: 124,
                month: 2,
                day: 29,
                day_of_week: 4,
            },
            Time {
                hour: 12,
                minute: 0,
                second: 0,
                hundredths: 0,
            },
        ),
        datum: BACnetAuditLogDatum::AuditNotification(item),
    };
    write_v1_fixture(&storage, record.clone());
    let original = storage
        .load(oid(ObjectType::AUDIT_LOG, 7))
        .unwrap()
        .unwrap();
    assert!(original.completed_receipts.is_empty());
    assert_eq!(original.records[0].record, record);
    let mut log = AuditLogObject::new(7, "v1", 2, storage.clone()).unwrap();
    assert!(log.audit_log_forwarding_internal().is_none());
    log.set_member_of(Some(parent()));
    let (server, wire) = start(
        10,
        log,
        Some(DeviceBinding::local(oid(ObjectType::DEVICE, 20), [2]).unwrap()),
    )
    .await;
    let mut f = Fixture {
        server,
        wire,
        store: Arc::new(MemoryPersistence::default()),
    };
    settle().await;
    assert!(
        f.requests().is_empty(),
        "reopening and reapplying Member_Of never replay records"
    );
    assert_eq!(
        storage
            .load(oid(ObjectType::AUDIT_LOG, 7))
            .unwrap()
            .unwrap(),
        original
    );
    // A pre-v2 request has no ledger identity. Exact old content is a fresh
    // request (ACK + v2 receipt), although the complete record does not change.
    assert!(matches!(
        f.confirmed(201, &[3], data.clone()).await,
        Some(Apdu::SimpleAck(_))
    ));
    let accepted = storage
        .load(oid(ObjectType::AUDIT_LOG, 7))
        .unwrap()
        .unwrap();
    assert_eq!(accepted.records, original.records);
    assert_eq!(accepted.total_record_count, 1);
    assert_eq!(accepted.generation, original.generation + 1);
    assert_eq!(accepted.completed_receipts.len(), 1);
    assert_eq!(
        &std::fs::read(&storage.slot_paths()[0]).unwrap()[8..10],
        &2u16.to_be_bytes()
    );
    assert!(f.confirmed(201, &[3], data.clone()).await.is_none());
    settle().await;
    assert!(f.requests().is_empty());
    f.server.stop().await.unwrap();
    let mut log = AuditLogObject::new(7, "v2", 2, storage.clone()).unwrap();
    log.set_member_of(Some(parent()));
    let (server, wire) = start(
        10,
        log,
        Some(DeviceBinding::local(oid(ObjectType::DEVICE, 20), [2]).unwrap()),
    )
    .await;
    let mut reopened = Fixture {
        server,
        wire,
        store: Arc::new(MemoryPersistence::default()),
    };
    assert!(reopened.confirmed(201, &[3], data).await.is_none());
    settle().await;
    assert!(reopened.requests().is_empty());
    assert_eq!(
        storage
            .load(oid(ObjectType::AUDIT_LOG, 7))
            .unwrap()
            .unwrap(),
        accepted
    );
    reopened.server.stop().await.unwrap();
    std::fs::remove_dir_all(base).unwrap();
}
