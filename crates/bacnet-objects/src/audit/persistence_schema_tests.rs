//! Schema v1 and v2 snapshots, which 0.11.0 and earlier wrote, hold
//! log-status records with the three bits reversed. Loading one restores the
//! intended statuses, and the next commit saves schema v3.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::{
    BACnetAuditLogDatum, BACnetAuditLogRecord, BACnetAuditLogRecordResult,
};
use bacnet_types::enums::ObjectType;
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};

use super::persistence::{encode_snapshot_v1, encode_snapshot_v2};
use super::{AuditLogObject, AuditLogPersistence, AuditLogSnapshot, FileAuditLogPersistence};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_base(label: &str) -> PathBuf {
    let serial = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "rusty-bacnet-audit-schema-{label}-{}-{serial}",
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

/// The statuses each test logs, then a record that isn't a status.
fn intended() -> Vec<BACnetAuditLogDatum> {
    vec![
        BACnetAuditLogDatum::LogStatus(LogStatus::LOG_DISABLED),
        BACnetAuditLogDatum::LogStatus(LogStatus::BUFFER_PURGED),
        BACnetAuditLogDatum::LogStatus(LogStatus::LOG_DISABLED | LogStatus::BUFFER_PURGED),
        BACnetAuditLogDatum::LogStatus(LogStatus::LOG_INTERRUPTED),
        BACnetAuditLogDatum::TimeChange(1.5),
    ]
}

/// The log status today's encoder writes as the octet 0.11.0 wrote for
/// `datum`'s status: that encoder shifted the status bits up by five.
fn as_written_by_0_11(datum: &BACnetAuditLogDatum) -> BACnetAuditLogDatum {
    match datum {
        BACnetAuditLogDatum::LogStatus(status) => {
            BACnetAuditLogDatum::LogStatus(LogStatus::from_bacnet(&[status.bits() << 5]))
        }
        other => other.clone(),
    }
}

fn snapshot(datums: Vec<BACnetAuditLogDatum>, generation: u64) -> AuditLogSnapshot {
    let count = datums.len() as u64;
    AuditLogSnapshot {
        object_identifier: oid(),
        generation,
        capacity: 8,
        log_enable: true,
        total_record_count: count,
        records: datums
            .into_iter()
            .zip(1..)
            .map(|(datum, sequence_number)| BACnetAuditLogRecordResult {
                sequence_number,
                record: BACnetAuditLogRecord {
                    timestamp: (
                        Date {
                            year: 126,
                            month: 10,
                            day: 3,
                            day_of_week: 6,
                        },
                        Time {
                            hour: 12,
                            minute: 0,
                            second: 0,
                            hundredths: 0,
                        },
                    ),
                    datum,
                },
            })
            .collect(),
        completed_receipts: Vec::new(),
    }
}

fn datums(snapshot: &AuditLogSnapshot) -> Vec<BACnetAuditLogDatum> {
    snapshot
        .records
        .iter()
        .map(|result| result.record.datum.clone())
        .collect()
}

/// Whether `bytes` holds a log-status choice carrying `octet`.
fn holds_status_octet(bytes: &[u8], octet: u8) -> bool {
    bytes.windows(3).any(|window| window == [0x0A, 0x05, octet])
}

/// Load a pre-v3 file holding 0.11.0's bytes, check that the statuses come
/// back as logged, and that the next commit writes them as v3.
fn assert_converts(encode: fn(&AuditLogSnapshot) -> Result<Vec<u8>, Error>, label: &str) {
    let base = temp_base(label);
    let storage = Arc::new(FileAuditLogPersistence::new(&base).unwrap());
    let old = snapshot(intended().iter().map(as_written_by_0_11).collect(), 1);
    let bytes = encode(&old).unwrap();
    // 0.11.0 wrote log-disabled as 0x20, both bits as 0x60 and
    // log-interrupted as 0x80.
    for octet in [0x20, 0x40, 0x60, 0x80] {
        assert!(holds_status_octet(&bytes, octet), "{octet:#04x}");
    }
    std::fs::write(&storage.slot_paths()[1], &bytes).unwrap();

    let loaded = storage.load(oid()).unwrap().unwrap();
    assert_eq!(datums(&loaded), intended());

    // The object opens on the converted records, and its next commit (here
    // an appended record) saves schema v3 in the other slot.
    let mut log = AuditLogObject::new(1, "AL-1", 8, storage.clone()).unwrap();
    let mut appended = loaded.records[0].record.clone();
    appended.datum = BACnetAuditLogDatum::TimeChange(2.0);
    log.add_record(appended).unwrap();
    let saved = std::fs::read(&storage.slot_paths()[0]).unwrap();
    assert_eq!(&saved[8..10], &3u16.to_be_bytes());
    for octet in [0x80, 0x40, 0xC0, 0x20] {
        assert!(holds_status_octet(&saved, octet), "{octet:#04x}");
    }
    let reloaded = storage.load(oid()).unwrap().unwrap();
    let mut expected = intended();
    expected.push(BACnetAuditLogDatum::TimeChange(2.0));
    assert_eq!(datums(&reloaded), expected);
    cleanup(&base);
}

#[test]
fn schema_v2_log_status_records_convert_on_load_and_save_as_v3() {
    assert_converts(encode_snapshot_v2, "v2");
}

#[test]
fn schema_v1_log_status_records_convert_on_load() {
    assert_converts(encode_snapshot_v1, "v1");
}

#[test]
fn schema_v3_log_status_records_round_trip_unchanged() {
    let base = temp_base("v3");
    let storage = FileAuditLogPersistence::new(&base).unwrap();
    let current = snapshot(intended(), 1);
    storage.commit(&current).unwrap();
    let bytes = std::fs::read(&storage.slot_paths()[1]).unwrap();
    assert_eq!(&bytes[8..10], &3u16.to_be_bytes());
    assert!(holds_status_octet(&bytes, 0x80));
    assert_eq!(storage.load(oid()).unwrap().unwrap(), current);
    cleanup(&base);
}
