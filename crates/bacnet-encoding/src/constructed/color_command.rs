//! Codecs for the colour values of Addendum 135-2020ca (Clause 21):
//! `BACnetxyColor`, a Color object's Present_Value, Tracking_Value and
//! Default_Color, and `BACnetColorCommand`, the Color_Command of a Color or
//! Color Temperature object.
//!
//! An xy colour is two application-tagged REALs, x then y, with nothing
//! around them. The command is a SEQUENCE with no frame of its own: the
//! operation under primitive context tag 0, then any of five optional fields
//! in ascending tag order. The target colour sits between an opening and a
//! closing tag 1; the target colour temperature, fade time, ramp rate and
//! step increment are Unsigneds under tags 2 to 5.

use bacnet_types::constructed::{BACnetColorCommand, BACnetXyColor};
use bacnet_types::enums::ColorOperation;
use bacnet_types::error::Error;
use bytes::BytesMut;

use super::members::unsigned_field;
use super::tagged::{
    decode_app_fixed, decode_optional_ctx, expect_closing, expect_end, expect_opening,
    next_is_opening,
};
use crate::primitives;
use crate::tags::{self, app_tag};

/// The production name the command's decode errors carry.
const WHAT: &str = "color command";

/// The production name an xy colour's decode errors carry.
const XY: &str = "xy color";

/// Encode one `BACnetxyColor`: x, then y, each an application-tagged REAL.
pub fn encode_xy_color(buf: &mut BytesMut, value: &BACnetXyColor) {
    primitives::encode_app_real(buf, value.x);
    primitives::encode_app_real(buf, value.y);
}

/// Read one application-tagged REAL at `offset` for `what`.
fn app_real(data: &[u8], offset: usize, what: &str) -> Result<(f32, usize), Error> {
    let (octets, end) = decode_app_fixed(data, offset, app_tag::REAL, 4, what)?;
    Ok((primitives::decode_real(octets)?, end))
}

/// Decode one `BACnetxyColor` at `offset`: two application-tagged REALs.
///
/// Returns the colour and the offset just past y. Any REAL decodes, NaN and
/// values outside 0.0 to 1.0 included; the receiver checks the range.
/// Contents that run past the end of `data` are [`Error::BufferTooShort`];
/// a missing coordinate, another tag, or a REAL that isn't four octets long
/// is [`Error::Decoding`].
pub fn decode_xy_color(data: &[u8], offset: usize) -> Result<(BACnetXyColor, usize), Error> {
    let (x, offset) = app_real(data, offset, XY)?;
    let (y, offset) = app_real(data, offset, XY)?;
    Ok((BACnetXyColor { x, y }, offset))
}

/// Encode one unframed `BACnetColorCommand` SEQUENCE: the operation, then
/// each field that is present.
pub fn encode_color_command(buf: &mut BytesMut, value: &BACnetColorCommand) {
    primitives::encode_ctx_enumerated(buf, 0, value.operation.to_raw());
    if let Some(color) = &value.target_color {
        tags::encode_opening_tag(buf, 1);
        encode_xy_color(buf, color);
        tags::encode_closing_tag(buf, 1);
    }
    let unsigneds = [
        value.target_color_temperature,
        value.fade_time,
        value.ramp_rate,
        value.step_increment,
    ];
    for (tag, field) in (2..).zip(unsigneds) {
        if let Some(field) = field {
            primitives::encode_ctx_unsigned(buf, tag, field.into());
        }
    }
}

