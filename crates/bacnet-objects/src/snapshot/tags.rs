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
//!
//! A file holding a half of the Device clock's zone a peer wrote (#2395)
//! takes version 3's tag, [`MAGIC_WITH_ZONE`]: after the header, one octet
//! saying what follows (bit 0 UTC_Offset, bit 1 Daylight_Savings_Status,
//! bit 2 a Description; at least one of the first two set, no other bit),
//! UTC_Offset as two big-endian octets of minutes if held,
//! Daylight_Savings_Status as one octet, 0 or 1, if held, then version 2's
//! body with a Description or version 1's without. Only a snapshot holding
//! a zone half is written that way, so other files keep their octets. Only
//! a Device keeps its zone here; any other object refuses storage that
//! holds one.

use crate::prelude::*;
use bacnet_encoding::constructed::{decode_name_value, encode_name_value};
use bacnet_types::constructed::BACnetNameValue;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use super::PersistenceError;
use super::framing::{self, HEADER_LEN};
use crate::object_profile::{
    MAX_TAGS, MAX_TAGS_SNAPSHOT_BYTES, TagsSnapshot, WrittenZone, check_tags, no_space_error,
};
#[cfg(feature = "durable-writes")]
use crate::write_config::description::MAX_OCTETS as MAX_DESCRIPTION_OCTETS;

/// The format's tag, version 1: a snapshot without a written Description.
pub const MAGIC: [u8; 8] = *b"RBNTAG01";
/// Version 2's tag, for a snapshot with a written Description (#2364).
pub const MAGIC_WITH_DESCRIPTION: [u8; 8] = *b"RBNTAG02";
/// Version 3's tag, for a snapshot holding a half of the Device clock's
/// zone a peer wrote (#2395).
pub const MAGIC_WITH_ZONE: [u8; 8] = *b"RBNTAG03";

/// Version 3's octet saying what follows it.
const ZONE_OFFSET: u8 = 1;
const ZONE_SUMMER: u8 = 2;
const ZONE_DESCRIBED: u8 = 4;
/// The most octets a file may take, header included:
/// [`MAX_TAGS_SNAPSHOT_BYTES`].
pub const MAX_LEN: usize = MAX_TAGS_SNAPSHOT_BYTES;

/// The file octets for `snapshot`, saved for `object`.
///
/// Refuses ([`OutOfRange`](super::PersistenceErrorKind::RefusedOnEncode)) Tags a
/// write would refuse (more than `MAX_TAGS`, a `;` in a name, a constructed
/// value), a Description past 16 KiB, a UTC_Offset a write would refuse and
/// octets past [`MAX_LEN`]: what [`decode`] would refuse.
pub fn encode(
    object: ObjectIdentifier,
    snapshot: &TagsSnapshot,
) -> Result<Vec<u8>, PersistenceError> {
    encode_checked(object, snapshot).map_err(|_| {
        crate::snapshot::refused_on_encode(
            "holds Tags or a Description a write would refuse, or too many octets",
        )
        .in_format("Tags snapshot")
    })
}

/// The snapshot a file's `octets` hold for `object`.
///
/// Refuses octets past [`MAX_LEN`], without one of the three versions'
/// tags, saved for another object, with a presence octet other than 0 or 1
/// (or octets after a 0), a tag that does not decode, Tags [`encode`] would
/// refuse, a Description cut short, past 16 KiB or not UTF-8, or a zone a
/// write would not leave.
pub fn decode(object: ObjectIdentifier, octets: &[u8]) -> Result<TagsSnapshot, PersistenceError> {
    decode_file(object, octets).map_err(|error| error.in_format("Tags file"))
}

/// [`encode`], refusing with the error a write of the same Tags gets
/// (RESOURCES / NO_SPACE_TO_WRITE_PROPERTY past a cap, the value's own
/// refusal for a bad tag), as the file backend answers a save.
pub(crate) fn encode_checked(
    object: ObjectIdentifier,
    snapshot: &TagsSnapshot,
) -> Result<Vec<u8>, Error> {
    let magic = match (&snapshot.zone, &snapshot.description) {
        (zone, _) if !zone.is_empty() => &MAGIC_WITH_ZONE,
        (_, Some(_)) => &MAGIC_WITH_DESCRIPTION,
        (_, None) => &MAGIC,
    };
    let body = encode_body(
        &snapshot.zone,
        snapshot.description.as_deref(),
        snapshot.tags.as_deref(),
    )?;
    Ok(framing::seal(magic, object, &body))
}

