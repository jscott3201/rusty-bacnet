//! The Audit Log slot (#2047): its round trip without std, the kinds it
//! refuses with, the legacy schemas, the two-slot choice [`audit_log::newest`]
//! makes, and the octets the file backend writes.

use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::{
    BACnetAuditLogDatum, BACnetAuditLogRecord, BACnetAuditLogRecordResult,
};
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;

use super::tests::{DATE, at, oid, reseal};
use super::{SnapshotErrorKind, audit_log};
use crate::audit::persistence::codec::{SCHEMA_VERSION_V1, SCHEMA_VERSION_V2, encode_for_version};
use crate::audit::{
    AuditLogSnapshot, CompletedAuditReceipt, MAX_AUDIT_RECEIPT_KEY_BYTES, MAX_AUDIT_RECORDS,
};
use crate::durable::crc32;

fn object() -> ObjectIdentifier {
    oid(ObjectType::AUDIT_LOG, 2)
}

fn record(sequence_number: u64, datum: BACnetAuditLogDatum) -> BACnetAuditLogRecordResult {
    BACnetAuditLogRecordResult {
        sequence_number,
        record: BACnetAuditLogRecord {
            timestamp: (DATE, at(8)),
            datum,
        },
    }
}

fn snapshot(generation: u64) -> AuditLogSnapshot {
    AuditLogSnapshot {
        object_identifier: object(),
        generation,
        capacity: 4,
        log_enable: true,
        total_record_count: 7,
        records: vec![
            record(
                6,
                BACnetAuditLogDatum::LogStatus(LogStatus::LOG_INTERRUPTED),
            ),
            record(7, BACnetAuditLogDatum::TimeChange(2.5)),
        ],
        completed_receipts: vec![CompletedAuditReceipt::new(vec![4, 5], 9_000).unwrap()],
    }
}

fn slot(generation: u64) -> Vec<u8> {
    audit_log::encode(&snapshot(generation)).unwrap()
}

#[test]
fn a_snapshot_round_trips_in_version_3() {
    let octets = slot(3);
    assert_eq!(octets[..8], audit_log::MAGIC);
    assert_eq!(octets[8..10], audit_log::VERSION.to_be_bytes());
    assert_eq!(audit_log::decode(object(), &octets), Ok(snapshot(3)));
}

/// Versions 1 and 2 still load. Their log-status records were stored with
/// the bits reversed, so a version 2 slot of today's octets reads back
/// with them reversed, and saves again as version 3.
#[test]
fn versions_1_and_2_still_load() {
    let mut without_receipts = snapshot(3);
    without_receipts.completed_receipts.clear();
    without_receipts.records[0].record.datum = BACnetAuditLogDatum::TimeChange(1.0);
    let v1 = encode_for_version(&without_receipts, SCHEMA_VERSION_V1).unwrap();
    assert_eq!(audit_log::decode(object(), &v1), Ok(without_receipts));

    let v2 = encode_for_version(&snapshot(3), SCHEMA_VERSION_V2).unwrap();
    let loaded = audit_log::decode(object(), &v2).unwrap();
    let BACnetAuditLogDatum::LogStatus(status) = loaded.records[0].record.datum else {
        panic!("a log-status record");
    };
    assert_eq!(status, LogStatus::LOG_DISABLED);
    assert_eq!(audit_log::encode(&loaded).unwrap()[8..10], [0, 3]);
}

#[test]
fn a_slot_refuses_with_the_kind_it_names() {
    let octets = slot(3);
    let kind = |octets: &[u8], object| {
        let error = audit_log::decode(object, octets).unwrap_err();
        (error.kind(), error.is_damage())
    };
    let truncated = (SnapshotErrorKind::Truncated, true);
    assert_eq!(kind(&octets[..octets.len() - 1], object()), truncated);
    assert_eq!(kind(&octets[..20], object()), truncated);
    // An erased length field reads as cut short, never as an overflow.
    let mut erased = octets.clone();
    erased[22..26].copy_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(kind(&erased, object()), truncated);
    let mut flipped = octets.clone();
    flipped[30] ^= 1;
    assert_eq!(kind(&flipped, object()), (SnapshotErrorKind::Damaged, true));
    let mut newer = octets.clone();
    newer[8..10].copy_from_slice(&4u16.to_be_bytes());
    reseal(&mut newer);
    assert_eq!(
        kind(&newer, object()),
        (SnapshotErrorKind::UnknownVersion(4), false)
    );
    let other = oid(ObjectType::AUDIT_LOG, 3);
    assert_eq!(
        kind(&octets, other),
        (SnapshotErrorKind::OtherObject, false)
    );
    // A capacity past the cap, sealed: damage, not a refusal to save.
    let mut capacity = octets.clone();
    capacity[26..30].copy_from_slice(&(MAX_AUDIT_RECORDS + 1).to_be_bytes());
    reseal(&mut capacity);
    assert_eq!(
        kind(&capacity, object()),
        (SnapshotErrorKind::Damaged, true)
    );
    let mut huge = octets;
    huge.resize(audit_log::MAX_LEN + 1, 0);
    assert_eq!(kind(&huge, object()), (SnapshotErrorKind::TooLarge, true));
}

