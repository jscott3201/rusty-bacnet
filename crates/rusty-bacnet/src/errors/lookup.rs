//! The lookup errors (#2561): a device that can't be placed.
//!
//! `BacnetLookupError` derives from `BacnetError` and Python's
//! `LookupError`, so `except LookupError` catches a name that finds
//! nothing, as it does for a missing key. `BacnetAmbiguousDeviceError`
//! (#2560) is its subclass: two or more addresses claim the instance.
//! Both are built at first use, as `BacnetTransportError` is, because
//! pyo3's `create_exception!` takes one base.

use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyDict, PyTuple, PyType};

use super::BacnetError;

static LOOKUP_ERROR: PyOnceLock<Py<PyType>> = PyOnceLock::new();
static AMBIGUOUS_ERROR: PyOnceLock<Py<PyType>> = PyOnceLock::new();

/// The attributes both classes carry, None on the class.
const ATTRIBUTES: [&str; 3] = ["target", "instance", "candidates"];

fn class(py: Python<'_>, name: &str, bases: Bound<'_, PyTuple>, doc: &str) -> PyResult<Py<PyType>> {
    let namespace = PyDict::new(py);
    namespace.set_item("__module__", "rusty_bacnet")?;
    namespace.set_item("__doc__", doc)?;
    for attribute in ATTRIBUTES {
        namespace.set_item(attribute, py.None())?;
    }
    let class = py
        .import("builtins")?
        .getattr("type")?
        .call1((name, bases, namespace))?;
    Ok(class.cast_into::<PyType>()?.unbind())
}

/// `BacnetLookupError`.
pub(crate) fn lookup_error_type(py: Python<'_>) -> PyResult<&Py<PyType>> {
    LOOKUP_ERROR.get_or_try_init(py, || {
        let bases = PyTuple::new(
            py,
            [
                py.get_type::<BacnetError>().into_any(),
                py.import("builtins")?.getattr("LookupError")?,
            ],
        )?;
        class(
            py,
            "BacnetLookupError",
            bases,
            "No device could be placed for the name given: no address claims it and none \
             answered a Who-Is for it, or no router is known for its network. Also a \
             LookupError. Nothing was sent to the device.",
        )
    })
}

/// `BacnetAmbiguousDeviceError`, a `BacnetLookupError`.
pub(crate) fn ambiguous_error_type(py: Python<'_>) -> PyResult<&Py<PyType>> {
    AMBIGUOUS_ERROR.get_or_try_init(py, || {
        let bases = PyTuple::new(py, [lookup_error_type(py)?.bind(py)])?;
        class(
            py,
            "BacnetAmbiguousDeviceError",
            bases,
            "Two or more addresses claim the device instance named: name one of \
             `candidates` by its address. Nothing was sent.",
        )
    })
}

/// What a lookup error says besides its message.
pub(crate) struct Lookup {
    /// The device as it was named, when the error says.
    pub(crate) target: Option<String>,
    /// The instance two or more addresses claim; `None` for a device no
    /// address claims.
    pub(crate) instance: Option<u32>,
    /// Every address claiming the instance, as (network, MAC), network 0
    /// for this one: a `BACnetClient.device()` target each. Empty for a
    /// device no address claims.
    pub(crate) candidates: Vec<(u16, Vec<u8>)>,
}

/// `BacnetAmbiguousDeviceError` when `lookup` names an instance with
/// candidates, else `BacnetLookupError`. Falls back to a plain
/// `BacnetError` when Python is finalizing.
pub(crate) fn lookup_error(message: String, lookup: Lookup) -> PyErr {
    let fallback = message.clone();
    crate::py_async::attach(|py| {
        let class = match lookup.instance {
            Some(_) => ambiguous_error_type(py)?,
            None => lookup_error_type(py)?,
        };
        let error = class.bind(py).call1((message,))?;
        error.setattr("target", lookup.target)?;
        error.setattr("instance", lookup.instance)?;
        error.setattr("candidates", lookup.candidates)?;
        Ok(PyErr::from_value(error))
    })
    .unwrap_or_else(|_| BacnetError::new_err(fallback))
}

/// Add both classes to the module.
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add("BacnetLookupError", lookup_error_type(py)?.bind(py))?;
    m.add(
        "BacnetAmbiguousDeviceError",
        ambiguous_error_type(py)?.bind(py),
    )
}