/// The body after the header for `tags`, a written `description` and the
/// `zone` halves written, which come first, checked as a write is.
fn encode_body(
    zone: &WrittenZone,
    description: Option<&str>,
    tags: Option<&[BACnetNameValue]>,
) -> Result<BytesMut, Error> {
    let mut body = BytesMut::new();
    if !zone.is_empty() {
        if !zone.is_valid() {
            return Err(crate::common::value_out_of_range_error());
        }
        let mut held = if description.is_some() {
            ZONE_DESCRIBED
        } else {
            0
        };
        if zone.utc_offset.is_some() {
            held |= ZONE_OFFSET;
        }
        if zone.daylight_savings_status.is_some() {
            held |= ZONE_SUMMER;
        }
        body.extend_from_slice(&[held]);
        if let Some(minutes) = zone.utc_offset {
            body.extend_from_slice(&minutes.to_be_bytes());
        }
        if let Some(summer) = zone.daylight_savings_status {
            body.extend_from_slice(&[u8::from(summer)]);
        }
    }
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

/// Whether a snapshot of `zone`, `description` and `tags` saves, or the
/// refusal a write leaving it gets: what a prospective write must fit.
pub(crate) fn fits(
    zone: &WrittenZone,
    description: Option<&str>,
    tags: Option<&[BACnetNameValue]>,
) -> Result<(), Error> {
    encode_body(zone, description, tags).map(|_| ())
}

/// The octets version 3 gives `zone` ahead of the Description: none for no
/// half held, else the octet saying what follows and each half held.
#[cfg(feature = "durable-writes")]
fn zone_len(zone: &WrittenZone) -> usize {
    if zone.is_empty() {
        return 0;
    }
    1 + zone.utc_offset.map_or(0, |_| 2) + zone.daylight_savings_status.map_or(0, |_| 1)
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
pub(crate) fn fits_len(
    zone: &WrittenZone,
    description: Option<&str>,
    tags_len: Option<usize>,
) -> Result<(), Error> {
    // A Description's length in four octets, then its text, as
    // `framing::put_description` writes it and refuses past its cap.
    let description_len = match description {
        Some(text) if text.len() > MAX_DESCRIPTION_OCTETS => return Err(no_space_error()),
        Some(text) => 4 + text.len(),
        None => 0,
    };
    // The presence octet, then the tags.
    if zone_len(zone) + description_len + 1 + tags_len.unwrap_or(0) > MAX_LEN - HEADER_LEN {
        return Err(no_space_error());
    }
    Ok(())
}

fn decode_file(object: ObjectIdentifier, octets: &[u8]) -> Result<TagsSnapshot, PersistenceError> {
    let zoned = octets.get(..MAGIC_WITH_ZONE.len()) == Some(&MAGIC_WITH_ZONE[..]);
    let (zone, description, body) = if zoned {
        let (_, body) = framing::open(octets, &MAGIC_WITH_ZONE, None, MAX_LEN, object)?;
        let (zone, described, body) = take_zone(body)?;
        if described {
            let (text, body) = framing::take_description(body)?;
            (zone, Some(text), body)
        } else {
            (zone, None, body)
        }
    } else {
        let (description, body) =
            framing::open_described(octets, (&MAGIC, &MAGIC_WITH_DESCRIPTION), MAX_LEN, object)?;
        (WrittenZone::default(), description, body)
    };
    let tags = match body.split_first() {
        Some((0, [])) => None,
        Some((1, tags)) => Some(framing::decode_capped(tags, MAX_TAGS, decode_name_value)?),
        None => return Err(crate::snapshot::truncated("has no valid presence marker")),
        _ => return Err(crate::snapshot::damaged("has no valid presence marker")),
    };
    let snapshot = TagsSnapshot {
        tags,
        description,
        zone,
    };
    check_loaded(&snapshot)?;
    Ok(snapshot)
}

/// Version 3's zone halves at the start of `body`, whether a Description
/// follows them, and the rest.
fn take_zone(body: &[u8]) -> Result<(WrittenZone, bool, &[u8]), PersistenceError> {
    let short = || crate::snapshot::truncated("ends inside its zone");
    let (&held, mut rest) = body.split_first().ok_or_else(short)?;
    if held & !(ZONE_OFFSET | ZONE_SUMMER | ZONE_DESCRIBED) != 0
        || held & (ZONE_OFFSET | ZONE_SUMMER) == 0
    {
        return Err(crate::snapshot::damaged("holds no valid zone marker"));
    }
    let mut zone = WrittenZone::default();
    if held & ZONE_OFFSET != 0 {
        let (minutes, after) = rest.split_first_chunk::<2>().ok_or_else(short)?;
        zone.utc_offset = Some(i16::from_be_bytes(*minutes));
        rest = after;
    }
    if held & ZONE_SUMMER != 0 {
        let (&summer, after) = rest.split_first().ok_or_else(short)?;
        zone.daylight_savings_status = Some(match summer {
            0 => false,
            1 => true,
            _ => {
                return Err(crate::snapshot::damaged(
                    "holds a daylight-saving octet past 1",
                ));
            }
        });
        rest = after;
    }
    Ok((zone, held & ZONE_DESCRIBED != 0, rest))
}

/// Refuse loaded Tags, or a loaded Description, a write would refuse, as
/// damage: the file backend's decode and a custom backend's load answer
/// alike.
pub(crate) fn check_loaded(snapshot: &TagsSnapshot) -> Result<(), PersistenceError> {
    fits(
        &snapshot.zone,
        snapshot.description.as_deref(),
        snapshot.tags.as_deref(),
    )
    .map_err(|_| {
        crate::snapshot::damaged("holds Tags, a Description or a zone a write would refuse")
    })
}
