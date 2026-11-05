//! Keeping a Notification Class's written Recipient_List across a restart
//! (Clause 12.21.8, #1315).
//!
//! As for the Notification Forwarder
//! ([`NotificationForwarderPersistence`](crate::notification_forwarder::NotificationForwarderPersistence)),
//! the storage belongs to the application. The class loads it once, when it
//! is built, and saves the list on its own writer thread each time a write
//! changes it. The bundled server waits for that save with the database
//! guard dropped; a write nobody staged waits where it is ([`crate::durable`]
//! lists those paths).

use crate::prelude::*;
#[cfg(feature = "std")]
use std::path::Path;

use bacnet_types::constructed::BACnetDestination;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

#[cfg(feature = "std")]
use crate::durable::file::ObjectFile;
#[cfg(feature = "std")]
use crate::snapshot::notification_class as format;

/// What a Notification Class keeps across a restart: a written
/// Recipient_List, and a written Description (#2364).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NotificationClassSnapshot {
    /// Recipient_List, in list order, once a write has set it. `None` until
    /// then: the destinations the application configures apply at each
    /// start.
    pub recipient_list: Option<Vec<BACnetDestination>>,
    /// Description, once a write has set it (#2364); at most 16 KiB of
    /// UTF-8, as a class with storage refuses a longer write. `None` until
    /// then: the application's `set_description` applies at each start.
    pub description: Option<String>,
}

/// Application-owned storage for one Notification Class's Recipient_List.
///
/// The class calls it from its writer thread, one call at a time. A written
/// list lands in storage a moment before the class serves it: see [storage
/// leads the served state](crate::durable#storage-leads-the-served-state).
///
/// An implementation must not own the object it serves, or the database
/// holding it: the object's writer thread calls it, and dropping the object
/// there would wait for that same thread to end, forever.
pub trait NotificationClassPersistence: Send + Sync {
    /// What was last saved for `class`, or `None` when nothing was saved.
    fn load(&self, class: ObjectIdentifier) -> Result<Option<NotificationClassSnapshot>, Error>;

    /// Durably replace what is saved for `class` with `snapshot`.
    fn save(
        &self,
        class: ObjectIdentifier,
        snapshot: &NotificationClassSnapshot,
    ) -> Result<(), Error>;
}

/// The most octets a file may take ([`snapshot::notification_class::MAX_LEN`]).
///
/// [`snapshot::notification_class::MAX_LEN`]: crate::snapshot::notification_class::MAX_LEN
#[cfg(feature = "std")]
pub(super) const MAX_FILE_BYTES: u64 = format::MAX_LEN as u64;

/// A file holding one Notification Class's Recipient_List, replaced whole on
/// each save, the same way as [`FileNotificationForwarderPersistence`]: a
/// failed save leaves the previous list in place, and a completed one
/// survives a power loss. It does not coordinate between processes.
///
/// [`FileNotificationForwarderPersistence`]: crate::notification_forwarder::FileNotificationForwarderPersistence
///
/// The file holds what [`snapshot::notification_class::encode`] returns; that module
/// documents the format.
///
/// [`snapshot::notification_class::encode`]: crate::snapshot::notification_class::encode
#[derive(Clone, Debug)]
#[cfg(feature = "std")]
pub struct FileNotificationClassPersistence {
    file: ObjectFile,
}

#[cfg(feature = "std")]
impl FileNotificationClassPersistence {
    /// Keep the Recipient_List in the file at `path`.
    pub fn new(path: impl AsRef<Path>) -> Result<Self, Error> {
        Ok(Self {
            file: ObjectFile::new(path.as_ref(), MAX_FILE_BYTES, "Notification Class file")?,
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

    /// The file the Recipient_List is kept in.
    pub fn path(&self) -> &Path {
        self.file.path()
    }
}

#[cfg(feature = "std")]
impl NotificationClassPersistence for FileNotificationClassPersistence {
    fn load(&self, class: ObjectIdentifier) -> Result<Option<NotificationClassSnapshot>, Error> {
        let Some(octets) = self.file.read()? else {
            return Ok(None);
        };
        Ok(Some(format::decode(class, &octets)?))
    }

    fn save(
        &self,
        class: ObjectIdentifier,
        snapshot: &NotificationClassSnapshot,
    ) -> Result<(), Error> {
        self.file.save_octets(format::encode(class, snapshot)?)
    }
}
