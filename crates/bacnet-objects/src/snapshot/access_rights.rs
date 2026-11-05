//! An Access Rights object's written rule arrays, Enable, Accompaniment and
//! Description (`RBNACR01`, and `RBNACR02` with a Description), as
//! `FileAccessRightsPersistence` (with std) writes its file.
//!
//! After the magic tag and the four-octet object identifier, BACnet
//! encodings of up to four members, in this order, each only once a write
//! has set it:
//!
//! - context tag 0, opening, then each Positive_Access_Rules rule in its
//!   `BACnetAccessRule` encoding, then context tag 0, closing;
//! - the same for Negative_Access_Rules under context tag 1;
//! - Enable as a context-tagged BOOLEAN with tag 2;
//! - context tag 3, opening, then Accompaniment in its
//!   `BACnetDeviceObjectReference` encoding, then context tag 3, closing.
//!
//! A file written before Accompaniment was kept (#1393) is one without that
//! member. Decoding doesn't check the rules or the reference themselves;
//! the object does, as it loads them.
//!
//! A file holding a written Description (#2364) takes version 2's tag,
//! [`MAGIC_WITH_DESCRIPTION`]: the Description, its length in four
//! big-endian octets and then at most 16 KiB of UTF-8, comes between the
//! header and those members. Unlike Accompaniment it is no new member, which
//! a build that predates it would take for damage: [`encode`] writes version
//! 1 for a snapshot without one, so its octets don't change, and a build
//! that reads only version 1 refuses version 2 as an unknown version.

use crate::prelude::*;
use bacnet_encoding::constructed::tagged::{
    decode_ctx_boolean, expect_closing, expect_opening, next_is_context, next_is_opening,
};
use bacnet_encoding::constructed::{
    decode_access_rule, decode_device_object_reference, encode_access_rule,
    encode_device_object_reference,
};
use bacnet_encoding::{primitives, tags};
use bacnet_types::constructed::{BACnetAccessRule, BACnetDeviceObjectReference};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

use super::SnapshotError;
use super::framing;
use crate::access_control::{AccessRightsSnapshot, MAX_ACCESS_RULES};

/// The format's tag, version 1: a snapshot without a written Description.
pub const MAGIC: [u8; 8] = *b"RBNACR01";
/// Version 2's tag, for a snapshot with a written Description (#2364).
pub const MAGIC_WITH_DESCRIPTION: [u8; 8] = *b"RBNACR02";
/// The most octets a file may take: a rule this object accepts encodes in
/// at most 40 octets, so two full arrays of [`MAX_ACCESS_RULES`] take about
/// 80 KiB, Accompaniment at most 12 octets more and a Description at most
/// 16 KiB.
pub const MAX_LEN: usize = 128 * 1024;

const POSITIVE_TAG: u8 = 0;
const NEGATIVE_TAG: u8 = 1;
const ENABLE_TAG: u8 = 2;
const ACCOMPANIMENT_TAG: u8 = 3;
/// The label the tag decoders take.
const WHAT: &str = "Access Rights file";

/// The file octets for `snapshot`, saved for `object`.
///
/// Refuses ([`OutOfRange`](super::SnapshotErrorKind::OutOfRange)) an array
/// of more than [`MAX_ACCESS_RULES`] rules, a Description past 16 KiB and
/// octets past [`MAX_LEN`]: what [`decode`] would refuse.
pub fn encode(
    object: ObjectIdentifier,
    snapshot: &AccessRightsSnapshot,
) -> Result<Vec<u8>, SnapshotError> {
    encode_file(object, snapshot).map_err(|error| error.in_format("Access Rights snapshot"))
}

/// The snapshot a file's `octets` hold for `object`.
///
/// Refuses octets past [`MAX_LEN`], without either version's tag, saved for
/// another object, an array of more than [`MAX_ACCESS_RULES`] rules,
/// members out of order or repeated, a rule or reference that does not
/// decode, octets after the last member, and a Description cut short, past
/// 16 KiB or not UTF-8.
pub fn decode(
    object: ObjectIdentifier,
    octets: &[u8],
) -> Result<AccessRightsSnapshot, SnapshotError> {
    decode_file(object, octets).map_err(|error| error.in_format("Access Rights file"))
}

