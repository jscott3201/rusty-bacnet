//! Explicit persistence for one AuditLog object. The object calls it from its
//! writer thread, never under the database guard ([`crate::durable`]).

// Without std there is no file backend, so the snapshot format goes unused.
#![cfg_attr(not(feature = "std"), allow(dead_code))]

use crate::prelude::*;
#[cfg(feature = "std")]
use std::fs::{File, OpenOptions};
#[cfg(feature = "std")]
use std::io::{ErrorKind, Read, Write};
#[cfg(feature = "std")]
use std::path::{Path, PathBuf};

use bacnet_encoding::constructed::{decode_audit_log_record, encode_audit_log_record};
use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::{
    BACnetAuditLogDatum, BACnetAuditLogRecord, BACnetAuditLogRecordResult,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use crate::durable::crc32;

use super::receipt::{self, CompletedAuditReceipt};

mod receipt_codec;

/// Maximum configured AuditLog capacity accepted by the repository.
pub const MAX_AUDIT_RECORDS: u32 = 10_000;

const MAGIC: &[u8; 8] = b"RBALOG01";
const SCHEMA_VERSION_V1: u16 = 1;
/// Adds completed receipts. Like v1, it holds log-status records in the
/// reversed bit order of releases up to 0.11.0.
const SCHEMA_VERSION_V2: u16 = 2;
/// Holds log-status records in the bit0-first order of every BACnet bit
/// string.
const SCHEMA_VERSION: u16 = 3;
const HEADER_LEN: usize = 8 + 2 + 4 + 8 + 4;
const TRAILER_LEN: usize = 4;
const MAX_RECORD_BYTES: usize = 1024 * 1024;
const MAX_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;

/// Complete durable state for exactly one AuditLog object.
#[derive(Clone, Debug, PartialEq)]
pub struct AuditLogSnapshot {
    /// AuditLog identity that owns this state.
    pub object_identifier: ObjectIdentifier,
    /// Nonzero two-slot snapshot generation.
    pub generation: u64,
    /// Persisted ring-buffer capacity: Buffer_Size, as configured or last
    /// written.
    pub capacity: u32,
    /// Persisted Enable policy.
    pub log_enable: bool,
    /// Persisted Total_Record_Count and next sequence source.
    pub total_record_count: u64,
    /// Oldest-to-newest retained typed records.
    pub records: Vec<BACnetAuditLogRecordResult>,
    /// Bounded completed confirmed-request receipts owned by this Audit Log.
    pub completed_receipts: Vec<CompletedAuditReceipt>,
}

/// Application-owned persistence port for one AuditLog object.
///
/// The object calls it from its writer thread, one call at a time. A commit
/// lands in storage a moment before the log serves it: see [storage leads
/// the served state](crate::durable#storage-leads-the-served-state).
///
/// An implementation must not own the object it serves, or the database
/// holding it: the object's writer thread calls it, and dropping the object
/// there would wait for that same thread to end, forever.
pub trait AuditLogPersistence: Send + Sync {
    /// Load the newest valid compatible snapshot, or `None` if neither slot exists.
    fn load(&self, expected_object: ObjectIdentifier) -> Result<Option<AuditLogSnapshot>, Error>;

    /// Durably replace one slot with a complete prospective snapshot.
    fn commit(&self, snapshot: &AuditLogSnapshot) -> Result<(), Error>;
}

/// Two-slot, versioned and checksummed file snapshot backend.
///
/// Each commit fully writes and synchronizes one slot, and the first commit to
/// a slot also synchronizes its directory (on Unix; see
/// [`durable`](crate::durable)). This protects the previous valid slot from a
/// failed commit, but does not provide multi-process coordination or stronger
/// portable power-loss guarantees than `sync_all`.
#[derive(Clone, Debug)]
#[cfg(feature = "std")]
pub struct FileAuditLogPersistence {
    slot_paths: [PathBuf; 2],
    /// Test support: keep the slots here instead of at `slot_paths`.
    #[cfg(feature = "test-support")]
    memory: Option<crate::durable::MemoryFiles>,
}

#[cfg(feature = "std")]
impl FileAuditLogPersistence {
    /// Use `base_path.slot0` and `base_path.slot1` for this object's snapshots.
    pub fn new(base_path: impl AsRef<Path>) -> Result<Self, Error> {
        let base_path = base_path.as_ref();
        if base_path.as_os_str().is_empty() {
            return Err(Error::OutOfRange(
                "AuditLog persistence base path must not be empty".into(),
            ));
        }
        let base = base_path.as_os_str();
        let mut slot0 = base.to_os_string();
        slot0.push(".slot0");
        let mut slot1 = base.to_os_string();
        slot1.push(".slot1");
        Ok(Self {
            slot_paths: [PathBuf::from(slot0), PathBuf::from(slot1)],
            #[cfg(feature = "test-support")]
            memory: None,
        })
    }

    /// Test support, not a stable API: keep the slots in `memory`'s
    /// entries 0 and 1 instead of on disk, decoding, selecting and encoding
    /// them as on disk (#2233). [`slot_paths`](Self::slot_paths) are then
    /// empty placeholders, which cost no allocation.
    /// Nothing reaches disk: the state is gone when the process ends, so
    /// never use this outside tests.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn in_memory(memory: crate::durable::MemoryFiles) -> Self {
        Self {
            slot_paths: [PathBuf::new(), PathBuf::new()],
            memory: Some(memory),
        }
    }

    /// Slot `slot`'s octets, or `None` when it doesn't exist.
    fn read_slot(&self, slot: usize) -> Result<Option<Vec<u8>>, SlotReadFailure> {
        #[cfg(feature = "test-support")]
        if let Some(memory) = &self.memory {
            let data = memory.read(slot, MAX_SNAPSHOT_BYTES + 1);
            if data
                .as_ref()
                .is_some_and(|data| data.len() > MAX_SNAPSHOT_BYTES)
            {
                return Err(SlotReadFailure::Recoverable(Error::OutOfRange(format!(
                    "AuditLog snapshot length exceeds {MAX_SNAPSHOT_BYTES}"
                ))));
            }
            return Ok(data);
        }
        read_slot_file(&self.slot_paths[slot])
    }

    /// Concrete slot paths, exposed for application backup and diagnostics.
    pub fn slot_paths(&self) -> [PathBuf; 2] {
        self.slot_paths.clone()
    }
}

