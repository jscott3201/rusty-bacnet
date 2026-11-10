//! Keeping a colour object's Present_Value, and a written default, across a
//! restart (#1526, #1956).
//!
//! Addendum 135-2020ca lets a zero default, Default_Color (0, 0) or a
//! Default_Color_Temperature of 0, mean that the output comes back at the
//! value it held before the restart (Clauses 12.X.8 and 12.Y.4). That needs
//! Present_Value kept somewhere that outlives the process; as for the
//! Notification Class ([`NotificationClassPersistence`]), the storage
//! belongs to the application. The object loads it once, as it takes the
//! storage, and saves each new Present_Value on its own writer thread
//! ([`crate::durable`]), so no file I/O happens under the database guard.
//!
//! A default a peer writes is kept with it, so a BMS that writes a zero
//! default to ask for the restart rule gets it at the next start. Unlike
//! Present_Value, a default write is configuration: it saves before the
//! object serves it, as a Notification Class's Recipient_List does, and is
//! refused with DEVICE / OPERATIONAL_PROBLEM, keeping the old default, when
//! the save fails. A written default wins over the configured one at the
//! next start (`set_default_color` and `set_default_color_temperature` then
//! check their argument without taking it).
//!
//! A Description a peer writes is kept with them (#2364), saved before it
//! is answered and refused when the save fails, as a default write is. Its
//! `clear_written_description` drops the saved one;
//! `clear_written_default` leaves it alone.
//!
//! Unlike a Notification Class's Recipient_List, a Present_Value change
//! doesn't wait for its save, and a save that fails doesn't refuse it: the
//! value is a live output, which a command, a fade's STOP or the start of
//! the device can move, and a light shouldn't refuse a colour because a
//! disk is full. Saves coalesce, so a burst of changes costs one save of the
//! newest value. A save that fails is logged, and tried again with the next
//! change, as the server stops, and as the object is dropped.
//!
//! [`NotificationClassPersistence`]: crate::notification_class::NotificationClassPersistence

#[cfg(feature = "std")]
use std::path::Path;

use crate::prelude::*;
use bacnet_types::constructed::BACnetXyColor;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use super::command;
#[cfg(feature = "std")]
use crate::durable::file::ObjectFile;
#[cfg(feature = "std")]
use crate::snapshot::color as format;

/// A colour output's Present_Value, as storage keeps it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColorOutputValue {
    /// A Color object's CIE 1931 xy colour.
    Xy(BACnetXyColor),
    /// A Color Temperature object's temperature in kelvin.
    Kelvin(u32),
}

impl ColorOutputValue {
    /// Whether the value is one the object could hold: both xy
    /// coordinates 0.0 to 1.0, or a temperature of 1000 to 30000 K.
    pub fn is_valid(&self) -> bool {
        match *self {
            Self::Xy(color) => command::xy_in_range(color),
            Self::Kelvin(kelvin) => command::KELVIN.contains(&kelvin),
        }
    }

    /// Whether the value is one the object takes as its default: one it
    /// could hold, or a temperature of 0 (an xy (0, 0) is in range anyway).
    pub fn is_valid_default(&self) -> bool {
        self.is_valid() || *self == Self::Kelvin(0)
    }
}

/// What a Color or Color Temperature object keeps across a restart.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColorOutputSnapshot {
    /// The last Present_Value the object held, once something drove the
    /// output; `None` before that.
    pub present_value: Option<ColorOutputValue>,
    /// Default_Color or Default_Color_Temperature, once a write has set it
    /// (#1956). `None` until then: the configured default applies at each
    /// start.
    pub default: Option<ColorOutputValue>,
    /// Description, once a write has set it (#2364), at most 16 KiB of
    /// UTF-8; `None` until then.
    pub description: Option<String>,
}

/// Application-owned storage for one colour object's Present_Value.
///
/// The object calls `load` once, as it takes the storage, and `save` from
/// its writer thread, one call at a time. A backend that refuses a save
/// returns an error and keeps what it held.
///
/// An implementation must not own the object it serves, or the database
/// holding it: the object's writer thread calls it, and dropping the object
/// there would wait for that same thread to end, forever.
pub trait ColorOutputPersistence: Send + Sync {
    /// What was last saved for `object`, or `None` when nothing was saved.
    fn load(&self, object: ObjectIdentifier) -> Result<Option<ColorOutputSnapshot>, Error>;

    /// Durably replace what is saved for `object` with `snapshot`.
    fn save(&self, object: ObjectIdentifier, snapshot: &ColorOutputSnapshot) -> Result<(), Error>;
}

/// A file holding one colour object's Present_Value, written default and
/// written Description, replaced whole on each save the same way as
/// [`FileNotificationClassPersistence`]: a failed save leaves the previous
/// file in place, and a completed one survives a power loss. A file of the
/// first version (`RBNCLR01`) loads with no default, and only a file
/// holding a Description takes the newest (`RBNCLR03`). It does not
/// coordinate between processes.
///
/// [`FileNotificationClassPersistence`]: crate::notification_class::FileNotificationClassPersistence
///
/// The file holds what [`snapshot::color::encode`] returns; that module
/// documents the format.
///
/// [`snapshot::color::encode`]: crate::snapshot::color::encode
#[derive(Clone, Debug)]
#[cfg(feature = "std")]
pub struct FileColorOutputPersistence {
    file: ObjectFile,
}

#[cfg(feature = "std")]
impl FileColorOutputPersistence {
    /// Keep the value in the file at `path`.
    pub fn new(path: impl AsRef<Path>) -> Result<Self, Error> {
        Ok(Self {
            file: ObjectFile::new(path.as_ref(), format::MAX_LEN as u64, "Color output file")?,
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

    /// The file the value is kept in.
    pub fn path(&self) -> &Path {
        self.file.path()
    }
}

#[cfg(feature = "std")]
impl ColorOutputPersistence for FileColorOutputPersistence {
    fn load(&self, object: ObjectIdentifier) -> Result<Option<ColorOutputSnapshot>, Error> {
        let Some(octets) = self.file.read()? else {
            return Ok(None);
        };
        Ok(Some(format::decode(object, &octets)?))
    }

    fn save(&self, object: ObjectIdentifier, snapshot: &ColorOutputSnapshot) -> Result<(), Error> {
        self.file.save_octets(format::encode(object, snapshot)?)
    }
}
