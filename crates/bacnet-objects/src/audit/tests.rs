use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use bacnet_types::constructed::{
    BACnetAuditLogDatum, BACnetAuditLogRecord, BACnetAuditLogRecordResult, BACnetAuditNotification,
    BACnetRecipient,
};
use bacnet_types::enums::{AuditOperation, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, Time};

use crate::clock::{ClockFrame, ClockReader};
use crate::traits::BACnetObject;

use super::{
    AuditLogObject, AuditLogPersistence, AuditLogSnapshot, AuditReporterObject,
    FileAuditLogPersistence, MAX_AUDIT_RECORDS,
};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_base(label: &str) -> PathBuf {
    let serial = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "rusty-bacnet-audit-{label}-{}-{serial}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("state")
}

fn cleanup_base(base: &std::path::Path) {
    if let Some(parent) = base.parent() {
        let _ = std::fs::remove_dir_all(parent);
    }
}

fn rewrite_snapshot_checksum(data: &mut [u8]) {
    let checksum_start = data.len() - 4;
    let mut crc = !0u32;
    for byte in &data[..checksum_start] {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    data[checksum_start..].copy_from_slice(&(!crc).to_be_bytes());
}

fn oid(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::AUDIT_LOG, instance).unwrap()
}

fn timestamp(minute: u8) -> (Date, Time) {
    (
        Date {
            year: 124,
            month: 2,
            day: 29,
            day_of_week: 4,
        },
        Time {
            hour: 12,
            minute,
            second: 0,
            hundredths: 0,
        },
    )
}

fn record(minute: u8, datum: BACnetAuditLogDatum) -> BACnetAuditLogRecord {
    BACnetAuditLogRecord {
        timestamp: timestamp(minute),
        datum,
    }
}

#[derive(Clone)]
struct FixedClock(Option<ClockFrame>);

impl ClockReader for FixedClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        self.0
    }
}

fn frame(minute: u8) -> ClockFrame {
    ClockFrame {
        local_date: timestamp(minute).0,
        local_time: timestamp(minute).1,
        utc_offset: 0,
        daylight_savings_status: false,
    }
}

fn assert_operational_problem(error: Error) {
    assert!(matches!(
        error,
        Error::Protocol { class, code }
            if class == ErrorClass::DEVICE.to_raw() as u32
                && code == ErrorCode::OPERATIONAL_PROBLEM.to_raw() as u32
    ));
}

fn assert_matches_snapshot(log: &AuditLogObject, snapshot: &AuditLogSnapshot) {
    assert_eq!(log.generation(), snapshot.generation);
    assert_eq!(log.buffer_size(), snapshot.capacity);
    assert_eq!(log.log_enable(), snapshot.log_enable);
    assert_eq!(log.total_record_count(), snapshot.total_record_count);
    assert_eq!(
        log.records().iter().cloned().collect::<Vec<_>>(),
        snapshot.records
    );
}

#[derive(Default)]
struct MemoryPersistence {
    snapshot: Mutex<Option<AuditLogSnapshot>>,
    fail_write: AtomicBool,
    fail_sync: AtomicBool,
}

impl MemoryPersistence {
    fn with_snapshot(snapshot: AuditLogSnapshot) -> Self {
        Self {
            snapshot: Mutex::new(Some(snapshot)),
            ..Self::default()
        }
    }
}

impl AuditLogPersistence for MemoryPersistence {
    fn load(&self, _expected_object: ObjectIdentifier) -> Result<Option<AuditLogSnapshot>, Error> {
        Ok(self.snapshot.lock().unwrap().clone())
    }

    fn commit(&self, snapshot: &AuditLogSnapshot) -> Result<(), Error> {
        if self.fail_write.load(Ordering::Acquire) {
            return Err(Error::Transport(std::io::Error::other(
                "injected write failure",
            )));
        }
        if self.fail_sync.load(Ordering::Acquire) {
            return Err(Error::Transport(std::io::Error::other(
                "injected sync failure",
            )));
        }
        *self.snapshot.lock().unwrap() = Some(snapshot.clone());
        Ok(())
    }
}

