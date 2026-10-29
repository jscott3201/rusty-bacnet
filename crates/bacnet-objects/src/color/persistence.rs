//! Keeping a colour object's Present_Value across a restart (#1526).
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
}

/// What a Color or Color Temperature object keeps across a restart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorOutputSnapshot {
    /// The last Present_Value the object held.
    pub present_value: ColorOutputValue,
}

/// Application-owned storage for one colour object's Present_Value.
///
/// The object calls `load` once, as it takes the storage, and `save` from
/// its writer thread, one call at a time. A backend that refuses a save
/// returns an error and keeps what it held.
pub trait ColorOutputPersistence: Send + Sync {
    /// What was last saved for `object`, or `None` when nothing was saved.
    fn load(&self, object: ObjectIdentifier) -> Result<Option<ColorOutputSnapshot>, Error>;

    /// Durably replace what is saved for `object` with `snapshot`.
    fn save(&self, object: ObjectIdentifier, snapshot: &ColorOutputSnapshot) -> Result<(), Error>;
}

/// The format's tag.
#[cfg(feature = "std")]
const MAGIC: &[u8; 8] = b"RBNCLR01";
/// The body is at most a kind octet and two REALs; anything much larger is
/// not a file this backend wrote.
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

/// A file holding one colour object's Present_Value, replaced whole on each
/// save the same way as [`FileNotificationClassPersistence`]: a failed save
/// leaves the previous value in place, and a completed one survives a power
/// loss.
///
/// The file holds a magic tag, the object's identifier, one octet naming the
/// kind of value, and the value: two big-endian REALs for an xy colour, a
/// big-endian 32-bit kelvin for a temperature. A file of the wrong size, of
/// the other kind, or with a value outside the object's range, does not
/// load. It does not coordinate between processes.
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
            file: ObjectFile::new(path.as_ref(), MAGIC, MAX_FILE_BYTES, "Color output")?,
        })
    }

    /// The file the Present_Value is kept in.
    pub fn path(&self) -> &Path {
        self.file.path()
    }
}

#[cfg(feature = "std")]
impl ColorOutputPersistence for FileColorOutputPersistence {
    fn load(&self, object: ObjectIdentifier) -> Result<Option<ColorOutputSnapshot>, Error> {
        let Some(body) = self.file.load(object)? else {
            return Ok(None);
        };
        let present_value = match body.as_slice() {
            [KIND_XY, x0, x1, x2, x3, y0, y1, y2, y3] => ColorOutputValue::Xy(BACnetXyColor::new(
                f32::from_be_bytes([*x0, *x1, *x2, *x3]),
                f32::from_be_bytes([*y0, *y1, *y2, *y3]),
            )),
            [KIND_KELVIN, k0, k1, k2, k3] => {
                ColorOutputValue::Kelvin(u32::from_be_bytes([*k0, *k1, *k2, *k3]))
            }
            _ => return Err(self.file.corrupt("holds no valid value")),
        };
        if !present_value.is_valid() {
            return Err(self.file.corrupt("holds a value out of range"));
        }
        Ok(Some(ColorOutputSnapshot { present_value }))
    }

    fn save(&self, object: ObjectIdentifier, snapshot: &ColorOutputSnapshot) -> Result<(), Error> {
        let mut body = Vec::with_capacity(9);
        match snapshot.present_value {
            ColorOutputValue::Xy(color) => {
                body.push(KIND_XY);
                body.extend_from_slice(&color.x.to_be_bytes());
                body.extend_from_slice(&color.y.to_be_bytes());
            }
            ColorOutputValue::Kelvin(kelvin) => {
                body.push(KIND_KELVIN);
                body.extend_from_slice(&kelvin.to_be_bytes());
            }
        }
        self.file.save(object, &body)
    }
}
