//! [`PyValue`]: a value a high-level call read or sent, in both forms.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::PropertyValue;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyString};

use crate::types::{PyPropertyValue, decode_read_value};

/// One value as a high-level call read it, or sent it: `value`, the typed
/// form a `read_property` of the same property gives; `text`, as the
/// `bacnet` tool writes it; and `data`, the shape of the tool's JSON.
/// Refuses `copy` and `pickle`.
#[pyclass(name = "Value", module = "rusty_bacnet", frozen)]
pub(crate) struct PyValue {
    inner: bacnet_ops::Value,
    /// `inner`'s octets decoded as the binding decodes a read of the same
    /// property, done once.
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
        // The octets came from a decoded answer, or were encoded here, so
        // their framing holds; the raw octets are the fallback the other
        // read results keep.
        let typed =
            decode_read_value(object_type, property, index, &value.raw).unwrap_or_else(|_| {
                PyPropertyValue::from_rust(PropertyValue::ApplicationData(value.raw.clone()))
            });
        Self {
            inner: value.clone(),
            typed,
        }
    }

    /// The text, as Rust code here reads it.
    pub(crate) fn text(&self) -> &str {
        &self.inner.text
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

    /// The value as text: every digit, a state or enumeration by its name.
    #[getter(text)]
    fn text_py(&self) -> &str {
        &self.inner.text
    }

    /// The value as plain data, the shape of the `bacnet` tool's JSON:
    /// None, a bool, an int, a float or a str, or a dict or list of them
    /// (an enumeration is `{"value": 2, "name": "..."}`). A new copy each
    /// time.
    #[getter]
    fn data(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        data_to_py(py, &self.inner.data)
    }

    fn __str__(&self) -> &str {
        &self.inner.text
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let text = PyString::new(py, &self.inner.text).repr()?;
        Ok(format!("Value({text})"))
    }

    /// Refuses `copy` and `pickle`, as the other received values do. See
    /// [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}

/// `datum` as plain Python data, through its JSON: the shape the `bacnet`
/// tool writes, a REAL as the shortest decimal that reads back as it.
pub(crate) fn data_to_py(py: Python<'_>, datum: &bacnet_ops::format::Datum) -> PyResult<Py<PyAny>> {
    let json = serde_json::to_string(datum)
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text))
        .map_err(|error| PyValueError::new_err(error.to_string()))?;
    json_to_py(py, &json)
}

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
