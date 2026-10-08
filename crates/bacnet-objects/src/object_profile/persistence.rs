//! Application-owned storage of written Tags, independent of row provisioning.

use std::path::Path;

use bacnet_encoding::constructed::{decode_name_value, encode_name_value};
use bacnet_types::constructed::BACnetNameValue;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use super::{check_tags, no_space_error, MAX_TAGS};
use crate::durable::file::ObjectFile;

/// Maximum complete persisted Tags representation, including its 12-byte
/// identity/version header and presence octet: 1 MiB. This opt-in storage
/// bound does not apply to memory-only objects and is not a BACnet limit.
pub const MAX_TAGS_SNAPSHOT_BYTES: usize = 1024 * 1024;
const HEADER_BYTES: usize = 12;
const MAGIC: &[u8; 8] = b"RBNTAG01";

/// The last successfully written Tags array, independent of whether the
/// application currently provisions the row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TagsSnapshot {
    /// `None` means no saved write. `Some([])` is a saved empty array and
    /// overrides configured Tags when the application provisions the row.
    pub tags: Option<Vec<BACnetNameValue>>,
}

/// Application-owned storage shared by all objects supporting durable Tags.
///
/// Loads happen during construction. Saves run in order on the object's
/// writer thread, before it serves the new array. A backend must return an
/// error without replacing its previous snapshot if it refuses a save.
/// The backend defines its durability guarantees; completion of a save
/// wait alone does not establish success. See [`crate::durable`].
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
///
/// The format holds a version tag, object identifier, presence octet and
/// concatenated BACnetNameValue elements. Both reads and writes enforce
/// [`MAX_TAGS_SNAPSHOT_BYTES`] and [`MAX_TAGS`].
#[derive(Clone, Debug)]
pub struct FileTagsPersistence {
    file: ObjectFile,
}

impl FileTagsPersistence {
    /// Use the explicit `path` for this object's snapshot.
    pub fn new(path: impl AsRef<Path>) -> Result<Self, Error> {
        Ok(Self {
            file: ObjectFile::new(path.as_ref(), MAGIC, MAX_TAGS_SNAPSHOT_BYTES as u64, "Tags")?,
        })
    }

    /// The application-selected snapshot path.
    pub fn path(&self) -> &Path {
        self.file.path()
    }
}

/// Validate custom snapshots and prospective writes with exactly the same
/// semantic and serialized-size bounds as the file backend.
pub(super) fn encode_snapshot(snapshot: &TagsSnapshot) -> Result<BytesMut, Error> {
    let mut body = BytesMut::new();
    body.extend_from_slice(&[u8::from(snapshot.tags.is_some())]);
    if let Some(tags) = &snapshot.tags {
        check_tags(tags)?;
        for tag in tags {
            encode_name_value(&mut body, tag)?;
            if body.len() > MAX_TAGS_SNAPSHOT_BYTES - HEADER_BYTES {
                return Err(no_space_error());
            }
        }
    }
    Ok(body)
}

impl TagsPersistence for FileTagsPersistence {
    fn load(&self, object: ObjectIdentifier) -> Result<Option<TagsSnapshot>, Error> {
        let Some(body) = self.file.load(object)? else {
            return Ok(None);
        };
        let tags = match body.split_first() {
            Some((0, [])) => None,
            Some((1, bytes)) => Some(self.file.decode_capped(
                bytes,
                MAX_TAGS,
                decode_name_value,
            )?),
            _ => return Err(self.file.corrupt("has no valid presence marker")),
        };
        let snapshot = TagsSnapshot { tags };
        encode_snapshot(&snapshot)?;
        Ok(Some(snapshot))
    }

    fn save(&self, object: ObjectIdentifier, snapshot: &TagsSnapshot) -> Result<(), Error> {
        let body = encode_snapshot(snapshot)?;
        self.file.save(object, &body)
    }
}
