//! Python exception types mapping Rust BACnet errors.

use bacnet_transport::bip::BvlcExchangeError;
use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier};
use bacnet_types::error::{Error, ErrorDetail, RemoteError};
use pyo3::create_exception;
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyBytes, PyDict, PyList, PyTuple, PyType};

use crate::types::int_enums::{self, PyIntEnum};
use crate::types::{PyObjectIdentifier, PyPropertyIdentifier};

// Exception hierarchy: BacnetError (base) with subtypes
create_exception!(rusty_bacnet, BacnetError, pyo3::exceptions::PyException);
create_exception!(rusty_bacnet, BacnetProtocolError, BacnetError);
create_exception!(rusty_bacnet, BacnetTimeoutError, BacnetError);
create_exception!(rusty_bacnet, BacnetRejectError, BacnetError);
create_exception!(rusty_bacnet, BacnetAbortError, BacnetError);
create_exception!(rusty_bacnet, BacnetLogNotAdvancingError, BacnetError);
create_exception!(rusty_bacnet, BacnetReadRangeViolationError, BacnetError);
create_exception!(rusty_bacnet, BacnetBvlcNakError, BacnetError);
create_exception!(rusty_bacnet, BacnetBvlcUnexpectedResponseError, BacnetError);

/// `BacnetTransportError`, built at first use: pyo3's `create_exception!`
/// takes one base, and this class derives from both `BacnetError` and
/// `OSError` (#1120).
static TRANSPORT_ERROR: PyOnceLock<Py<PyType>> = PyOnceLock::new();

/// `errno` module names for the `io::ErrorKind`s that have a counterpart,
/// used when the error did not come from the operating system. The module
/// resolves each name to this platform's value (Windows reports the Winsock
/// codes), so `e.errno == errno.EADDRINUSE` holds everywhere.
const KIND_ERRNO_NAMES: [(std::io::ErrorKind, &str); 15] = [
    (std::io::ErrorKind::AddrInUse, "EADDRINUSE"),
    (std::io::ErrorKind::AddrNotAvailable, "EADDRNOTAVAIL"),
    (std::io::ErrorKind::ConnectionRefused, "ECONNREFUSED"),
    (std::io::ErrorKind::ConnectionReset, "ECONNRESET"),
    (std::io::ErrorKind::ConnectionAborted, "ECONNABORTED"),
    (std::io::ErrorKind::NotConnected, "ENOTCONN"),
    (std::io::ErrorKind::PermissionDenied, "EACCES"),
    (std::io::ErrorKind::NotFound, "ENOENT"),
    (std::io::ErrorKind::AlreadyExists, "EEXIST"),
    (std::io::ErrorKind::BrokenPipe, "EPIPE"),
    (std::io::ErrorKind::TimedOut, "ETIMEDOUT"),
    (std::io::ErrorKind::WouldBlock, "EAGAIN"),
    (std::io::ErrorKind::Interrupted, "EINTR"),
    (std::io::ErrorKind::InvalidInput, "EINVAL"),
    (std::io::ErrorKind::HostUnreachable, "EHOSTUNREACH"),
];

/// The `BacnetProtocolError` attributes a structured Clause 21 error body
/// fills in. Each is `None` unless the body carried it, including on the
/// class itself.
const DETAIL_ATTRIBUTES: [&str; 7] = [
    "first_failed_element_number",
    "first_failed_write_attempt",
    "first_failed_subscription",
    "vendor_id",
    "service_number",
    "error_parameters",
    "vt_session_identifiers",
];

