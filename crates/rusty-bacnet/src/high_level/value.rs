//! [`PyValue`]: a value a high-level call read or sent, in both forms.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::PropertyValue;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList, PyString, PyTuple};

use crate::types::{PyPropertyValue, decode_read_value};

/// One value as a high-level call read it, or sent it: `value`, the typed
/// form a `read_property` of the same property gives; `text`, as the
/// `bacnet` tool writes it; `data`, the shape of the tool's JSON; and
/// `raw`, the octets on the wire.
///
/// A value class: two are equal when they hold the same octets, text and
/// data, they hash alike, and one copies and pickles, rebuilt from those
/// parts and the property it is a value of. It holds nothing
/// of the client or the device it came from.
#[pyclass(name = "Value", module = "rusty_bacnet", frozen)]
pub(crate) struct PyValue {
    /// What the value is a value of: it decides how `raw` decodes.
    object_type: ObjectType,
    property: PropertyIdentifier,
    index: Option<u32>,
    raw: Vec<u8>,
    text: String,
    /// The formatter's typed data, as its JSON.
    data: serde_json::Value,
    /// `raw` decoded as the binding decodes a read of the same property,
    /// done once.
    typed: PyPropertyValue,
}

impl PyValue {
    /// `value`, a value of `property` (at `index` within it) of an object
    /// of `object_type`.
    pub(crate) fn new(
        object_type: ObjectType,
        property: PropertyIdentifier,
        index: Option<u32>,
        value: &bacnet_ops::Value,
    ) -> Self {
        Self::from_parts(
            object_type,
            property,
            index,
            value.raw.clone(),
            value.text.clone(),
            json_of(&value.data),
        )
    }

    fn from_parts(
        object_type: ObjectType,
        property: PropertyIdentifier,
        index: Option<u32>,
        raw: Vec<u8>,
        text: String,
        data: serde_json::Value,
    ) -> Self {
        // The octets came from a decoded answer, or were encoded here, so
        // their framing holds; the raw octets are the fallback the other
        // read results keep.
        let typed = decode_read_value(object_type, property, index, &raw).unwrap_or_else(|_| {
            PyPropertyValue::from_rust(PropertyValue::ApplicationData(raw.clone()))
        });
        Self {
            object_type,
            property,
            index,
            raw,
            text,
            data,
            typed,
        }
    }

    /// The text, as Rust code here reads it.
    pub(crate) fn text(&self) -> &str {
        &self.text
    }
}

#[pymethods]
impl PyValue {
    /// The value as a `PropertyValue`, shaped as `read_property` of the
    /// same property returns it.
    #[getter]
    pub(crate) fn value(&self) -> PyPropertyValue {
        self.typed.clone()
    }

    /// The value as text: every digit, a state or enumeration by its name,
    /// and the object's units or state text when the same call read them.
    #[getter(text)]
    fn text_py(&self) -> &str {
        &self.text
    }

    /// The value as plain data, the shape of the `bacnet` tool's JSON:
    /// None, a bool, an int, a float or a str, or a dict or list of them
    /// (an enumeration is `{"value": 2, "name": "..."}`). A new copy each
    /// time.
    #[getter]
    fn data(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        json_to_py(py, &self.data)
    }

