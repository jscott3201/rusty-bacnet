//! A high-level call's [`OpsError`] as a Python exception (#2561).
//!
//! Text that names nothing a call can use raises ValueError; a device that
//! didn't answer, BacnetTimeoutError or BacnetTransportError; the device's
//! own Error, Reject or Abort, the protocol, reject and abort errors the
//! typed calls raise; a device that can't be placed, BacnetLookupError, or
//! its subclass BacnetAmbiguousDeviceError when two addresses claim it; a
//! device that breaks a paged read's rules, BacnetPagingError; and a watch
//! the client's hub has no room for, RuntimeError. Three are the write
//! calls' own:
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

use bacnet_ops::error::PagingFault;
use bacnet_ops::label::{kebab, object_label};
use bacnet_ops::target::AddressStyle;
use bacnet_ops::watch::WatchError;
use bacnet_ops::{OpsError, Scope, WriteDenied, Written};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::{Error, Lifecycle, RemoteError};
use bacnet_types::primitives::ObjectIdentifier;
use pyo3::create_exception;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;

use super::priorities::PyWritten;
use crate::errors::{
    BacnetError, BacnetProtocolError, BacnetTimeoutError, DETAIL_ATTRIBUTES, Lookup,
    lifecycle_error, lookup_error, set_enum_attr, to_py_err,
};
use crate::types::{PyObjectIdentifier, PyPropertyIdentifier};

create_exception!(rusty_bacnet, BacnetWriteDeniedError, BacnetError);
create_exception!(rusty_bacnet, BacnetReadBackError, BacnetError);
create_exception!(rusty_bacnet, BacnetPagingError, BacnetError);
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
        let timeout = BacnetTimeoutError::new_err(message);
        // After a router said it was busy, the timeout names it (#2724).
        if let OpsError::Unreachable {
            source: Error::RouterBusyTimeout(busy),
            ..
        }
        | OpsError::Client(Error::RouterBusyTimeout(busy)) = &error
        {
            let _ = crate::py_async::attach(|py| {
                crate::errors::set_busy(timeout.value(py).as_any(), busy);
                Ok(())
            });
        }
        return timeout;
    }
    match error {
        OpsError::Parse { reason, .. } => PyValueError::new_err(super::link_hint(reason)),
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
        } => lookup_error(
            message,
            Lookup::ambiguous(
                instance,
                candidates
                    .iter()
                    .map(|address| (address.network(), address.mac().to_vec()))
                    .collect(),
            ),
        ),
        OpsError::ObjectList { source, .. } => object_list(message, *source, style),
        // A write through a following handle not sent (#2795): its
        // addresses in the link's own form in the message, and as
        // (network, MAC) in `at` and `candidate`, an unconfirmed hint kept
        // out of `candidates`, which are real choices.
        OpsError::NotConfirmed {
            instance,
            at,
            candidate,
        } => {
            let mut text = format!(
                "device {instance} isn't confirmed at {}, where the handle reaches it \
                 (another device or none answers there, or no route to it is known), and a \
                 write never goes elsewhere on its own",
                at.styled(style)
            );
            if let Some(candidate) = &candidate {
                text.push_str(&format!(
                    "; the discovery table has it at {}",
                    candidate.styled(style)
                ));
            }
            text.push_str(": read through the handle first, which looks for the device");
            lookup_error(
                text,
                Lookup {
                    target: Some(instance.to_string()),
                    instance: Some(instance),
                    ambiguous: false,
                    candidates: Vec::new(),
                    at: Some((at.network(), at.mac().to_vec())),
                    candidate: candidate.map(|address| (address.network(), address.mac().to_vec())),
                },
            )
        }
        OpsError::UnknownDevice { target, asked } => lookup_error(
            match asked {
                // Ops' own text names the Rust call that widens the scope.
                Some(Scope::Local) => format!(
                    "device {target} did not answer a local Who-Is, the only one asked: \
                     name its network in the target ('5:{target}' for network 5), or \
                     give BACnetClient resolve_scope='global' or a network number"
                ),
                _ => message,
            },
            Lookup::unknown(target),
        ),
        OpsError::Paging(fault) => paging(message, fault),
        // A hub shut or a watch closed: the binding's own lifecycle state
        // (#2819), `BacnetStateError`, still a RuntimeError.
        OpsError::Watch(WatchError::Closed | WatchError::Shut) => {
            lifecycle_error(Lifecycle::Closed, message)
        }
        // The hub's own limit, or no runtime: nothing was watched, and the
        // device said nothing.
        OpsError::Watch(_) => PyRuntimeError::new_err(message),
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
        // `OpsError` is non-exhaustive: a variant added after this was
        // written. `every_variant_is_mapped` (the tests) lists the ones
        // mapped here, so a new one is noticed when that list is updated.
        _ => BacnetError::new_err(message),
    }
}

/// An [`OpsError::ObjectList`]: the exception its `source` raises, of the
/// same class and with its attributes, saying which element of the
/// Object_List failed. An element's read that timed out is a
/// BacnetTimeoutError before this is reached.
fn object_list(message: String, source: OpsError, style: AddressStyle) -> PyErr {
    let cause = ops_err(source, style);
    let fallback = message.clone();
    crate::py_async::attach(|py| {
        let class = cause.get_type(py);
        let raised = class.call1((message,))?;
        if let (Ok(from), Ok(to)) = (
            cause.value(py).getattr("__dict__"),
            raised.getattr("__dict__"),
        ) {
            to.call_method1("update", (from,))?;
        }
        Ok(PyErr::from_value(raised))
    })
    .unwrap_or_else(|_| BacnetError::new_err(fallback))
}

/// BacnetPagingError for `fault`, with the paged service and, for a
/// repeated object, the object.
fn paging(message: String, fault: PagingFault) -> PyErr {
    let error = BacnetPagingError::new_err(message);
    let (service, object) = match fault {
        PagingFault::Repeated { service, object } => (service, Some(object)),
        PagingFault::Stalled { service } => (service, None),
        // Non-exhaustive: a later fault says what it can.
        _ => ("", None),
    };
    let _ = crate::py_async::attach(|py| {
        let value = error.value(py);
        let _ = value.setattr("service", service);
        let _ = value.setattr("object_id", object.map(PyObjectIdentifier::from_rust));
        Ok(())
    });
    error
}

/// BacnetWriteDeniedError for `denied`, saying how to allow the priority.
pub(super) fn write_denied(denied: &WriteDenied) -> PyErr {
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

/// Register the exception types, their attributes None on the class.
pub(super) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    let paging = py.get_type::<BacnetPagingError>();
    paging.setattr("service", py.None())?;
    paging.setattr("object_id", py.None())?;
    m.add("BacnetPagingError", paging)?;
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
