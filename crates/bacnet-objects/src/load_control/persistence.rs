//! Keeping a Load Control's shed request across a restart (Clause 12.28,
//! #2479).
//!
//! The clause has a device keep the request properties of a request that
//! is pending or under way through a restart, and look at the request again
//! as though Start_Time had just been written. As for the Notification
//! Class ([`NotificationClassPersistence`]), the storage belongs to the
//! application: an object built with
//! [`LoadControlObject::with_persistence`](super::LoadControlObject::with_persistence)
//! loads it once and saves on its own writer ([`crate::durable`]). The
//! `saving` module says when.
//!
//! [`NotificationClassPersistence`]: crate::notification_class::NotificationClassPersistence

use crate::prelude::*;
#[cfg(feature = "std")]
use std::path::Path;

use bacnet_types::constructed::BACnetShedLevel;
use bacnet_types::enums::ShedState;
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};

use crate::clock::UNSPECIFIED_DATETIME;
#[cfg(feature = "std")]
use crate::durable::file::ObjectFile;
#[cfg(feature = "std")]
use crate::snapshot::load_control as format;

/// What a Load Control keeps across a restart: its shed request as it
/// stands, its agreed Duty_Window, its shed state, Enable, and a written
/// Description.
///
/// [`Default`] is an idle object's: SHED_INACTIVE, level 0, Start_Time
/// unspecified, the three durations zero, enabled, and no Description.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadControlSnapshot {
    /// Present_Value. A rebuilt object takes any state but SHED_INACTIVE as
    /// a request to look at again: it starts SHED_REQUEST_PENDING. One
    /// saved SHED_COMPLIANT or SHED_NON_COMPLIANT was being shed, so its
    /// end calls [`LoadShedder::stop`](super::LoadShedder::stop) even if
    /// the shedder is never asked about it.
    pub state: ShedState,
    /// Requested_Shed_Level.
    pub requested_shed_level: BACnetShedLevel,
    /// Start_Time.
    pub start_time: (Date, Time),
    /// Shed_Duration, in minutes.
    pub shed_duration: u64,
    /// Duty_Window, in minutes.
    pub duty_window: u64,
    /// The agreed Duty_Window, in minutes
    /// ([`set_agreed_duty_window`](super::LoadControlObject::set_agreed_duty_window)).
    pub agreed_duty_window: u64,
    /// Enable. A rebuilt object restored FALSE holds no request, as a
    /// written FALSE leaves it.
    pub enable: bool,
    /// Description, once a write has set it (#2364); at most 16 KiB of
    /// UTF-8, as an object with storage refuses a longer write. `None`
    /// until then.
    pub description: Option<String>,
}

impl Default for LoadControlSnapshot {
    fn default() -> Self {
        Self {
            state: ShedState::SHED_INACTIVE,
            requested_shed_level: BACnetShedLevel::Level(0),
            start_time: UNSPECIFIED_DATETIME,
            shed_duration: 0,
            duty_window: 0,
            agreed_duty_window: 0,
            enable: true,
            description: None,
        }
    }
}

/// Application-owned storage for one Load Control's shed request.
///
/// The object calls `load` once, as it is built, and `save` from its writer
/// thread, one call at a time. A written request lands in storage a moment
/// before the object serves it: see [storage leads the served
/// state](crate::durable#storage-leads-the-served-state). A backend that
/// refuses a save returns an error and keeps what it held; the write that
/// asked for it is then refused.
///
/// An implementation must not own the object it serves, or the database
/// holding it: the object's writer thread calls it, and dropping the object
/// there would wait for that same thread to end, forever.
pub trait LoadControlPersistence: Send + Sync {
    /// What was last saved for `object`, or `None` when nothing was saved.
    fn load(&self, object: ObjectIdentifier) -> Result<Option<LoadControlSnapshot>, Error>;

    /// Durably replace what is saved for `object` with `snapshot`.
    fn save(&self, object: ObjectIdentifier, snapshot: &LoadControlSnapshot) -> Result<(), Error>;
}

/// A file holding one Load Control's shed request, replaced whole on each
/// save the same way as
/// [`FileNotificationClassPersistence`](crate::notification_class::FileNotificationClassPersistence):
/// a failed save leaves the previous file in place, and a completed one
/// survives a power loss. It does not coordinate between processes.
///
/// The file holds what [`snapshot::load_control::encode`] returns; that
/// module documents the format.
///
/// [`snapshot::load_control::encode`]: crate::snapshot::load_control::encode
#[derive(Clone, Debug)]
#[cfg(feature = "std")]
pub struct FileLoadControlPersistence {
    file: ObjectFile,
}

#[cfg(feature = "std")]
impl FileLoadControlPersistence {
    /// Keep the shed request in the file at `path`. Refuses an empty path.
    pub fn new(path: impl AsRef<Path>) -> Result<Self, Error> {
        Ok(Self {
            file: ObjectFile::new(path.as_ref(), format::MAX_LEN as u64, "Load Control file")?,
        })
    }

    /// Test support, not a stable API: keep the file in `memory`'s entry 0
    /// instead of on disk, decoding and encoding it as on disk.
    /// [`path`](Self::path) is then a placeholder. Nothing reaches disk:
    /// the state is gone when the process ends, so never use this outside
    /// tests.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn in_memory(memory: crate::durable::MemoryFiles) -> Self {
        let mut store = Self::new("in-memory").expect("a path that isn't empty");
        store.file.keep_in(memory);
        store
    }

    /// The file the shed request is kept in.
    pub fn path(&self) -> &Path {
        self.file.path()
    }
}

#[cfg(feature = "std")]
impl LoadControlPersistence for FileLoadControlPersistence {
    fn load(&self, object: ObjectIdentifier) -> Result<Option<LoadControlSnapshot>, Error> {
        let Some(octets) = self.file.read()? else {
            return Ok(None);
        };
        Ok(Some(format::decode(object, &octets)?))
    }

    fn save(&self, object: ObjectIdentifier, snapshot: &LoadControlSnapshot) -> Result<(), Error> {
        self.file.save_octets(format::encode(object, snapshot)?)
    }
}
