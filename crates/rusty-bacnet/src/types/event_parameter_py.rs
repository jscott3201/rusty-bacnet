//! The Python form of an Event Enrollment's Event_Parameters (#1499,
//! #2090): a mapping whose `kind` names the algorithm, with that
//! algorithm's members.
//!
//! - `change_of_bitstring`: `time_delay`, `bitmask` and `list_of_values`,
//!   each bit string a `BitString` as a `bit_string` PropertyValue reads.
//! - `change_of_state`: `time_delay` and `list_of_values`, each state a
//!   `(choice_tag, value)` pair: a bool for tag 0, an int otherwise, and the
//!   contents as `bytes` for a vendor tag 64 to 254.
//! - `change_of_value`: `time_delay`, then `bitmask` (a `BitString`) or
//!   `referenced_property_increment` (a float), the other `None`.
//! - `command_failure`: `time_delay` and `feedback_property_reference`.
//! - `floating_limit`: `time_delay`, `setpoint_reference` (a
//!   `DeviceObjectPropertyReference` mapping), `low_diff_limit`,
//!   `high_diff_limit` and `deadband`.
//! - `out_of_range`, `double_out_of_range`: `time_delay`, `low_limit`,
//!   `high_limit` and `deadband`, floats.
//! - `signed_out_of_range`, `unsigned_out_of_range`: the same members as
//!   ints.
//! - `unsigned_range`: `time_delay`, `low_limit` and `high_limit`.
//! - `change_of_life_safety`: `time_delay`,
//!   `list_of_life_safety_alarm_values` and `list_of_alarm_values` (life
//!   safety states as ints), and `mode_property_reference`.
//! - `extended`: `vendor_id`, `extended_event_type` and `parameters`, the
//!   encoded parameter list as `bytes`.
//! - `buffer_ready`: `notification_threshold` and
//!   `previous_notification_count`.
//! - `access_event`: `list_of_access_events` (ints) and
//!   `access_event_time_reference`.
//! - `change_of_characterstring`: `time_delay` and `list_of_alarm_values`,
//!   strings.
//! - `change_of_status_flags`: `time_delay` and `selected_flags`, a
//!   `BitString` naming the status flags.
//! - `change_of_discrete_value`: `time_delay`.
//! - `change_of_timer`: `time_delay`, `alarm_values` (timer states as
//!   ints) and `update_time_reference`.
//! - `none`: no other key.
//! - `other`: an alternative under a tag the standard doesn't assign: `tag`
//!   and `data`, the alternative's contents as `bytes`.
//!
//! References are `DeviceObjectPropertyReference` mappings. The form is for
//! reading. The value itself keeps the octets it was read from, so writing
//! it back sends them unchanged.

use bacnet_encoding::constructed::encode_property_state;
use bacnet_encoding::tags::decode_tag;
use bacnet_types::constructed::{
    BACnetEventParameter, BACnetPropertyStates, ChangeOfValueCriteria,
};
use bytes::BytesMut;
use pyo3::IntoPyObjectExt;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};

use super::PyBitString;
use super::constructed_py::property_reference_to_py;
use crate::errors::to_py_err;

