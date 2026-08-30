//! Explicit, synchronous persistence for one AuditLog object.

use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

use bacnet_encoding::constructed::{decode_audit_log_record, encode_audit_log_record};
use bacnet_types::constructed::BACnetAuditLogRecordResult;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

/// Maximum configured AuditLog capacity accepted by the repository.
pub const MAX_AUDIT_RECORDS: u32 = 10_000;

const MAGIC: &[u8; 8] = b"RBALOG01";
const SCHEMA_VERSION: u16 = 1;
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
    /// Persisted ring-buffer capacity.
    pub capacity: u32,
    /// Persisted Enable policy.
    pub log_enable: bool,
    /// Persisted Total_Record_Count and next sequence source.
    pub total_record_count: u64,
    /// Oldest-to-newest retained typed records.
    pub records: Vec<BACnetAuditLogRecordResult>,
}

/// Application-owned persistence port for one AuditLog object.
pub trait AuditLogPersistence: Send + Sync {
    /// Load the newest valid compatible snapshot, or `None` if neither slot exists.
    fn load(&self, expected_object: ObjectIdentifier) -> Result<Option<AuditLogSnapshot>, Error>;

    /// Durably replace one slot with a complete prospective snapshot.
    fn commit(&self, snapshot: &AuditLogSnapshot) -> Result<(), Error>;
}

/// Two-slot, versioned and checksummed file snapshot backend.
///
/// Each commit fully writes and synchronizes one slot. This protects the
/// previous valid slot from a failed commit, but does not provide multi-process
/// coordination or stronger portable power-loss guarantees than `sync_all`.
#[derive(Clone, Debug)]
pub struct FileAuditLogPersistence {
    slot_paths: [PathBuf; 2],
}

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
        })
    }

    /// Concrete slot paths, exposed for application backup and diagnostics.
    pub fn slot_paths(&self) -> [PathBuf; 2] {
        self.slot_paths.clone()
    }
}

enum SlotState {
    Missing,
    Recoverable(Error),
    Fatal(Error),
    Valid(AuditLogSnapshot),
}

enum SlotReadFailure {
    Recoverable(Error),
    Fatal(Error),
}

impl AuditLogPersistence for FileAuditLogPersistence {
    fn load(&self, expected_object: ObjectIdentifier) -> Result<Option<AuditLogSnapshot>, Error> {
        let slots = [
            read_slot(&self.slot_paths[0], expected_object),
            read_slot(&self.slot_paths[1], expected_object),
        ];
        select_newest_slot(slots)
    }

    fn commit(&self, snapshot: &AuditLogSnapshot) -> Result<(), Error> {
        let bytes = encode_snapshot(snapshot)?;
        let path = &self.slot_paths[(snapshot.generation % 2) as usize];
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }

        let mut file = match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                OpenOptions::new().write(true).truncate(true).open(path)?
            }
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
        Ok(())
    }
}

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

fn invalidate_failed_slot(file: &File) {
    // Best effort only: the other slot remains the last known valid state.
    // No stronger portable power-loss behavior is promised after an I/O error.
    let _ = file.set_len(0);
    let _ = file.sync_all();
}

fn read_slot(path: &Path, expected_object: ObjectIdentifier) -> SlotState {
    match read_slot_inner(path, expected_object) {
        Ok(Some(snapshot)) => SlotState::Valid(snapshot),
        Ok(None) => SlotState::Missing,
        Err(SlotReadFailure::Recoverable(error)) => SlotState::Recoverable(error),
        Err(SlotReadFailure::Fatal(error)) => SlotState::Fatal(error),
    }
}

