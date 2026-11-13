//! An object's written Tags (`RBNTAG01`, and `RBNTAG02` with a
//! Description), as `FileTagsPersistence` (with std) writes its file.
//!
//! After the magic tag and the four-octet object identifier, one presence
//! octet: 0 for no written Tags, and nothing after it, or 1 followed by each
//! tag's BACnetNameValue encoding, filling the rest (none for a written
//! empty array). A file holds at most [`MAX_TAGS`]
//! tags and [`MAX_LEN`] octets, and only tags a write would take: no `;` in
//! a name and no constructed value.
//!
//! A file holding a written Description (#2364) takes version 2's tag,
//! [`MAGIC_WITH_DESCRIPTION`]: the Description, its length in four
//! big-endian octets and then at most 16 KiB of UTF-8, comes between the
//! header and that body, inside the same [`MAX_LEN`]. [`encode`] writes
//! version 1 for a snapshot without one, so its octets don't change, and a
//! build that reads only version 1 refuses version 2 as an unknown version.
//! Only a Device, a Lighting Output and a Binary Lighting Output keep their
//! Description here; any other object refuses storage that holds one.

use crate::prelude::*;
use bacnet_encoding::constructed::{decode_name_value, encode_name_value};
use bacnet_types::constructed::BACnetNameValue;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use super::SnapshotError;
use super::framing::{self, HEADER_LEN};
use crate::object_profile::{
    MAX_TAGS, MAX_TAGS_SNAPSHOT_BYTES, TagsSnapshot, check_tags, no_space_error,
};
#[cfg(feature = "durable-writes")]
use crate::write_config::description::MAX_OCTETS as MAX_DESCRIPTION_OCTETS;

/// The format's tag, version 1: a snapshot without a written Description.
pub const MAGIC: [u8; 8] = *b"RBNTAG01";
/// Version 2's tag, for a snapshot with a written Description (#2364).
pub const MAGIC_WITH_DESCRIPTION: [u8; 8] = *b"RBNTAG02";
/// The most octets a file may take, header included:
/// [`MAX_TAGS_SNAPSHOT_BYTES`].
pub const MAX_LEN: usize = MAX_TAGS_SNAPSHOT_BYTES;

/// The file octets for `snapshot`, saved for `object`.
///
/// Refuses ([`OutOfRange`](super::SnapshotErrorKind::OutOfRange)) Tags a
/// write would refuse (more than `MAX_TAGS`, a `;` in a name, a constructed
/// value), a Description past 16 KiB and octets past [`MAX_LEN`]: what
/// [`decode`] would refuse.
pub fn encode(object: ObjectIdentifier, snapshot: &TagsSnapshot) -> Result<Vec<u8>, SnapshotError> {
    encode_checked(object, snapshot).map_err(|_| {
        SnapshotError::out_of_range(
            "holds Tags or a Description a write would refuse, or too many octets",
        )
        .in_format("Tags snapshot")
    })
}

/// The snapshot a file's `octets` hold for `object`.
///
/// Refuses octets past [`MAX_LEN`], without either version's tag, saved for
/// another object, with a presence octet other than 0 or 1 (or octets after
/// a 0), a tag that does not decode, Tags [`encode`] would refuse, or a
/// Description cut short, past 16 KiB or not UTF-8.
pub fn decode(object: ObjectIdentifier, octets: &[u8]) -> Result<TagsSnapshot, SnapshotError> {
    decode_file(object, octets).map_err(|error| error.in_format("Tags file"))
}

/// [`encode`], refusing with the error a write of the same Tags gets
/// (RESOURCES / NO_SPACE_TO_WRITE_PROPERTY past a cap, the value's own
/// refusal for a bad tag), as the file backend answers a save.
pub(crate) fn encode_checked(
    object: ObjectIdentifier,
    snapshot: &TagsSnapshot,
) -> Result<Vec<u8>, Error> {
    let magic = match snapshot.description {
        Some(_) => &MAGIC_WITH_DESCRIPTION,
        None => &MAGIC,
    };
    let body = encode_body(snapshot.description.as_deref(), snapshot.tags.as_deref())?;
    Ok(framing::seal(magic, object, &body))
}

