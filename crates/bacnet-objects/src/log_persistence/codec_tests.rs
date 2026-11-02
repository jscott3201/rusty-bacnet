//! Slot checks #2235 left uncovered (#2183): trailing payload octets and a
//! zero generation in a slot whose checksum is sound, a record too long for
//! a slot, and the file store's own I/O paths that need no fault injection.

use std::path::Path;

use bacnet_types::constructed::{BACnetEventLogRecord, BACnetLogRecord, LogDatum};
use bacnet_types::enums::ObjectType;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use super::codec::{self, Decoded, HEADER_LEN, TRAILER_LEN};
use super::tests::{DATE, TempBase, at, event_record};
use super::*;
use crate::clock::UNSPECIFIED_DATETIME;
use crate::durable::crc32;

fn snapshot(generation: u64) -> LogSnapshot<BACnetEventLogRecord> {
    LogSnapshot {
        object_identifier: ObjectIdentifier::new(ObjectType::EVENT_LOG, 5).unwrap(),
        generation,
        buffer_size: 4,
        log_enable: true,
        stop_when_full: false,
        total_record_count: 2,
        records: (1..3).map(event_record).collect(),
        last_notify_record: 0,
        records_since_notification_from: 0,
        notification_threshold: 0,
        start_time: UNSPECIFIED_DATETIME,
        stop_time: (DATE, at(23)),
        acquisition: None,
        log_device_object_property: Vec::new(),
    }
}

fn oid() -> ObjectIdentifier {
    snapshot(1).object_identifier
}

/// Rewrite the declared payload length and the checksum after an edit, so
/// the slot passes its integrity check and only the edit is wrong.
fn reseal(data: &mut [u8]) {
    let payload = (data.len() - HEADER_LEN - TRAILER_LEN) as u32;
    data[22..26].copy_from_slice(&payload.to_be_bytes());
    let at = data.len() - TRAILER_LEN;
    let checksum = crc32(&data[..at]);
    data[at..].copy_from_slice(&checksum.to_be_bytes());
}

fn corrupt_reason(data: &[u8]) -> String {
    match codec::decode::<BACnetEventLogRecord>(data, oid()) {
        Decoded::Corrupt(Error::Encoding(message)) => message,
        Decoded::Corrupt(other) => panic!("expected an encoding error, got {other:?}"),
        Decoded::Valid(_) => panic!("expected a damaged slot, got a valid one"),
        Decoded::Refused(error) => panic!("expected a damaged slot, got a refusal: {error:?}"),
    }
}

#[test]
fn the_untouched_slot_decodes() {
    let data = codec::encode(&snapshot(3)).unwrap();
    match codec::decode::<BACnetEventLogRecord>(&data, oid()) {
        Decoded::Valid(decoded) => assert_eq!(decoded, snapshot(3)),
        _ => panic!("a fresh slot must decode"),
    }
}

#[test]
fn a_payload_with_trailing_octets_is_damage() {
    let mut data = codec::encode(&snapshot(3)).unwrap();
    // One octet after the record list, before the checksum.
    data.insert(data.len() - TRAILER_LEN, 0);
    reseal(&mut data);
    assert!(corrupt_reason(&data).contains("trailing octets"));
}

#[test]
fn a_sealed_slot_at_generation_zero_is_damage() {
    let mut data = codec::encode(&snapshot(3)).unwrap();
    data[14..22].copy_from_slice(&0u64.to_be_bytes());
    reseal(&mut data);
    assert!(corrupt_reason(&data).contains("generation must be nonzero"));
}

#[test]
fn a_record_longer_than_a_slot_element_is_never_written() {
    // Two application-tagged Octet Strings of 600 KiB each: complete values
    // under the encoder's per-value limit, so the record encodes, but
    // together past the per-element cap.
    let length = 600 * 1024;
    let mut value = Vec::new();
    for _ in 0..2 {
        value.extend_from_slice(&[0x65, 0xFF]);
        value.extend_from_slice(&(length as u32).to_be_bytes());
        value.resize(value.len() + length, 0xAB);
    }
    let snapshot = LogSnapshot {
        object_identifier: ObjectIdentifier::new(ObjectType::TREND_LOG, 5).unwrap(),
        generation: 1,
        buffer_size: 1,
        log_enable: true,
        stop_when_full: false,
        total_record_count: 1,
        records: vec![BACnetLogRecord {
            date: DATE,
            time: at(1),
            log_datum: LogDatum::AnyValue(value),
            status_flags: None,
        }],
        last_notify_record: 0,
        records_since_notification_from: 0,
        notification_threshold: 0,
        start_time: UNSPECIFIED_DATETIME,
        stop_time: UNSPECIFIED_DATETIME,
        acquisition: Some(LogAcquisitionSettings {
            logging_type: bacnet_types::enums::LoggingType::POLLED,
            log_interval: 100,
            align_intervals: false,
            interval_offset: 0,
        }),
        log_device_object_property: vec![super::tests::reference(1)],
    };
    match codec::encode(&snapshot) {
        Err(Error::OutOfRange(message)) => {
            assert!(message.contains("element length"), "{message}")
        }
        other => panic!("expected the element cap, got {other:?}"),
    }
}

#[test]
fn both_slots_at_one_generation_and_identical_load_as_that_snapshot() {
    let base = TempBase::new("twin-slots");
    let store = FileLogPersistence::new(base.path()).unwrap();
    store.commit(&snapshot(2)).unwrap();
    let [slot0, slot1] = store.slot_paths();
    std::fs::copy(&slot0, &slot1).unwrap();
    assert_eq!(store.load(oid()).unwrap(), Some(snapshot(2)));
}

#[test]
fn a_commit_creates_a_missing_parent_directory() {
    let base = TempBase::new("nested");
    let nested = base.path().join("logs").join("state");
    let store = FileLogPersistence::new(&nested).unwrap();
    store.commit(&snapshot(1)).unwrap();
    assert!(nested.parent().unwrap().is_dir());
    assert_eq!(store.load(oid()).unwrap(), Some(snapshot(1)));
}

/// A plain file where the slots' directory belongs.
fn file_in_the_way(base: &Path) -> std::path::PathBuf {
    let blocker = base.with_extension("blocker");
    std::fs::write(&blocker, b"not a directory").unwrap();
    blocker.join("state")
}

#[test]
fn a_slot_that_cannot_be_created_fails_the_commit() {
    let base = TempBase::new("blocked-commit");
    let store = FileLogPersistence::new(file_in_the_way(base.path())).unwrap();
    let error = LogPersistence::<BACnetEventLogRecord>::commit(&store, &snapshot(1)).unwrap_err();
    assert!(!matches!(error, Error::Encoding(_)), "{error:?}");
}

#[test]
fn a_slot_that_cannot_be_read_fails_the_load() {
    let base = TempBase::new("unreadable");
    let store = FileLogPersistence::new(base.path()).unwrap();
    // A directory at the first slot's path can't be read as a slot on any
    // platform.
    std::fs::create_dir_all(&store.slot_paths()[0]).unwrap();
    let error = LogPersistence::<BACnetEventLogRecord>::load(&store, oid()).unwrap_err();
    assert!(!matches!(error, Error::Encoding(_)), "{error:?}");
}
