//! Keeping a forwarder's Recipient_List and Subscribed_Recipients across a
//! restart (Clauses 12.51.8 and 12.51.9).
//!
//! Like [`AuditLogPersistence`](crate::audit::AuditLogPersistence), the
//! storage belongs to the application. The forwarder loads it once, when it is
//! built, and saves both lists whenever either changes. Saves run on the
//! forwarder's own writer thread. The bundled server waits for them with the
//! database guard dropped; a write nobody staged waits where it is
//! ([`crate::durable`] lists those paths). Each saved Subscribed_Recipients
//! entry carries the whole minutes it had left at the save.

use crate::prelude::*;
#[cfg(feature = "std")]
use std::path::Path;

use bacnet_types::constructed::{BACnetDestination, BACnetEventNotificationSubscription};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

#[cfg(feature = "std")]
use crate::durable::file::ObjectFile;
#[cfg(feature = "std")]
use crate::snapshot::notification_forwarder as format;

/// What a forwarder keeps across a restart: both of its lists, and a
/// written Description (#2364).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ForwarderSnapshot {
    /// Recipient_List, in list order, once a write has set it. `None` until
    /// then: the destinations the application configures apply at each
    /// start.
    pub recipient_list: Option<Vec<BACnetDestination>>,
    /// The live Subscribed_Recipients entries, in list order, each with the
    /// whole minutes it had left when saved.
    pub subscribed_recipients: Vec<BACnetEventNotificationSubscription>,
    /// Description, once a write has set it (#2364); at most 16 KiB of
    /// UTF-8, as a forwarder with storage refuses a longer write. `None`
    /// until then: the application's `set_description` applies at each
    /// start.
    pub description: Option<String>,
}

/// Application-owned storage for one Notification Forwarder's lists.
///
/// The forwarder calls it from its writer thread, one call at a time. A
/// written list lands in storage a moment before the forwarder serves it: see
/// [storage leads the served
/// state](crate::durable#storage-leads-the-served-state).
///
/// An implementation must not own the object it serves, or the database
/// holding it: the object's writer thread calls it, and dropping the object
/// there would wait for that same thread to end, forever.
pub trait NotificationForwarderPersistence: Send + Sync {
    /// The lists last saved for `forwarder`, or `None` when nothing was
    /// saved.
    fn load(&self, forwarder: ObjectIdentifier) -> Result<Option<ForwarderSnapshot>, Error>;

    /// Durably replace what is saved for `forwarder` with `snapshot`.
    fn save(&self, forwarder: ObjectIdentifier, snapshot: &ForwarderSnapshot) -> Result<(), Error>;
}

/// The most octets a file may take ([`snapshot::notification_forwarder::MAX_LEN`]).
///
/// [`snapshot::notification_forwarder::MAX_LEN`]: crate::snapshot::notification_forwarder::MAX_LEN
#[cfg(feature = "std")]
pub(super) const MAX_FILE_BYTES: u64 = format::MAX_LEN as u64;

/// A file holding one forwarder's lists, replaced whole on each save.
///
/// A save writes a sibling `.tmp` file, synchronizes it, renames it over the
/// old one and then synchronizes the directory, so a failed save leaves the
/// previous lists in place and a completed one survives a power loss. Once
/// the rename succeeds the save has landed: a directory sync that fails is
/// logged, not returned. (On Windows the directory is not synchronized: see
/// the [`durable`](crate::durable) module.) It does not coordinate between
/// processes.
///
/// The file holds what [`snapshot::notification_forwarder::encode`] returns; that module
/// documents the format.
///
/// [`snapshot::notification_forwarder::encode`]: crate::snapshot::notification_forwarder::encode
#[derive(Clone, Debug)]
#[cfg(feature = "std")]
pub struct FileNotificationForwarderPersistence {
    file: ObjectFile,
}

#[cfg(feature = "std")]
impl FileNotificationForwarderPersistence {
    /// Keep the lists in the file at `path`.
    pub fn new(path: impl AsRef<Path>) -> Result<Self, Error> {
        Ok(Self {
            file: ObjectFile::new(path.as_ref(), MAX_FILE_BYTES, "Notification Forwarder file")?,
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

    /// The file the lists are kept in.
    pub fn path(&self) -> &Path {
        self.file.path()
    }
}

#[cfg(feature = "std")]
impl NotificationForwarderPersistence for FileNotificationForwarderPersistence {
    fn load(&self, forwarder: ObjectIdentifier) -> Result<Option<ForwarderSnapshot>, Error> {
        let Some(octets) = self.file.read()? else {
            return Ok(None);
        };
        Ok(Some(format::decode(forwarder, &octets)?))
    }

    fn save(&self, forwarder: ObjectIdentifier, snapshot: &ForwarderSnapshot) -> Result<(), Error> {
        self.file.save_octets(format::encode(forwarder, snapshot)?)
    }
}