#[test]
fn audit_restart_restores_exact_typed_state() {
    let base = temp_base("restart");
    let storage = Arc::new(FileAuditLogPersistence::new(&base).unwrap());
    let mut log = AuditLogObject::new(1, "AL-1", 3, storage.clone()).unwrap();
    assert_eq!(
        log.add_record(record(1, BACnetAuditLogDatum::TimeChange(1.5)))
            .unwrap(),
        Some(1)
    );
    assert_eq!(
        log.add_record(record(2, BACnetAuditLogDatum::LogStatus(0)))
            .unwrap(),
        Some(2)
    );
    let expected = log.records().clone();
    drop(log);

    let reopened = AuditLogObject::new(1, "AL-1", 3, storage).unwrap();
    assert_eq!(reopened.records(), &expected);
    assert_eq!(reopened.buffer_size(), 3);
    assert!(reopened.log_enable());
    assert_eq!(reopened.total_record_count(), 2);
    cleanup_base(&base);
}

#[test]
fn audit_capacity_eviction_and_total_wrap_are_transactional() {
    let persistence = Arc::new(MemoryPersistence::with_snapshot(AuditLogSnapshot {
        object_identifier: oid(1),
        generation: 7,
        capacity: 2,
        log_enable: true,
        total_record_count: u64::MAX,
        records: vec![BACnetAuditLogRecordResult {
            sequence_number: u64::MAX,
            record: record(0, BACnetAuditLogDatum::LogStatus(0)),
        }],
    }));
    let mut log = AuditLogObject::new(1, "AL-1", 2, persistence).unwrap();
    assert_eq!(
        log.add_record(record(1, BACnetAuditLogDatum::TimeChange(2.0)))
            .unwrap(),
        Some(1)
    );
    assert_eq!(
        log.add_record(record(2, BACnetAuditLogDatum::TimeChange(3.0)))
            .unwrap(),
        Some(2)
    );
    assert_eq!(log.total_record_count(), 2);
    assert_eq!(
        log.records()
            .iter()
            .map(|entry| entry.sequence_number)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[test]
fn zero_capacity_counts_then_discards_ordinary_and_status_records() {
    let persistence = Arc::new(MemoryPersistence::default());
    let mut log = AuditLogObject::new(1, "AL-1", 0, persistence).unwrap();
    log.bind_clock_internal(Some(Arc::new(FixedClock(Some(frame(1))))));

    assert_eq!(
        log.add_record(record(0, BACnetAuditLogDatum::TimeChange(1.0)))
            .unwrap(),
        Some(1)
    );
    assert!(log.records().is_empty());
    assert_eq!(log.purge().unwrap(), 2);
    assert!(log.records().is_empty());
    assert_eq!(log.total_record_count(), 2);
}

#[test]
fn zero_capacity_validates_enabled_records_before_advancing_sequence() {
    let persistence = Arc::new(MemoryPersistence::default());
    let mut log = AuditLogObject::new(1, "AL-1", 0, persistence.clone()).unwrap();
    let before = persistence.snapshot.lock().unwrap().clone().unwrap();

    let mut invalid_timestamp = record(1, BACnetAuditLogDatum::LogStatus(0));
    invalid_timestamp.timestamp.1.hour = 24;
    let invalid_status = record(1, BACnetAuditLogDatum::LogStatus(0b1000));
    let device = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
    let mut notification = BACnetAuditNotification {
        source_timestamp: None,
        target_timestamp: None,
        source_device: BACnetRecipient::Device(device),
        source_object: None,
        operation: AuditOperation::GENERAL,
        source_comment: None,
        target_comment: None,
        invoke_id: None,
        source_user_id: None,
        source_user_role: None,
        target_device: BACnetRecipient::Device(device),
        target_object: None,
        target_property: None,
        target_priority: Some(0),
        target_value: None,
        current_value: None,
        result: None,
    };
    let invalid_notification = record(
        1,
        BACnetAuditLogDatum::AuditNotification(notification.clone()),
    );
    notification.target_priority = None;
    notification.target_value = Some(vec![0; 1024 * 1024 + 1]);
    let oversized_notification = record(1, BACnetAuditLogDatum::AuditNotification(notification));

    for invalid_record in [
        invalid_timestamp,
        invalid_status,
        invalid_notification,
        oversized_notification,
    ] {
        assert!(log.add_record(invalid_record).is_err());
        assert_matches_snapshot(&log, &before);
        assert_eq!(persistence.snapshot.lock().unwrap().as_ref(), Some(&before));
    }

    let disabled = Arc::new(MemoryPersistence::with_snapshot(AuditLogSnapshot {
        object_identifier: oid(2),
        generation: 1,
        capacity: 0,
        log_enable: false,
        total_record_count: 0,
        records: Vec::new(),
    }));
    let mut disabled_log = AuditLogObject::new(2, "AL-2", 0, disabled.clone()).unwrap();
    let disabled_before = disabled.snapshot.lock().unwrap().clone().unwrap();
    assert_eq!(
        disabled_log
            .add_record(record(1, BACnetAuditLogDatum::LogStatus(0b1000)))
            .unwrap(),
        None
    );
    assert_matches_snapshot(&disabled_log, &disabled_before);
    assert_eq!(
        disabled.snapshot.lock().unwrap().as_ref(),
        Some(&disabled_before)
    );
}

#[test]
fn newer_slot_corruption_falls_back_but_no_valid_slot_fails_closed() {
    let base = temp_base("fallback");
    let storage = Arc::new(FileAuditLogPersistence::new(&base).unwrap());
    let mut log = AuditLogObject::new(1, "AL-1", 4, storage.clone()).unwrap();
    log.add_record(record(1, BACnetAuditLogDatum::LogStatus(0)))
        .unwrap();
    log.add_record(record(2, BACnetAuditLogDatum::TimeChange(1.0)))
        .unwrap();
    let paths = storage.slot_paths();
    let newest = paths[(log.generation() % 2) as usize].clone();
    let older_count = 1;
    drop(log);

    std::fs::write(&newest, b"damaged-newer-slot").unwrap();
    let recovered = AuditLogObject::new(1, "AL-1", 4, storage.clone()).unwrap();
    assert_eq!(recovered.records().len(), older_count);
    drop(recovered);

    std::fs::write(&paths[0], b"invalid-a").unwrap();
    std::fs::write(&paths[1], b"invalid-b").unwrap();
    assert!(AuditLogObject::new(1, "AL-1", 4, storage).is_err());
    assert_eq!(std::fs::read(&paths[0]).unwrap(), b"invalid-a");
    assert_eq!(std::fs::read(&paths[1]).unwrap(), b"invalid-b");
    cleanup_base(&base);
}

#[test]
fn corrupted_header_falls_back_but_checksum_valid_incompatibility_is_fatal() {
    let base = temp_base("incompatible-newer");
    let storage = Arc::new(FileAuditLogPersistence::new(&base).unwrap());
    let mut log = AuditLogObject::new(1, "AL-1", 2, storage.clone()).unwrap();
    log.add_record(record(1, BACnetAuditLogDatum::LogStatus(0)))
        .unwrap();
    let paths = storage.slot_paths();
    let newest = paths[(log.generation() % 2) as usize].clone();
    drop(log);

    let original = std::fs::read(&newest).unwrap();
    let mut unknown_version = original.clone();
    unknown_version[8..10].copy_from_slice(&2u16.to_be_bytes());
    std::fs::write(&newest, &unknown_version).unwrap();
    let recovered = AuditLogObject::new(1, "AL-1", 2, storage.clone()).unwrap();
    assert!(recovered.records().is_empty());
    drop(recovered);

    let mut wrong_identity = original.clone();
    wrong_identity[10..14].copy_from_slice(&oid(2).encode());
    std::fs::write(&newest, &wrong_identity).unwrap();
    let recovered = AuditLogObject::new(1, "AL-1", 2, storage.clone()).unwrap();
    assert!(recovered.records().is_empty());
    drop(recovered);

    rewrite_snapshot_checksum(&mut unknown_version);
    std::fs::write(&newest, unknown_version).unwrap();
    assert!(AuditLogObject::new(1, "AL-1", 2, storage.clone()).is_err());

    rewrite_snapshot_checksum(&mut wrong_identity);
    std::fs::write(&newest, wrong_identity).unwrap();
    assert!(AuditLogObject::new(1, "AL-1", 2, storage).is_err());
    cleanup_base(&base);
}

#[test]
fn persistence_identity_version_capacity_and_length_validation_fail_closed() {
    let base = temp_base("validation");
    let storage = Arc::new(FileAuditLogPersistence::new(&base).unwrap());
    let log = AuditLogObject::new(1, "AL-1", 2, storage.clone()).unwrap();
    let paths = storage.slot_paths();
    let active = paths[(log.generation() % 2) as usize].clone();
    drop(log);

    let original = std::fs::read(&active).unwrap();
    let mut wrong_identity = original.clone();
    wrong_identity[10..14].copy_from_slice(&oid(2).encode());
    std::fs::write(&active, &wrong_identity).unwrap();
    assert!(AuditLogObject::new(1, "AL-1", 2, storage.clone()).is_err());

    let mut unknown_version = original.clone();
    unknown_version[8..10].copy_from_slice(&2u16.to_be_bytes());
    std::fs::write(&active, &unknown_version).unwrap();
    assert!(AuditLogObject::new(1, "AL-1", 2, storage.clone()).is_err());

    std::fs::write(&active, &original[..original.len() - 1]).unwrap();
    assert!(AuditLogObject::new(1, "AL-1", 2, storage.clone()).is_err());
    assert!(AuditLogObject::new(1, "AL-1", MAX_AUDIT_RECORDS + 1, storage).is_err());

    let exact = Arc::new(MemoryPersistence::default());
    assert!(AuditLogObject::new(2, "AL-2", MAX_AUDIT_RECORDS, exact).is_ok());
    let zero_sequence = Arc::new(MemoryPersistence::with_snapshot(AuditLogSnapshot {
        object_identifier: oid(2),
        generation: 1,
        capacity: 2,
        log_enable: true,
        total_record_count: 1,
        records: vec![
            BACnetAuditLogRecordResult {
                sequence_number: 0,
                record: record(0, BACnetAuditLogDatum::LogStatus(0)),
            },
            BACnetAuditLogRecordResult {
                sequence_number: 1,
                record: record(1, BACnetAuditLogDatum::LogStatus(0)),
            },
        ],
    }));
    assert!(AuditLogObject::new(2, "AL-2", 2, zero_sequence).is_err());
    assert!(FileAuditLogPersistence::new("").is_err());
    cleanup_base(&base);
}

#[test]
fn oversized_record_is_rejected_before_durable_or_memory_mutation() {
    let base = temp_base("record-bound");
    let storage = Arc::new(FileAuditLogPersistence::new(&base).unwrap());
    let mut log = AuditLogObject::new(1, "AL-1", 1, storage.clone()).unwrap();
    let device = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
    let notification = BACnetAuditNotification {
        source_timestamp: None,
        target_timestamp: None,
        source_device: BACnetRecipient::Device(device),
        source_object: None,
        operation: AuditOperation::GENERAL,
        source_comment: None,
        target_comment: None,
        invoke_id: None,
        source_user_id: None,
        source_user_role: None,
        target_device: BACnetRecipient::Device(device),
        target_object: None,
        target_property: None,
        target_priority: None,
        target_value: Some(vec![0; 1024 * 1024 + 1]),
        current_value: None,
        result: None,
    };
    assert!(log
        .add_record(record(
            1,
            BACnetAuditLogDatum::AuditNotification(notification),
        ))
        .is_err());
    assert!(log.records().is_empty());
    assert_eq!(log.total_record_count(), 0);

    let reopened = AuditLogObject::new(1, "AL-1", 1, storage).unwrap();
    assert!(reopened.records().is_empty());
    assert_eq!(reopened.total_record_count(), 0);
    cleanup_base(&base);
}

#[test]
fn custom_backend_cannot_bypass_record_or_snapshot_validation() {
    let mut invalid_timestamp = record(1, BACnetAuditLogDatum::LogStatus(0));
    invalid_timestamp.timestamp.1.hour = 24;
    let invalid_status = record(1, BACnetAuditLogDatum::LogStatus(0b1000));

    for invalid_record in [invalid_timestamp, invalid_status] {
        let loaded = Arc::new(MemoryPersistence::with_snapshot(AuditLogSnapshot {
            object_identifier: oid(1),
            generation: 1,
            capacity: 2,
            log_enable: true,
            total_record_count: 1,
            records: vec![BACnetAuditLogRecordResult {
                sequence_number: 1,
                record: invalid_record.clone(),
            }],
        }));
        assert!(AuditLogObject::new(1, "AL-1", 2, loaded).is_err());

        let persistence = Arc::new(MemoryPersistence::default());
        let mut log = AuditLogObject::new(1, "AL-1", 2, persistence.clone()).unwrap();
        let before = persistence.snapshot.lock().unwrap().clone().unwrap();
        assert!(log.add_record(invalid_record).is_err());
        assert_matches_snapshot(&log, &before);
        assert_eq!(persistence.snapshot.lock().unwrap().as_ref(), Some(&before));
    }

    let over_capacity = Arc::new(MemoryPersistence::with_snapshot(AuditLogSnapshot {
        object_identifier: oid(1),
        generation: 1,
        capacity: 1,
        log_enable: true,
        total_record_count: 2,
        records: vec![
            BACnetAuditLogRecordResult {
                sequence_number: 1,
                record: record(1, BACnetAuditLogDatum::LogStatus(0)),
            },
            BACnetAuditLogRecordResult {
                sequence_number: 2,
                record: record(2, BACnetAuditLogDatum::LogStatus(0)),
            },
        ],
    }));
    assert!(AuditLogObject::new(1, "AL-1", 1, over_capacity).is_err());

    let persistence = Arc::new(MemoryPersistence::default());
    let mut log = AuditLogObject::new(1, "AL-1", 1, persistence.clone()).unwrap();
    let before = persistence.snapshot.lock().unwrap().clone().unwrap();
    let device = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
    let oversized = BACnetAuditNotification {
        source_timestamp: None,
        target_timestamp: None,
        source_device: BACnetRecipient::Device(device),
        source_object: None,
        operation: AuditOperation::GENERAL,
        source_comment: None,
        target_comment: None,
        invoke_id: None,
        source_user_id: None,
        source_user_role: None,
        target_device: BACnetRecipient::Device(device),
        target_object: None,
        target_property: None,
        target_priority: None,
        target_value: Some(vec![0; 1024 * 1024 + 1]),
        current_value: None,
        result: None,
    };
    assert!(log
        .add_record(record(1, BACnetAuditLogDatum::AuditNotification(oversized),))
        .is_err());
    assert_matches_snapshot(&log, &before);
    assert_eq!(persistence.snapshot.lock().unwrap().as_ref(), Some(&before));
}

#[test]
fn write_and_sync_failures_preserve_memory_and_prior_snapshot() {
    for sync_failure in [false, true] {
        let persistence = Arc::new(MemoryPersistence::default());
        let mut log = AuditLogObject::new(1, "AL-1", 2, persistence.clone()).unwrap();
        let before = persistence.snapshot.lock().unwrap().clone().unwrap();
        if sync_failure {
            persistence.fail_sync.store(true, Ordering::Release);
        } else {
            persistence.fail_write.store(true, Ordering::Release);
        }

        assert!(log
            .add_record(record(1, BACnetAuditLogDatum::TimeChange(1.0)))
            .is_err());
        assert!(log.records().is_empty());
        assert_eq!(log.total_record_count(), 0);
        assert_eq!(persistence.snapshot.lock().unwrap().as_ref(), Some(&before));
    }
}

#[test]
fn log_enable_rollback_restores_exact_state_and_propagates_commit_failure() {
    let persistence = Arc::new(MemoryPersistence::default());
    let mut log = AuditLogObject::new(1, "AL-1", 4, persistence.clone()).unwrap();
    log.add_record(record(1, BACnetAuditLogDatum::TimeChange(1.0)))
        .unwrap();
    log.bind_clock_internal(Some(Arc::new(FixedClock(Some(frame(2))))));
    let before = persistence.snapshot.lock().unwrap().clone().unwrap();

    assert!(log
        .capture_write_property_rollback(
            PropertyIdentifier::LOG_ENABLE,
            &PropertyValue::Boolean(true),
        )
        .is_none());
    assert!(log
        .capture_write_property_rollback(
            PropertyIdentifier::LOG_ENABLE,
            &PropertyValue::Unsigned(0),
        )
        .is_none());

    let rollback = log
        .capture_write_property_rollback(
            PropertyIdentifier::LOG_ENABLE,
            &PropertyValue::Boolean(false),
        )
        .unwrap();
    log.write_property(
        PropertyIdentifier::LOG_ENABLE,
        None,
        PropertyValue::Boolean(false),
        None,
    )
    .unwrap();
    log.restore_write_property_rollback(rollback).unwrap();

    let mut restored = before;
    restored.generation = log.generation();
    assert_matches_snapshot(&log, &restored);
    assert_eq!(
        persistence.snapshot.lock().unwrap().as_ref(),
        Some(&restored)
    );

    let rollback = log
        .capture_write_property_rollback(
            PropertyIdentifier::LOG_ENABLE,
            &PropertyValue::Boolean(false),
        )
        .unwrap();
    log.write_property(
        PropertyIdentifier::LOG_ENABLE,
        None,
        PropertyValue::Boolean(false),
        None,
    )
    .unwrap();
    let changed = persistence.snapshot.lock().unwrap().clone().unwrap();
    persistence.fail_sync.store(true, Ordering::Release);
    assert!(log.restore_write_property_rollback(rollback).is_err());
    assert_matches_snapshot(&log, &changed);
    assert_eq!(
        persistence.snapshot.lock().unwrap().as_ref(),
        Some(&changed)
    );
}

#[test]
fn clocked_enable_and_purge_statuses_persist_and_record_count_is_read_only() {
    let persistence = Arc::new(MemoryPersistence::default());
    let mut log = AuditLogObject::new(1, "AL-1", 4, persistence.clone()).unwrap();
    let before = persistence.snapshot.lock().unwrap().clone().unwrap();

    let error = log
        .write_property(
            PropertyIdentifier::LOG_ENABLE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap_err();
    assert_operational_problem(error);
    assert_matches_snapshot(&log, &before);
    assert_eq!(persistence.snapshot.lock().unwrap().as_ref(), Some(&before));
    assert_operational_problem(log.purge().unwrap_err());
    assert_matches_snapshot(&log, &before);
    assert_eq!(persistence.snapshot.lock().unwrap().as_ref(), Some(&before));

    let mut invalid = frame(1);
    invalid.local_time.hour = 24;
    log.bind_clock_internal(Some(Arc::new(FixedClock(Some(invalid)))));
    let error = log
        .write_property(
            PropertyIdentifier::LOG_ENABLE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap_err();
    assert_operational_problem(error);
    assert_matches_snapshot(&log, &before);
    assert_eq!(persistence.snapshot.lock().unwrap().as_ref(), Some(&before));
    assert_operational_problem(log.purge().unwrap_err());
    assert_matches_snapshot(&log, &before);
    assert_eq!(persistence.snapshot.lock().unwrap().as_ref(), Some(&before));

    log.bind_clock_internal(Some(Arc::new(FixedClock(Some(frame(2))))));
    log.write_property(
        PropertyIdentifier::LOG_ENABLE,
        None,
        PropertyValue::Boolean(false),
        None,
    )
    .unwrap();
    assert!(!log.log_enable());
    assert!(matches!(
        log.records().back().unwrap().record.datum,
        BACnetAuditLogDatum::LogStatus(0b001)
    ));

    assert_eq!(log.purge().unwrap(), 2);
    assert_eq!(log.records().len(), 1);
    assert!(matches!(
        log.records().back().unwrap().record.datum,
        BACnetAuditLogDatum::LogStatus(0b010)
    ));

    let snapshot = persistence.snapshot.lock().unwrap().clone().unwrap();
    assert!(!snapshot.log_enable);
    assert_eq!(
        snapshot.records,
        log.records().iter().cloned().collect::<Vec<_>>()
    );

    let count_before = log.records().len();
    assert!(log
        .write_property(
            PropertyIdentifier::RECORD_COUNT,
            None,
            PropertyValue::Unsigned(0),
            None,
        )
        .is_err());
    assert_eq!(log.records().len(), count_before);
}

#[test]
fn audit_log_read_object_type_and_reporter_behavior_remain_available() {
    let storage = Arc::new(MemoryPersistence::default());
    let log = AuditLogObject::new(1, "AL-1", 100, storage).unwrap();
    assert_eq!(
        log.read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap(),
        PropertyValue::Enumerated(ObjectType::AUDIT_LOG.to_raw())
    );

    let reporter = AuditReporterObject::new(1, "AR-1").unwrap();
    assert_eq!(
        reporter
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap(),
        PropertyValue::Enumerated(ObjectType::AUDIT_REPORTER.to_raw())
    );
}
