//! Keeping what peers write to an Access Rights object across a restart
//! (#1392).
//!
//! Clause 12.34 says nothing about restarts, so this is the stack's own
//! choice, made because peers provision access rules over the network
//! (#1330, #1332): a head end writes the rules once and expects the
//! controller to keep them. As for the Notification Class
//! ([`NotificationClassPersistence`](crate::notification_class::NotificationClassPersistence)),
//! the storage belongs to the application. The object loads it once, when it
//! is built, and saves on its own writer thread each time a write changes
//! Positive_Access_Rules, Negative_Access_Rules, Enable or Accompaniment
//! (#1393). The bundled server waits for that save with the database guard
//! dropped; a write nobody staged waits where it is ([`crate::durable`] lists
//! those paths).

use crate::prelude::*;
#[cfg(feature = "std")]
use std::path::Path;

use bacnet_types::constructed::{BACnetAccessRule, BACnetDeviceObjectReference};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

#[cfg(feature = "std")]
use crate::durable::file::ObjectFile;
#[cfg(feature = "std")]
use crate::snapshot::access_rights as format;

/// What an Access Rights object keeps across a restart. Each member is
/// `None` until a write sets it: what the application configures applies at
/// each start until then.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AccessRightsSnapshot {
    /// Positive_Access_Rules, in array order, once a write has set them.
    pub positive_access_rules: Option<Vec<BACnetAccessRule>>,
    /// Negative_Access_Rules, in array order, once a write has set them.
    pub negative_access_rules: Option<Vec<BACnetAccessRule>>,
    /// Enable, once a write has set it.
    pub enable: Option<bool>,
    /// Accompaniment, once a write has set it (#1393).
    pub accompaniment: Option<BACnetDeviceObjectReference>,
    /// Description, once a write has set it (#2364); at most 16 KiB of
    /// UTF-8, as an object with storage refuses a longer write.
    pub description: Option<String>,
}

/// Application-owned storage for one Access Rights object's rule arrays,
/// Enable and Accompaniment.
///
/// The object calls it from its writer thread, one call at a time. A written
/// value lands in storage a moment before the object serves it: see [storage
/// leads the served state](crate::durable#storage-leads-the-served-state).
///
/// An implementation must not own the object it serves, or the database
/// holding it: the object's writer thread calls it, and dropping the object
/// there would wait for that same thread to end, forever.
pub trait AccessRightsPersistence: Send + Sync {
    /// What was last saved for `rights`, or `None` when nothing was saved.
    fn load(&self, rights: ObjectIdentifier) -> Result<Option<AccessRightsSnapshot>, Error>;

    /// Durably replace what is saved for `rights` with `snapshot`.
    fn save(&self, rights: ObjectIdentifier, snapshot: &AccessRightsSnapshot) -> Result<(), Error>;
}

/// The most octets a file may take ([`snapshot::access_rights::MAX_LEN`]).
///
/// [`snapshot::access_rights::MAX_LEN`]: crate::snapshot::access_rights::MAX_LEN
#[cfg(feature = "std")]
pub(super) const MAX_FILE_BYTES: u64 = format::MAX_LEN as u64;

/// A file holding one Access Rights object's written rule arrays, Enable and
/// Accompaniment, replaced whole on each save, the same way as
/// [`FileNotificationClassPersistence`]: a failed save leaves the previous
/// state in place, and a completed one survives a power loss. It does not
/// coordinate between processes.
///
/// [`FileNotificationClassPersistence`]: crate::notification_class::FileNotificationClassPersistence
///
/// The file holds what [`snapshot::access_rights::encode`] returns; that module
/// documents the format.
///
/// [`snapshot::access_rights::encode`]: crate::snapshot::access_rights::encode
#[derive(Clone, Debug)]
#[cfg(feature = "std")]
pub struct FileAccessRightsPersistence {
    file: ObjectFile,
}

#[cfg(feature = "std")]
impl FileAccessRightsPersistence {
    /// Keep the rule arrays, Enable and Accompaniment in the file at `path`.
    pub fn new(path: impl AsRef<Path>) -> Result<Self, Error> {
        Ok(Self {
            file: ObjectFile::new(path.as_ref(), MAX_FILE_BYTES, "Access Rights file")?,
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

    /// The file the rule arrays, Enable and Accompaniment are kept in.
    pub fn path(&self) -> &Path {
        self.file.path()
    }
}

#[cfg(feature = "std")]
impl AccessRightsPersistence for FileAccessRightsPersistence {
    fn load(&self, rights: ObjectIdentifier) -> Result<Option<AccessRightsSnapshot>, Error> {
        let Some(octets) = self.file.read()? else {
            return Ok(None);
        };
        Ok(Some(format::decode(rights, &octets)?))
    }

    fn save(&self, rights: ObjectIdentifier, snapshot: &AccessRightsSnapshot) -> Result<(), Error> {
        self.file.save_octets(format::encode(rights, snapshot)?)
    }
}
