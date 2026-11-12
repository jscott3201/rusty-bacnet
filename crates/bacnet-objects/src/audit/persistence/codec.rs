//! The octets of one Audit Log snapshot slot (`RBALOG01`), public as
//! [`crate::snapshot::audit_log`], and the checks every snapshot passes
//! before it is written or after it is read.
//!
//! A slot is a 26-octet header (magic tag, schema version, object
//! identifier, generation, payload length), the payload, and a CRC-32 of
//! everything before it. The payload holds the capacity, a Boolean Enable
//! octet, Total_Record_Count, the record count, then each record's sequence
//! number, length and Clause 21 encoding, and from schema version 2 the
//! completed receipts. Every integer is big-endian.

use crate::prelude::*;
use bacnet_encoding::constructed::{
    decode_audit_log_record, encode_audit_log_record, is_record_too_deep,
};
use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::{
    BACnetAuditLogDatum, BACnetAuditLogRecord, BACnetAuditLogRecordResult, BACnetRecipient,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use super::{AuditLogSnapshot, MAX_AUDIT_RECORDS};
use crate::audit::receipt;
use crate::durable::crc32;
use crate::snapshot::{SnapshotError, SnapshotErrorKind};

#[path = "receipt_codec.rs"]
mod receipt_codec;

pub(crate) const MAGIC: &[u8; 8] = b"RBALOG01";
pub(crate) const SCHEMA_VERSION_V1: u16 = 1;
/// Adds completed receipts. Like v1, it holds log-status records in the
/// reversed bit order of releases up to 0.11.0.
pub(crate) const SCHEMA_VERSION_V2: u16 = 2;
/// Holds log-status records in the bit0-first order of every BACnet bit
/// string.
pub(crate) const SCHEMA_VERSION: u16 = 3;
pub(crate) const HEADER_LEN: usize = 8 + 2 + 4 + 8 + 4;
pub(crate) const TRAILER_LEN: usize = 4;
const MAX_RECORD_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;
/// What an Audit Log snapshot's error text is led by.
pub(crate) const WHAT: &str = "AuditLog snapshot";

/// The slot octets for `snapshot` in today's schema. Every refusal is
/// [`OutOfRange`](SnapshotErrorKind::OutOfRange): nothing was stored.
pub(crate) fn encode(snapshot: &AuditLogSnapshot) -> Result<Vec<u8>, SnapshotError> {
    encode_for_version(snapshot, SCHEMA_VERSION)
}

/// [`encode`] under the header of schema `version`. Only tests write an
/// older one; the records are encoded as today's codec does.
pub(crate) fn encode_for_version(
    snapshot: &AuditLogSnapshot,
    version: u16,
) -> Result<Vec<u8>, SnapshotError> {
    encode_slot(snapshot, version).map_err(|error| error.refusing().in_format(WHAT))
}

fn encode_slot(snapshot: &AuditLogSnapshot, version: u16) -> Result<Vec<u8>, SnapshotError> {
    check(snapshot)?;
    if version == SCHEMA_VERSION_V1 && !snapshot.completed_receipts.is_empty() {
        return Err(SnapshotError::out_of_range(
            "in schema version 1 cannot hold completed receipts",
        ));
    }
    let too_long = SnapshotError::out_of_range("length exceeds the slot cap");
    // The header goes first, its payload length filled in at the end, so
    // the payload is written in place rather than copied in after (#2233).
    let mut data = Vec::new();
    data.extend_from_slice(MAGIC);
    data.extend_from_slice(&version.to_be_bytes());
    data.extend_from_slice(&snapshot.object_identifier.encode());
    data.extend_from_slice(&snapshot.generation.to_be_bytes());
    data.extend_from_slice(&[0; 4]);
    data.extend_from_slice(&snapshot.capacity.to_be_bytes());
    data.push(u8::from(snapshot.log_enable));
    data.extend_from_slice(&snapshot.total_record_count.to_be_bytes());
    data.extend_from_slice(&(snapshot.records.len() as u32).to_be_bytes());
    // One buffer for every record, cleared before each.
    let mut record = record_buffer(snapshot.records.len());
    for result in &snapshot.records {
        record_len(&result.record, &mut record)?;
        let projected = data
            .len()
            .checked_add(8 + 4 + record.len())
            .and_then(|length| length.checked_add(TRAILER_LEN))
            .ok_or(too_long)?;
        if projected > MAX_SNAPSHOT_BYTES {
            return Err(too_long);
        }
        data.extend_from_slice(&result.sequence_number.to_be_bytes());
        data.extend_from_slice(&(record.len() as u32).to_be_bytes());
        data.extend_from_slice(&record);
    }
    if version != SCHEMA_VERSION_V1 {
        receipt_codec::encode(&snapshot.completed_receipts, &mut data)?;
    }
    let total_len = data.len().checked_add(TRAILER_LEN).ok_or(too_long)?;
    if total_len > MAX_SNAPSHOT_BYTES {
        return Err(too_long);
    }
    let payload_len = (data.len() - HEADER_LEN) as u32;
    data[HEADER_LEN - 4..HEADER_LEN].copy_from_slice(&payload_len.to_be_bytes());
    let checksum = crc32(&data);
    data.extend_from_slice(&checksum.to_be_bytes());
    Ok(data)
}

/// Read one slot's octets as a snapshot for `expected`.
///
/// A slot that is damaged, or not a snapshot at all, fails with an error
/// that [is damage](SnapshotError::is_damage): the other slot may still
/// serve. A sound slot of another object, or of a schema version this build
/// doesn't read, fails with one that isn't.
pub(crate) fn decode(
    data: &[u8],
    expected: ObjectIdentifier,
) -> Result<AuditLogSnapshot, SnapshotError> {
    decode_slot(data, expected).map_err(|error| error.in_format(WHAT))
}

fn decode_slot(data: &[u8], expected: ObjectIdentifier) -> Result<AuditLogSnapshot, SnapshotError> {
    verify_integrity(data)?;
    let version = u16::from_be_bytes([data[8], data[9]]);
    if !matches!(
        version,
        SCHEMA_VERSION_V1 | SCHEMA_VERSION_V2 | SCHEMA_VERSION
    ) {
        return Err(SnapshotError::new(
            SnapshotErrorKind::UnknownVersion(version),
            "",
        ));
    }
    let object_identifier = ObjectIdentifier::decode(&data[10..14])
        .map_err(|_| SnapshotError::damaged("object identifier does not decode"))?;
    if object_identifier != expected {
        return Err(SnapshotError::new(
            SnapshotErrorKind::OtherObject,
            "object identity does not match",
        ));
    }
    decode_payload(data, object_identifier, version)
}

pub(crate) fn verify_integrity(data: &[u8]) -> Result<(), SnapshotError> {
    if data.len() > MAX_SNAPSHOT_BYTES {
        return Err(SnapshotError::new(
            SnapshotErrorKind::TooLarge,
            "length exceeds the slot cap",
        ));
    }
    if data.len() < HEADER_LEN + TRAILER_LEN {
        return Err(SnapshotError::truncated("is truncated"));
    }
    if &data[..8] != MAGIC {
        return Err(SnapshotError::new(
            SnapshotErrorKind::NotThisFormat,
            "magic is invalid",
        ));
    }
    // In 64 bits: an erased length field (all ones) would overflow a 32-bit
    // usize.
    let payload_len = u64::from(u32::from_be_bytes(data[22..26].try_into().unwrap()));
    let declared = (HEADER_LEN + TRAILER_LEN) as u64 + payload_len;
    if declared > data.len() as u64 {
        return Err(SnapshotError::truncated(
            "declared length does not match file length",
        ));
    }
    if declared != data.len() as u64 {
        return Err(SnapshotError::damaged(
            "declared length does not match file length",
        ));
    }
    let checksum_start = data.len() - TRAILER_LEN;
    let stored_checksum = u32::from_be_bytes(data[checksum_start..].try_into().unwrap());
    if crc32(&data[..checksum_start]) != stored_checksum {
        return Err(SnapshotError::damaged("checksum is invalid"));
    }
    Ok(())
}

fn decode_payload(
    data: &[u8],
    object_identifier: ObjectIdentifier,
    version: u16,
) -> Result<AuditLogSnapshot, SnapshotError> {
    let generation = u64::from_be_bytes(data[14..22].try_into().unwrap());
    if generation == 0 {
        return Err(SnapshotError::damaged("generation must be nonzero"));
    }
    let checksum_start = data.len() - TRAILER_LEN;
    let payload = &data[HEADER_LEN..checksum_start];
    let mut offset = 0;
    let capacity = take_u32(payload, &mut offset, "capacity")?;
    if capacity > MAX_AUDIT_RECORDS {
        return Err(SnapshotError::damaged("persisted capacity exceeds the cap"));
    }
    let log_enable = match take(payload, &mut offset, 1, "log-enable")?[0] {
        0 => false,
        1 => true,
        _ => {
            return Err(SnapshotError::damaged(
                "persisted log-enable is not Boolean",
            ));
        }
    };
    let total_record_count = take_u64(payload, &mut offset, "total-record-count")?;
    let record_count = take_u32(payload, &mut offset, "record-count")?;
    if record_count > capacity {
        return Err(SnapshotError::damaged(
            "persisted record count exceeds its capacity",
        ));
    }
    // Each record takes at least its sequence number and length: reserve no
    // more than the octets left could hold.
    let room = (payload.len() - offset) / (8 + 4);
    let mut records = Vec::with_capacity((record_count as usize).min(room));
    for _ in 0..record_count {
        let sequence_number = take_u64(payload, &mut offset, "sequence-number")?;
        let record_len = take_u32(payload, &mut offset, "record-length")? as usize;
        if record_len > MAX_RECORD_BYTES {
            return Err(SnapshotError::damaged(
                "persisted record length exceeds the record cap",
            ));
        }
        let record_data = take(payload, &mut offset, record_len, "record")?;
        let mut record = decode_audit_log_record(record_data)
            .map_err(|_| SnapshotError::damaged("record does not decode"))?;
        if version != SCHEMA_VERSION {
            restore_pre_v3_log_status(&mut record);
        }
        records.push(BACnetAuditLogRecordResult {
            sequence_number,
            record,
        });
    }
    let completed_receipts = if version == SCHEMA_VERSION_V1 {
        Vec::new()
    } else {
        receipt_codec::decode(payload, &mut offset)?
    };
    if offset != payload.len() {
        return Err(SnapshotError::damaged("payload has trailing bytes"));
    }

    let mut snapshot = AuditLogSnapshot {
        object_identifier,
        generation,
        capacity,
        log_enable,
        total_record_count,
        records,
        completed_receipts,
    };
    // A record that decodes but would not encode again could never be
    // served, so such a snapshot is damaged. The exception is a record an
    // older build saved nested deeper than an AuditLogQuery-ACK carries
    // (#2483): every field of it is sound, so it is dropped and the rest
    // restores. Only a snapshot that fails its check pays for the search.
    if check(&snapshot).is_err() {
        drop_too_deep(&mut snapshot);
    }
    check(&snapshot).map_err(SnapshotError::damaging)?;
    Ok(snapshot)
}

/// Drop the records of `snapshot` that fail only the whole-record depth
/// check, logging a warning when there are any. The sequence numbers stay
/// contiguous up to Total_Record_Count: each record older than a dropped
/// one takes the next number up, as the other logs' records do. A record
/// that fails for any other reason stays, for the snapshot check to refuse.
pub(super) fn drop_too_deep(snapshot: &mut AuditLogSnapshot) {
    let mut scratch = record_buffer(snapshot.records.len());
    let mut dropped = 0u64;
    let mut kept = Vec::with_capacity(snapshot.records.len());
    // Newest first, so each kept record knows how many newer ones went.
    for mut result in core::mem::take(&mut snapshot.records).into_iter().rev() {
        scratch.clear();
        if check_record_dynamic_size(&result.record).is_some()
            && encode_audit_log_record(&result.record, &mut scratch)
                .is_err_and(|error| is_record_too_deep(&error))
        {
            dropped += 1;
            continue;
        }
        result.sequence_number = advance(result.sequence_number, dropped);
        kept.push(result);
    }
    kept.reverse();
    snapshot.records = kept;
    if dropped != 0 {
        log_warn!(
            audit_log = %snapshot.object_identifier,
            dropped,
            "Dropping saved audit records nested too deep for an AuditLogQuery-ACK to carry"
        );
    }
}

/// `sequence_number` moved `by` places on, wrapping from the largest
/// Unsigned64 to 1 as Total_Record_Count does. Zero, which no record may
/// hold, stays zero for the snapshot check to refuse.
fn advance(sequence_number: u64, by: u64) -> u64 {
    let Some(from_zero) = sequence_number.checked_sub(1) else {
        return 0;
    };
    let moved = (u128::from(from_zero) + u128::from(by)) % u128::from(u64::MAX);
    // Below u64::MAX by the modulus, so the cast keeps it.
    moved as u64 + 1
}

/// Recover the log status a v1 or v2 snapshot meant to store.
///
/// Up to 0.11.0 the encoder shifted the three status bits up by five, so
/// log-disabled (bit 0) landed on the `0x20` bit that belongs to
/// log-interrupted, and log-interrupted on `0x80`. Today's decoder reads
/// that octet bit0-first, which yields the intended three bits in reverse
/// order: reversing them once more restores the record. Other records are
/// left as they are.
fn restore_pre_v3_log_status(record: &mut BACnetAuditLogRecord) {
    if let BACnetAuditLogDatum::LogStatus(status) = &mut record.datum {
        *status = LogStatus::from_bits_truncate(status.bits().reverse_bits() >> 5);
    }
}

/// Check what any snapshot must hold, whether it is about to be written or
/// was just read: a nonzero generation, the capacity and record count
/// within their caps, valid receipts, contiguous nonzero sequence numbers
/// ending at Total_Record_Count, and a size that fits a slot. A cap or size
/// is [`OutOfRange`](SnapshotErrorKind::OutOfRange) and an inconsistency
/// [`Damaged`](SnapshotErrorKind::Damaged), as the log has always reported
/// them; the codec relabels both for its direction.
pub(crate) fn check(snapshot: &AuditLogSnapshot) -> Result<(), SnapshotError> {
    let too_long = SnapshotError::out_of_range("length exceeds the slot cap");
    if snapshot.generation == 0 {
        return Err(SnapshotError::out_of_range("generation must be nonzero"));
    }
    if snapshot.capacity > MAX_AUDIT_RECORDS {
        return Err(SnapshotError::out_of_range("capacity exceeds the cap"));
    }
    if snapshot.records.len() > snapshot.capacity as usize {
        return Err(SnapshotError::out_of_range(
            "record count exceeds its capacity",
        ));
    }
    receipt::validate_receipts(&snapshot.completed_receipts).map_err(|error| match error {
        Error::OutOfRange(_) => {
            SnapshotError::out_of_range("holds completed receipts past their caps")
        }
        _ => SnapshotError::damaged("holds a completed receipt key that is empty or repeated"),
    })?;
    // A counted log may hold no records: one grown from Buffer_Size 0 keeps
    // its count with an empty ring (#1238).
    if (snapshot.total_record_count == 0 && !snapshot.records.is_empty())
        || snapshot
            .records
            .last()
            .is_some_and(|last| last.sequence_number != snapshot.total_record_count)
    {
        return Err(SnapshotError::damaged(
            "record identities do not match Total_Record_Count",
        ));
    }
    for pair in snapshot.records.windows(2) {
        let expected = if pair[0].sequence_number == u64::MAX {
            1
        } else {
            pair[0].sequence_number + 1
        };
        if pair[0].sequence_number == 0 || pair[1].sequence_number != expected {
            return Err(SnapshotError::damaged(
                "sequence identities are not contiguous",
            ));
        }
    }
    if snapshot
        .records
        .first()
        .is_some_and(|first| first.sequence_number == 0)
    {
        return Err(SnapshotError::damaged(
            "sequence identities must be nonzero",
        ));
    }

    let mut projected_len = HEADER_LEN + 4 + 1 + 8 + 4 + TRAILER_LEN;
    let mut scratch = record_buffer(snapshot.records.len());
    for result in &snapshot.records {
        let encoded_len = record_len(&result.record, &mut scratch)?;
        projected_len = projected_len
            .checked_add(8 + 4 + encoded_len)
            .ok_or(too_long)?;
        if projected_len > MAX_SNAPSHOT_BYTES {
            return Err(too_long);
        }
    }
    // An empty receipt list contributes no projected bytes here so every valid
    // schema-v1 snapshot remains loadable. The v2 encoder's final concrete
    // length check still includes its four-byte zero count.
    let receipt_len = if snapshot.completed_receipts.is_empty() {
        0
    } else {
        receipt_codec::checked_len(&snapshot.completed_receipts)?
    };
    projected_len = projected_len.checked_add(receipt_len).ok_or(too_long)?;
    if projected_len > MAX_SNAPSHOT_BYTES {
        return Err(too_long);
    }
    Ok(())
}

/// A buffer to encode `count` records in one at a time: room for a typical
/// one from the start, and no allocation at all for none (#2233).
fn record_buffer(count: usize) -> BytesMut {
    BytesMut::with_capacity(if count == 0 { 0 } else { 128 })
}

/// The encoded length of `record`, encoding it into `scratch`, which it
/// clears first, so a caller checking many records reuses one buffer
/// (#2233). Refuses ([`OutOfRange`](SnapshotErrorKind::OutOfRange)) a
/// record past the record cap or one that doesn't encode.
pub(crate) fn record_len(
    record: &BACnetAuditLogRecord,
    scratch: &mut BytesMut,
) -> Result<usize, SnapshotError> {
    let too_long = SnapshotError::out_of_range("record length exceeds the record cap");
    check_record_dynamic_size(record).ok_or(too_long)?;
    scratch.clear();
    encode_audit_log_record(record, scratch)
        .map_err(|_| SnapshotError::out_of_range("record does not encode"))?;
    if scratch.len() > MAX_RECORD_BYTES {
        return Err(too_long);
    }
    Ok(scratch.len())
}

/// `Some` when the variable-length parts of `record` fit the record cap
/// together, before anything is encoded.
fn check_record_dynamic_size(record: &BACnetAuditLogRecord) -> Option<()> {
    let notification = match &record.datum {
        BACnetAuditLogDatum::AuditNotification(notification) => notification,
        // A datum kept as received (#2483) is only its octets.
        BACnetAuditLogDatum::Opaque(opaque) => {
            return (opaque.data.len() <= MAX_RECORD_BYTES).then_some(());
        }
        BACnetAuditLogDatum::LogStatus(_) | BACnetAuditLogDatum::TimeChange(_) => {
            return Some(());
        }
    };
    let recipient_size = |recipient: &BACnetRecipient| match recipient {
        BACnetRecipient::Device(_) => 4,
        BACnetRecipient::Address(address) => address.mac_address.len(),
    };
    let lengths = [
        recipient_size(&notification.source_device),
        recipient_size(&notification.target_device),
        notification.source_comment.as_ref().map_or(0, String::len),
        notification.target_comment.as_ref().map_or(0, String::len),
        notification.target_value.as_ref().map_or(0, Vec::len),
        notification.current_value.as_ref().map_or(0, Vec::len),
    ];
    let dynamic = lengths
        .into_iter()
        .try_fold(0usize, |sum, length| sum.checked_add(length))?;
    (dynamic <= MAX_RECORD_BYTES).then_some(())
}

fn take<'a>(
    data: &'a [u8],
    offset: &mut usize,
    length: usize,
    field: &'static str,
) -> Result<&'a [u8], SnapshotError> {
    let end = offset
        .checked_add(length)
        .filter(|end| *end <= data.len())
        .ok_or(SnapshotError::truncated("is truncated").in_field(field))?;
    let value = &data[*offset..end];
    *offset = end;
    Ok(value)
}

fn take_u32(data: &[u8], offset: &mut usize, field: &'static str) -> Result<u32, SnapshotError> {
    Ok(u32::from_be_bytes(
        take(data, offset, 4, field)?.try_into().unwrap(),
    ))
}

fn take_u64(data: &[u8], offset: &mut usize, field: &'static str) -> Result<u64, SnapshotError> {
    Ok(u64::from_be_bytes(
        take(data, offset, 8, field)?.try_into().unwrap(),
    ))
}