#[cfg(feature = "std")]
enum SlotState {
    Missing,
    Recoverable(Error),
    Fatal(Error),
    Valid(AuditLogSnapshot),
}

#[cfg(feature = "std")]
enum SlotReadFailure {
    Recoverable(Error),
    Fatal(Error),
}

#[cfg(feature = "std")]
impl AuditLogPersistence for FileAuditLogPersistence {
    fn load(&self, expected_object: ObjectIdentifier) -> Result<Option<AuditLogSnapshot>, Error> {
        let slots = [0, 1].map(|slot| slot_state(self.read_slot(slot), expected_object));
        select_newest_slot(slots)
    }

    fn commit(&self, snapshot: &AuditLogSnapshot) -> Result<(), Error> {
        let bytes = encode_snapshot(snapshot)?;
        let slot = (snapshot.generation % 2) as usize;
        #[cfg(feature = "test-support")]
        if let Some(memory) = &self.memory {
            memory.write(slot, bytes);
            return Ok(());
        }
        let path = &self.slot_paths[slot];
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }

        let (mut file, created) = match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(file) => (file, true),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => (
                OpenOptions::new().write(true).truncate(true).open(path)?,
                false,
            ),
            Err(error) => return Err(error.into()),
        };
        if let Err(error) = file.write_all(&bytes) {
            invalidate_failed_slot(&file);
            return Err(error.into());
        }
        if let Err(error) = file.sync_all() {
            invalidate_failed_slot(&file);
            return Err(error.into());
        }
        // A slot this commit created exists only once its directory entry is
        // durable too. The commit has landed by now, so a sync that fails is
        // logged rather than failing it.
        if created {
            crate::durable::sync_parent_dir(path);
        }
        Ok(())
    }
}

