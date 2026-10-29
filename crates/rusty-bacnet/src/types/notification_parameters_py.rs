//! An event notification's values (`event_values`) as a Python mapping
//! (#2073): `kind` names the event type in snake case (`out_of_range`), and
//! each value is under its member's name.
//!
//! Status flags and other bit strings are `BitString`s, a property state is
//! a `(choice_tag, value)` pair as in Event_Parameters, a life safety
//! operation is a `LifeSafetyOperation`, and the enumerations Python has no
//! class for (life safety states and modes, reliabilities, access events,
//! timer states and transitions) are their numbers. A value the standard
//! leaves open (a command-failure's commanded and feedback values, a
//! discrete value, a reported property value) is a `PropertyValue`, or a
//! list of them when it holds several. A date and time is a `(date, time)`
//! pair of tuples, a reference a `DeviceObjectPropertyReference` mapping, an
//! object reference an `ObjectIdentifier` or `(device, object)` pair, and an
//! authentication factor `(format_type, format_class, value)` as
//! `report_access_event_local` takes it. An optional member the device left
//! out is `None`.

use bacnet_encoding::constructed::{decode_authentication_factor, decode_bacnet_property_value};
use bacnet_encoding::primitives::decode_application_value;
use bacnet_types::constructed::{BACnetPropertyValue, ChangeOfValueChoice, NotificationParameters};
use bacnet_types::primitives::{Date, PropertyValue, Time};
use pyo3::IntoPyObjectExt;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};

use super::constructed_py::{object_reference_to_py, property_reference_to_py};
use super::event_parameter_py::property_state;
use super::{
    PyBitString, PyLifeSafetyOperation, PyPropertyIdentifier, PyPropertyValue, date_value,
    time_value,
};

/// Property values read from a change-of-reliability notification before
/// the rest is left as one `application_data` value.
const MAX_PROPERTY_VALUES: usize = 256;

/// `(date, time)` as Python reads them.
type DateTimeValue = ((u16, u8, u8, u8), (u8, u8, u8, u8));

fn date_time(date: &Date, time: &Time) -> DateTimeValue {
    (date_value(date), time_value(time))
}

/// Encoded application values as `PropertyValue`s: one value as itself,
/// several as a list. Octets that don't decode are one `application_data`
/// value holding them.
fn abstract_value(py: Python<'_>, octets: &[u8]) -> PyResult<Py<PyAny>> {
    let mut values = Vec::new();
    let mut offset = 0;
    while offset < octets.len() {
        match decode_application_value(octets, offset) {
            Ok((value, next)) => {
                values.push(value);
                offset = next;
            }
            Err(_) => {
                let whole = PropertyValue::ApplicationData(octets.to_vec());
                return Bound::new(py, PyPropertyValue::from_rust(whole))?.into_py_any(py);
            }
        }
    }
    if values.len() == 1 {
        let value = values.remove(0);
        return Bound::new(py, PyPropertyValue::from_rust(value))?.into_py_any(py);
    }
    let list = PyList::empty(py);
    for value in values {
        list.append(Bound::new(py, PyPropertyValue::from_rust(value))?)?;
    }
    list.into_py_any(py)
}

/// One reported property value as a mapping.
fn property_value<'py>(
    py: Python<'py>,
    value: &BACnetPropertyValue,
) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    dict.set_item(
        "property_identifier",
        PyPropertyIdentifier {
            inner: value.property_identifier,
        },
    )?;
    dict.set_item("property_array_index", value.property_array_index)?;
    dict.set_item("value", abstract_value(py, &value.value)?)?;
    dict.set_item("priority", value.priority)?;
    Ok(dict)
}

/// The encoded property values of a change-of-reliability notification as
/// a list of mappings, or one `application_data` value when they don't
/// decode.
fn property_values(py: Python<'_>, octets: &[u8]) -> PyResult<Py<PyAny>> {
    let list = PyList::empty(py);
    let mut offset = 0;
    while offset < octets.len() {
        let decoded = (list.len() < MAX_PROPERTY_VALUES)
            .then(|| decode_bacnet_property_value(octets, offset).ok())
            .flatten();
        let Some((value, next)) = decoded else {
            let whole = PropertyValue::ApplicationData(octets.to_vec());
            return Bound::new(py, PyPropertyValue::from_rust(whole))?.into_py_any(py);
        };
        list.append(property_value(py, &value)?)?;
        offset = next;
    }
    list.into_py_any(py)
}

