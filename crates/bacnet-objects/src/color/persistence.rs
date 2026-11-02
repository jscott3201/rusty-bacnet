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

use bacnet_types::constructed::BACnetXyColor;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use super::command;
#[cfg(feature = "std")]
use crate::durable::file::ObjectFile;

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
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ColorOutputSnapshot {
    /// The last Present_Value the object held, once something drove the
    /// output; `None` before that.
    pub present_value: Option<ColorOutputValue>,
    /// Default_Color or Default_Color_Temperature, once a write has set it
    /// (#1956). `None` until then: the configured default applies at each
    /// start.
    pub default: Option<ColorOutputValue>,
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

/// The format's tag. Version 2 keeps a written default too (#1956).
#[cfg(feature = "std")]
const MAGIC: &[u8; 8] = b"RBNCLR02";
/// Version 1's tag, which still loads: a kind octet and Present_Value.
#[cfg(feature = "std")]
const MAGIC_V1: &[u8; 8] = b"RBNCLR01";
/// The body is at most a kind octet and two values of two REALs each, with
/// their presence octets; anything much larger is not a file this backend
/// wrote.
#[cfg(feature = "std")]
const MAX_FILE_BYTES: u64 = 64;
/// The kind octet of an xy colour, followed by x and y as big-endian IEEE
/// 754 single-precision values.
#[cfg(feature = "std")]
const KIND_XY: u8 = 1;
/// The kind octet of a colour temperature, followed by the kelvin as a
/// big-endian 32-bit unsigned value.
#[cfg(feature = "std")]
const KIND_KELVIN: u8 = 2;

/// A file holding one colour object's Present_Value and written default,
/// replaced whole on each save the same way as
/// [`FileNotificationClassPersistence`]: a failed save leaves the previous
/// file in place, and a completed one survives a power loss.
///
/// The file holds a magic tag, the object's identifier, one octet naming the
/// kind of value, then Present_Value and the default, each behind a presence
/// octet (0 or 1): two big-endian REALs for an xy colour, a big-endian 32-bit
/// kelvin for a temperature. A file of the wrong size, or with a value outside
/// the object's range, does not load. A file of the previous version, a kind
/// octet and Present_Value alone, loads with no default. It does not
/// coordinate between processes.
///
/// [`FileNotificationClassPersistence`]: crate::notification_class::FileNotificationClassPersistence
#[derive(Clone, Debug)]
#[cfg(feature = "std")]
pub struct FileColorOutputPersistence {
    file: ObjectFile,
}

#[cfg(feature = "std")]
impl FileColorOutputPersistence {
    /// Keep the Present_Value in the file at `path`. Refuses an empty path.
    pub fn new(path: impl AsRef<Path>) -> Result<Self, Error> {
        Ok(Self {
            file: ObjectFile::new(path.as_ref(), MAGIC, MAX_FILE_BYTES, "Color output")?
                .with_previous(MAGIC_V1),
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

    /// The file the Present_Value and default are kept in.
    pub fn path(&self) -> &Path {
        self.file.path()
    }
}

#[cfg(feature = "std")]
impl ColorOutputPersistence for FileColorOutputPersistence {
    fn load(&self, object: ObjectIdentifier) -> Result<Option<ColorOutputSnapshot>, Error> {
        let Some((previous, body)) = self.file.load_versioned(object)? else {
            return Ok(None);
        };
        let Some((&kind, rest)) = body.split_first() else {
            return Err(self.file.corrupt("holds no valid value"));
        };
        let width = match kind {
            KIND_XY => 8,
            KIND_KELVIN => 4,
            _ => return Err(self.file.corrupt("holds no valid value")),
        };
        let value = |octets: &[u8]| match kind {
            KIND_XY => ColorOutputValue::Xy(BACnetXyColor::new(
                f32::from_be_bytes(octets[..4].try_into().expect("four")),
                f32::from_be_bytes(octets[4..].try_into().expect("four")),
            )),
            _ => ColorOutputValue::Kelvin(u32::from_be_bytes(octets.try_into().expect("four"))),
        };
        let snapshot = if previous {
            if rest.len() != width {
                return Err(self.file.corrupt("holds no valid value"));
            }
            ColorOutputSnapshot {
                present_value: Some(value(rest)),
                default: None,
            }
        } else {
            // Each value behind its presence octet, and nothing after them.
            let mut rest = rest;
            let mut next = || match rest.split_first() {
                Some((0, after)) => {
                    rest = after;
                    Ok(None)
                }
                Some((1, after)) if after.len() >= width => {
                    let (octets, after) = after.split_at(width);
                    rest = after;
                    Ok(Some(value(octets)))
                }
                _ => Err(self.file.corrupt("holds no valid value")),
            };
            let present_value = next()?;
            let default = next()?;
            if !rest.is_empty() {
                return Err(self.file.corrupt("holds no valid value"));
            }
            ColorOutputSnapshot {
                present_value,
                default,
            }
        };
        if snapshot
            .present_value
            .is_some_and(|value| !value.is_valid())
            || snapshot
                .default
                .is_some_and(|value| !value.is_valid_default())
        {
            return Err(self.file.corrupt("holds a value out of range"));
        }
        Ok(Some(snapshot))
    }

    fn save(&self, object: ObjectIdentifier, snapshot: &ColorOutputSnapshot) -> Result<(), Error> {
        use bacnet_types::enums::ObjectType;
        // The kind follows the values; with neither, the object type.
        let kind = match snapshot.present_value.or(snapshot.default) {
            Some(ColorOutputValue::Xy(_)) => KIND_XY,
            Some(ColorOutputValue::Kelvin(_)) => KIND_KELVIN,
            None if object.object_type() == ObjectType::COLOR_TEMPERATURE => KIND_KELVIN,
            None => KIND_XY,
        };
        let mut body = Vec::with_capacity(19);
        body.push(kind);
        for value in [snapshot.present_value, snapshot.default] {
            match value {
                None => body.push(0),
                Some(ColorOutputValue::Xy(color)) if kind == KIND_XY => {
                    body.push(1);
                    body.extend_from_slice(&color.x.to_be_bytes());
                    body.extend_from_slice(&color.y.to_be_bytes());
                }
                Some(ColorOutputValue::Kelvin(kelvin)) if kind == KIND_KELVIN => {
                    body.push(1);
                    body.extend_from_slice(&kelvin.to_be_bytes());
                }
                Some(_) => {
                    return Err(Error::OutOfRange(
                        "a colour and a temperature in one snapshot".into(),
                    ));
                }
            }
        }
        self.file.save(object, &body)
    }
}
