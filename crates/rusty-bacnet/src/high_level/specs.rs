//! What a `Device` call's arguments name (#2561): a device ([`Named`]), an
//! object, a property and a value to write. Each takes the grammar's text
//! (`"ai:1"`, `"pv"`, `"priority-array[8]"`, `"72.5@8"`) or the typed form
//! the typed calls take, and is checked when the call is made, before its
//! Future exists: a wrong type raises TypeError and text the grammar
//! refuses ValueError.

use bacnet_client::discovery::DiscoveredDevice;
use bacnet_ops::device::{DeviceRef, Discovered};
use bacnet_ops::parse::parse_property;
use bacnet_ops::write::WriteValue;
use bacnet_ops::{DeviceAddress, IntoObject, IntoWriteValue, OpsError, PropertyRef};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyBytes, PyFloat, PyInt, PyList, PyString, PyTuple};

use super::discovered::PyDiscovered;
use crate::types::{PyDiscoveredDevice, PyObjectIdentifier, PyPropertyIdentifier, PyPropertyValue};

/// The highest device instance a target names: 4194303 is the wildcard.
const MAX_INSTANCE: u32 = 4_194_302;

/// What a Python `Device` was named by, kept so the handle can be made
/// again after its client restarts.
#[derive(Debug, Clone)]
pub(crate) enum Named {
    /// A target in the grammar.
    Text(String),
    /// A device instance.
    Instance(u32),
    /// An address, from a `(network, mac)` pair.
    Address(DeviceAddress),
    /// A `discover()` row.
    Found(Box<Discovered>),
    /// A row of the discovery table.
    Entry(Box<DiscoveredDevice>),
}

impl Named {
    /// The name as `bacnet_ops::Client::device` takes it.
    pub(crate) fn as_ref(&self) -> DeviceRef<'_> {
        match self {
            Self::Text(text) => DeviceRef::Text(text),
            Self::Instance(instance) => DeviceRef::Instance(*instance),
            Self::Address(address) => DeviceRef::Address(address.clone()),
            Self::Found(found) => DeviceRef::Found(found),
            Self::Entry(entry) => DeviceRef::Entry(entry),
        }
    }
}

/// The type's name, for a TypeError.
fn type_name(value: &Bound<'_, PyAny>) -> String {
    value
        .get_type()
        .name()
        .map_or_else(|_| "object".to_owned(), |name| name.to_string())
}

/// A ValueError with a refusal's own words.
fn refused(error: OpsError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// An int that isn't a bool, as a `u32`; `None` for anything else.
fn plain_int<'py>(value: &Bound<'py, PyAny>) -> Option<Bound<'py, PyInt>> {
    (!value.is_instance_of::<PyBool>())
        .then(|| value.cast::<PyInt>().ok().cloned())
        .flatten()
}

/// A `device()` target: text in the target grammar, an int instance, a
/// `discover()` row, a `DiscoveredDevice`, or a `(network, mac)` pair (a
/// lookup error's candidate). Text is checked against the grammar here;
/// whether the client's link can reach what it names is checked on await.
pub(crate) fn target_arg(target: &Bound<'_, PyAny>) -> PyResult<Named> {
    if let Ok(text) = target.cast::<PyString>() {
        let text = text.to_str()?.trim();
        use bacnet_ops::target::{Target, parse_target};
        let named = parse_target(text).map_err(PyValueError::new_err)?;
        // The wildcard instance and past it name no device, as text or int.
        if let Target::Instance(instance) | Target::Routed { instance, .. } = named
            && instance > MAX_INSTANCE
        {
            return Err(PyValueError::new_err(format!(
                "a device instance is 0 to {MAX_INSTANCE}, not {instance}"
            )));
        }
        return Ok(Named::Text(text.to_owned()));
    }
    if let Some(instance) = plain_int(target) {
        return match instance.extract::<u32>() {
            Ok(instance) if instance <= MAX_INSTANCE => Ok(Named::Instance(instance)),
            _ => Err(PyValueError::new_err(format!(
                "a device instance is 0 to {MAX_INSTANCE}, not {target}"
            ))),
        };
    }
    if let Ok(found) = target.cast::<PyDiscovered>() {
        return Ok(Named::Found(Box::new(found.get().found().clone())));
    }
    if let Ok(entry) = target.cast::<PyDiscoveredDevice>() {
        return Ok(Named::Entry(Box::new(entry.get().rust().clone())));
    }
    if let Ok(pair) = target.cast::<PyTuple>()
        && pair.len() == 2
        && let Some(network) = plain_int(&pair.get_item(0)?)
        && let Ok(mac) = pair.get_item(1)?.cast::<PyBytes>()
    {
        let network = network.extract::<u16>().map_err(|_| {
            PyValueError::new_err("a (network, mac) target's network is 0 to 65534")
        })?;
        return DeviceAddress::new(network, mac.as_bytes())
            .map(Named::Address)
            .map_err(PyValueError::new_err);
    }
    Err(PyTypeError::new_err(format!(
        "a device target is text ('192.168.1.10', '2:1234'), an int instance, a discover() \
         row, a DiscoveredDevice or a (network, mac) pair, not {}",
        type_name(target)
    )))
}

