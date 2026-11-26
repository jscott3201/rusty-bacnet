//! Application-owned storage of written Tags, independent of row provisioning.

use crate::prelude::*;
#[cfg(feature = "std")]
use std::path::Path;

use bacnet_types::constructed::BACnetNameValue;
use bacnet_types::error::Error;
#[cfg(feature = "std")]
use bacnet_types::error::Refusal;
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
    /// The halves of the Device clock's zone a peer wrote (#2395), on a
    /// Device whose clock takes such writes; none held until then. Only a
    /// Device keeps them here. Every other object saves none and refuses
    /// storage that holds one.
    pub zone: WrittenZone,
}

/// The halves of the Device clock's zone a peer wrote and storage keeps
/// (#2395): each is `None` until a write sets it, and then wins over the
/// zone the host gives the clock, until the Device's
/// `clear_written_zone`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WrittenZone {
    /// UTC_Offset: minutes west of UTC, a multiple of 15 within -1440 to
    /// 1440.
    pub utc_offset: Option<i16>,
    /// Daylight_Savings_Status.
    pub daylight_savings_status: Option<bool>,
}

impl WrittenZone {
    /// Whether neither half is held.
    pub fn is_empty(&self) -> bool {
        self.utc_offset.is_none() && self.daylight_savings_status.is_none()
    }

    /// The zone `(utc_offset, daylight_savings_status)` with each held half
    /// in place of the one given.
    pub fn over(&self, (utc_offset, daylight_savings_status): (i16, bool)) -> (i16, bool) {
        (
            self.utc_offset.unwrap_or(utc_offset),
            self.daylight_savings_status
                .unwrap_or(daylight_savings_status),
        )
    }

    /// Whether every held half is one a write takes: an offset that is a
    /// multiple of 15 within -1440 to 1440.
    pub(crate) fn is_valid(&self) -> bool {
        self.utc_offset.is_none_or(offset_written)
    }

    /// These halves with a write of `value` to `property` held too: an
    /// INTEGER UTC_Offset within the range [`is_valid`](Self::is_valid)
    /// names, else VALUE_OUT_OF_RANGE, or a BOOLEAN Daylight_Savings_Status;
    /// INVALID_DATA_TYPE for another datatype.
    #[cfg(any(feature = "zone-writes", feature = "durable-writes"))]
    pub(crate) fn with_write(
        mut self,
        property: bacnet_types::enums::PropertyIdentifier,
        value: &bacnet_types::primitives::PropertyValue,
    ) -> Result<Self, Error> {
        use bacnet_types::enums::PropertyIdentifier as P;
        use bacnet_types::primitives::PropertyValue as V;
        match (property, value) {
            (P::UTC_OFFSET, V::Signed(minutes)) => {
                let minutes = i16::try_from(*minutes)
                    .ok()
                    .filter(|minutes| offset_written(*minutes))
                    .ok_or_else(crate::common::value_out_of_range_error)?;
                self.utc_offset = Some(minutes);
            }
            (P::DAYLIGHT_SAVINGS_STATUS, V::Boolean(summer)) => {
                self.daylight_savings_status = Some(*summer);
            }
            _ => return Err(crate::common::invalid_data_type_error()),
        }
        Ok(self)
    }
}

/// Whether a written UTC_Offset may hold `minutes`: a multiple of 15
/// within a day either way.
fn offset_written(minutes: i16) -> bool {
    (-1440..=1440).contains(&minutes) && minutes % 15 == 0
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
/// Its errors name the file's path (#2730) and keep their variant: an I/O
/// failure is `Error::Transport`, and a file it can't load (damaged, cut
/// short, or of a version this build doesn't read) is
/// `Error::Persistence` with its kind (#1833), as for the other file
/// backends. A protocol error a write would get passes through as it is.
/// A file saved for another object is the one configuration mistake: the
/// same path given to two objects. It is `Error::InvalidConfig`, naming the
/// object it belongs to and the one loading it, and saying how to go on:
/// remove or move the file, or give each object a path of its own.
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

    /// `error`, its text naming the file (#2730), its variant and kind kept:
    /// an I/O failure keeps its I/O kind, and persisted state (a damaged
    /// file, a version this build doesn't read, a snapshot refused as it is
    /// saved) keeps its own (#1833). A protocol error, which a write is
    /// answered with, is left as it is.
    fn named(&self, error: Error) -> Error {
        let path = self.path().display().to_string();
        match error {
            Error::Transport(io) => {
                Error::Transport(std::io::Error::new(io.kind(), format!("{path}: {io}")))
            }
            persisted @ Error::Persistence(_) => persisted.with_context(&path),
            other => other,
        }
    }

    /// The refusal of `octets`, saved for another object than `object`.
    fn other_object(&self, object: ObjectIdentifier, octets: &[u8]) -> Error {
        let owner = octets
            .get(8..crate::snapshot::framing::HEADER_LEN)
            .and_then(|identifier| ObjectIdentifier::decode(identifier).ok())
            .map_or_else(|| "another object".to_owned(), |owner| owner.to_string());
        Error::InvalidConfig(Refusal::text(format!(
            "{}: the Tags file belongs to {owner}, not {object}; remove or move it, or give \
             each object a path of its own",
            self.path().display()
        )))
    }
}

#[cfg(feature = "std")]
impl TagsPersistence for FileTagsPersistence {
    fn load(&self, object: ObjectIdentifier) -> Result<Option<TagsSnapshot>, Error> {
        let octets = match self.file.read_octets() {
            Ok(Some(octets)) => octets,
            Ok(None) => return Ok(None),
            Err(error) => return Err(self.named(error)),
        };
        format::decode(object, &octets)
            .map(Some)
            .map_err(|error| match error.kind() {
                crate::snapshot::PersistenceErrorKind::OtherObject => {
                    self.other_object(object, &octets)
                }
                _ => self.named(error.into()),
            })
    }

    fn save(&self, object: ObjectIdentifier, snapshot: &TagsSnapshot) -> Result<(), Error> {
        format::encode_checked(object, snapshot)
            .and_then(|octets| self.file.save_octets(octets))
            .map_err(|error| self.named(error))
    }
}
