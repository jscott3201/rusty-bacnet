//! An object's written Tags (`RBNTAG01`), as `FileTagsPersistence` (with
//! std) writes its file.
//!
//! After the magic tag and the four-octet object identifier, one presence
//! octet: 0 for no written Tags, and nothing after it, or 1 followed by each
//! tag's BACnetNameValue encoding, filling the rest (none for a written
//! empty array). A file holds at most [`MAX_TAGS`]
//! tags and [`MAX_LEN`] octets, and only tags a write would take: no `;` in
//! a name and no constructed value.

use crate::prelude::*;
use bacnet_encoding::constructed::{decode_name_value, encode_name_value};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use super::SnapshotError;
use super::framing::{self, HEADER_LEN};
use crate::object_profile::{
    MAX_TAGS, MAX_TAGS_SNAPSHOT_BYTES, TagsSnapshot, check_tags, no_space_error,
};

/// The format's tag.
pub const MAGIC: [u8; 8] = *b"RBNTAG01";
/// The most octets a file may take, header included:
/// [`MAX_TAGS_SNAPSHOT_BYTES`].
pub const MAX_LEN: usize = MAX_TAGS_SNAPSHOT_BYTES;

/// The file octets for `snapshot`, saved for `object`.
///
/// Refuses ([`OutOfRange`](super::SnapshotErrorKind::OutOfRange)) Tags a
/// write would refuse (more than `MAX_TAGS`, a `;` in a name, a constructed
/// value) and octets past [`MAX_LEN`]: what [`decode`] would refuse.
pub fn encode(object: ObjectIdentifier, snapshot: &TagsSnapshot) -> Result<Vec<u8>, SnapshotError> {
    encode_checked(object, snapshot).map_err(|_| {
        SnapshotError::out_of_range("holds Tags a write would refuse, or too many octets")
            .in_format("Tags snapshot")
    })
}

/// The snapshot a file's `octets` hold for `object`.
///
/// Refuses octets past [`MAX_LEN`], without this format's tag, saved for
/// another object, with a presence octet other than 0 or 1 (or octets after
/// a 0), a tag that does not decode, or Tags [`encode`] would refuse.
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
    Ok(framing::seal(&MAGIC, object, &encode_body(snapshot)?))
}

/// The body after the header, checked as a write is: what a prospective
/// write must fit too.
pub(crate) fn encode_body(snapshot: &TagsSnapshot) -> Result<BytesMut, Error> {
    let mut body = BytesMut::new();
    body.extend_from_slice(&[u8::from(snapshot.tags.is_some())]);
    if let Some(tags) = &snapshot.tags {
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

fn decode_file(object: ObjectIdentifier, octets: &[u8]) -> Result<TagsSnapshot, SnapshotError> {
    let (_, body) = framing::open(octets, &MAGIC, None, MAX_LEN, object)?;
    let tags = match body.split_first() {
        Some((0, [])) => None,
        Some((1, tags)) => Some(framing::decode_capped(tags, MAX_TAGS, decode_name_value)?),
        None => return Err(SnapshotError::truncated("has no valid presence marker")),
        _ => return Err(SnapshotError::damaged("has no valid presence marker")),
    };
    let snapshot = TagsSnapshot { tags };
    encode_body(&snapshot)
        .map_err(|_| SnapshotError::damaged("holds Tags a write would refuse"))?;
    Ok(snapshot)
}