/// Convert a Rust `Error` into a Python exception.
///
/// Protocol errors, rejects, and aborts carry structured attributes
/// (`error_class`/`error_code` or `reason`) so Python callers can inspect them
/// programmatically without parsing the message string. Each is a member of
/// its IntEnum class (`ErrorClass`, `ErrorCode`, `RejectReason`,
/// `AbortReason`), which compares equal to the wire integer (#1816). A protocol error also
/// carries what a structured error body adds, `None` when absent:
/// `first_failed_element_number` (ChangeList-Error, CreateObject-Error),
/// `first_failed_write_attempt` (WritePropertyMultiple-Error) and
/// `first_failed_subscription` (SubscribeCOVPropertyMultiple-Error) as
/// object/property/index dicts, `vendor_id`, `service_number` and
/// `error_parameters` (ConfirmedPrivateTransfer-Error), and
/// `vt_session_identifiers` (VTClose-Error). Futures call this on binding
/// threads, so the attributes are set through the exit gate (#1002).
pub fn to_py_err(err: Error) -> PyErr {
    match err {
        Error::Remote(RemoteError::Error { class, code }) => protocol_error(
            format!(
                "BACnet error: class={} code={}",
                class.to_raw(),
                code.to_raw()
            ),
            class,
            code,
            None,
        ),
        Error::Structured {
            class,
            code,
            detail,
        } => protocol_error(
            format!(
                "BACnet error: class={} code={}, {detail}",
                class.to_raw(),
                code.to_raw()
            ),
            class,
            code,
            Some(*detail),
        ),
        Error::Timeout(_) => BacnetTimeoutError::new_err(err.to_string()),
        Error::Remote(RemoteError::Reject { reason }) => {
            let py_err =
                BacnetRejectError::new_err(format!("BACnet reject: reason={}", reason.to_raw()));
            let _ = crate::py_async::attach(|py| {
                set_enum_attr(py_err.value(py).as_any(), "reason", reason);
                Ok(())
            });
            py_err
        }
        Error::Remote(RemoteError::Abort { reason }) => {
            let py_err =
                BacnetAbortError::new_err(format!("BACnet abort: reason={}", reason.to_raw()));
            let _ = crate::py_async::attach(|py| {
                set_enum_attr(py_err.value(py).as_any(), "reason", reason);
                Ok(())
            });
            py_err
        }
        // A hub dial that ran out of time is still a timeout (#1839).
        Error::Transport(io)
            if matches!(
                bacnet_transport::sc::ScConnectError::from_io_error(&io),
                Some(bacnet_transport::sc::ScConnectError::DialTimeout { .. })
            ) =>
        {
            BacnetTimeoutError::new_err(io.to_string())
        }
        Error::Transport(io) => transport_error(io),
        Error::ReadRangeViolation(rule) => {
            // A strict ReadRange refused an answer that broke a rule (#1531):
            // `rule` names it, so a caller can read again leniently.
            let py_err = BacnetReadRangeViolationError::new_err(err.to_string());
            let _ = crate::py_async::attach(|py| {
                let _ = py_err.value(py).setattr("rule", rule.name());
                Ok(())
            });
            py_err
        }
        Error::LogNotAdvancing {
            requested,
            returned,
        } => {
            // A paged log read the device answered behind the cursor, or
            // empty where its counts say records are (#1530): a caller reads
            // such a log by position instead.
            let py_err = BacnetLogNotAdvancingError::new_err(err.to_string());
            let _ = crate::py_async::attach(|py| {
                let val = py_err.value(py);
                let _ = val.setattr("requested", requested);
                let _ = val.setattr("returned", returned);
                Ok(())
            });
            py_err
        }
        _ => BacnetError::new_err(err.to_string()),
    }
}

/// Convert a failed BVLC management exchange (`read_bdt` and its siblings,
/// #1841). A NAK raises `BacnetBvlcNakError` with the BBMD's `result_code`;
/// no reply raises `BacnetTimeoutError`, so the two never share a class. A
/// reply of the wrong kind raises `BacnetBvlcUnexpectedResponseError` with
/// the `expected` and `got` BVLC function codes. A request the transport
/// refused before sending it (busy, or no free slot) raises a plain
/// `BacnetError`, and a local failure raises what [`to_py_err`] makes of it.
pub(crate) fn bvlc_exchange_err(err: BvlcExchangeError) -> PyErr {
    match err {
        BvlcExchangeError::Nak(code) => {
            let py_err = BacnetBvlcNakError::new_err(err.to_string());
            let _ = crate::py_async::attach(|py| {
                let _ = py_err.value(py).setattr("result_code", code.to_raw());
                Ok(())
            });
            py_err
        }
        BvlcExchangeError::NoReply(_) => BacnetTimeoutError::new_err(err.to_string()),
        BvlcExchangeError::UnexpectedResponse { expected, got } => {
            let py_err = BacnetBvlcUnexpectedResponseError::new_err(err.to_string());
            let _ = crate::py_async::attach(|py| {
                let value = py_err.value(py);
                let _ = value.setattr("expected", expected.to_raw());
                let _ = value.setattr("got", got.to_raw());
                Ok(())
            });
            py_err
        }
        BvlcExchangeError::Refused(_) => BacnetError::new_err(err.to_string()),
        BvlcExchangeError::Transport(error) => to_py_err(error),
        other => BacnetError::new_err(other.to_string()),
    }
}