#[cfg(feature = "std")]
fn select_newest_slot(slots: [SlotState; 2]) -> Result<Option<AuditLogSnapshot>, Error> {
    let mut newest: Option<AuditLogSnapshot> = None;
    let mut present = false;
    let mut last_error = None;

    for slot in slots {
        match slot {
            SlotState::Missing => {}
            SlotState::Recoverable(error) => {
                present = true;
                last_error = Some(error);
            }
            SlotState::Fatal(error) => return Err(error),
            SlotState::Valid(snapshot) => {
                present = true;
                match newest.as_ref() {
                    None => newest = Some(snapshot),
                    Some(current) if snapshot.generation > current.generation => {
                        newest = Some(snapshot);
                    }
                    Some(current) if snapshot.generation == current.generation => {
                        if encode_snapshot(&snapshot)? != encode_snapshot(current)? {
                            return Err(Error::Encoding(
                                "AuditLog slots contain divergent snapshots at the same generation"
                                    .into(),
                            ));
                        }
                    }
                    Some(_) => {}
                }
            }
        }
    }

    if newest.is_some() {
        Ok(newest)
    } else if present {
        Err(last_error.unwrap_or_else(|| {
            Error::Encoding("AuditLog persistence has no valid compatible slot".into())
        }))
    } else {
        Ok(None)
    }
}

#[cfg(feature = "std")]
fn invalidate_failed_slot(file: &File) {
    // Best effort only: the other slot remains the last known valid state.
    // No stronger portable power-loss behavior is promised after an I/O error.
    let _ = file.set_len(0);
    let _ = file.sync_all();
}

/// What a slot read as `read` holds for `expected_object`.
#[cfg(feature = "std")]
fn slot_state(
    read: Result<Option<Vec<u8>>, SlotReadFailure>,
    expected_object: ObjectIdentifier,
) -> SlotState {
    match read.and_then(|data| {
        data.map(|data| decode_slot(&data, expected_object))
            .transpose()
    }) {
        Ok(Some(snapshot)) => SlotState::Valid(snapshot),
        Ok(None) => SlotState::Missing,
        Err(SlotReadFailure::Recoverable(error)) => SlotState::Recoverable(error),
        Err(SlotReadFailure::Fatal(error)) => SlotState::Fatal(error),
    }
}

/// The octets of the slot file at `path`, or `None` when it doesn't exist.
#[cfg(feature = "std")]
fn read_slot_file(path: &Path) -> Result<Option<Vec<u8>>, SlotReadFailure> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(SlotReadFailure::Fatal(error.into())),
    };
    let metadata = file
        .metadata()
        .map_err(|error| SlotReadFailure::Fatal(error.into()))?;
    let length = usize::try_from(metadata.len()).map_err(|_| {
        SlotReadFailure::Recoverable(Error::OutOfRange(
            "AuditLog snapshot length exceeds usize".into(),
        ))
    })?;
    if length > MAX_SNAPSHOT_BYTES {
        return Err(SlotReadFailure::Recoverable(Error::OutOfRange(format!(
            "AuditLog snapshot length {length} exceeds {MAX_SNAPSHOT_BYTES}"
        ))));
    }
    let mut data = Vec::with_capacity(length.min(MAX_SNAPSHOT_BYTES + 1));
    file.take((MAX_SNAPSHOT_BYTES + 1) as u64)
        .read_to_end(&mut data)
        .map_err(|error| SlotReadFailure::Fatal(error.into()))?;
    if data.len() > MAX_SNAPSHOT_BYTES {
        return Err(SlotReadFailure::Recoverable(Error::OutOfRange(format!(
            "AuditLog snapshot length exceeds {MAX_SNAPSHOT_BYTES}"
        ))));
    }
    if data.len() != length {
        return Err(SlotReadFailure::Fatal(Error::Encoding(
            "AuditLog snapshot changed while being read".into(),
        )));
    }
    Ok(Some(data))
}

