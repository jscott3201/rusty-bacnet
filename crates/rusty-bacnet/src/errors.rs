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
pub fn to_py_err(err: Error) -> PyErr {
    match err {
        Error::Protocol { class, code } => {
            BacnetProtocolError::new_err(format!("BACnet error: class={class} code={code}"))
        }
        Error::Timeout(_) => BacnetTimeoutError::new_err(err.to_string()),
        Error::Reject { reason } => {
            BacnetRejectError::new_err(format!("BACnet reject: reason={reason}"))
        }
        Error::Abort { reason } => {
            BacnetAbortError::new_err(format!("BACnet abort: reason={reason}"))
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
