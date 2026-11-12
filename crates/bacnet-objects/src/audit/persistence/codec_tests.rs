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

/// A record whose datum is a later revision's alternative around `levels`
/// nested empty values: with the datum field and its own tag, a record
/// `levels + 2` deep.
fn nested_record(sequence_number: u64, levels: usize) -> BACnetAuditLogRecordResult {
    let mut result = status_record(sequence_number);
    let body = [vec![0x1E; levels], vec![0x1F; levels]].concat();
    result.record.datum = BACnetAuditLogDatum::Opaque(bacnet_types::constructed::OpaqueChoice {
        tag: 3,
        constructed: true,
        data: body,
    });
    result
}

/// `data` with the record numbered `sequence_number` nested `extra` levels
/// deeper, as an older build could have saved it: empty `[1]`s inside its
/// innermost value, its length prefix and the payload length grown to
/// match, the slot resealed.
fn deepened(mut data: Vec<u8>, sequence_number: u64, extra: usize) -> Vec<u8> {
    // Past the capacity, Enable, Total_Record_Count and the record count,
    // each record is its sequence number, its four-octet length and its
    // octets.
    let mut at = HEADER_LEN + 4 + 1 + 8 + 4;
    let length = loop {
        let number = u64::from_be_bytes(data[at..at + 8].try_into().unwrap());
        at += 8;
        let length = u32::from_be_bytes(data[at..at + 4].try_into().unwrap()) as usize;
        if number == sequence_number {
            break length;
        }
        at += 4 + length;
    };
    let record = &data[at + 4..at + 4 + length];
    let innermost = record
        .windows(2)
        .position(|pair| pair == [0x1E, 0x1F])
        .unwrap()
        + 1;
    let insert = [vec![0x1E; extra], vec![0x1F; extra]].concat();
    data.splice(at + 4 + innermost..at + 4 + innermost, insert);
    let grown = (length + 2 * extra) as u32;
    data[at..at + 4].copy_from_slice(&grown.to_be_bytes());
    let payload = u32::from_be_bytes(data[HEADER_LEN - 4..HEADER_LEN].try_into().unwrap());
    data[HEADER_LEN - 4..HEADER_LEN].copy_from_slice(&(payload + 2 * extra as u32).to_be_bytes());
    let checksum_start = data.len() - TRAILER_LEN;
    let checksum = crc32(&data[..checksum_start]);
    data[checksum_start..].copy_from_slice(&checksum.to_be_bytes());
    data
}

/// A record nested as deep as an AuditLogQuery-ACK carries is stored and
/// read back; one level deeper is refused before it's stored (#2483).
#[test]
fn a_record_nested_too_deep_for_a_query_is_never_encoded() {
    let mut deepest = snapshot();
    deepest.records[1] = nested_record(2, 28);
    let data = encode_snapshot(&deepest).unwrap();
    assert_eq!(decode(&data).unwrap(), deepest);

    let mut too_deep = snapshot();
    too_deep.records[1] = nested_record(2, 29);
    assert_out_of_range(encode_snapshot(&too_deep), "record does not encode");
}

/// A record an older build saved nested deeper than an AuditLogQuery-ACK
/// carries (#2483), by one level or by the two more the record decoder
/// alone takes, is dropped on load, the records older than it each taking
/// the next number up, and the rest restores; across the Unsigned64 wrap
/// too. One that doesn't decode at all is still damage.
#[test]
fn a_saved_record_nested_too_deep_is_dropped_and_the_rest_restores() {
    for extra in [1, 2] {
        let mut saved = snapshot();
        saved.total_record_count = 3;
        saved.records = vec![status_record(1), nested_record(2, 28), status_record(3)];
        let data = deepened(encode_snapshot(&saved).unwrap(), 2, extra);
        let mut expected = saved.clone();
        expected.records = vec![status_record(2), status_record(3)];
        assert_eq!(decode(&data).unwrap(), expected, "{extra}");
    }

    // The newest record dropped: the older ones move up across the wrap
    // from the largest sequence number to 1.
    let mut saved = snapshot();
    saved.total_record_count = 1;
    saved.records = vec![
        status_record(u64::MAX - 1),
        status_record(u64::MAX),
        nested_record(1, 28),
    ];
    let data = deepened(encode_snapshot(&saved).unwrap(), 1, 1);
    let mut expected = saved.clone();
    expected.records = vec![status_record(u64::MAX), status_record(1)];
    assert_eq!(decode(&data).unwrap(), expected);

    // Nested past the tag decoder's limit, the record doesn't decode: the
    // slot is damaged, not trimmed.
    let mut saved = snapshot();
    saved.records[1] = nested_record(2, 28);
    let data = deepened(encode_snapshot(&saved).unwrap(), 2, 3);
    assert_encoding(decode(&data), "record does not decode");
}

/// Only the depth check drops a record: one that fails to encode for any
/// other reason (a target priority of 0, which a decoded record can't hold)
/// stays for the snapshot check to refuse as damage.
#[test]
fn only_a_record_too_deep_is_dropped() {
    use bacnet_types::constructed::{BACnetAuditNotification, BACnetRecipient};
    use bacnet_types::enums::AuditOperation;
    let device = BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap());
    let mut unencodable = status_record(3);
    unencodable.record.datum = BACnetAuditLogDatum::AuditNotification(BACnetAuditNotification {
        source_timestamp: None,
        target_timestamp: None,
        source_device: device.clone(),
        source_object: None,
        operation: AuditOperation::WRITE,
        source_comment: None,
        target_comment: None,
        invoke_id: None,
        source_user_id: None,
        source_user_role: None,
        target_device: device,
        target_object: None,
        target_property: None,
        target_priority: Some(0),
        target_value: None,
        current_value: None,
        result: None,
    });
    let mut trimmed = snapshot();
    trimmed.total_record_count = 3;
    trimmed.records = vec![status_record(1), nested_record(2, 29), unencodable.clone()];
    codec::drop_too_deep(&mut trimmed);
    assert_eq!(trimmed.records, vec![status_record(2), unencodable]);
    assert_out_of_range(encode_snapshot(&trimmed), "record does not encode");
}