fn encode_file(
    object: ObjectIdentifier,
    snapshot: &AccessRightsSnapshot,
) -> Result<Vec<u8>, SnapshotError> {
    let mut body = BytesMut::new();
    for (tag, rules) in [
        (POSITIVE_TAG, &snapshot.positive_access_rules),
        (NEGATIVE_TAG, &snapshot.negative_access_rules),
    ] {
        if let Some(rules) = rules {
            if rules.len() > MAX_ACCESS_RULES {
                return Err(SnapshotError::out_of_range(
                    "holds more rules than the object keeps",
                ));
            }
            tags::encode_opening_tag(&mut body, tag);
            for rule in rules {
                encode_access_rule(&mut body, rule);
            }
            tags::encode_closing_tag(&mut body, tag);
        }
    }
    if let Some(enable) = snapshot.enable {
        primitives::encode_ctx_boolean(&mut body, ENABLE_TAG, enable);
    }
    if let Some(reference) = &snapshot.accompaniment {
        tags::encode_opening_tag(&mut body, ACCOMPANIMENT_TAG);
        encode_device_object_reference(&mut body, reference);
        tags::encode_closing_tag(&mut body, ACCOMPANIMENT_TAG);
    }
    let octets = framing::seal_described(
        (&MAGIC, &MAGIC_WITH_DESCRIPTION),
        object,
        snapshot.description.as_deref(),
        &body,
    )?;
    if octets.len() > MAX_LEN {
        return Err(SnapshotError::out_of_range("is too large to save"));
    }
    Ok(octets)
}

fn undecodable() -> SnapshotError {
    SnapshotError::damaged("holds an entry that does not decode")
}

fn decode_file(
    object: ObjectIdentifier,
    octets: &[u8],
) -> Result<AccessRightsSnapshot, SnapshotError> {
    let (description, body) =
        framing::open_described(octets, (&MAGIC, &MAGIC_WITH_DESCRIPTION), MAX_LEN, object)?;
    let mut pos = 0;
    let positive_access_rules = decode_array(body, &mut pos, POSITIVE_TAG)?;
    let negative_access_rules = decode_array(body, &mut pos, NEGATIVE_TAG)?;
    let enable = if next_is_context(body, pos, ENABLE_TAG).map_err(|_| undecodable())? {
        let (enable, end) =
            decode_ctx_boolean(body, pos, ENABLE_TAG, WHAT).map_err(|_| undecodable())?;
        pos = end;
        Some(enable)
    } else {
        None
    };
    let accompaniment = decode_accompaniment(body, &mut pos)?;
    if pos != body.len() {
        return Err(undecodable());
    }
    Ok(AccessRightsSnapshot {
        positive_access_rules,
        negative_access_rules,
        enable,
        accompaniment,
        description,
    })
}

/// The rule array framed by context tag `tag` at `*pos`, if one is there,
/// moving `*pos` past it.
fn decode_array(
    body: &[u8],
    pos: &mut usize,
    tag: u8,
) -> Result<Option<Vec<BACnetAccessRule>>, SnapshotError> {
    if !next_is_opening(body, *pos, tag).map_err(|_| undecodable())? {
        return Ok(None);
    }
    let mut at = expect_opening(body, *pos, tag, WHAT).map_err(|_| undecodable())?;
    let mut rules = Vec::new();
    loop {
        // Running out of octets here means the frame never closed.
        let (next, after) = tags::decode_tag(body, at).map_err(|_| undecodable())?;
        if next.is_closing_tag(tag) {
            *pos = after;
            return Ok(Some(rules));
        }
        if rules.len() == MAX_ACCESS_RULES {
            return Err(SnapshotError::damaged("holds more entries than the cap"));
        }
        let (rule, end) = decode_access_rule(body, at).map_err(|_| undecodable())?;
        rules.push(rule);
        at = end;
    }
}

/// The Accompaniment framed by context tag 3 at `*pos`, if one is there,
/// moving `*pos` past it.
fn decode_accompaniment(
    body: &[u8],
    pos: &mut usize,
) -> Result<Option<BACnetDeviceObjectReference>, SnapshotError> {
    if !next_is_opening(body, *pos, ACCOMPANIMENT_TAG).map_err(|_| undecodable())? {
        return Ok(None);
    }
    let decoded = expect_opening(body, *pos, ACCOMPANIMENT_TAG, WHAT)
        .and_then(|at| decode_device_object_reference(body, at))
        .and_then(|(reference, at)| {
            expect_closing(body, at, ACCOMPANIMENT_TAG, WHAT).map(|end| (reference, end))
        });
    let (reference, end) = decoded.map_err(|_| undecodable())?;
    *pos = end;
    Ok(Some(reference))
}