/// `BacnetTransportError` for an `io::Error`: `errno` is the operating
/// system's code when the error carries one, else the code for its kind where
/// one exists, else `None`. `strerror` is the error's message. Falls back to
/// a plain `BacnetError` when Python is finalizing.
fn transport_error(io: std::io::Error) -> PyErr {
    let message = io.to_string();
    crate::py_async::attach(|py| {
        let errno = io.raw_os_error().or_else(|| kind_errno(py, io.kind()));
        let class = transport_error_type(py)?.bind(py);
        let instance = match errno {
            Some(errno) => class.call1((errno, &message))?,
            None => class.call1((&message,))?,
        };
        if let Some(info) = bacnet_transport::sc::ScConnectError::from_io_error(&io)
            .and_then(|error| error.untrusted_peer_info())
        {
            instance.setattr(
                "sc_untrusted_peer",
                Py::new(py, crate::sc_peer::PyScUntrustedPeerInfo(info.clone()))?,
            )?;
        }
        Ok(PyErr::from_value(instance))
    })
    .unwrap_or_else(|_| BacnetError::new_err(message))
}

fn kind_errno(py: Python<'_>, kind: std::io::ErrorKind) -> Option<i32> {
    let name = KIND_ERRNO_NAMES.iter().find(|(k, _)| *k == kind)?.1;
    py.import("errno").ok()?.getattr(name).ok()?.extract().ok()
}

fn transport_error_type(py: Python<'_>) -> PyResult<&Py<PyType>> {
    TRANSPORT_ERROR.get_or_try_init(py, || {
        let builtins = py.import("builtins")?;
        let bases = PyTuple::new(
            py,
            [
                py.get_type::<BacnetError>().into_any(),
                builtins.getattr("OSError")?,
            ],
        )?;
        let namespace = PyDict::new(py);
        namespace.set_item("__module__", "rusty_bacnet")?;
        namespace.set_item("sc_untrusted_peer", py.None())?;
        namespace.set_item(
            "__doc__",
            "A transport I/O failure: a bind, listen, dial or socket error. \
             Also an OSError, with errno and strerror from the underlying error.",
        )?;
        let class = builtins
            .getattr("type")?
            .call1(("BacnetTransportError", bases, namespace))?;
        Ok(class.cast_into::<PyType>()?.unbind())
    })
}

/// Set `name` on `error` to `value`'s IntEnum member, which compares equal
/// to the wire integer (#915, #1816); the plain integer if the member can't
/// be made.
fn set_enum_attr<T: PyIntEnum>(error: &Bound<'_, PyAny>, name: &str, value: T) {
    let _ = match int_enums::to_py(error.py(), value) {
        Ok(member) => error.setattr(name, member),
        Err(_) => error.setattr(name, value.raw()),
    };
}

/// `BacnetProtocolError` with `error_class` and `error_code` as `ErrorClass`
/// and `ErrorCode` members, which compare equal to the wire integers.
fn protocol_error(
    message: String,
    class: ErrorClass,
    code: ErrorCode,
    detail: Option<ErrorDetail>,
) -> PyErr {
    let py_err = BacnetProtocolError::new_err(message);
    let _ = crate::py_async::attach(|py| {
        let val = py_err.value(py);
        set_enum_attr(val.as_any(), "error_class", class);
        set_enum_attr(val.as_any(), "error_code", code);
        for name in DETAIL_ATTRIBUTES {
            let _ = val.setattr(name, py.None());
        }
        if let Some(detail) = &detail {
            let _ = set_detail(py, val.as_any(), detail);
        }
        Ok(())
    });
    py_err
}