/// `parameter` as its Python mapping.
pub(super) fn event_parameter_to_py<'py>(
    py: Python<'py>,
    parameter: &BACnetEventParameter,
) -> PyResult<Bound<'py, PyDict>> {
    use BACnetEventParameter as EP;
    let dict = PyDict::new(py);
    match parameter {
        EP::ChangeOfBitstring {
            time_delay,
            bitmask,
            list_of_values,
        } => {
            dict.set_item("kind", "change_of_bitstring")?;
            dict.set_item("time_delay", time_delay)?;
            dict.set_item("bitmask", bit_string(py, bitmask.0, &bitmask.1)?)?;
            let values = PyList::empty(py);
            for (unused_bits, data) in list_of_values {
                values.append(bit_string(py, *unused_bits, data)?)?;
            }
            dict.set_item("list_of_values", values)?;
        }
        EP::ChangeOfState {
            time_delay,
            list_of_values,
        } => {
            dict.set_item("kind", "change_of_state")?;
            dict.set_item("time_delay", time_delay)?;
            let values = PyList::empty(py);
            for state in list_of_values {
                values.append(property_state(py, state)?)?;
            }
            dict.set_item("list_of_values", values)?;
        }
        EP::ChangeOfValue {
            time_delay,
            criteria,
        } => {
            dict.set_item("kind", "change_of_value")?;
            dict.set_item("time_delay", time_delay)?;
            let (bitmask, increment) = match criteria {
                ChangeOfValueCriteria::Bitmask { unused_bits, data } => {
                    (Some(bit_string(py, *unused_bits, data)?), None)
                }
                ChangeOfValueCriteria::ReferencedPropertyIncrement(increment) => {
                    (None, Some(f64::from(*increment)))
                }
            };
            dict.set_item("bitmask", bitmask)?;
            dict.set_item("referenced_property_increment", increment)?;
        }
        EP::FloatingLimit {
            time_delay,
            setpoint_reference,
            low_diff_limit,
            high_diff_limit,
            deadband,
        } => {
            dict.set_item("kind", "floating_limit")?;
            dict.set_item("time_delay", time_delay)?;
            dict.set_item(
                "setpoint_reference",
                property_reference_to_py(py, setpoint_reference)?,
            )?;
            dict.set_item("low_diff_limit", f64::from(*low_diff_limit))?;
            dict.set_item("high_diff_limit", f64::from(*high_diff_limit))?;
            dict.set_item("deadband", f64::from(*deadband))?;
        }
        EP::OutOfRange {
            time_delay,
            low_limit,
            high_limit,
            deadband,
        } => {
            dict.set_item("kind", "out_of_range")?;
            dict.set_item("time_delay", time_delay)?;
            dict.set_item("low_limit", f64::from(*low_limit))?;
            dict.set_item("high_limit", f64::from(*high_limit))?;
            dict.set_item("deadband", f64::from(*deadband))?;
        }
        EP::Extended {
            vendor_id,
            extended_event_type,
            parameters,
        } => {
            dict.set_item("kind", "extended")?;
            dict.set_item("vendor_id", vendor_id)?;
            dict.set_item("extended_event_type", extended_event_type)?;
            dict.set_item("parameters", PyBytes::new(py, parameters))?;
        }
        EP::CommandFailure {
            time_delay,
            feedback_property_reference,
        } => {
            dict.set_item("kind", "command_failure")?;
            dict.set_item("time_delay", time_delay)?;
            dict.set_item(
                "feedback_property_reference",
                property_reference_to_py(py, feedback_property_reference)?,
            )?;
        }
        EP::ChangeOfLifeSafety {
            time_delay,
            list_of_life_safety_alarm_values,
            list_of_alarm_values,
            mode_property_reference,
        } => {
            dict.set_item("kind", "change_of_life_safety")?;
            dict.set_item("time_delay", time_delay)?;
            dict.set_item(
                "list_of_life_safety_alarm_values",
                raw_values(list_of_life_safety_alarm_values.iter().map(|s| s.to_raw())),
            )?;
            dict.set_item(
                "list_of_alarm_values",
                raw_values(list_of_alarm_values.iter().map(|s| s.to_raw())),
            )?;
            dict.set_item(
                "mode_property_reference",
                property_reference_to_py(py, mode_property_reference)?,
            )?;
        }
        EP::BufferReady {
            notification_threshold,
            previous_notification_count,
        } => {
            dict.set_item("kind", "buffer_ready")?;
            dict.set_item("notification_threshold", notification_threshold)?;
            dict.set_item("previous_notification_count", previous_notification_count)?;
        }
        EP::UnsignedRange {
            time_delay,
            low_limit,
            high_limit,
        } => {
            dict.set_item("kind", "unsigned_range")?;
            dict.set_item("time_delay", time_delay)?;
            dict.set_item("low_limit", low_limit)?;
            dict.set_item("high_limit", high_limit)?;
        }
        EP::AccessEvent {
            list_of_access_events,
            access_event_time_reference,
        } => {
            dict.set_item("kind", "access_event")?;
            dict.set_item(
                "list_of_access_events",
                raw_values(list_of_access_events.iter().map(|e| e.to_raw())),
            )?;
            dict.set_item(
                "access_event_time_reference",
                property_reference_to_py(py, access_event_time_reference)?,
            )?;
        }
        EP::DoubleOutOfRange {
            time_delay,
            low_limit,
            high_limit,
            deadband,
        } => {
            dict.set_item("kind", "double_out_of_range")?;
            dict.set_item("time_delay", time_delay)?;
            dict.set_item("low_limit", low_limit)?;
            dict.set_item("high_limit", high_limit)?;
            dict.set_item("deadband", deadband)?;
        }
        EP::SignedOutOfRange {
            time_delay,
            low_limit,
            high_limit,
            deadband,
        } => {
            dict.set_item("kind", "signed_out_of_range")?;
            dict.set_item("time_delay", time_delay)?;
            dict.set_item("low_limit", low_limit)?;
            dict.set_item("high_limit", high_limit)?;
            dict.set_item("deadband", deadband)?;
        }
        EP::UnsignedOutOfRange {
            time_delay,
            low_limit,
            high_limit,
            deadband,
        } => {
            dict.set_item("kind", "unsigned_out_of_range")?;
            dict.set_item("time_delay", time_delay)?;
            dict.set_item("low_limit", low_limit)?;
            dict.set_item("high_limit", high_limit)?;
            dict.set_item("deadband", deadband)?;
        }
        EP::ChangeOfCharacterstring {
            time_delay,
            list_of_alarm_values,
        } => {
            dict.set_item("kind", "change_of_characterstring")?;
            dict.set_item("time_delay", time_delay)?;
            dict.set_item("list_of_alarm_values", list_of_alarm_values)?;
        }
        EP::ChangeOfStatusFlags {
            time_delay,
            selected_flags,
        } => {
            dict.set_item("kind", "change_of_status_flags")?;
            dict.set_item("time_delay", time_delay)?;
            dict.set_item("selected_flags", PyBitString::status_flags(*selected_flags))?;
        }
        EP::None => {
            dict.set_item("kind", "none")?;
        }
        EP::ChangeOfDiscreteValue { time_delay } => {
            dict.set_item("kind", "change_of_discrete_value")?;
            dict.set_item("time_delay", time_delay)?;
        }
        EP::ChangeOfTimer {
            time_delay,
            alarm_values,
            update_time_reference,
        } => {
            dict.set_item("kind", "change_of_timer")?;
            dict.set_item("time_delay", time_delay)?;
            dict.set_item(
                "alarm_values",
                raw_values(alarm_values.iter().map(|s| s.to_raw())),
            )?;
            dict.set_item(
                "update_time_reference",
                property_reference_to_py(py, update_time_reference)?,
            )?;
        }
        EP::Opaque { tag, data } => {
            dict.set_item("kind", "other")?;
            dict.set_item("tag", tag)?;
            dict.set_item("data", PyBytes::new(py, data))?;
        }
    }
    Ok(dict)
}

