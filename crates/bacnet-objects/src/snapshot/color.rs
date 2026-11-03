//! A colour output's Present_Value and written default, for the Color and
//! Color Temperature objects (`RBNCLR02`; `RBNCLR01` still loads), as
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

use crate::prelude::*;
use bacnet_types::constructed::BACnetXyColor;
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;

use super::SnapshotError;
use super::framing;
use crate::color::{ColorOutputSnapshot, ColorOutputValue};

/// The format's tag: version 2, which keeps a written default too (#1956).
pub const MAGIC: [u8; 8] = *b"RBNCLR02";
/// Version 1's tag, which [`decode`] still reads and [`encode`] never
/// writes.
pub const MAGIC_V1: [u8; 8] = *b"RBNCLR01";
/// The kind octet of an xy colour.
pub const KIND_XY: u8 = 1;
/// The kind octet of a colour temperature.
pub const KIND_KELVIN: u8 = 2;
/// The most octets a file may take: a kind octet and two values of two
/// REALs each, with their presence octets, come to well under it.
pub const MAX_LEN: usize = 64;

/// The file octets for `snapshot`, saved for `object`.
///
/// The kind follows the values, or with neither, the object's type.
/// Refuses ([`OutOfRange`](super::SnapshotErrorKind::OutOfRange)) a colour
/// and a temperature in one snapshot, and a value the object couldn't hold:
/// what [`decode`] would refuse.
pub fn encode(
    object: ObjectIdentifier,
    snapshot: &ColorOutputSnapshot,
) -> Result<Vec<u8>, SnapshotError> {
    encode_file(object, snapshot).map_err(|error| error.in_format("Color output snapshot"))
}

/// The snapshot a file's `octets` hold for `object`.
///
/// Refuses octets past [`MAX_LEN`], without either version's tag, saved for
/// another object, of an unknown kind, of the wrong size, or holding a value
/// the object couldn't hold.
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
    Ok(framing::seal(&MAGIC, object, &body))
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
    let (previous, body) = framing::open(octets, &MAGIC, Some(&MAGIC_V1), MAX_LEN, object)?;
    let no_value = SnapshotError::damaged("holds no valid value");
    let Some((&kind, rest)) = body.split_first() else {
        return Err(SnapshotError::truncated("holds no valid value"));
    };
    let width = match kind {
        KIND_XY => 8,
        KIND_KELVIN => 4,
        _ => return Err(no_value),
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
        }
    };
    if !in_range(&snapshot) {
        return Err(SnapshotError::damaged("holds a value out of range"));
    }
    Ok(snapshot)
}
