//! The two-slot file store for log snapshots.

use crate::prelude::*;
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

use bacnet_types::constructed::{BACnetEventLogRecord, BACnetLogMultipleRecord, BACnetLogRecord};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use super::codec::{self, Decoded, MAX_SNAPSHOT_BYTES, SnapshotRecord};
use super::{LogPersistence, LogSnapshot};

/// Two-slot, versioned and checksummed file store for one log's snapshots,
/// as [`FileAuditLogPersistence`](crate::audit::FileAuditLogPersistence)
/// keeps an Audit Log's.
///
/// Commits alternate between `base.slot0` and `base.slot1` by generation,
/// and each one fully writes and synchronizes its slot, so a commit cut off
/// part way leaves the other slot's snapshot to load. The first commit to a
/// slot also synchronizes its directory (on Unix; see
/// [`durable`](crate::durable)).
///
/// Loading takes the newer valid slot. A slot that is damaged (truncated,
/// a bad checksum, or contents that don't decode or don't hold together) is
/// passed over for the other one, with a warning; loading fails when both
/// are damaged. It also fails, without falling back, when a slot belongs to
/// another object or uses a schema this build can't read, when reading
/// either slot fails, and when both slots hold different snapshots of one
/// generation. It does not coordinate several processes.
#[derive(Clone, Debug)]
pub struct FileLogPersistence {
    slot_paths: [PathBuf; 2],
}

impl FileLogPersistence {
    /// Use `base_path.slot0` and `base_path.slot1`. An empty path is refused
    /// with [`Error::OutOfRange`].
    pub fn new(base_path: impl AsRef<Path>) -> Result<Self, Error> {
        let base = base_path.as_ref().as_os_str();
        if base.is_empty() {
            return Err(Error::OutOfRange(
                "log persistence base path must not be empty".into(),
            ));
        }
        let slot = |suffix: &str| {
            let mut path = base.to_os_string();
            path.push(suffix);
            PathBuf::from(path)
        };
        Ok(Self {
            slot_paths: [slot(".slot0"), slot(".slot1")],
        })
    }

    /// The two slot paths, for application backup and diagnostics.
    pub fn slot_paths(&self) -> [PathBuf; 2] {
        self.slot_paths.clone()
    }

    fn load_slots<R: SnapshotRecord>(
        &self,
        expected: ObjectIdentifier,
    ) -> Result<Option<LogSnapshot<R>>, Error> {
        let mut newest: Option<LogSnapshot<R>> = None;
        let mut damage = None;
        for path in &self.slot_paths {
            let Some(data) = read_slot(path)? else {
                continue;
            };
            match codec::decode::<R>(&data, expected) {
                Decoded::Refused(error) => return Err(error),
                Decoded::Corrupt(error) => {
                    log_warn!(slot = %path.display(), %error, "Passing over a damaged log snapshot slot");
                    damage = Some(error);
                }
                Decoded::Valid(snapshot) => match &newest {
                    Some(current) if current.generation > snapshot.generation => {}
                    Some(current) if current.generation == snapshot.generation => {
                        if *current != snapshot {
                            return Err(Error::Encoding(
                                "log snapshot slots hold different snapshots of one generation"
                                    .into(),
                            ));
                        }
                    }
                    _ => newest = Some(snapshot),
                },
            }
        }
        match (newest, damage) {
            (Some(snapshot), _) => Ok(Some(snapshot)),
            (None, Some(error)) => Err(error),
            (None, None) => Ok(None),
        }
    }

    fn commit_slot<R: SnapshotRecord>(&self, snapshot: &LogSnapshot<R>) -> Result<(), Error> {
        let bytes = codec::encode(snapshot)?;
        let path = &self.slot_paths[(snapshot.generation % 2) as usize];
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            && !parent.exists()
        {
            std::fs::create_dir_all(parent)?;
            // The new directory's own entry has to be durable too, or a
            // power loss could take the slot with it.
            crate::durable::sync_parent_dir(parent);
        }
        let (mut file, created) = match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(file) => (file, true),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => (
                OpenOptions::new().write(true).truncate(true).open(path)?,
                false,
            ),
            Err(error) => return Err(error.into()),
        };
        if let Err(error) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
            // Best effort: the other slot keeps the last good snapshot.
            let _ = file.set_len(0);
            let _ = file.sync_all();
            return Err(error.into());
        }
        if created {
            crate::durable::sync_parent_dir(path);
        }
        Ok(())
    }
}

/// A slot's octets, or `None` when it doesn't exist. A slot past the size
/// cap is read only up to one octet past it, which then fails its length
/// check as damage.
fn read_slot(path: &Path) -> Result<Option<Vec<u8>>, Error> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut data = Vec::new();
    file.take(MAX_SNAPSHOT_BYTES as u64 + 1)
        .read_to_end(&mut data)?;
    Ok(Some(data))
}

macro_rules! file_log_persistence {
    ($($record:ty),*) => {$(
        impl LogPersistence<$record> for FileLogPersistence {
            fn load(
                &self,
                expected_object: ObjectIdentifier,
            ) -> Result<Option<LogSnapshot<$record>>, Error> {
                self.load_slots(expected_object)
            }

            fn commit(&self, snapshot: &LogSnapshot<$record>) -> Result<(), Error> {
                self.commit_slot(snapshot)
            }
        }
    )*};
}

file_log_persistence!(
    BACnetLogRecord,
    BACnetEventLogRecord,
    BACnetLogMultipleRecord
);
