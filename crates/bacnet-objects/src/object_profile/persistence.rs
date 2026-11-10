//! Application-owned storage of written Tags, independent of row provisioning.

use crate::prelude::*;
#[cfg(feature = "std")]
use std::path::Path;

use bacnet_types::constructed::BACnetNameValue;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

#[cfg(feature = "std")]
use crate::durable::file::ObjectFile;
#[cfg(feature = "std")]
use crate::snapshot::tags as format;

/// Maximum complete persisted Tags representation, including its 12-byte
/// identity/version header and presence octet: 1 MiB. This opt-in storage
/// bound does not apply to memory-only objects and is not a BACnet limit.
pub const MAX_TAGS_SNAPSHOT_BYTES: usize = 1024 * 1024;

/// The last successfully written Tags array, independent of whether the
/// application currently provisions the row, and, on the object types that
/// keep it here, the last written Description.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TagsSnapshot {
    /// `None` means no saved write. `Some([])` is a saved empty array and
    /// overrides configured Tags when the application provisions the row.
    pub tags: Option<Vec<BACnetNameValue>>,
    /// Description, once a write has set it (#2364), at most 16 KiB of
    /// UTF-8; `None` until then. A Device, a Lighting Output and a Binary
    /// Lighting Output keep theirs here. Every other object saves `None`
    /// and refuses storage that holds one.
    pub description: Option<String>,
}

/// Application-owned storage shared by all objects supporting durable Tags.
///
/// Loads happen during construction. Saves run in order on the object's
/// writer thread, before it serves the new array. A backend must return an
/// error without replacing its previous snapshot if it refuses a save.
/// The backend defines its durability guarantees; completion of a save
/// wait alone does not establish success. See [`crate::durable`].
///
/// An implementation must not own the object it serves, or the database
/// holding it: the object's writer thread calls it, and dropping the object
/// there would wait for that same thread to end, forever.
pub trait TagsPersistence: Send + Sync {
    /// Load the saved snapshot for this exact object, or `None` if absent.
    fn load(&self, object: ObjectIdentifier) -> Result<Option<TagsSnapshot>, Error>;

    /// Replace this object's saved snapshot, or refuse without changing it.
    fn save(&self, object: ObjectIdentifier, snapshot: &TagsSnapshot) -> Result<(), Error>;
}

/// One object's Tags in an application-selected file.
///
/// Uses the existing synchronized temporary-file and atomic replacement
/// backend. Parent-directory synchronization after rename is best effort:
/// a failure is logged, not returned after the replacement has happened.
/// A successful call or a local round trip is not power-loss qualification.
/// This backend does not coordinate multiple owners or processes at one path.
/// Both reads and writes enforce [`MAX_TAGS_SNAPSHOT_BYTES`] and
/// [`MAX_TAGS`](super::MAX_TAGS); a save that a write would refuse fails
/// with the error the write gets.
///
/// The file holds what [`snapshot::tags::encode`] returns; that module
/// documents the format.
///
/// [`snapshot::tags::encode`]: crate::snapshot::tags::encode
#[derive(Clone, Debug)]
#[cfg(feature = "std")]
pub struct FileTagsPersistence {
    file: ObjectFile,
}

#[cfg(feature = "std")]
impl FileTagsPersistence {
    /// Use the explicit `path` for this object's snapshot.
    pub fn new(path: impl AsRef<Path>) -> Result<Self, Error> {
        Ok(Self {
            file: ObjectFile::new(path.as_ref(), MAX_TAGS_SNAPSHOT_BYTES as u64, "Tags file")?,
        })
    }

    /// Test support, not a stable API: keep the file in `memory`'s entry 0
    /// instead of on disk, decoding and encoding it as on disk (#2233).
    /// [`path`](Self::path) is then a placeholder.
    /// Nothing reaches disk: the state is gone when the process ends, so
    /// never use this outside tests.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn in_memory(memory: crate::durable::MemoryFiles) -> Self {
        let mut store = Self::new("in-memory").expect("a path that isn't empty");
        store.file.keep_in(memory);
        store
    }

    /// The application-selected snapshot path.
    pub fn path(&self) -> &Path {
        self.file.path()
    }
}

#[cfg(feature = "std")]
impl TagsPersistence for FileTagsPersistence {
    fn load(&self, object: ObjectIdentifier) -> Result<Option<TagsSnapshot>, Error> {
        let Some(octets) = self.file.read()? else {
            return Ok(None);
        };
        Ok(Some(format::decode(object, &octets)?))
    }

    fn save(&self, object: ObjectIdentifier, snapshot: &TagsSnapshot) -> Result<(), Error> {
        self.file
            .save_octets(format::encode_checked(object, snapshot)?)
    }
}