/// A decoded command, the offset just past its last field, and the first
/// field too wide for its type, if any.
pub(super) type Fields = (BACnetColorCommand, usize, Option<&'static str>);

/// Read the target colour framed in tag 1 at `offset`, if one opens there.
fn target_color(data: &[u8], offset: usize) -> Result<(Option<BACnetXyColor>, usize), Error> {
    if !next_is_opening(data, offset, 1)? {
        return Ok((None, offset));
    }
    let content = expect_opening(data, offset, 1, WHAT)?;
    let (color, end) = decode_xy_color(data, content)?;
    let end = expect_closing(data, end, 1, WHAT)?;
    Ok((Some(color), end))
}

/// Read the fields of one unframed `BACnetColorCommand` at `offset`,
/// stopping at the first tag that isn't the next optional field. A field
/// too wide for its type is returned rather than refused, so the caller can
/// check the rest of its input first.
pub(super) fn decode_fields(data: &[u8], offset: usize) -> Result<Fields, Error> {
    let mut oversized = None;
    let (operation, offset) = unsigned_field::<u32>(
        data,
        offset,
        0,
        WHAT,
        "operation [0] exceeds 32 bits",
        &mut oversized,
    )?;
    let (target_color, offset) = target_color(data, offset)?;
    let mut unsigned = |offset: usize, tag: u8, too_wide: &'static str| {
        decode_optional_ctx(data, offset, tag, WHAT, |data, at, tag, _| {
            unsigned_field::<u32>(data, at, tag, WHAT, too_wide, &mut oversized)
        })
    };
    let (target_color_temperature, offset) =
        unsigned(offset, 2, "target-color-temperature [2] exceeds 32 bits")?;
    let (fade_time, offset) = unsigned(offset, 3, "fade-time [3] exceeds 32 bits")?;
    let (ramp_rate, offset) = unsigned(offset, 4, "ramp-rate [4] exceeds 32 bits")?;
    let (step_increment, offset) = unsigned(offset, 5, "step-increment [5] exceeds 32 bits")?;
    let command = BACnetColorCommand {
        operation: ColorOperation::from_raw(operation),
        target_color,
        target_color_temperature,
        fade_time,
        ramp_rate,
        step_increment,
    };
    Ok((command, offset, oversized))
}

/// The [`Error::OutOfRange`] for a field too wide for its type.
fn too_wide(field: &'static str) -> Error {
    Error::OutOfRange(format!("{WHAT} {field}"))
}

/// Decode one unframed `BACnetColorCommand` SEQUENCE at `offset`.
///
/// Returns the command and the offset just past its last field. Decoding
/// stops at the first tag that isn't the next optional field, so a caller
/// that expects exactly one command must check that the returned offset
/// reaches the end of its data, or the closing tag of its frame. A caller
/// whose whole input must be one command uses [`decode_color_command_value`]
/// instead, which checks that before it reports a field too wide for its
/// type.
///
/// Field values aren't range-checked here; the receiver does that. Failures
/// come in three kinds:
///
/// - contents that run past the end of `data`: [`Error::BufferTooShort`];
/// - any other malformed field (a missing operation, an Unsigned with no
///   contents octets, or one of more than four that opens with a zero octet,
///   or a target colour that isn't two four-octet application REALs closed
///   by tag 1, a frame cut off at a tag boundary included): [`Error::Decoding`];
/// - a well-formed command whose operation or an Unsigned field needs more
///   than 32 bits: [`Error::OutOfRange`], naming the first such field. A
///   malformed field anywhere takes precedence.
///
/// Up to four contents octets, an Unsigned or ENUMERATED may open with zero
/// octets; [`encode_color_command`] always writes the shortest form.
pub fn decode_color_command(
    data: &[u8],
    offset: usize,
) -> Result<(BACnetColorCommand, usize), Error> {
    let (command, end, oversized) = decode_fields(data, offset)?;
    match oversized {
        Some(field) => Err(too_wide(field)),
        None => Ok((command, end)),
    }
}

/// Decode a `BACnetColorCommand` that fills `data`, as a WriteProperty of
/// Color_Command carries it.
///
/// Fails as [`decode_color_command`] does, and with [`Error::Decoding`] when
/// octets follow the last field. A field too wide for its type is
/// [`Error::OutOfRange`] only when the rest of `data` is one well-formed
/// command, so a broken input is always reported as broken.
pub fn decode_color_command_value(data: &[u8]) -> Result<BACnetColorCommand, Error> {
    let (command, end, oversized) = decode_fields(data, 0)?;
    expect_end(data, end, end, WHAT)?;
    match oversized {
        Some(field) => Err(too_wide(field)),
        None => Ok(command),
    }
}