/// One slot's octets as a snapshot for `expected_object`.
#[cfg(feature = "std")]
fn decode_slot(
    data: &[u8],
    expected_object: ObjectIdentifier,
) -> Result<AuditLogSnapshot, SlotReadFailure> {
    verify_snapshot_integrity(data).map_err(SlotReadFailure::Recoverable)?;
    let version = u16::from_be_bytes(data[8..10].try_into().unwrap());
    if !matches!(
        version,
        SCHEMA_VERSION_V1 | SCHEMA_VERSION_V2 | SCHEMA_VERSION
    ) {
        return Err(SlotReadFailure::Fatal(Error::Encoding(format!(
            "AuditLog snapshot schema version {version} is unsupported"
        ))));
    }
    let object_identifier =
        ObjectIdentifier::decode(&data[10..14]).map_err(SlotReadFailure::Recoverable)?;
    if object_identifier != expected_object {
        return Err(SlotReadFailure::Fatal(Error::Encoding(
            "AuditLog snapshot object identity does not match".into(),
        )));
    }
    decode_verified_snapshot(data, object_identifier, version).map_err(SlotReadFailure::Recoverable)
}

fn verify_snapshot_integrity(data: &[u8]) -> Result<(), Error> {
    if data.len() > MAX_SNAPSHOT_BYTES {
        return Err(Error::OutOfRange(format!(
            "AuditLog snapshot length {} exceeds {MAX_SNAPSHOT_BYTES}",
            data.len()
        )));
    }
    if data.len() < HEADER_LEN + TRAILER_LEN {
        return Err(Error::Encoding("AuditLog snapshot is truncated".into()));
    }
    if &data[..8] != MAGIC {
        return Err(Error::Encoding("AuditLog snapshot magic is invalid".into()));
    }
    let payload_len = u32::from_be_bytes(data[22..26].try_into().unwrap()) as usize;
    let expected_len = HEADER_LEN
        .checked_add(payload_len)
        .and_then(|length| length.checked_add(TRAILER_LEN))
        .ok_or_else(|| Error::OutOfRange("AuditLog snapshot length overflow".into()))?;
    if expected_len != data.len() {
        return Err(Error::Encoding(
            "AuditLog snapshot declared length does not match file length".into(),
        ));
    }
    let checksum_start = data.len() - TRAILER_LEN;
    let stored_checksum = u32::from_be_bytes(data[checksum_start..].try_into().unwrap());
    if crc32(&data[..checksum_start]) != stored_checksum {
        return Err(Error::Encoding(
            "AuditLog snapshot checksum is invalid".into(),
        ));
    }
    Ok(())
}

fn encode_snapshot(snapshot: &AuditLogSnapshot) -> Result<Vec<u8>, Error> {
    encode_snapshot_for_version(snapshot, SCHEMA_VERSION)
}

#[cfg(all(test, feature = "std"))]
pub(super) fn encode_snapshot_v1(snapshot: &AuditLogSnapshot) -> Result<Vec<u8>, Error> {
    if !snapshot.completed_receipts.is_empty() {
        return Err(Error::Encoding(
            "AuditLog schema-v1 snapshot cannot contain completed receipts".into(),
        ));
    }
    encode_snapshot_for_version(snapshot, SCHEMA_VERSION_V1)
}

/// Encode `snapshot` under the schema-v2 header. The records are encoded as
/// today's codec does, so a test reproduces a 0.11.0 file by handing it the
/// log statuses that encode to the old octets.
#[cfg(all(test, feature = "std"))]
pub(super) fn encode_snapshot_v2(snapshot: &AuditLogSnapshot) -> Result<Vec<u8>, Error> {
    encode_snapshot_for_version(snapshot, SCHEMA_VERSION_V2)
}