    /// The value's octets as the device sent them, or as a write sends
    /// them: application-tagged.
    #[getter]
    fn raw<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.raw)
    }

    fn __str__(&self) -> &str {
        &self.text
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let text = PyString::new(py, &self.text).repr()?;
        Ok(format!("Value({text})"))
    }

    /// Equal to another `Value` with the same octets, text and data, as
    /// the Rust `Value` compares; what it is a value of isn't compared.
    /// NotImplemented for anything else, so Python tries the other side.
    fn __eq__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> Py<PyAny> {
        match other.cast::<Self>() {
            Ok(other) => {
                let other = other.get();
                let equal =
                    (&self.raw, &self.text, &self.data) == (&other.raw, &other.text, &other.data);
                pyo3::types::PyBool::new(py, equal)
                    .to_owned()
                    .into_any()
                    .unbind()
            }
            Err(_) => py.NotImplemented(),
        }
    }

    fn __hash__(&self, py: Python<'_>) -> PyResult<isize> {
        (PyBytes::new(py, &self.raw), &self.text)
            .into_pyobject(py)?
            .hash()
    }

    /// Copy and pickle by the parts `_rebuild` takes: nothing here is live.
    fn __reduce__<'py>(
        slf: &Bound<'py, Self>,
    ) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyTuple>)> {
        let py = slf.py();
        let value = slf.get();
        let args = (
            value.object_type.to_raw(),
            value.property.to_raw(),
            value.index,
            PyBytes::new(py, &value.raw),
            &value.text,
            value.data.to_string(),
        )
            .into_pyobject(py)?;
        Ok((slf.get_type().getattr("_rebuild")?, args))
    }

    /// A value from a pickle's parts: the object type and property it is a
    /// value of, its octets, its text and its data as JSON. Octets that
    /// don't decode for that property read back as raw application data,
    /// as they did when first read; data that isn't JSON raises ValueError.
    #[staticmethod]
    fn _rebuild(
        object_type: u32,
        property: u32,
        index: Option<u32>,
        raw: &Bound<'_, PyBytes>,
        text: String,
        data: &str,
    ) -> PyResult<Self> {
        let data = serde_json::from_str(data)
            .map_err(|error| PyValueError::new_err(format!("Value data is not JSON: {error}")))?;
        Ok(Self::from_parts(
            ObjectType::from_raw(object_type),
            PropertyIdentifier::from_raw(property),
            index,
            raw.as_bytes().to_vec(),
            text,
            data,
        ))
    }
}

/// `json` as plain Python data: the shape the `bacnet` tool writes, a REAL
/// as the shortest decimal that reads back as it.
fn json_to_py(py: Python<'_>, json: &serde_json::Value) -> PyResult<Py<PyAny>> {
    use serde_json::Value as Json;
    Ok(match json {
        Json::Null => py.None(),
        Json::Bool(value) => value.into_pyobject(py)?.to_owned().into_any().unbind(),
        Json::Number(number) => match (number.as_u64(), number.as_i64()) {
            (Some(value), _) => value.into_pyobject(py)?.into_any().unbind(),
            (None, Some(value)) => value.into_pyobject(py)?.into_any().unbind(),
            _ => number
                .as_f64()
                .unwrap_or(f64::NAN)
                .into_pyobject(py)?
                .into_any()
                .unbind(),
        },
        Json::String(text) => text.into_pyobject(py)?.into_any().unbind(),
        Json::Array(items) => {
            let list = PyList::empty(py);
            for item in items {
                list.append(json_to_py(py, item)?)?;
            }
            list.into_any().unbind()
        }
        Json::Object(fields) => {
            let dict = PyDict::new(py);
            for (key, value) in fields {
                dict.set_item(key, json_to_py(py, value)?)?;
            }
            dict.into_any().unbind()
        }
    })
}

/// `datum` as its JSON, through the text the `bacnet` tool writes: a REAL
/// is the shortest decimal that reads back as it (0.1, where widening the
/// f32 would give 0.10000000149011612). A `Datum` always has a JSON form;
/// one that somehow had none would read as None.
fn json_of(datum: &bacnet_ops::format::Datum) -> serde_json::Value {
    serde_json::to_string(datum)
        .and_then(|text| serde_json::from_str(&text))
        .unwrap_or(serde_json::Value::Null)
}

/// `datum` as plain Python data, for the tests.
#[cfg(test)]
pub(crate) fn data_to_py(py: Python<'_>, datum: &bacnet_ops::format::Datum) -> PyResult<Py<PyAny>> {
    json_to_py(py, &json_of(datum))
}