fn set_detail(py: Python<'_>, error: &Bound<'_, PyAny>, detail: &ErrorDetail) -> PyResult<()> {
    match detail {
        ErrorDetail::FirstFailedElementNumber(number) => {
            error.setattr("first_failed_element_number", number)
        }
        ErrorDetail::FirstFailedWriteAttempt(attempt) => {
            error.setattr("first_failed_write_attempt", reference(py, attempt)?)
        }
        ErrorDetail::FirstFailedSubscription(subscription) => {
            error.setattr("first_failed_subscription", reference(py, subscription)?)
        }
        ErrorDetail::PrivateTransfer {
            vendor_id,
            service_number,
            error_parameters,
        } => {
            error.setattr("vendor_id", vendor_id)?;
            error.setattr("service_number", service_number)?;
            error.setattr(
                "error_parameters",
                error_parameters
                    .as_deref()
                    .map(|parameters| PyBytes::new(py, parameters)),
            )
        }
        ErrorDetail::VtSessionIdentifiers(sessions) => {
            error.setattr("vt_session_identifiers", PyList::new(py, sessions.iter())?)
        }
    }
}

/// `{"object_identifier", "property_identifier", "property_array_index"}`.
fn reference<'py>(
    py: Python<'py>,
    reference: &BACnetObjectPropertyReference,
) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(py);
    result.set_item(
        "object_identifier",
        PyObjectIdentifier::from_rust(reference.object_identifier),
    )?;
    result.set_item(
        "property_identifier",
        PyPropertyIdentifier {
            inner: PropertyIdentifier::from_raw(reference.property_identifier),
        },
    )?;
    result.set_item("property_array_index", reference.property_array_index)?;
    Ok(result)
}