fn encode_snapshot_for_version(
    snapshot: &AuditLogSnapshot,
    version: u16,
) -> Result<Vec<u8>, Error> {
    validate_snapshot(snapshot)?;
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
        validate_record_dynamic_size(&result.record)?;
        record.clear();
        encode_audit_log_record(&result.record, &mut record)?;
        if record.len() > MAX_RECORD_BYTES {
            return Err(Error::OutOfRange(format!(
                "AuditLog record length {} exceeds {MAX_RECORD_BYTES}",
                record.len()
            )));
        }
        let projected = data
            .len()
            .checked_add(8 + 4 + record.len())
            .and_then(|length| length.checked_add(TRAILER_LEN))
            .ok_or_else(|| Error::OutOfRange("AuditLog snapshot length overflow".into()))?;
        if projected > MAX_SNAPSHOT_BYTES {
            return Err(Error::OutOfRange(format!(
                "AuditLog snapshot length exceeds {MAX_SNAPSHOT_BYTES}"
            )));
        }
        data.extend_from_slice(&result.sequence_number.to_be_bytes());
        data.extend_from_slice(&(record.len() as u32).to_be_bytes());
        data.extend_from_slice(&record);
    }
    if version != SCHEMA_VERSION_V1 {
        receipt_codec::encode(&snapshot.completed_receipts, &mut data)?;
    }
    let total_len = data
        .len()
        .checked_add(TRAILER_LEN)
        .ok_or_else(|| Error::OutOfRange("AuditLog snapshot length overflow".into()))?;
    if total_len > MAX_SNAPSHOT_BYTES {
        return Err(Error::OutOfRange(format!(
            "AuditLog snapshot length {total_len} exceeds {MAX_SNAPSHOT_BYTES}"
        )));
    }
    let payload_len = (data.len() - HEADER_LEN) as u32;
    data[HEADER_LEN - 4..HEADER_LEN].copy_from_slice(&payload_len.to_be_bytes());
    let checksum = crc32(&data);
    data.extend_from_slice(&checksum.to_be_bytes());
    Ok(data)
}