#[test]
fn a_snapshot_that_does_not_hold_together_never_encodes() {
    let mut zero = snapshot(3);
    zero.generation = 0;
    let mut gap = snapshot(3);
    gap.records[0].sequence_number = 4;
    for refused in [zero, gap] {
        let error = audit_log::encode(&refused).unwrap_err();
        assert_eq!(error.kind(), SnapshotErrorKind::OutOfRange);
        assert!(!error.is_damage());
    }
}

#[test]
fn the_newer_slot_wins_and_damage_is_passed_over() {
    let newest = |slots| audit_log::newest(object(), slots);
    assert_eq!(newest([None, None]), Ok(None));
    let (three, four) = (slot(3), slot(4));
    let chosen = newest([Some(&four), Some(&three)]).unwrap().unwrap();
    assert_eq!((chosen.slot, chosen.generation), (0, 4));
    let cut = &four[..four.len() - 1];
    let chosen = newest([Some(cut), Some(&three)]).unwrap().unwrap();
    assert_eq!((chosen.slot, chosen.generation), (1, 3));
    assert!(chosen.passed_over[0].is_some_and(|error| error.is_damage()));
    // Nothing sound: the damage, not an empty store.
    for slots in [[Some(cut), None], [Some(cut), Some(cut)]] {
        assert_eq!(
            newest(slots).unwrap_err().kind(),
            SnapshotErrorKind::Truncated
        );
    }
    // A sound slot this build must not take stops the choice.
    let mut newer = slot(5);
    newer[8..10].copy_from_slice(&4u16.to_be_bytes());
    reseal(&mut newer);
    assert!(newest([Some(&three), Some(&newer)]).is_err());
}

/// One generation twice is one snapshot when both encode to the same
/// version 3 octets, whatever version each slot was saved in.
#[test]
fn one_generation_twice_is_compared_as_version_3() {
    let newest = |slots| audit_log::newest(object(), slots);
    let three = slot(3);
    let mut legacy = snapshot(3);
    legacy.records[0].record.datum = BACnetAuditLogDatum::TimeChange(1.0);
    let v2 = encode_for_version(&legacy, SCHEMA_VERSION_V2).unwrap();
    let v3 = audit_log::encode(&legacy).unwrap();
    assert_ne!(v2, v3);
    let chosen = newest([Some(&v3), Some(&v2)]).unwrap().unwrap();
    assert_eq!((chosen.slot, chosen.snapshot), (0, legacy));
    let error = newest([Some(&three), Some(&v3)]).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::Damaged);
    assert!(error.to_string().contains("same generation"), "{error}");
}

/// A receipt key at its cap round-trips; a slot whose key runs one octet
/// past it is damage.
#[test]
fn a_receipt_key_at_the_cap_loads_and_one_past_it_does_not() {
    let mut at_cap = snapshot(3);
    at_cap.completed_receipts =
        vec![CompletedAuditReceipt::new(vec![0xAB; MAX_AUDIT_RECEIPT_KEY_BYTES], 1).unwrap()];
    let octets = audit_log::encode(&at_cap).unwrap();
    assert_eq!(audit_log::decode(object(), &octets), Ok(at_cap));

    // The key's length field sits before the key and the checksum.
    let mut past = octets;
    let at = past.len() - 4 - MAX_AUDIT_RECEIPT_KEY_BYTES - 4;
    past[at..at + 4].copy_from_slice(&(MAX_AUDIT_RECEIPT_KEY_BYTES as u32 + 1).to_be_bytes());
    past.insert(past.len() - 4, 0xAB);
    let payload = (past.len() - 26 - 4) as u32;
    past[22..26].copy_from_slice(&payload.to_be_bytes());
    let end = past.len() - 4;
    let checksum = crc32(&past[..end]);
    past[end..].copy_from_slice(&checksum.to_be_bytes());
    let error = audit_log::decode(object(), &past).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::Damaged);
}

/// A tie compares octets, not values: a NaN record matches itself, and
/// -0.0 differs from +0.0, as the file backend has always decided.
#[test]
fn a_tie_compares_octets_not_values() {
    let newest = |slots| audit_log::newest(object(), slots);
    let with = |value: f32| {
        let mut snapshot = snapshot(3);
        snapshot.records[1].record.datum = BACnetAuditLogDatum::TimeChange(value);
        audit_log::encode(&snapshot).unwrap()
    };
    let nan = with(f32::NAN);
    let chosen = newest([Some(&nan), Some(&nan)]).unwrap().unwrap();
    assert_eq!(chosen.slot, 0);
    let (negative, positive) = (with(-0.0), with(0.0));
    let error = newest([Some(&negative), Some(&positive)]).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::Damaged);
}

/// The file backend writes what `encode` returns, and loads a slot
/// `encode` wrote.
// std: the file backend.
#[cfg(feature = "std")]
#[test]
fn the_file_backend_writes_and_reads_the_public_octets() {
    use crate::audit::{AuditLogPersistence, FileAuditLogPersistence};
    use crate::snapshot::tests::file_backends::Scratch;

    let scratch = Scratch::new();
    let store = FileAuditLogPersistence::new(scratch.path("audit")).unwrap();
    store.commit(&snapshot(3)).unwrap();
    let [_, slot1] = store.slot_paths();
    let written = std::fs::read(&slot1).unwrap();
    assert_eq!(written, slot(3));
    assert_eq!(audit_log::decode(object(), &written), Ok(snapshot(3)));
    let [slot0, _] = store.slot_paths();
    std::fs::write(&slot0, slot(4)).unwrap();
    assert_eq!(store.load(object()).unwrap(), Some(snapshot(4)));
}