/// An object: an `ObjectIdentifier`, or text such as `"av:1"`.
pub(crate) fn object_arg(object: &Bound<'_, PyAny>) -> PyResult<ObjectIdentifier> {
    if let Ok(object) = object.cast::<PyObjectIdentifier>() {
        return Ok(object.get().to_rust());
    }
    if let Ok(text) = object.cast::<PyString>() {
        return text.to_str()?.into_object().map_err(refused);
    }
    Err(PyTypeError::new_err(format!(
        "an object is an ObjectIdentifier or text such as 'ai:1', not {}",
        type_name(object)
    )))
}

/// A property and the element its text names: a `PropertyIdentifier`, or
/// text such as `"pv"` or `"priority-array[8]"`.
fn property_part(property: &Bound<'_, PyAny>) -> PyResult<(PropertyIdentifier, Option<u32>)> {
    if let Ok(property) = property.cast::<PyPropertyIdentifier>() {
        return Ok((property.get().to_rust(), None));
    }
    if let Ok(text) = property.cast::<PyString>() {
        return parse_property(text.to_str()?.trim()).map_err(PyValueError::new_err);
    }
    Err(PyTypeError::new_err(format!(
        "a property is a PropertyIdentifier or text such as 'pv', not {}",
        type_name(property)
    )))
}

/// The property `object`, `property` and `index` name together. With no
/// `property`, `object` may be the whole of it as text (`"ai:1 pv"`), and
/// an object alone is its Present_Value. An element named twice, in the
/// text and by `index`, must be the same one.
pub(crate) fn property_arg(
    object: &Bound<'_, PyAny>,
    property: Option<&Bound<'_, PyAny>>,
    index: Option<u32>,
) -> PyResult<PropertyRef> {
    let property = property.filter(|property| !property.is_none());
    let named = match (property, object.cast::<PyString>()) {
        (None, Ok(text)) => text.to_str()?.parse::<PropertyRef>().map_err(refused)?,
        (None, Err(_)) => PropertyRef::new(object_arg(object)?, PropertyIdentifier::PRESENT_VALUE),
        (Some(property), _) => {
            let object = object_arg(object)?;
            let (property, element) = property_part(property)?;
            PropertyRef {
                object,
                property,
                index: element,
            }
        }
    };
    match (named.index, index) {
        (Some(element), Some(index)) if element != index => Err(PyValueError::new_err(format!(
            "the property names element {element} and index={index}: give one"
        ))),
        (element, index) => Ok(PropertyRef {
            index: element.or(index),
            ..named
        }),
    }
}

/// One item of a `read_many` list: text (`"ai:1 pv"`, `"ai:1"`), an
/// `ObjectIdentifier` (its Present_Value), or an `(object, property)` or
/// `(object, property, index)` tuple or list of the forms above.
pub(crate) fn property_item(item: &Bound<'_, PyAny>) -> PyResult<PropertyRef> {
    let parts: Vec<Bound<'_, PyAny>> = if let Ok(tuple) = item.cast::<PyTuple>() {
        tuple.iter().collect()
    } else if let Ok(list) = item.cast::<PyList>() {
        list.iter().collect()
    } else {
        return property_arg(item, None, None);
    };
    match parts.as_slice() {
        [object, property] => property_arg(object, Some(property), None),
        [object, property, index] => {
            let index = plain_int(index)
                .and_then(|index| index.extract::<u32>().ok())
                .ok_or_else(|| {
                    PyValueError::new_err("an (object, property, index) index is an int from 0")
                })?;
            property_arg(object, Some(property), Some(index))
        }
        _ => Err(PyValueError::new_err(format!(
            "a property is (object, property) or (object, property, index), not {} items",
            parts.len()
        ))),
    }
}

/// A value to write: text in the value grammar, read for the property's
/// datatype when the call runs (it may end in `@N`, a priority), or a
/// typed `PropertyValue`, sent as it is.
pub(crate) enum WriteArg {
    /// Text, or a number's text.
    Text(String),
    /// A typed value.
    Typed(PropertyValue),
}

impl IntoWriteValue for WriteArg {
    fn into_write_value(self, target: &PropertyRef) -> Result<WriteValue, OpsError> {
        match self {
            Self::Text(text) => text.into_write_value(target),
            Self::Typed(value) => value.into_write_value(target),
        }
    }
}

/// A `value` argument: a str, a `PropertyValue`, or an int or float, which
/// is read as its decimal text for the property's datatype (`72.5` writes
/// REAL 72.5 to an analog Present_Value). A bool raises TypeError: it
/// could mean a BOOLEAN, a binary state or 1, so say which (`"active"`,
/// `PropertyValue.boolean(True)`).
pub(crate) fn value_arg(value: &Bound<'_, PyAny>) -> PyResult<WriteArg> {
    if let Ok(text) = value.cast::<PyString>() {
        return Ok(WriteArg::Text(text.to_str()?.to_owned()));
    }
    if let Ok(typed) = value.cast::<PyPropertyValue>() {
        return Ok(WriteArg::Typed(typed.get().inner.clone()));
    }
    let number = plain_int(value).is_some() || value.is_instance_of::<PyFloat>();
    if number {
        return Ok(WriteArg::Text(value.str()?.to_str()?.to_owned()));
    }
    Err(PyTypeError::new_err(format!(
        "a value to write is text ('72.5', 'active', '72.5@8'), an int, a float or a \
         PropertyValue, not {}",
        type_name(value)
    )))
}
