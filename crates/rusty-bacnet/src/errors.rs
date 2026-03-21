//! Python exception types mapping Rust BACnet errors.

use bacnet_types::error::Error;
use pyo3::create_exception;
use pyo3::prelude::*;

// Exception hierarchy: BacnetError (base) with subtypes
create_exception!(rusty_bacnet, BacnetError, pyo3::exceptions::PyException);
create_exception!(rusty_bacnet, BacnetProtocolError, BacnetError);
create_exception!(rusty_bacnet, BacnetTimeoutError, BacnetError);
create_exception!(rusty_bacnet, BacnetRejectError, BacnetError);
create_exception!(rusty_bacnet, BacnetAbortError, BacnetError);

/// Convert a Rust `Error` into a Python exception.
///
/// Protocol errors, rejects, and aborts carry structured integer attributes
/// (`error_class`/`error_code` or `reason`) so Python callers can inspect them
/// programmatically without parsing the message string.
pub fn to_py_err(err: Error) -> PyErr {
    match err {
        Error::Protocol { class, code } => {
            let py_err =
                BacnetProtocolError::new_err(format!("BACnet error: class={class} code={code}"));
            Python::attach(|py| {
                let val = py_err.value(py);
                let _ = val.setattr("error_class", class);
                let _ = val.setattr("error_code", code);
            });
            py_err
        }
        Error::Timeout(_) => BacnetTimeoutError::new_err(err.to_string()),
        Error::Reject { reason } => {
            let py_err = BacnetRejectError::new_err(format!("BACnet reject: reason={reason}"));
            Python::attach(|py| {
                let val = py_err.value(py);
                let _ = val.setattr("reason", reason);
            });
            py_err
        }
        Error::Abort { reason } => {
            let py_err = BacnetAbortError::new_err(format!("BACnet abort: reason={reason}"));
            Python::attach(|py| {
                let val = py_err.value(py);
                let _ = val.setattr("reason", reason);
            });
            py_err
        }
        _ => BacnetError::new_err(err.to_string()),
    }
}

/// Register exception types with the module.
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("BacnetError", m.py().get_type::<BacnetError>())?;
    m.add(
        "BacnetProtocolError",
        m.py().get_type::<BacnetProtocolError>(),
    )?;
    m.add(
        "BacnetTimeoutError",
        m.py().get_type::<BacnetTimeoutError>(),
    )?;
    m.add("BacnetRejectError", m.py().get_type::<BacnetRejectError>())?;
    m.add("BacnetAbortError", m.py().get_type::<BacnetAbortError>())?;
    Ok(())
}
