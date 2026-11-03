//! The Audit Log snapshot codec on its own, without a file: the checks a
//! saved snapshot passes before it is written and after it is read (#2183).
//! These run in the alloc-only build too.

use super::codec::{self, HEADER_LEN, MAX_SNAPSHOT_BYTES, TRAILER_LEN};
use super::*;
use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::{BACnetAuditLogDatum, BACnetAuditLogRecord};
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::{Date, Time};

use crate::durable::crc32;

fn oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::AUDIT_LOG, 1).unwrap()
}

fn status_record(sequence_number: u64) -> BACnetAuditLogRecordResult {
    BACnetAuditLogRecordResult {
        sequence_number,
        record: BACnetAuditLogRecord {
            timestamp: (
                Date {
                    year: 126,
                    month: 11,
                    day: 2,
                    day_of_week: 1,
                },
                Time {
                    hour: 8,
                    minute: 0,
                    second: 0,
                    hundredths: 0,
                },
            ),
            datum: BACnetAuditLogDatum::LogStatus(LogStatus::LOG_DISABLED),
        },
    }
}

/// A disabled log holding two records and one receipt.
fn snapshot() -> AuditLogSnapshot {
    AuditLogSnapshot {
        object_identifier: oid(),
        generation: 3,
        capacity: 4,
        log_enable: false,
        total_record_count: 2,
        records: vec![status_record(1), status_record(2)],
        completed_receipts: vec![CompletedAuditReceipt::new(vec![1, 2, 3], 1_000).unwrap()],
    }
}

/// Decode `data` as a read slot is decoded, with the stack's error.
fn decode(data: &[u8]) -> Result<AuditLogSnapshot, Error> {
    Ok(codec::decode(data, oid())?)
}

/// Today's slot for `snapshot`, with the stack's error.
fn encode_snapshot(snapshot: &AuditLogSnapshot) -> Result<Vec<u8>, Error> {
    Ok(codec::encode(snapshot)?)
}

/// Overwrite `octets` at `offset` and seal the snapshot again, so only the
/// field changed and the checksum still holds.
fn patched(mut data: Vec<u8>, offset: usize, octets: &[u8]) -> Vec<u8> {
    data[offset..offset + octets.len()].copy_from_slice(octets);
    let checksum_start = data.len() - TRAILER_LEN;
    let checksum = crc32(&data[..checksum_start]);
    data[checksum_start..].copy_from_slice(&checksum.to_be_bytes());
    data
}

fn assert_out_of_range(result: Result<impl core::fmt::Debug, Error>, needle: &str) {
    match result {
        Err(Error::OutOfRange(message)) => assert!(message.contains(needle), "{message}"),
        other => panic!("expected OutOfRange with {needle:?}, got {other:?}"),
    }
}

fn assert_encoding(result: Result<impl core::fmt::Debug, Error>, needle: &str) {
    match result {
        Err(Error::Encoding(message)) => assert!(message.contains(needle), "{message}"),
        other => panic!("expected Encoding with {needle:?}, got {other:?}"),
    }
}

#[test]
fn a_disabled_log_with_records_and_a_receipt_round_trips() {
    let expected = snapshot();
    let data = encode_snapshot(&expected).unwrap();
    assert_eq!(decode(&data).unwrap(), expected);
}

#[test]
fn a_sealed_snapshot_at_generation_zero_is_damage() {
    let data = patched(
        encode_snapshot(&snapshot()).unwrap(),
        14,
        &0u64.to_be_bytes(),
    );
    assert_encoding(decode(&data), "generation must be nonzero");
}

#[test]
fn a_sealed_capacity_past_the_cap_is_refused() {
    let data = patched(
        encode_snapshot(&snapshot()).unwrap(),
        HEADER_LEN,
        &(MAX_AUDIT_RECORDS + 1).to_be_bytes(),
    );
    assert_encoding(decode(&data), "persisted capacity");
}

#[test]
fn a_sealed_log_enable_that_is_not_boolean_is_refused() {
    let data = patched(encode_snapshot(&snapshot()).unwrap(), HEADER_LEN + 4, &[2]);
    assert_encoding(decode(&data), "not Boolean");
}

#[test]
fn a_snapshot_past_the_size_cap_is_refused_before_its_header_is_read() {
    // Zeroed, so the allocation stays cheap; the size alone refuses it.
    let data = vec![0; MAX_SNAPSHOT_BYTES + 1];
    let refused = codec::verify_integrity(&data).unwrap_err();
    assert!(refused.is_damage());
    assert_encoding(Err::<(), Error>(refused.into()), "exceeds");
}

#[test]
fn a_snapshot_that_cannot_hold_together_is_never_encoded() {
    let mut zero_generation = snapshot();
    zero_generation.generation = 0;
    assert_out_of_range(encode_snapshot(&zero_generation), "generation");

    let mut over_cap = snapshot();
    over_cap.capacity = MAX_AUDIT_RECORDS + 1;
    assert_out_of_range(encode_snapshot(&over_cap), "capacity");

    // Schema v1 has no room for receipts.
    assert_out_of_range(encode_snapshot_v1(&snapshot()), "completed receipts");
    // Schema v1 stored log statuses with their bits reversed, so a record
    // of another kind shows the round trip.
    let mut without_receipts = snapshot();
    without_receipts.completed_receipts.clear();
    for result in &mut without_receipts.records {
        result.record.datum = BACnetAuditLogDatum::TimeChange(1.5);
    }
    let v1 = encode_snapshot_v1(&without_receipts).unwrap();
    assert_eq!(v1[8..10], codec::SCHEMA_VERSION_V1.to_be_bytes());
    assert_eq!(decode(&v1).unwrap(), without_receipts);
}
