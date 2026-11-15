//! A high-level call's [`OpsError`] as a Python exception (#2561).
//!
//! Text that names nothing a call can use raises ValueError; a device that
//! didn't answer, BacnetTimeoutError or BacnetTransportError; the device's
//! own Error, Reject or Abort, the protocol, reject and abort errors the
//! typed calls raise. Three are the write calls' own:
//!
//! - BacnetWriteDeniedError: the client's write policy refused the
//!   priority, so nothing was sent;
//! - BacnetReadBackError: the device acknowledged the write, which took
//!   effect, but the read after it failed (its `__cause__`);
//! - BacnetNoPriorityArrayError, a BacnetProtocolError: the device says
//!   the object has no Priority_Array.
//!
//! The last two carry the acknowledged write as `written`, so a caller
//! tells "sent" from "not sent" without parsing text, and never sends a
//! relinquish twice.

use bacnet_ops::label::{kebab, object_label};
use bacnet_ops::target::AddressStyle;
use bacnet_ops::{OpsError, WriteDenied, Written};
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::ObjectIdentifier;
use pyo3::create_exception;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;

use super::priorities::PyWritten;
use crate::errors::{
    BacnetError, BacnetProtocolError, BacnetTimeoutError, DETAIL_ATTRIBUTES, set_enum_attr,
    to_py_err,
};
use crate::types::{PyObjectIdentifier, PyPropertyIdentifier};

create_exception!(rusty_bacnet, BacnetWriteDeniedError, BacnetError);
create_exception!(rusty_bacnet, BacnetReadBackError, BacnetError);
create_exception!(
    rusty_bacnet,
    BacnetNoPriorityArrayError,
    BacnetProtocolError
);

/// The attributes BacnetWriteDeniedError carries, None on the class.
const DENIED_ATTRIBUTES: [&str; 5] = [
    "priority",
    "relinquish",
    "object_id",
    "property_id",
    "array_index",
];

/// `error` as the Python exception a high-level call raises, a written
/// device's address in `style`, the client's link's. Futures call this on
/// binding threads, so attributes are set through the exit gate.
pub(crate) fn ops_err(error: OpsError, style: AddressStyle) -> PyErr {
    let message = error.to_string();
    // A request that timed out, however the client reported it (the
    // transaction state machine's abort included), is a timeout.
    if error.timed_out() {
        return BacnetTimeoutError::new_err(message);
    }
    match error {
        OpsError::Parse { reason, .. } => PyValueError::new_err(reason),
        OpsError::Unreachable { source, .. } | OpsError::Client(source) => to_py_err(source),
        OpsError::Refused {
            error: RemoteError::Error { class, code },
            detail: Some(detail),
            ..
        } => to_py_err(Error::Structured {
            class,
            code,
            detail,
        }),
        OpsError::Refused { error, .. } => to_py_err(Error::Remote(error)),
        OpsError::Ambiguous {
            instance,
            candidates,
        } => to_py_err(Error::AmbiguousDevice {
            instance,
            candidates: candidates.into_iter().map(BACnetAddress::from).collect(),
        }),
        OpsError::Denied(denied) => write_denied(&denied),
        // As the binding's own SC startup reports TLS material it refused.
        OpsError::Tls(_) => PyRuntimeError::new_err(format!("TLS config error: {message}")),
        OpsError::ReadBack { written, source } => {
            let error = BacnetReadBackError::new_err(message);
            let cause = ops_err(*source, style);
            let _ = crate::py_async::attach(|py| {
                set_written(py, &error, Some(&written), style);
                error.set_cause(py, Some(cause));
                Ok(())
            });
            error
        }
        OpsError::NoPriorityArray {
            object,
            relinquished,
            ..
        } => no_priority_array(message, object, relinquished.as_deref(), style),
        // UnknownDevice and Paging, which no priority call meets, and any
        // later variant.
        _ => BacnetError::new_err(message),
    }
}

/// BacnetWriteDeniedError for `denied`, saying how to allow the priority.
fn write_denied(denied: &WriteDenied) -> PyErr {
    let priority = denied.priority;
    let what = if denied.relinquish {
        "relinquish"
    } else {
        "write"
    };
    let target = denied.property;
    let index = target.index.map(|i| format!("[{i}]")).unwrap_or_default();
    let error = BacnetWriteDeniedError::new_err(format!(
        "not sent: the client's write policy refuses priority {priority} ({}) for the \
         {what} of {} {}{index}; allow it with \
         BACnetClient(write_policy=WritePolicy().allow_priorities([{priority}]))",
        priority.purpose(),
        object_label(target.object),
        kebab(target.property),
    ));
    let _ = crate::py_async::attach(|py| {
        let value = error.value(py);
        let _ = value.setattr("priority", priority.get());
        let _ = value.setattr("relinquish", denied.relinquish);
        let _ = value.setattr("object_id", PyObjectIdentifier::from_rust(target.object));
        let _ = value.setattr(
            "property_id",
            PyPropertyIdentifier {
                inner: target.property,
            },
        );
        let _ = value.setattr("array_index", target.index);
        Ok(())
    });
    error
}

/// BacnetNoPriorityArrayError, an [`OpsError::NoPriorityArray`] with
/// `message`: a protocol error with the device's PROPERTY /
/// UNKNOWN_PROPERTY, the `object`, and the relinquish acknowledged before
/// the read, if one was.
fn no_priority_array(
    message: String,
    object: ObjectIdentifier,
    relinquished: Option<&Written>,
    style: AddressStyle,
) -> PyErr {
    let raised = BacnetNoPriorityArrayError::new_err(message);
    let _ = crate::py_async::attach(|py| {
        let value = raised.value(py);
        set_enum_attr(value.as_any(), "error_class", ErrorClass::PROPERTY);
        set_enum_attr(value.as_any(), "error_code", ErrorCode::UNKNOWN_PROPERTY);
        // An unknown property's answer has no structured body.
        for name in DETAIL_ATTRIBUTES {
            let _ = value.setattr(name, py.None());
        }
        let _ = value.setattr("object_id", PyObjectIdentifier::from_rust(object));
        set_written(py, &raised, relinquished, style);
        Ok(())
    });
    raised
}

/// Set `written` on `error`: the acknowledged write, its address in
/// `style`, or None.
fn set_written(py: Python<'_>, error: &PyErr, written: Option<&Written>, style: AddressStyle) {
    let written = written
        .map(|written| PyWritten::new(py, written, style).and_then(|w| Py::new(py, w)))
        .transpose();
    match written {
        Ok(Some(written)) => {
            let _ = error.value(py).setattr("written", written);
        }
        Ok(None) => {
            let _ = error.value(py).setattr("written", py.None());
        }
        Err(_) => {}
    }
}

/// Register the three exception types, their attributes None on the class.
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    let denied = py.get_type::<BacnetWriteDeniedError>();
    for name in DENIED_ATTRIBUTES {
        denied.setattr(name, py.None())?;
    }
    m.add("BacnetWriteDeniedError", denied)?;
    let read_back = py.get_type::<BacnetReadBackError>();
    read_back.setattr("written", py.None())?;
    m.add("BacnetReadBackError", read_back)?;
    let none = py.get_type::<BacnetNoPriorityArrayError>();
    none.setattr("object_id", py.None())?;
    none.setattr("written", py.None())?;
    m.add("BacnetNoPriorityArrayError", none)?;
    Ok(())
}