/// Register exception types with the module.
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("BacnetError", m.py().get_type::<BacnetError>())?;
    let protocol = m.py().get_type::<BacnetProtocolError>();
    for name in DETAIL_ATTRIBUTES {
        protocol.setattr(name, m.py().None())?;
    }
    m.add("BacnetProtocolError", protocol)?;
    m.add(
        "BacnetTimeoutError",
        m.py().get_type::<BacnetTimeoutError>(),
    )?;
    m.add("BacnetRejectError", m.py().get_type::<BacnetRejectError>())?;
    m.add("BacnetAbortError", m.py().get_type::<BacnetAbortError>())?;
    m.add(
        "BacnetLogNotAdvancingError",
        m.py().get_type::<BacnetLogNotAdvancingError>(),
    )?;
    m.add(
        "BacnetReadRangeViolationError",
        m.py().get_type::<BacnetReadRangeViolationError>(),
    )?;
    m.add(
        "BacnetTransportError",
        transport_error_type(m.py())?.bind(m.py()),
    )?;
    let nak = m.py().get_type::<BacnetBvlcNakError>();
    nak.setattr("result_code", m.py().None())?;
    m.add("BacnetBvlcNakError", nak)?;
    let unexpected = m.py().get_type::<BacnetBvlcUnexpectedResponseError>();
    unexpected.setattr("expected", m.py().None())?;
    unexpected.setattr("got", m.py().None())?;
    m.add("BacnetBvlcUnexpectedResponseError", unexpected)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::ObjectType;
    use bacnet_types::primitives::ObjectIdentifier;

    #[test]
    fn protocol_errors_carry_their_structured_detail() {
        Python::initialize();
        Python::attach(|py| {
            let number = |error: &PyErr, name: &str| -> Option<u32> {
                error.value(py).getattr(name).unwrap().extract().unwrap()
            };
            let none = |error: &PyErr, name: &str| error.value(py).getattr(name).unwrap().is_none();
            // SERVICES / LIST_ELEMENT_NOT_FOUND at element 2 (#1026).
            let change_list = to_py_err(Error::with_detail(
                ErrorClass::SERVICES,
                ErrorCode::LIST_ELEMENT_NOT_FOUND,
                Some(ErrorDetail::FirstFailedElementNumber(2)),
            ));
            assert!(change_list.is_instance_of::<BacnetProtocolError>(py));
            assert_eq!(number(&change_list, "error_class"), Some(5));
            assert_eq!(number(&change_list, "error_code"), Some(81));
            assert_eq!(number(&change_list, "first_failed_element_number"), Some(2));
            assert!(none(&change_list, "vendor_id"));

            let plain = to_py_err(Error::protocol(
                ErrorClass::PROPERTY,
                ErrorCode::UNKNOWN_OBJECT,
            ));
            assert!(plain.is_instance_of::<BacnetProtocolError>(py));
            assert_eq!(number(&plain, "error_code"), Some(31));
            assert_eq!(number(&plain, "first_failed_element_number"), None);

            // #1047: the other bodies.
            let write = to_py_err(Error::with_detail(
                ErrorClass::PROPERTY,
                ErrorCode::WRITE_ACCESS_DENIED,
                Some(ErrorDetail::FirstFailedWriteAttempt(
                    BACnetObjectPropertyReference::new_indexed(
                        ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 7).unwrap(),
                        PropertyIdentifier::PRESENT_VALUE.to_raw(),
                        3,
                    ),
                )),
            ));
            let attempt = write
                .value(py)
                .getattr("first_failed_write_attempt")
                .unwrap();
            let oid: PyObjectIdentifier = attempt
                .get_item("object_identifier")
                .unwrap()
                .extract()
                .unwrap();
            assert_eq!(
                oid.to_rust(),
                ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 7).unwrap()
            );
            let index: Option<u32> = attempt
                .get_item("property_array_index")
                .unwrap()
                .extract()
                .unwrap();
            assert_eq!(index, Some(3));
            assert!(none(&write, "first_failed_subscription"));

            let private = to_py_err(Error::with_detail(
                ErrorClass::SERVICES,
                ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
                Some(ErrorDetail::PrivateTransfer {
                    vendor_id: 555,
                    service_number: 7,
                    error_parameters: Some(vec![0x21, 1]),
                }),
            ));
            assert_eq!(number(&private, "vendor_id"), Some(555));
            assert_eq!(number(&private, "service_number"), Some(7));
            let parameters: Vec<u8> = private
                .value(py)
                .getattr("error_parameters")
                .unwrap()
                .extract()
                .unwrap();
            assert_eq!(parameters, [0x21, 1]);

            let vt_close = to_py_err(Error::with_detail(
                ErrorClass::SERVICES,
                ErrorCode::VT_SESSION_TERMINATION_FAILURE,
                Some(ErrorDetail::VtSessionIdentifiers(vec![1, 4])),
            ));
            let sessions: Vec<u32> = vt_close
                .value(py)
                .getattr("vt_session_identifiers")
                .unwrap()
                .extract()
                .unwrap();
            assert_eq!(sessions, [1, 4]);
        });
    }

    #[test]
    fn bvlc_exchange_errors_keep_their_classes_apart() {
        use bacnet_transport::bvll::{BvlcFunction, BvlcResultCode};
        Python::initialize();
        Python::attach(|py| {
            let attr = |error: &PyErr, name: &str| -> u16 {
                error.value(py).getattr(name).unwrap().extract().unwrap()
            };
            let nak = bvlc_exchange_err(BvlcExchangeError::Nak(
                BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK,
            ));
            assert!(nak.is_instance_of::<BacnetBvlcNakError>(py));
            assert!(!nak.is_instance_of::<BacnetTimeoutError>(py));
            assert_eq!(attr(&nak, "result_code"), 0x0030);

            let silent = bvlc_exchange_err(BvlcExchangeError::NoReply(
                std::time::Duration::from_secs(3),
            ));
            assert!(silent.is_instance_of::<BacnetTimeoutError>(py));
            assert!(!silent.is_instance_of::<BacnetBvlcNakError>(py));

            let mismatch = bvlc_exchange_err(BvlcExchangeError::UnexpectedResponse {
                expected: BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE_ACK,
                got: BvlcFunction::BVLC_RESULT,
            });
            assert!(mismatch.is_instance_of::<BacnetBvlcUnexpectedResponseError>(py));
            assert!(!mismatch.is_instance_of::<BacnetBvlcNakError>(py));
            assert_eq!(
                (attr(&mismatch, "expected"), attr(&mismatch, "got")),
                (0x03, 0x00)
            );

            let local = bvlc_exchange_err(BvlcExchangeError::Transport(Error::Encoding(
                "refused".into(),
            )));
            assert!(local.value(py).get_type().is(py.get_type::<BacnetError>()));

            let busy = bvlc_exchange_err(BvlcExchangeError::Refused(
                bacnet_transport::bip::ExchangeRefusal::Full,
            ));
            assert!(busy.value(py).get_type().is(py.get_type::<BacnetError>()));
        });
    }

    #[test]
    fn definite_send_length_refusal_uses_the_existing_base_exception() {
        Python::initialize();
        Python::attach(|py| {
            let error = to_py_err(Error::SendTooLong {
                length: 1500,
                limit: 480,
            });
            assert!(error.value(py).get_type().is(py.get_type::<BacnetError>()));
            assert_eq!(
                error.value(py).str().unwrap().to_str().unwrap(),
                "send refused before transmission: 1500 octets exceed the 480-octet limit"
            );
        });
    }

    #[test]
    fn transport_errors_are_oserrors_with_errno() {
        Python::initialize();
        Python::attach(|py| {
            let errno = |error: &PyErr| -> Option<i32> {
                error.value(py).getattr("errno").unwrap().extract().unwrap()
            };
            let os_error = std::io::Error::from_raw_os_error(98);
            let from_os = to_py_err(Error::Transport(os_error));
            assert!(from_os.is_instance(py, transport_error_type(py).unwrap().bind(py)));
            assert!(from_os.is_instance_of::<BacnetError>(py));
            assert!(from_os.is_instance_of::<pyo3::exceptions::PyOSError>(py));
            assert_eq!(errno(&from_os), Some(98));
            assert!(!from_os.value(py).getattr("strerror").unwrap().is_none());

            // No OS code: the kind's errno, resolved by Python's errno module.
            let from_kind = to_py_err(Error::Transport(std::io::Error::new(
                std::io::ErrorKind::AddrInUse,
                "hub bind",
            )));
            let expected: i32 = py
                .import("errno")
                .unwrap()
                .getattr("EADDRINUSE")
                .unwrap()
                .extract()
                .unwrap();
            assert_eq!(errno(&from_kind), Some(expected));
            assert_eq!(
                from_kind
                    .value(py)
                    .getattr("strerror")
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "hub bind"
            );

            // A kind with no counterpart leaves errno None.
            let other = to_py_err(Error::Transport(std::io::Error::other("closed")));
            assert!(other.is_instance(py, transport_error_type(py).unwrap().bind(py)));
            assert_eq!(errno(&other), None);
        });
    }

    /// A raised error's class, code and reason are members of their IntEnum
    /// classes, equal to the wire numbers; a value without a name is a
    /// pseudo-member (#1816).
    #[test]
    fn raised_errors_carry_int_enum_members() {
        use bacnet_types::enums::{AbortReason, RejectReason};
        Python::initialize();
        Python::attach(|py| {
            let attr = |error: &PyErr, name: &str| error.value(py).getattr(name).unwrap();
            let check = |value: Bound<'_, PyAny>, class: &Bound<'_, PyType>, raw: u32| {
                assert!(value.is_instance(class).unwrap(), "{value:?}");
                assert_eq!(value.extract::<u32>().unwrap(), raw);
            };
            let classes = (
                int_enums::class::<ErrorClass>(py).unwrap().clone(),
                int_enums::class::<ErrorCode>(py).unwrap().clone(),
            );
            let plain = to_py_err(Error::protocol(
                ErrorClass::PROPERTY,
                ErrorCode::from_raw(300),
            ));
            check(attr(&plain, "error_class"), &classes.0, 2);
            check(attr(&plain, "error_code"), &classes.1, 300);
            assert!(
                attr(&plain, "error_code")
                    .getattr("name")
                    .unwrap()
                    .is_none()
            );

            let reject = to_py_err(Error::reject(RejectReason::UNRECOGNIZED_SERVICE));
            assert!(reject.is_instance_of::<BacnetRejectError>(py));
            let class = int_enums::class::<RejectReason>(py).unwrap();
            check(attr(&reject, "reason"), class, 9);

            let abort = to_py_err(Error::abort(AbortReason::from_raw(200)));
            assert!(abort.is_instance_of::<BacnetAbortError>(py));
            let class = int_enums::class::<AbortReason>(py).unwrap();
            check(attr(&abort, "reason"), class, 200);
            // The message keeps its numbers.
            assert_eq!(abort.value(py).to_string(), "BACnet abort: reason=200");
        });
    }
}