/// An authentication factor as `(format_type, format_class, value)`, or a
/// `PropertyValue` of its octets when they aren't one.
fn authentication_factor(py: Python<'_>, octets: &[u8]) -> PyResult<Py<PyAny>> {
    match decode_authentication_factor(octets, 0) {
        Ok((factor, end)) if end == octets.len() => (
            factor.format_type.to_raw(),
            factor.format_class,
            PyBytes::new(py, &factor.value),
        )
            .into_py_any(py),
        _ => abstract_value(py, octets),
    }
}

/// `parameters` as its Python mapping.
pub(crate) fn notification_parameters_to_py<'py>(
    py: Python<'py>,
    parameters: &NotificationParameters,
) -> PyResult<Bound<'py, PyDict>> {
    use NotificationParameters as N;
    let dict = PyDict::new(py);
    let flags = |dict: &Bound<'py, PyDict>, name: &str, flags| {
        dict.set_item(name, PyBitString::status_flags(flags))
    };
    match parameters {
        N::ChangeOfBitstring {
            referenced_bitstring: (unused_bits, data),
            status_flags,
        } => {
            dict.set_item("kind", "change_of_bitstring")?;
            dict.set_item("referenced_bitstring", PyBitString::raw(*unused_bits, data))?;
            flags(&dict, "status_flags", *status_flags)?;
        }
        N::ChangeOfState {
            new_state,
            status_flags,
        } => {
            dict.set_item("kind", "change_of_state")?;
            dict.set_item("new_state", property_state(py, new_state)?)?;
            flags(&dict, "status_flags", *status_flags)?;
        }
        N::ChangeOfValue {
            new_value,
            status_flags,
        } => {
            dict.set_item("kind", "change_of_value")?;
            let (bits, value) = match new_value {
                ChangeOfValueChoice::ChangedBits { unused_bits, data } => {
                    (Some(PyBitString::raw(*unused_bits, data)), None)
                }
                ChangeOfValueChoice::ChangedValue(value) => (None, Some(f64::from(*value))),
            };
            dict.set_item("changed_bits", bits)?;
            dict.set_item("changed_value", value)?;
            flags(&dict, "status_flags", *status_flags)?;
        }
        N::CommandFailure {
            command_value,
            status_flags,
            feedback_value,
        } => {
            dict.set_item("kind", "command_failure")?;
            dict.set_item("command_value", abstract_value(py, command_value)?)?;
            flags(&dict, "status_flags", *status_flags)?;
            dict.set_item("feedback_value", abstract_value(py, feedback_value)?)?;
        }
        N::FloatingLimit {
            reference_value,
            status_flags,
            setpoint_value,
            error_limit,
        } => {
            dict.set_item("kind", "floating_limit")?;
            dict.set_item("reference_value", f64::from(*reference_value))?;
            flags(&dict, "status_flags", *status_flags)?;
            dict.set_item("setpoint_value", f64::from(*setpoint_value))?;
            dict.set_item("error_limit", f64::from(*error_limit))?;
        }
        N::OutOfRange {
            exceeding_value,
            status_flags,
            deadband,
            exceeded_limit,
        } => {
            dict.set_item("kind", "out_of_range")?;
            dict.set_item("exceeding_value", f64::from(*exceeding_value))?;
            flags(&dict, "status_flags", *status_flags)?;
            dict.set_item("deadband", f64::from(*deadband))?;
            dict.set_item("exceeded_limit", f64::from(*exceeded_limit))?;
        }
        N::ComplexEventType { property_values } => {
            dict.set_item("kind", "complex_event_type")?;
            let list = PyList::empty(py);
            for value in property_values {
                list.append(property_value(py, value)?)?;
            }
            dict.set_item("property_values", list)?;
        }
        N::ChangeOfLifeSafety {
            new_state,
            new_mode,
            status_flags,
            operation_expected,
        } => {
            dict.set_item("kind", "change_of_life_safety")?;
            dict.set_item("new_state", new_state.to_raw())?;
            dict.set_item("new_mode", new_mode.to_raw())?;
            flags(&dict, "status_flags", *status_flags)?;
            dict.set_item(
                "operation_expected",
                PyLifeSafetyOperation {
                    inner: *operation_expected,
                },
            )?;
        }
        N::Extended {
            vendor_id,
            extended_event_type,
            parameters,
        } => {
            dict.set_item("kind", "extended")?;
            dict.set_item("vendor_id", vendor_id)?;
            dict.set_item("extended_event_type", extended_event_type)?;
            dict.set_item("parameters", abstract_value(py, parameters)?)?;
        }
        N::BufferReady {
            buffer_property,
            previous_notification,
            current_notification,
        } => {
            dict.set_item("kind", "buffer_ready")?;
            dict.set_item(
                "buffer_property",
                property_reference_to_py(py, buffer_property)?,
            )?;
            dict.set_item("previous_notification", previous_notification)?;
            dict.set_item("current_notification", current_notification)?;
        }
        N::UnsignedRange {
            exceeding_value,
            status_flags,
            exceeded_limit,
        } => {
            dict.set_item("kind", "unsigned_range")?;
            dict.set_item("exceeding_value", exceeding_value)?;
            flags(&dict, "status_flags", *status_flags)?;
            dict.set_item("exceeded_limit", exceeded_limit)?;
        }
        N::AccessEvent {
            access_event,
            status_flags,
            access_event_tag,
            access_event_time: (date, time),
            access_credential,
            authentication_factor: factor,
        } => {
            dict.set_item("kind", "access_event")?;
            dict.set_item("access_event", access_event.to_raw())?;
            flags(&dict, "status_flags", *status_flags)?;
            dict.set_item("access_event_tag", access_event_tag)?;
            dict.set_item("access_event_time", date_time(date, time))?;
            dict.set_item(
                "access_credential",
                object_reference_to_py(py, access_credential)?,
            )?;
            let factor = match factor {
                Some(octets) => Some(authentication_factor(py, octets)?),
                None => None,
            };
            dict.set_item("authentication_factor", factor)?;
        }
        N::DoubleOutOfRange {
            exceeding_value,
            status_flags,
            deadband,
            exceeded_limit,
        } => {
            dict.set_item("kind", "double_out_of_range")?;
            dict.set_item("exceeding_value", exceeding_value)?;
            flags(&dict, "status_flags", *status_flags)?;
            dict.set_item("deadband", deadband)?;
            dict.set_item("exceeded_limit", exceeded_limit)?;
        }
        N::SignedOutOfRange {
            exceeding_value,
            status_flags,
            deadband,
            exceeded_limit,
        } => {
            dict.set_item("kind", "signed_out_of_range")?;
            dict.set_item("exceeding_value", exceeding_value)?;
            flags(&dict, "status_flags", *status_flags)?;
            dict.set_item("deadband", deadband)?;
            dict.set_item("exceeded_limit", exceeded_limit)?;
        }
        N::UnsignedOutOfRange {
            exceeding_value,
            status_flags,
            deadband,
            exceeded_limit,
        } => {
            dict.set_item("kind", "unsigned_out_of_range")?;
            dict.set_item("exceeding_value", exceeding_value)?;
            flags(&dict, "status_flags", *status_flags)?;
            dict.set_item("deadband", deadband)?;
            dict.set_item("exceeded_limit", exceeded_limit)?;
        }
        N::ChangeOfCharacterstring {
            changed_value,
            status_flags,
            alarm_value,
        } => {
            dict.set_item("kind", "change_of_characterstring")?;
            dict.set_item("changed_value", changed_value)?;
            flags(&dict, "status_flags", *status_flags)?;
            dict.set_item("alarm_value", alarm_value)?;
        }
        N::ChangeOfStatusFlags {
            present_value,
            referenced_flags,
        } => {
            dict.set_item("kind", "change_of_status_flags")?;
            let value = match present_value {
                Some(octets) => Some(abstract_value(py, octets)?),
                None => None,
            };
            dict.set_item("present_value", value)?;
            flags(&dict, "referenced_flags", *referenced_flags)?;
        }
        N::ChangeOfReliability {
            reliability,
            status_flags,
            property_values: octets,
        } => {
            dict.set_item("kind", "change_of_reliability")?;
            dict.set_item("reliability", reliability.to_raw())?;
            flags(&dict, "status_flags", *status_flags)?;
            dict.set_item("property_values", property_values(py, octets)?)?;
        }
        N::ChangeOfDiscreteValue {
            new_value,
            status_flags,
        } => {
            dict.set_item("kind", "change_of_discrete_value")?;
            dict.set_item("new_value", abstract_value(py, new_value)?)?;
            flags(&dict, "status_flags", *status_flags)?;
        }
        N::ChangeOfTimer {
            new_state,
            status_flags,
            update_time: (date, time),
            last_state_change,
            initial_timeout,
            expiration_time,
        } => {
            dict.set_item("kind", "change_of_timer")?;
            dict.set_item("new_state", new_state.to_raw())?;
            flags(&dict, "status_flags", *status_flags)?;
            dict.set_item("update_time", date_time(date, time))?;
            dict.set_item(
                "last_state_change",
                last_state_change.map(|change| change.to_raw()),
            )?;
            dict.set_item("initial_timeout", initial_timeout)?;
            dict.set_item(
                "expiration_time",
                expiration_time
                    .as_ref()
                    .map(|(date, time)| date_time(date, time)),
            )?;
        }
    }
    Ok(dict)
}

#[cfg(test)]
#[path = "notification_parameters_py_tests.rs"]
mod tests;
