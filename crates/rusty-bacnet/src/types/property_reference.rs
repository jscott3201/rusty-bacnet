//! Python boundary for `BACnetDeviceObjectPropertyReference` values: the
//! members of a Channel (#1262) and of a Trend Log Multiple (#1235), a Trend
//! Log's reference (#1480), and an access rule's time range (#1316).
//!
//! A member list takes, per element, either a tuple naming a property in this
//! device, `(object, property)` or `(object, property, array_index)`, or a
//! `DeviceObjectPropertyReference` mapping, which can also carry a
//! `device_identifier`. A time range is a mapping. This layer checks shapes,
//! Python types and the device member (a non-Device raises ValueError, as for
//! `door_members`, #1285); the object taking the references decides the
//! rest, such as how many it holds.

use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, device_identifier_is_device};
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;
use pyo3::exceptions::{PyOverflowError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyMapping, PyTuple};

use super::PyPropertyIdentifier;
use super::mapping::{
    fixed_integer, mapping, object_identifier, optional_item, required_item, validate_keys,
};

const REQUIRED: &[&str] = &["object_identifier", "property_identifier"];
const OPTIONAL: &[&str] = &["property_array_index", "device_identifier"];

/// Read `value`, a list whose elements are each a reference tuple or a
/// reference mapping, in order. `name` labels the errors.
pub(crate) fn property_references_from_py(
    value: &Bound<'_, PyAny>,
    name: &str,
) -> PyResult<Vec<BACnetDeviceObjectPropertyReference>> {
    let items: Vec<Bound<'_, PyAny>> = value
        .extract()
        .map_err(|_| PyTypeError::new_err(format!("{name} must be a list")))?;
    items
        .iter()
        .enumerate()
        .map(|(index, item)| tuple_or_mapping(item, &format!("{name}[{index}]")))
        .collect()
}

/// Read one reference, a tuple or a mapping as each element of
/// [`property_references_from_py`] is. `name` labels the errors.
pub(crate) fn property_reference_from_py(
    value: &Bound<'_, PyAny>,
    name: &str,
) -> PyResult<BACnetDeviceObjectPropertyReference> {
    tuple_or_mapping(value, name)
}

fn tuple_or_mapping(
    value: &Bound<'_, PyAny>,
    name: &str,
) -> PyResult<BACnetDeviceObjectPropertyReference> {
    if let Ok(tuple) = value.cast::<PyTuple>() {
        return local_tuple(tuple, name);
    }
    if value.cast::<PyMapping>().is_ok() {
        return property_reference(value, name);
    }
    Err(PyTypeError::new_err(format!(
        "{name} must be an (object, property) or (object, property, array_index) tuple, \
         or a mapping"
    )))
}

/// A property of an object in this device. An array index outside
/// unsigned32 raises OverflowError, as `add_group`'s indexes do.
fn local_tuple(
    tuple: &Bound<'_, PyTuple>,
    name: &str,
) -> PyResult<BACnetDeviceObjectPropertyReference> {
    if !(2..=3).contains(&tuple.len()) {
        return Err(PyTypeError::new_err(format!(
            "{name} must have 2 or 3 items, got {}",
            tuple.len()
        )));
    }
    let object = object_identifier(&tuple.get_item(0)?, &format!("{name} object"))?;
    let property = property_identifier(&tuple.get_item(1)?, &format!("{name} property"))?;
    let reference = BACnetDeviceObjectPropertyReference::new_local(object, property);
    if tuple.len() == 2 {
        return Ok(reference);
    }
    let index = tuple.get_item(2)?;
    if index.is_none() {
        return Ok(reference);
    }
    let index = index.extract::<u32>().map_err(|error| {
        if error.is_instance_of::<PyOverflowError>(tuple.py()) {
            error
        } else {
            PyTypeError::new_err(format!("{name} array index must be an int or None"))
        }
    })?;
    Ok(reference.with_index(index))
}

/// Read one `DeviceObjectPropertyReference` mapping: `object_identifier` and
/// `property_identifier`, with the optional `property_array_index` and
/// `device_identifier`. `name` labels the errors.
pub(crate) fn property_reference(
    value: &Bound<'_, PyAny>,
    name: &str,
) -> PyResult<BACnetDeviceObjectPropertyReference> {
    let value = mapping(value, name)?;
    validate_keys(value, name, REQUIRED, OPTIONAL)?;
    let field = |key: &str| format!("{name}.{key}");
    let property_identifier = property_identifier(
        &required_item(value, name, "property_identifier")?,
        &field("property_identifier"),
    )?;
    let device_identifier = optional_item(value, "device_identifier")?
        .map(|item| object_identifier(&item, &field("device_identifier")))
        .transpose()?;
    check_device(device_identifier, name)?;
    Ok(BACnetDeviceObjectPropertyReference {
        object_identifier: object_identifier(
            &required_item(value, name, "object_identifier")?,
            &field("object_identifier"),
        )?,
        property_identifier,
        property_array_index: optional_item(value, "property_array_index")?
            .map(|item| fixed_integer::<u32>(&item, &field("property_array_index")))
            .transpose()?,
        device_identifier,
    })
}

/// The raw identifier of a `PropertyIdentifier`; anything else is a
/// TypeError.
fn property_identifier(value: &Bound<'_, PyAny>, name: &str) -> PyResult<u32> {
    value
        .extract::<PyPropertyIdentifier>()
        .map(|property| property.to_rust().to_raw())
        .map_err(|_| PyTypeError::new_err(format!("{name} must be a PropertyIdentifier")))
}

/// ValueError for a device member that isn't a Device object identifier.
pub(crate) fn check_device(device: Option<ObjectIdentifier>, name: &str) -> PyResult<()> {
    if device_identifier_is_device(device) {
        Ok(())
    } else {
        Err(PyValueError::new_err(format!(
            "{name}: the device must be a Device object identifier"
        )))
    }
}

/// The Device identifier a reference names to stay inside a server whose
/// Device is `instance`. The rule is `ObjectDatabase::local_device`'s: the
/// wildcard instance 4194303 names no particular device, so it has none.
pub(crate) fn local_device(instance: u32) -> Option<ObjectIdentifier> {
    ObjectIdentifier::new(ObjectType::DEVICE, instance)
        .ok()
        .filter(|device| device.instance_number() != ObjectIdentifier::WILDCARD_INSTANCE)
}

/// Drop the Device member of each reference that names `local`, so a member
/// pointing inside the server is stored in the local form the server gives
/// the same member written over the network. Other references are left as
/// they are, for the object to judge.
pub(crate) fn localize(
    references: &mut [BACnetDeviceObjectPropertyReference],
    local: Option<ObjectIdentifier>,
) {
    let Some(local) = local else {
        return;
    };
    for reference in references {
        if reference.device_identifier == Some(local) {
            reference.device_identifier = None;
        }
    }
}

#[cfg(test)]
#[path = "property_reference_tests.rs"]
mod tests;