/// The body after the header for `tags` and a written `description`, which
/// comes first, checked as a write is.
fn encode_body(
    description: Option<&str>,
    tags: Option<&[BACnetNameValue]>,
) -> Result<BytesMut, Error> {
    let mut body = BytesMut::new();
    if let Some(text) = description {
        framing::put_description(&mut body, text).map_err(|_| no_space_error())?;
    }
    body.extend_from_slice(&[u8::from(tags.is_some())]);
    if let Some(tags) = tags {
        check_tags(tags)?;
        for tag in tags {
            encode_name_value(&mut body, tag)?;
            if body.len() > MAX_LEN - HEADER_LEN {
                return Err(no_space_error());
            }
        }
    }
    Ok(body)
}

/// Whether a snapshot of `description` and `tags` saves, or the refusal a
/// write leaving it gets: what a prospective write must fit.
pub(crate) fn fits(
    description: Option<&str>,
    tags: Option<&[BACnetNameValue]>,
) -> Result<(), Error> {
    encode_body(description, tags).map(|_| ())
}

/// The octets [`encode_body`] gives `tags`, each encoded in turn in
/// `scratch`: what a stage counts once and keeps current as its writes
/// change the Tags, rather than encoding the whole body for each (#2546).
#[cfg(feature = "durable-writes")]
pub(crate) fn tags_len(tags: &[BACnetNameValue], scratch: &mut BytesMut) -> Result<usize, Error> {
    tags.iter().try_fold(0, |len, tag| {
        scratch.clear();
        encode_name_value(scratch, tag)?;
        Ok(len + scratch.len())
    })
}

/// [`fits`] for a written `description` and Tags whose encodings take
/// `tags_len` octets ([`tags_len`]; `None` for no written Tags), when every
/// tag is one a write takes, as every tag served or saved is. The answer is
/// the same: the body only grows as each tag goes in, so judging its whole
/// length once judges each length [`encode_body`] judges on the way.
#[cfg(feature = "durable-writes")]
pub(crate) fn fits_len(description: Option<&str>, tags_len: Option<usize>) -> Result<(), Error> {
    // A Description's length in four octets, then its text, as
    // `framing::put_description` writes it and refuses past its cap.
    let description_len = match description {
        Some(text) if text.len() > MAX_DESCRIPTION_OCTETS => return Err(no_space_error()),
        Some(text) => 4 + text.len(),
        None => 0,
    };
    // The presence octet, then the tags.
    if description_len + 1 + tags_len.unwrap_or(0) > MAX_LEN - HEADER_LEN {
        return Err(no_space_error());
    }
    Ok(())
}

fn decode_file(object: ObjectIdentifier, octets: &[u8]) -> Result<TagsSnapshot, SnapshotError> {
    let (description, body) =
        framing::open_described(octets, (&MAGIC, &MAGIC_WITH_DESCRIPTION), MAX_LEN, object)?;
    let tags = match body.split_first() {
        Some((0, [])) => None,
        Some((1, tags)) => Some(framing::decode_capped(tags, MAX_TAGS, decode_name_value)?),
        None => return Err(SnapshotError::truncated("has no valid presence marker")),
        _ => return Err(SnapshotError::damaged("has no valid presence marker")),
    };
    let snapshot = TagsSnapshot { tags, description };
    check_loaded(&snapshot)?;
    Ok(snapshot)
}

/// Refuse loaded Tags, or a loaded Description, a write would refuse, as
/// damage: the file backend's decode and a custom backend's load answer
/// alike.
pub(crate) fn check_loaded(snapshot: &TagsSnapshot) -> Result<(), SnapshotError> {
    fits(snapshot.description.as_deref(), snapshot.tags.as_deref())
        .map_err(|_| SnapshotError::damaged("holds Tags or a Description a write would refuse"))
}
