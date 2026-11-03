//! Explicit persistence for one AuditLog object. The object calls it from its
//! writer thread, never under the database guard ([`crate::durable`]).

use crate::prelude::*;
#[cfg(feature = "std")]
use std::fs::{File, OpenOptions};
#[cfg(feature = "std")]
use std::io::{ErrorKind, Read, Write};
#[cfg(feature = "std")]
use std::path::{Path, PathBuf};

use bacnet_types::constructed::BACnetAuditLogRecordResult;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use super::receipt::CompletedAuditReceipt;

pub(crate) mod codec;
#[cfg(feature = "std")]
use crate::snapshot::{SnapshotError, SnapshotErrorKind, audit_log};
#[cfg(feature = "std")]
use codec::MAX_SNAPSHOT_BYTES;

/// Maximum configured AuditLog capacity accepted by the repository.
pub const MAX_AUDIT_RECORDS: u32 = 10_000;

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
///
/// Each slot holds what [`snapshot::audit_log::encode`] returns, and loading
/// picks a slot as [`snapshot::audit_log::newest`] does; that module
/// documents the format and the rules.
///
/// [`snapshot::audit_log::encode`]: crate::snapshot::audit_log::encode
/// [`snapshot::audit_log::newest`]: crate::snapshot::audit_log::newest
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
                return Err(SlotReadFailure::Recoverable(too_large()));
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
enum SlotReadFailure {
    /// The slot can't be a snapshot (it is past the size cap): damage, so
    /// the other slot may still serve.
    Recoverable(SnapshotError),
    /// Reading failed: the load fails.
    Fatal(Error),
}

/// The refusal of a slot past the size cap, found before reading it.
#[cfg(feature = "std")]
fn too_large() -> SnapshotError {
    SnapshotError::new(SnapshotErrorKind::TooLarge, "length exceeds the slot cap")
        .in_format(codec::WHAT)
}

#[cfg(feature = "std")]
impl AuditLogPersistence for FileAuditLogPersistence {
    fn load(&self, expected_object: ObjectIdentifier) -> Result<Option<AuditLogSnapshot>, Error> {
        let reads = [0, 1].map(|slot| self.read_slot(slot));
        choose_slot(reads, expected_object)
    }

    fn commit(&self, snapshot: &AuditLogSnapshot) -> Result<(), Error> {
        let bytes = audit_log::encode(snapshot)?;
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

/// The snapshot to load from the two slots' reads, by
/// [`audit_log::newest`]'s rules. A read that failed fails the load, in
/// slot order: slot 0 refused as a snapshot this log must not take is
/// reported ahead of slot 1's read failure, as each slot's state always
/// was. A slot found past the size cap without being read counts as
/// damage, reported only when nothing loads and no read slot was damaged.
#[cfg(feature = "std")]
fn choose_slot(
    reads: [Result<Option<Vec<u8>>, SlotReadFailure>; 2],
    expected_object: ObjectIdentifier,
) -> Result<Option<AuditLogSnapshot>, Error> {
    let mut skipped = None;
    let mut slots: [Option<Vec<u8>>; 2] = [None, None];
    for (slot, read) in reads.into_iter().enumerate() {
        match read {
            Ok(octets) => slots[slot] = octets,
            Err(SlotReadFailure::Fatal(error)) => {
                // Slot 0 is chosen from before slot 1 fails: its refusal
                // comes first.
                if let Some(Err(refused)) = slots[0]
                    .as_deref()
                    .filter(|_| slot == 1)
                    .map(|octets| audit_log::decode(expected_object, octets))
                    && !refused.is_damage()
                {
                    return Err(refused.into());
                }
                return Err(error);
            }
            Err(SlotReadFailure::Recoverable(error)) => skipped = Some(error),
        }
    }
    let chosen = audit_log::newest(expected_object, [slots[0].as_deref(), slots[1].as_deref()]);
    match (chosen, skipped) {
        (Ok(None), Some(error)) => Err(error.into()),
        (Ok(chosen), _) => Ok(chosen.map(|chosen| chosen.snapshot)),
        (Err(error), _) => Err(error.into()),
    }
}

#[cfg(feature = "std")]
fn invalidate_failed_slot(file: &File) {
    // Best effort only: the other slot remains the last known valid state.
    // No stronger portable power-loss behavior is promised after an I/O error.
    let _ = file.set_len(0);
    let _ = file.sync_all();
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
    let length = usize::try_from(metadata.len())
        .ok()
        .filter(|length| *length <= MAX_SNAPSHOT_BYTES)
        .ok_or(SlotReadFailure::Recoverable(too_large()))?;
    let mut data = Vec::with_capacity(length.min(MAX_SNAPSHOT_BYTES + 1));
    file.take((MAX_SNAPSHOT_BYTES + 1) as u64)
        .read_to_end(&mut data)
        .map_err(|error| SlotReadFailure::Fatal(error.into()))?;
    if data.len() > MAX_SNAPSHOT_BYTES {
        return Err(SlotReadFailure::Recoverable(too_large()));
    }
    if data.len() != length {
        return Err(SlotReadFailure::Fatal(Error::Encoding(
            "AuditLog snapshot changed while being read".into(),
        )));
    }
    Ok(Some(data))
}

/// Check `snapshot` as the codec does before writing it: the stack's error,
/// [`Error::OutOfRange`] for a cap or size, [`Error::Encoding`] for state
/// that doesn't hold together.
pub(super) fn validate_snapshot(snapshot: &AuditLogSnapshot) -> Result<(), Error> {
    codec::check(snapshot).map_err(|error| error.in_format(codec::WHAT).into())
}

/// The encoded length of `record`, refusing one past the record cap.
pub(super) fn validate_record(
    record: &bacnet_types::constructed::BACnetAuditLogRecord,
) -> Result<usize, Error> {
    codec::record_len(record, &mut BytesMut::new())
        .map_err(|error| error.in_format(codec::WHAT).into())
}

/// A schema-v1 slot for `snapshot`, which must hold no receipts.
#[cfg(test)]
pub(super) fn encode_snapshot_v1(snapshot: &AuditLogSnapshot) -> Result<Vec<u8>, Error> {
    Ok(codec::encode_for_version(
        snapshot,
        codec::SCHEMA_VERSION_V1,
    )?)
}

/// Encode `snapshot` under the schema-v2 header. The records are encoded as
/// today's codec does, so a test reproduces a 0.11.0 file by handing it the
/// log statuses that encode to the old octets.
// std: only the file-backed schema tests write v2.
#[cfg(all(test, feature = "std"))]
pub(super) fn encode_snapshot_v2(snapshot: &AuditLogSnapshot) -> Result<Vec<u8>, Error> {
    Ok(codec::encode_for_version(
        snapshot,
        codec::SCHEMA_VERSION_V2,
    )?)
}

#[cfg(test)]
mod codec_tests;

// std: these read and write slot files.
#[cfg(all(test, feature = "std"))]
mod file_tests;
