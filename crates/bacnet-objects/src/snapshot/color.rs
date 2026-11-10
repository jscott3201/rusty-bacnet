//! A colour output's Present_Value, written default and written
//! Description, for the Color and Color Temperature objects (`RBNCLR02`,
//! and `RBNCLR03` with a Description; `RBNCLR01` still loads), as
//! `FileColorOutputPersistence` (with std) writes its file.
//!
//! After the magic tag and the four-octet object identifier, one octet
//! names the kind of value ([`KIND_XY`] or [`KIND_KELVIN`]); then
//! Present_Value and the default, each behind a presence octet (0 or 1): two
//! big-endian IEEE 754 single-precision values for an xy colour, a
//! big-endian 32-bit kelvin for a temperature. A version 1 file
//! ([`MAGIC_V1`]) holds the kind octet and Present_Value alone, with no
//! presence octet, and loads with no default. Every value must be one the
//! object could hold (a default may also be a temperature of 0).
//!
//! A file holding a written Description (#2364) takes version 3's tag,
//! [`MAGIC_WITH_DESCRIPTION`]: the Description, its length in four
//! big-endian octets and then at most 16 KiB of UTF-8, comes between the
//! header and version 2's body. [`encode`] writes version 2 for a snapshot
//! without one, so its octets don't change, and a build that reads only
//! versions 1 and 2 refuses version 3 as an unknown version.

use crate::prelude::*;
use bacnet_types::constructed::BACnetXyColor;
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;

use super::framing;
use super::{SnapshotError, SnapshotErrorKind};
use crate::color::{ColorOutputSnapshot, ColorOutputValue};
use crate::write_config::description::MAX_OCTETS as MAX_DESCRIPTION_OCTETS;

/// The format's tag: version 2, which keeps a written default too (#1956),
/// for a snapshot without a written Description.
pub const MAGIC: [u8; 8] = *b"RBNCLR02";
/// Version 3's tag, for a snapshot with a written Description (#2364).
pub const MAGIC_WITH_DESCRIPTION: [u8; 8] = *b"RBNCLR03";
/// Version 1's tag, which [`decode`] still reads and [`encode`] never
/// writes.
pub const MAGIC_V1: [u8; 8] = *b"RBNCLR01";
/// The kind octet of an xy colour.
pub const KIND_XY: u8 = 1;
/// The kind octet of a colour temperature.
pub const KIND_KELVIN: u8 = 2;
/// The most octets a file may take: a kind octet and two values of two
/// REALs each, with their presence octets, come to well under 64, and a
/// Description takes its four length octets and at most 16 KiB.
pub const MAX_LEN: usize = 64 + 4 + MAX_DESCRIPTION_OCTETS;

/// The file octets for `snapshot`, saved for `object`.
///
/// The kind follows the values, or with neither, the object's type.
/// Refuses ([`OutOfRange`](super::SnapshotErrorKind::OutOfRange)) a colour
/// and a temperature in one snapshot, a value the object couldn't hold and
/// a Description past 16 KiB: what [`decode`] would refuse.
pub fn encode(
    object: ObjectIdentifier,
    snapshot: &ColorOutputSnapshot,
) -> Result<Vec<u8>, SnapshotError> {
    encode_file(object, snapshot).map_err(|error| error.in_format("Color output snapshot"))
}

/// The snapshot a file's `octets` hold for `object`.
///
/// Refuses octets past [`MAX_LEN`], without a tag of versions 1 to 3, saved
/// for another object, of an unknown kind, of the wrong size, holding a
/// value the object couldn't hold, or with a Description cut short, past
/// 16 KiB or not UTF-8.
pub fn decode(
    object: ObjectIdentifier,
    octets: &[u8],
) -> Result<ColorOutputSnapshot, SnapshotError> {
    decode_file(object, octets).map_err(|error| error.in_format("Color output file"))
}

fn encode_file(
    object: ObjectIdentifier,
    snapshot: &ColorOutputSnapshot,
) -> Result<Vec<u8>, SnapshotError> {
    if !in_range(snapshot) {
        return Err(SnapshotError::out_of_range("holds a value out of range"));
    }
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
                return Err(SnapshotError::out_of_range(
                    "holds a colour and a temperature",
                ));
            }
        }
    }
    framing::seal_described(
        (&MAGIC, &MAGIC_WITH_DESCRIPTION),
        object,
        snapshot.description.as_deref(),
        &body,
    )
}

fn in_range(snapshot: &ColorOutputSnapshot) -> bool {
    snapshot.present_value.is_none_or(|value| value.is_valid())
        && snapshot
            .default
            .is_none_or(|value| value.is_valid_default())
}

fn decode_file(
    object: ObjectIdentifier,
    octets: &[u8],
) -> Result<ColorOutputSnapshot, SnapshotError> {
    // Version 3 is version 2's body behind a Description.
    let (description, previous, body) = if octets.starts_with(&MAGIC_WITH_DESCRIPTION) {
        let tags = (&MAGIC, &MAGIC_WITH_DESCRIPTION);
        let (description, body) = framing::open_described(octets, tags, MAX_LEN, object)?;
        (description, false, body)
    } else {
        let (previous, body) = framing::open(octets, &MAGIC, Some(&MAGIC_V1), MAX_LEN, object)?;
        (None, previous, body)
    };
    let no_value = SnapshotError::damaged("holds no valid value");
    let Some((&kind, rest)) = body.split_first() else {
        return Err(SnapshotError::truncated("holds no valid value"));
    };
    let width = match kind {
        KIND_XY => 8,
        KIND_KELVIN => 4,
        _ => {
            return Err(SnapshotError::new(
                SnapshotErrorKind::UnknownKind(kind),
                "holds a value of an unknown kind",
            ));
        }
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
            return Err(no_value);
        }
        ColorOutputSnapshot {
            present_value: Some(value(rest)),
            default: None,
            description,
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
            _ => Err(no_value),
        };
        let present_value = next()?;
        let default = next()?;
        if !rest.is_empty() {
            return Err(no_value);
        }
        ColorOutputSnapshot {
            present_value,
            default,
            description,
        }
    };
    if !in_range(&snapshot) {
        return Err(SnapshotError::damaged("holds a value out of range"));
    }
    Ok(snapshot)
}