/// Enumerated values as their numbers: Python has no class for these
/// enumerations (life safety states, access events, timer states).
fn raw_values(values: impl Iterator<Item = u32>) -> Vec<u32> {
    values.collect()
}

/// A bit string as a `bit_string` PropertyValue's `.value` reads: a
/// `BitString` of no known type.
fn bit_string<'py>(
    py: Python<'py>,
    unused_bits: u8,
    data: &[u8],
) -> PyResult<Bound<'py, PyBitString>> {
    Bound::new(py, PyBitString::raw(unused_bits, data))
}

/// One property state as `(choice_tag, value)`.
pub(super) fn property_state(py: Python<'_>, state: &BACnetPropertyStates) -> PyResult<Py<PyAny>> {
    use BACnetPropertyStates as S;
    match state {
        S::BooleanValue(value) => (0u32, *value).into_py_any(py),
        S::IntegerValue(value) => (41u32, *value).into_py_any(py),
        S::ExtendedValue(extended) => (extended.tag(), extended.value()).into_py_any(py),
        S::Other(other) => (u32::from(other.tag()), PyBytes::new(py, other.data())).into_py_any(py),
        _ => {
            // Every other alternative is one primitive under its choice tag,
            // so the tag its encoding opens with names it.
            let mut encoded = BytesMut::new();
            encode_property_state(&mut encoded, state).map_err(to_py_err)?;
            let (tag, _) = decode_tag(&encoded, 0).map_err(to_py_err)?;
            (u32::from(tag.number), state.as_u32()).into_py_any(py)
        }
    }
}