fn decode_verified_snapshot(
    data: &[u8],
    object_identifier: ObjectIdentifier,
    version: u16,
) -> Result<AuditLogSnapshot, Error> {
    let generation = u64::from_be_bytes(data[14..22].try_into().unwrap());
    if generation == 0 {
        return Err(Error::Encoding(
            "AuditLog snapshot generation must be nonzero".into(),
        ));
    }
    let checksum_start = data.len() - TRAILER_LEN;
    let payload = &data[HEADER_LEN..checksum_start];
    let mut offset = 0;
    let capacity = take_u32(payload, &mut offset, "capacity")?;
    if capacity > MAX_AUDIT_RECORDS {
        return Err(Error::OutOfRange(format!(
            "AuditLog persisted capacity {capacity} exceeds {MAX_AUDIT_RECORDS}"
        )));
    }
    let log_enable = match take(payload, &mut offset, 1, "log-enable")?[0] {
        0 => false,
        1 => true,
        _ => {
            return Err(Error::Encoding(
                "AuditLog persisted log-enable is not Boolean".into(),
            ));
        }
    };
    let total_record_count = take_u64(payload, &mut offset, "total-record-count")?;
    let record_count = take_u32(payload, &mut offset, "record-count")?;
    if record_count > capacity || record_count > MAX_AUDIT_RECORDS {
        return Err(Error::OutOfRange(format!(
            "AuditLog persisted record count {record_count} exceeds capacity {capacity}"
        )));
    }
    let mut records = Vec::with_capacity(record_count as usize);
    for _ in 0..record_count {
        let sequence_number = take_u64(payload, &mut offset, "sequence-number")?;
        let record_len = take_u32(payload, &mut offset, "record-length")? as usize;
        if record_len > MAX_RECORD_BYTES {
            return Err(Error::OutOfRange(format!(
                "AuditLog persisted record length {record_len} exceeds {MAX_RECORD_BYTES}"
            )));
        }
        let record_data = take(payload, &mut offset, record_len, "record")?;
        let mut record = decode_audit_log_record(record_data)?;
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
        return Err(Error::Encoding(
            "AuditLog snapshot payload has trailing bytes".into(),
        ));
    }

    let snapshot = AuditLogSnapshot {
        object_identifier,
        generation,
        capacity,
        log_enable,
        total_record_count,
        records,
        completed_receipts,
    };
    validate_snapshot(&snapshot)?;
    Ok(snapshot)
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

pub(super) fn validate_snapshot(snapshot: &AuditLogSnapshot) -> Result<(), Error> {
    if snapshot.generation == 0 {
        return Err(Error::OutOfRange(
            "AuditLog snapshot generation must be nonzero".into(),
        ));
    }
    if snapshot.capacity > MAX_AUDIT_RECORDS {
        return Err(Error::OutOfRange(format!(
            "AuditLog capacity {} exceeds {MAX_AUDIT_RECORDS}",
            snapshot.capacity
        )));
    }
    if snapshot.records.len() > snapshot.capacity as usize {
        return Err(Error::OutOfRange(format!(
            "AuditLog record count {} exceeds capacity {}",
            snapshot.records.len(),
            snapshot.capacity
        )));
    }
    receipt::validate_receipts(&snapshot.completed_receipts)?;
    // A counted log may hold no records: one grown from Buffer_Size 0 keeps
    // its count with an empty ring (#1238).
    if (snapshot.total_record_count == 0 && !snapshot.records.is_empty())
        || snapshot
            .records
            .last()
            .is_some_and(|last| last.sequence_number != snapshot.total_record_count)
    {
        return Err(Error::Encoding(
            "AuditLog snapshot record identities do not match Total_Record_Count".into(),
        ));
    }
    for pair in snapshot.records.windows(2) {
        let expected = if pair[0].sequence_number == u64::MAX {
            1
        } else {
            pair[0].sequence_number + 1
        };
        if pair[0].sequence_number == 0 || pair[1].sequence_number != expected {
            return Err(Error::Encoding(
                "AuditLog snapshot sequence identities are not contiguous".into(),
            ));
        }
    }
    if snapshot
        .records
        .first()
        .is_some_and(|first| first.sequence_number == 0)
    {
        return Err(Error::Encoding(
            "AuditLog snapshot sequence identities must be nonzero".into(),
        ));
    }

    let mut projected_len = HEADER_LEN
        .checked_add(4 + 1 + 8 + 4)
        .and_then(|length| length.checked_add(TRAILER_LEN))
        .ok_or_else(|| Error::OutOfRange("AuditLog snapshot length overflow".into()))?;
    let mut scratch = record_buffer(snapshot.records.len());
    for result in &snapshot.records {
        let encoded_len = record_len(&result.record, &mut scratch)?;
        projected_len = projected_len
            .checked_add(8 + 4 + encoded_len)
            .ok_or_else(|| Error::OutOfRange("AuditLog snapshot length overflow".into()))?;
        if projected_len > MAX_SNAPSHOT_BYTES {
            return Err(Error::OutOfRange(format!(
                "AuditLog snapshot length exceeds {MAX_SNAPSHOT_BYTES}"
            )));
        }
    }
    // An empty receipt list contributes no projected bytes here so every valid
    // schema-v1 snapshot remains loadable. The v2 encoder's final concrete
    // length check still includes its four-byte zero count.
    let receipt_len = if snapshot.completed_receipts.is_empty() {
        0
    } else {
        // `validate_receipts` passed them above.
        receipt_codec::checked_len(&snapshot.completed_receipts)?
    };
    projected_len = projected_len
        .checked_add(receipt_len)
        .ok_or_else(|| Error::OutOfRange("AuditLog snapshot length overflow".into()))?;
    if projected_len > MAX_SNAPSHOT_BYTES {
        return Err(Error::OutOfRange(format!(
            "AuditLog snapshot length exceeds {MAX_SNAPSHOT_BYTES}"
        )));
    }
    Ok(())
}

pub(super) fn validate_record(
    record: &bacnet_types::constructed::BACnetAuditLogRecord,
) -> Result<usize, Error> {
    record_len(record, &mut BytesMut::new())
}

/// A buffer to encode `count` records in one at a time: room for a typical
/// one from the start, and no allocation at all for none (#2233).
fn record_buffer(count: usize) -> BytesMut {
    BytesMut::with_capacity(if count == 0 { 0 } else { 128 })
}

/// [`validate_record`], encoding into `scratch`, which it clears first, so
/// a caller checking many records reuses one buffer (#2233).
fn record_len(
    record: &bacnet_types::constructed::BACnetAuditLogRecord,
    scratch: &mut BytesMut,
) -> Result<usize, Error> {
    validate_record_dynamic_size(record)?;
    scratch.clear();
    encode_audit_log_record(record, scratch)?;
    if scratch.len() > MAX_RECORD_BYTES {
        return Err(Error::OutOfRange(format!(
            "AuditLog record length {} exceeds {MAX_RECORD_BYTES}",
            scratch.len()
        )));
    }
    Ok(scratch.len())
}

fn validate_record_dynamic_size(
    record: &bacnet_types::constructed::BACnetAuditLogRecord,
) -> Result<(), Error> {
    let bacnet_types::constructed::BACnetAuditLogDatum::AuditNotification(notification) =
        &record.datum
    else {
        return Ok(());
    };
    let recipient_size = |recipient: &bacnet_types::constructed::BACnetRecipient| match recipient {
        bacnet_types::constructed::BACnetRecipient::Device(_) => 4,
        bacnet_types::constructed::BACnetRecipient::Address(address) => address.mac_address.len(),
    };
    let lengths = [
        recipient_size(&notification.source_device),
        recipient_size(&notification.target_device),
        notification.source_comment.as_ref().map_or(0, String::len),
        notification.target_comment.as_ref().map_or(0, String::len),
        notification.target_value.as_ref().map_or(0, Vec::len),
        notification.current_value.as_ref().map_or(0, Vec::len),
    ];
    let dynamic = lengths.into_iter().try_fold(0usize, |sum, length| {
        sum.checked_add(length)
            .ok_or_else(|| Error::OutOfRange("AuditLog record length overflow".into()))
    })?;
    if dynamic > MAX_RECORD_BYTES {
        return Err(Error::OutOfRange(format!(
            "AuditLog record dynamic length {dynamic} exceeds {MAX_RECORD_BYTES}"
        )));
    }
    Ok(())
}

fn take<'a>(
    data: &'a [u8],
    offset: &mut usize,
    length: usize,
    field: &str,
) -> Result<&'a [u8], Error> {
    let end = offset
        .checked_add(length)
        .ok_or_else(|| Error::OutOfRange(format!("AuditLog {field} length overflow")))?;
    if end > data.len() {
        return Err(Error::Encoding(format!(
            "AuditLog snapshot {field} is truncated"
        )));
    }
    let value = &data[*offset..end];
    *offset = end;
    Ok(value)
}

