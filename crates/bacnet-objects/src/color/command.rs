//! Color_Command of the Color and Color Temperature objects (Addendum
//! 135-2020ca): taking a written `BACnetColorCommand` and checking it against
//! what the object's table of colour commands allows.
//!
//! A write reaches the object as the SEQUENCE's octets in one
//! `ApplicationData`, decoded as `common::decode_command_write` describes.
//! The rules below come from the addendum's Color_Command subclauses and
//! their tables (12.X.6 and Table 12-X2 for Color, 12.Y.6 and Table 12-Y2 for
//! Color Temperature), and from the ranges its Clause 21 production puts on
//! the fields. A command either object refuses is VALUE_OUT_OF_RANGE.
//!
//! - NONE only reports that nothing was written, so neither object takes it.
//!   The enumeration has no vendor range, so nothing past STOP is taken
//!   either.
//! - A Color object takes FADE_TO_COLOR and STOP. FADE_TO_COLOR needs a
//!   target colour whose coordinates are both 0.0 to 1.0, the range of the
//!   object's Present_Value.
//! - A Color Temperature object takes FADE_TO_CCT, RAMP_TO_CCT, STEP_UP_CCT,
//!   STEP_DOWN_CCT and STOP. FADE_TO_CCT and RAMP_TO_CCT need a target colour
//!   temperature of 1000 to 30000 K, the object's range; the object clamps
//!   it to Min_Pres_Value and Max_Pres_Value when it carries the command out.
//! - A fade time must be 100 to 86,400,000 ms where a fade carries one; a
//!   ramp rate 1 to 30000 K/s where RAMP_TO_CCT does; a step increment 1 to
//!   30000 K where a step operation does.
//! - A field the operation has no use for is kept as written but not checked:
//!   the Color Temperature text has the object ignore such a field, and the
//!   Color object is held to the same rule.
//! - A missing target is refused as out of range, as Lighting Output refuses
//!   a FADE_TO without its target level.

use std::ops::RangeInclusive;

use bacnet_encoding::constructed::{decode_color_command_value, encode_color_command};
use bacnet_types::constructed::{BACnetColorCommand, BACnetXyColor};
use bacnet_types::enums::ColorOperation;
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;
use bytes::BytesMut;

use crate::common;

/// The fade times a colour command may carry, in milliseconds, which bound
/// Default_Fade_Time too (Clauses 12.X.10 and 12.Y.10).
pub(super) const FADE_TIME_MS: RangeInclusive<u32> = 100..=86_400_000;

/// The colour temperatures a Color Temperature object takes, in kelvin.
pub(super) const KELVIN: RangeInclusive<u32> = 1_000..=30_000;

/// The ramp rates (K/s) and step increments (K) a colour command may carry,
/// which bound Default_Ramp_Rate and Default_Step_Increment too (Clauses
/// 12.Y.11 and 12.Y.12).
pub(super) const KELVIN_STEP: RangeInclusive<u32> = 1..=30_000;

/// The range of each xy coordinate.
const COORDINATE: RangeInclusive<f32> = 0.0..=1.0;

/// Refuse a present `value` outside `range`.
fn within<T: PartialOrd>(value: Option<T>, range: RangeInclusive<T>) -> Result<(), Error> {
    match value {
        Some(value) if !range.contains(&value) => Err(common::value_out_of_range_error()),
        _ => Ok(()),
    }
}

/// A target the operation needs: missing is out of range.
fn required<T>(value: Option<T>) -> Result<T, Error> {
    value.ok_or_else(common::value_out_of_range_error)
}

/// Whether both coordinates of `color` are within 0.0 to 1.0; NaN is not.
pub(super) fn xy_in_range(color: BACnetXyColor) -> bool {
    COORDINATE.contains(&color.x) && COORDINATE.contains(&color.y)
}

/// Check `command` against what a Color object takes; see the module
/// documentation for the rules.
pub(super) fn check_color(command: &BACnetColorCommand) -> Result<(), Error> {
    match command.operation {
        ColorOperation::FADE_TO_COLOR => {
            if !xy_in_range(required(command.target_color)?) {
                return Err(common::value_out_of_range_error());
            }
            within(command.fade_time, FADE_TIME_MS)
        }
        ColorOperation::STOP => Ok(()),
        _ => Err(common::value_out_of_range_error()),
    }
}

/// Check `command` against what a Color Temperature object takes; see the
/// module documentation for the rules.
pub(super) fn check_color_temperature(command: &BACnetColorCommand) -> Result<(), Error> {
    match command.operation {
        ColorOperation::FADE_TO_CCT => {
            within(Some(required(command.target_color_temperature)?), KELVIN)?;
            within(command.fade_time, FADE_TIME_MS)
        }
        ColorOperation::RAMP_TO_CCT => {
            within(Some(required(command.target_color_temperature)?), KELVIN)?;
            within(command.ramp_rate, KELVIN_STEP)
        }
        ColorOperation::STEP_UP_CCT | ColorOperation::STEP_DOWN_CCT => {
            within(command.step_increment, KELVIN_STEP)
        }
        ColorOperation::STOP => Ok(()),
        _ => Err(common::value_out_of_range_error()),
    }
}

/// Decode a Color_Command write and check it with `check`.
pub(super) fn decode_write(
    value: PropertyValue,
    check: fn(&BACnetColorCommand) -> Result<(), Error>,
) -> Result<BACnetColorCommand, Error> {
    let command = common::decode_command_write(value, decode_color_command_value)?;
    check(&command)?;
    Ok(command)
}

/// The value Color_Command reads as.
pub(super) fn encode(command: &BACnetColorCommand) -> PropertyValue {
    let mut encoded = BytesMut::new();
    encode_color_command(&mut encoded, command);
    PropertyValue::ApplicationData(encoded.to_vec())
}
