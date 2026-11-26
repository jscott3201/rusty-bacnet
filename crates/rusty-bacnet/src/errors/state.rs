//! `BacnetStateError` (#1833): a call refused for the state its component
//! was in, such as a send on a transport that isn't started or has
//! stopped, a second start, or a BACnet/SC node whose hub isn't connected.
//! Nothing was done.
//!
//! It derives from `BacnetError` and Python's `RuntimeError`, so
//! `except BacnetError` and `except RuntimeError` both catch it. The
//! bindings' own lifecycle checks raise it too (#2819), through
//! [`lifecycle_error`]: a client, server, endpoint, router or hub called
//! before its start, during or after its stop, or started twice. Those
//! raised a plain `RuntimeError` before, which this class still is. `state`
//! names the state in the stable form [`Lifecycle::name`] gives. It is built
//! at first use, as `BacnetTransportError` is, because pyo3's
//! `create_exception!` takes one base.

use bacnet_types::error::Lifecycle;
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyDict, PyTuple, PyType};

use super::BacnetError;

static STATE_ERROR: PyOnceLock<Py<PyType>> = PyOnceLock::new();

/// `BacnetStateError`.
pub(crate) fn state_error_type(py: Python<'_>) -> PyResult<&Py<PyType>> {
    STATE_ERROR.get_or_try_init(py, || {
        let builtins = py.import("builtins")?;
        let bases = PyTuple::new(
            py,
            [
                py.get_type::<BacnetError>().into_any(),
                builtins.getattr("RuntimeError")?,
            ],
        )?;
        let namespace = PyDict::new(py);
        namespace.set_item("__module__", "rusty_bacnet")?;
        namespace.set_item("state", py.None())?;
        namespace.set_item(
            "__doc__",
            "A call refused for the state its component was in: a transport, client, server, \
             endpoint, router or hub not started or stopped, a second start, a closed channel, \
             a BACnet/SC node whose hub isn't connected. Nothing was done. Also a \
             RuntimeError. `state` is one of \
             'already_started', 'not_running', 'stopping', 'not_connected' or 'closed'.",
        )?;
        let class = builtins
            .getattr("type")?
            .call1(("BacnetStateError", bases, namespace))?;
        Ok(class.cast_into::<PyType>()?.unbind())
    })
}

/// `BacnetStateError` with `message`, its `state` set to `state`'s name,
/// for the Rust stack's [`Error::State`](bacnet_types::error::Error::State).
/// Falls back to a plain `BacnetError` when Python is finalizing.
pub(crate) fn state_error(message: String, state: Lifecycle) -> PyErr {
    build(message, state).unwrap_or_else(BacnetError::new_err)
}

/// `BacnetStateError` for one of the bindings' own lifecycle checks
/// (#2819), with the text that check has always raised and `state` chosen
/// as the Rust stack chooses it: `NotRunning` for a call before start or
/// after a stop the object can start again from, `AlreadyStarted` for a
/// second start or a call only an unstarted object takes, `Stopping` while
/// a stop is under way, `Closed` once it can't start again. Falls back to a
/// plain `RuntimeError`, what these checks raised before, when Python is
/// finalizing.
pub(crate) fn lifecycle_error(state: Lifecycle, message: impl Into<String>) -> PyErr {
    build(message.into(), state).unwrap_or_else(pyo3::exceptions::PyRuntimeError::new_err)
}

/// The exception, or `message` back when Python can't be reached. Binding
/// threads call this, so it attaches through the exit gate.
fn build(message: String, state: Lifecycle) -> Result<PyErr, String> {
    let fallback = message.clone();
    crate::py_async::attach(|py| {
        let error = state_error_type(py)?.bind(py).call1((message,))?;
        error.setattr("state", state.name())?;
        Ok(PyErr::from_value(error))
    })
    .map_err(|_| fallback)
}

/// Add the class to the module.
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add("BacnetStateError", state_error_type(py)?.bind(py))
}