fn take_u32(data: &[u8], offset: &mut usize, field: &str) -> Result<u32, Error> {
    Ok(u32::from_be_bytes(
        take(data, offset, 4, field)?.try_into().unwrap(),
    ))
}

fn take_u64(data: &[u8], offset: &mut usize, field: &str) -> Result<u64, Error> {
    Ok(u64::from_be_bytes(
        take(data, offset, 8, field)?.try_into().unwrap(),
    ))
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use bacnet_types::enums::ObjectType;

    fn snapshot() -> AuditLogSnapshot {
        AuditLogSnapshot {
            object_identifier: ObjectIdentifier::new(ObjectType::AUDIT_LOG, 1).unwrap(),
            generation: 1,
            capacity: 1,
            log_enable: true,
            total_record_count: 0,
            records: Vec::new(),
            completed_receipts: Vec::new(),
        }
    }

    fn io_failure() -> SlotState {
        SlotState::Fatal(Error::Transport(std::io::Error::other(
            "injected slot read failure",
        )))
    }

    #[test]
    fn audit_slot_selection_never_masks_io_failure_with_valid_peer() {
        assert!(matches!(
            select_newest_slot([io_failure(), SlotState::Valid(snapshot())]),
            Err(Error::Transport(_))
        ));
        assert!(matches!(
            select_newest_slot([SlotState::Valid(snapshot()), io_failure()]),
            Err(Error::Transport(_))
        ));
    }

    #[test]
    fn audit_slot_selection_requires_identical_same_generation_snapshots() {
        let expected = snapshot();
        let selected = select_newest_slot([
            SlotState::Valid(expected.clone()),
            SlotState::Valid(expected.clone()),
        ])
        .unwrap()
        .unwrap();
        assert_eq!(selected, expected);

        let mut divergent = snapshot();
        divergent.log_enable = false;
        assert!(matches!(
            select_newest_slot([
                SlotState::Valid(snapshot()),
                SlotState::Valid(divergent),
            ]),
            Err(Error::Encoding(message)) if message.contains("same generation")
        ));
    }
}
