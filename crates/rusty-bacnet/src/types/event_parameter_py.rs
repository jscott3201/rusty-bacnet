//! The Python form of an Event Enrollment's Event_Parameters (#1499): a
//! mapping whose `kind` names the algorithm, with that algorithm's members.
//!
//! - `change_of_bitstring`: `time_delay`, `bitmask` and `list_of_values`,
//!   each bit string a `{"unused_bits", "data"}` mapping as a `bit_string`
//!   PropertyValue reads.
//! - `change_of_state`: `time_delay` and `list_of_values`, each state a
//!   `(choice_tag, value)` pair: a bool for tag 0, an int otherwise, and the
//!   contents as `bytes` for a vendor tag 64 to 254.
//! - `change_of_value`: `time_delay`, then `bitmask` (a bit string mapping) or
//!   `referenced_property_increment` (a float), the other `None`.
//! - `floating_limit`: `time_delay`, `setpoint_reference` (a
//!   `DeviceObjectPropertyReference` mapping), `low_diff_limit`,
//!   `high_diff_limit` and `deadband`.
//! - `out_of_range`: `time_delay`, `low_limit`, `high_limit` and `deadband`.
//! - `extended`: `vendor_id`, `extended_event_type` and `parameters`, the
//!   encoded parameter list as `bytes`.
//! - `none`: no other key.
//! - `other`: an algorithm the stack keeps as encoded octets: `tag` and
//!   `data`, the alternative's contents as `bytes`.
//!
//! The form is for reading. The value itself keeps the octets it was read
//! from, so writing it back sends them unchanged.

use bacnet_encoding::constructed::encode_property_state;
use bacnet_encoding::tags::decode_tag;
use bacnet_types::constructed::{
    BACnetEventParameter, BACnetPropertyStates, ChangeOfValueCriteria,
};
use bytes::BytesMut;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};
use pyo3::IntoPyObjectExt;

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
        // The decoder gives `none` this form: tag 20 with no contents.
        EP::Opaque { tag: 20, data } if data.is_empty() => {
            dict.set_item("kind", "none")?;
        }
        EP::Opaque { tag, data } => {
            dict.set_item("kind", "other")?;
            dict.set_item("tag", tag)?;
            dict.set_item("data", PyBytes::new(py, data))?;
        }
    }
    Ok(dict)
}

/// A bit string as a `bit_string` PropertyValue's `.value` reads.
fn bit_string<'py>(py: Python<'py>, unused_bits: u8, data: &[u8]) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    dict.set_item("unused_bits", unused_bits)?;
    dict.set_item("data", PyBytes::new(py, data))?;
    Ok(dict)
}

/// One property state as `(choice_tag, value)`.
fn property_state(py: Python<'_>, state: &BACnetPropertyStates) -> PyResult<Py<PyAny>> {
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