fn read_slot_inner(
    path: &Path,
    expected_object: ObjectIdentifier,
) -> Result<Option<AuditLogSnapshot>, SlotReadFailure> {
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

    verify_snapshot_integrity(&data).map_err(SlotReadFailure::Recoverable)?;
    let version = u16::from_be_bytes(data[8..10].try_into().unwrap());
    if version != SCHEMA_VERSION {
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
    decode_verified_snapshot(&data, object_identifier)
        .map(Some)
        .map_err(SlotReadFailure::Recoverable)
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
    validate_snapshot(snapshot)?;
    let mut payload = Vec::new();
    payload.extend_from_slice(&snapshot.capacity.to_be_bytes());
    payload.push(u8::from(snapshot.log_enable));
    payload.extend_from_slice(&snapshot.total_record_count.to_be_bytes());
    payload.extend_from_slice(&(snapshot.records.len() as u32).to_be_bytes());
    for result in &snapshot.records {
        validate_record_dynamic_size(&result.record)?;
        let mut record = BytesMut::new();
        encode_audit_log_record(&result.record, &mut record)?;
        if record.len() > MAX_RECORD_BYTES {
            return Err(Error::OutOfRange(format!(
                "AuditLog record length {} exceeds {MAX_RECORD_BYTES}",
                record.len()
            )));
        }
        let projected = HEADER_LEN
            .checked_add(payload.len())
            .and_then(|length| length.checked_add(8 + 4 + record.len()))
            .and_then(|length| length.checked_add(TRAILER_LEN))
            .ok_or_else(|| Error::OutOfRange("AuditLog snapshot length overflow".into()))?;
        if projected > MAX_SNAPSHOT_BYTES {
            return Err(Error::OutOfRange(format!(
                "AuditLog snapshot length exceeds {MAX_SNAPSHOT_BYTES}"
            )));
        }
        payload.extend_from_slice(&result.sequence_number.to_be_bytes());
        payload.extend_from_slice(&(record.len() as u32).to_be_bytes());
        payload.extend_from_slice(&record);
    }
    let total_len = HEADER_LEN
        .checked_add(payload.len())
        .and_then(|length| length.checked_add(TRAILER_LEN))
        .ok_or_else(|| Error::OutOfRange("AuditLog snapshot length overflow".into()))?;
    if total_len > MAX_SNAPSHOT_BYTES {
        return Err(Error::OutOfRange(format!(
            "AuditLog snapshot length {total_len} exceeds {MAX_SNAPSHOT_BYTES}"
        )));
    }

    let mut data = Vec::with_capacity(total_len);
    data.extend_from_slice(MAGIC);
    data.extend_from_slice(&SCHEMA_VERSION.to_be_bytes());
    data.extend_from_slice(&snapshot.object_identifier.encode());
    data.extend_from_slice(&snapshot.generation.to_be_bytes());
    data.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    data.extend_from_slice(&payload);
    let checksum = crc32(&data);
    data.extend_from_slice(&checksum.to_be_bytes());
    Ok(data)
}

fn decode_verified_snapshot(
    data: &[u8],
    object_identifier: ObjectIdentifier,
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
        records.push(BACnetAuditLogRecordResult {
            sequence_number,
            record: decode_audit_log_record(record_data)?,
        });
    }
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
    };
    validate_snapshot(&snapshot)?;
    Ok(snapshot)
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
    if (snapshot.total_record_count == 0 && !snapshot.records.is_empty())
        || (snapshot.total_record_count != 0
            && snapshot.capacity != 0
            && snapshot.records.is_empty())
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
    for result in &snapshot.records {
        let encoded_len = validate_record(&result.record)?;
        projected_len = projected_len
            .checked_add(8 + 4 + encoded_len)
            .ok_or_else(|| Error::OutOfRange("AuditLog snapshot length overflow".into()))?;
        if projected_len > MAX_SNAPSHOT_BYTES {
            return Err(Error::OutOfRange(format!(
                "AuditLog snapshot length exceeds {MAX_SNAPSHOT_BYTES}"
            )));
        }
    }
    Ok(())
}

pub(super) fn validate_record(
    record: &bacnet_types::constructed::BACnetAuditLogRecord,
) -> Result<usize, Error> {
    validate_record_dynamic_size(record)?;
    let mut encoded = BytesMut::new();
    encode_audit_log_record(record, &mut encoded)?;
    if encoded.len() > MAX_RECORD_BYTES {
        return Err(Error::OutOfRange(format!(
            "AuditLog record length {} exceeds {MAX_RECORD_BYTES}",
            encoded.len()
        )));
    }
    Ok(encoded.len())
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

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

#[cfg(test)]
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
