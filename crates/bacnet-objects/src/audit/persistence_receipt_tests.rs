use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use bacnet_types::constructed::{
    BACnetAuditLogDatum, BACnetAuditLogRecord, BACnetAuditLogRecordResult,
};
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};

use super::persistence::encode_snapshot_v1;
use super::{
    AuditLogPersistence, AuditLogSnapshot, CompletedAuditReceipt, FileAuditLogPersistence,
    MAX_AUDIT_RECEIPT_KEY_BYTES, MAX_COMPLETED_AUDIT_RECEIPTS,
};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_base(label: &str) -> PathBuf {
    let serial = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "rusty-bacnet-audit-receipt-{label}-{}-{serial}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("state")
}

fn cleanup(base: &Path) {
    if let Some(parent) = base.parent() {
        let _ = std::fs::remove_dir_all(parent);
    }
}

fn oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::AUDIT_LOG, 1).unwrap()
}

fn snapshot(receipts: Vec<CompletedAuditReceipt>) -> AuditLogSnapshot {
    AuditLogSnapshot {
        object_identifier: oid(),
        generation: 1,
        capacity: 1,
        log_enable: true,
        total_record_count: 0,
        records: Vec::new(),
        completed_receipts: receipts,
    }
}

fn snapshot_with_record() -> AuditLogSnapshot {
    let mut snapshot = snapshot(Vec::new());
    snapshot.total_record_count = 1;
    snapshot.records.push(BACnetAuditLogRecordResult {
        sequence_number: 1,
        record: BACnetAuditLogRecord {
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
            datum: BACnetAuditLogDatum::LogStatus(0),
        },
    });
    snapshot
}

fn rewrite_checksum(data: &mut [u8]) {
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

#[test]
fn schema_v1_loads_with_empty_receipts_and_v2_round_trips_them() {
    let base = temp_base("migration");
    let storage = FileAuditLogPersistence::new(&base).unwrap();
    let v1 = snapshot_with_record();
    std::fs::write(&storage.slot_paths()[1], encode_snapshot_v1(&v1).unwrap()).unwrap();
    assert_eq!(storage.load(oid()).unwrap().unwrap(), v1);

    let receipt = CompletedAuditReceipt::new(b"exact-request".to_vec(), 1_725_000_000_123).unwrap();
    let v2 = snapshot(vec![receipt]);
    storage.commit(&v2).unwrap();
    assert_eq!(storage.load(oid()).unwrap().unwrap(), v2);
    cleanup(&base);
}

#[test]
fn unknown_and_malformed_v2_receipt_fields_fail_closed() {
    const RECEIPT_COUNT_OFFSET: usize = 26 + 4 + 1 + 8 + 4;
    const RECEIPT_KEY_LENGTH_OFFSET: usize = RECEIPT_COUNT_OFFSET + 4 + 8;

    let base = temp_base("malformed");
    let storage = FileAuditLogPersistence::new(&base).unwrap();
    let valid = snapshot(vec![CompletedAuditReceipt::new(vec![1], 100).unwrap()]);
    storage.commit(&valid).unwrap();
    let active = storage.slot_paths()[1].clone();
    let original = std::fs::read(&active).unwrap();

    let mut unknown = original.clone();
    unknown[8..10].copy_from_slice(&3u16.to_be_bytes());
    rewrite_checksum(&mut unknown);
    std::fs::write(&active, unknown).unwrap();
    assert!(storage.load(oid()).is_err());

    let mut count_overflow = original.clone();
    count_overflow[RECEIPT_COUNT_OFFSET..RECEIPT_COUNT_OFFSET + 4]
        .copy_from_slice(&((MAX_COMPLETED_AUDIT_RECEIPTS + 1) as u32).to_be_bytes());
    rewrite_checksum(&mut count_overflow);
    std::fs::write(&active, count_overflow).unwrap();
    assert!(storage.load(oid()).is_err());

    let mut key_overflow = original.clone();
    key_overflow[RECEIPT_KEY_LENGTH_OFFSET..RECEIPT_KEY_LENGTH_OFFSET + 4]
        .copy_from_slice(&((MAX_AUDIT_RECEIPT_KEY_BYTES + 1) as u32).to_be_bytes());
    rewrite_checksum(&mut key_overflow);
    std::fs::write(&active, key_overflow).unwrap();
    assert!(storage.load(oid()).is_err());

    let mut truncated_key = original;
    truncated_key[RECEIPT_KEY_LENGTH_OFFSET..RECEIPT_KEY_LENGTH_OFFSET + 4]
        .copy_from_slice(&2u32.to_be_bytes());
    rewrite_checksum(&mut truncated_key);
    std::fs::write(&active, truncated_key).unwrap();
    assert!(storage.load(oid()).is_err());
    cleanup(&base);
}
