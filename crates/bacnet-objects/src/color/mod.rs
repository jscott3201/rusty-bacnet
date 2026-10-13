//! Color (type 63) and Color Temperature (type 64) objects, added by
//! Addendum 135-2020ca (Clauses 12.X and 12.Y, Tables 12-X and 12-Y).
//!
//! A Color object holds a CIE 1931 xy colour, a Color Temperature object a
//! correlated colour temperature in kelvin. Both take a typed
//! `BACnetColorCommand` as Color_Command, checked against the operations
//! each allows (see `command`), and carry it out (#1474): fades and ramps
//! move Tracking_Value to the new Present_Value over time, with In_Progress
//! showing which, steps change the value at once, and STOP ends a fade or
//! ramp where it stands (see `engine`). A Present_Value write moves the
//! output as the object's Transition says.
//!
//! Neither object has a priority array, and their property tables have no
//! Status_Flags, Event_State, Reliability or Out_Of_Service, so the objects
//! serve none of them. Value_Source, once tracked, names the last writer of
//! Present_Value (#1552).

use std::ops::RangeInclusive;
use std::time::Duration;

use bacnet_types::constructed::BACnetXyColor;
use bacnet_types::enums::ColorTransition;
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use crate::audit::ObjectAuditPolicy;
use crate::common;

mod command;
mod engine;
mod metadata;
mod temperature;
mod xy;

pub use temperature::ColorTemperatureObject;
pub use xy::ColorObject;

#[cfg(test)]
mod command_tests;
#[cfg(test)]
mod engine_tests;
#[cfg(test)]
mod metadata_tests;
#[cfg(test)]
mod rows_tests;
#[cfg(test)]
mod value_source_tests;

/// The value an xy colour reads as: its two REALs, which encode as the
/// BACnetxyColor SEQUENCE.
fn xy_value(color: BACnetXyColor) -> PropertyValue {
    PropertyValue::List(vec![
        PropertyValue::Real(color.x),
        PropertyValue::Real(color.y),
    ])
}

/// An xy colour written to the object: the two REALs a WriteProperty of a
/// BACnetxyColor decodes to. A coordinate outside 0.0 to 1.0, NaN included,
/// is VALUE_OUT_OF_RANGE (Clause 12.X); any other value INVALID_DATA_TYPE.
fn written_xy(value: PropertyValue) -> Result<BACnetXyColor, Error> {
    let PropertyValue::List(items) = value else {
        return Err(common::invalid_data_type_error());
    };
    let [PropertyValue::Real(x), PropertyValue::Real(y)] = items.as_slice() else {
        return Err(common::invalid_data_type_error());
    };
    let color = BACnetXyColor::new(*x, *y);
    if command::xy_in_range(color) {
        Ok(color)
    } else {
        Err(common::value_out_of_range_error())
    }
}

/// An Unsigned written to the object that must lie within `range`: any other
/// value is VALUE_OUT_OF_RANGE, and any other datatype INVALID_DATA_TYPE.
fn written_unsigned(value: PropertyValue, range: RangeInclusive<u32>) -> Result<u32, Error> {
    let PropertyValue::Unsigned(value) = value else {
        return Err(common::invalid_data_type_error());
    };
    u32::try_from(value)
        .ok()
        .filter(|value| range.contains(value))
        .ok_or_else(common::value_out_of_range_error)
}

/// A Transition written to the object: NONE up to `last`, the last kind the
/// object has. Any other ENUMERATED is VALUE_OUT_OF_RANGE, and any other
/// datatype INVALID_DATA_TYPE.
fn written_transition(
    value: PropertyValue,
    last: ColorTransition,
) -> Result<ColorTransition, Error> {
    let PropertyValue::Enumerated(raw) = value else {
        return Err(common::invalid_data_type_error());
    };
    if raw > last.to_raw() {
        return Err(common::value_out_of_range_error());
    }
    Ok(ColorTransition::from_raw(raw))
}

/// An audit policy for an object with no commandable property: its
/// Audit_Priority_Filter left out (#1525).
fn noncommandable_audit_policy(policy: ObjectAuditPolicy) -> ObjectAuditPolicy {
    ObjectAuditPolicy {
        priority_filter: None,
        ..policy
    }
}

/// A fade time in milliseconds as a `Duration`.
fn milliseconds(fade_time: u32) -> Duration {
    Duration::from_millis(u64::from(fade_time))
}
