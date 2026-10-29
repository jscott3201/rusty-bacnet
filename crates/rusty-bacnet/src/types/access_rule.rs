//! Python mapping boundary for an Access Rights object's rule arrays
//! (#1316).
//!
//! Each rule is an `AccessRule` mapping. A missing or `None` time range or
//! location stands for ALWAYS or ALL, and a given one for SPECIFIED. This
//! layer checks shapes, Python types and the device member of each reference
//! (a non-Device raises ValueError, as for `door_members`, #1285);
//! `AccessRightsObject`'s setters decide the rest, such as a location that is
//! neither an Access Point nor an Access Zone.

use bacnet_types::constructed::{BACnetAccessRule, BACnetDeviceObjectReference};
use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBool};

use super::PyObjectIdentifier;
use super::mapping::{mapping, optional_item, required_item, validate_keys};
use super::property_reference::{check_device, property_reference};

const RULE_REQUIRED: &[&str] = &["enable"];
const RULE_OPTIONAL: &[&str] = &["time_range", "location"];

/// Read `rules`, a list of `AccessRule` mappings; `name` is the keyword the
/// errors name.
pub(crate) fn access_rules_from_py(
    rules: &Bound<'_, PyAny>,
    name: &str,
) -> PyResult<Vec<BACnetAccessRule>> {
    let rules: Vec<Bound<'_, PyAny>> = rules
        .extract()
        .map_err(|_| PyTypeError::new_err(format!("{name} must be a list")))?;
    rules
        .iter()
        .enumerate()
        .map(|(index, rule)| access_rule(rule, &format!("{name}[{index}]")))
        .collect()
}

fn access_rule(value: &Bound<'_, PyAny>, name: &str) -> PyResult<BACnetAccessRule> {
    let value = mapping(value, name)?;
    validate_keys(value, name, RULE_REQUIRED, RULE_OPTIONAL)?;
    let enable = required_item(value, name, "enable")?;
    if !enable.is_instance_of::<PyBool>() {
        return Err(PyTypeError::new_err(format!(
            "{name}.enable must be a bool"
        )));
    }
    let time_range = optional_item(value, "time_range")?
        .map(|item| property_reference(&item, &format!("{name}.time_range")))
        .transpose()?;
    let location = optional_item(value, "location")?
        .map(|item| device_object_reference(&item, &format!("{name}.location")))
        .transpose()?;
    Ok(BACnetAccessRule::new(
        time_range,
        location,
        enable.extract()?,
    ))
}

/// One BACnetDeviceObjectReference, a rule's location or an Access Rights
/// object's Accompaniment (#1393): an `ObjectIdentifier` in this device or a
/// `(device, object)` pair, the forms `door_members` takes. `name` is what
/// the errors name.
pub(crate) fn device_object_reference(
    value: &Bound<'_, PyAny>,
    name: &str,
) -> PyResult<BACnetDeviceObjectReference> {
    if let Ok(object) = value.extract::<PyObjectIdentifier>() {
        return Ok(object.to_rust().into());
    }
    let (device, object) = value
        .extract::<(PyObjectIdentifier, PyObjectIdentifier)>()
        .map_err(|_| {
            PyTypeError::new_err(format!(
                "{name} must be an ObjectIdentifier or a (device, object) pair"
            ))
        })?;
    let device_identifier = Some(device.to_rust());
    check_device(device_identifier, name)?;
    Ok(BACnetDeviceObjectReference {
        device_identifier,
        object_identifier: object.to_rust(),
    })
}

#[cfg(test)]
#[path = "access_rule_tests.rs"]
mod tests;
