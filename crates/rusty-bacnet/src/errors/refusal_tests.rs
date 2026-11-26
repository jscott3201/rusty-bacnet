//! The local refusals (#1833) raise their own text: a lifecycle state as
//! `BacnetStateError`, every other kind as the base exception, as the
//! untyped error they replace did, whose text led with its prefix.

use super::*;
use bacnet_types::error::{Lifecycle, PersistenceError, PersistenceErrorKind, Refusal};

/// Every lifecycle state raises `BacnetStateError` (also a `BacnetError`
/// and a `RuntimeError`) with its text and the state's name in `state`;
/// every other kind of local refusal is a plain `BacnetError` (not a
/// timeout, a transport error or a lookup error) whose message is the
/// refusal's text alone.
#[test]
fn local_refusals_raise_their_class_with_their_text() {
    Python::initialize();
    Python::attach(|py| {
        let states = [
            (
                Error::state(
                    Lifecycle::AlreadyStarted,
                    "router application port already started",
                ),
                "already_started",
            ),
            (
                Error::state(Lifecycle::NotRunning, "endpoint session is not running"),
                "not_running",
            ),
            (
                Error::state(Lifecycle::Stopping, "endpoint ingress is not running"),
                "stopping",
            ),
            (
                Error::state(Lifecycle::NotConnected, "not connected"),
                "not_connected",
            ),
            (
                Error::state(Lifecycle::Closed, "endpoint shutdown"),
                "closed",
            ),
            // What a send on a stopped transport gets.
            (bacnet_transport::port::stopped(), "not_running"),
            (bacnet_transport::port::stopped_for_good(), "closed"),
        ];
        let class = state_error_type(py).unwrap().bind(py);
        for (error, state) in states {
            let text = error.to_string();
            let raised = to_py_err(error);
            let value = raised.value(py);
            assert!(value.get_type().is(class), "{text}");
            assert!(value.is_instance_of::<BacnetError>(), "{text}");
            assert!(
                value.is_instance_of::<pyo3::exceptions::PyRuntimeError>(),
                "{text}"
            );
            assert_eq!(value.str().unwrap().to_str().unwrap(), text);
            let named: String = value.getattr("state").unwrap().extract().unwrap();
            assert_eq!(named, state, "{text}");
        }
        assert!(class.getattr("state").unwrap().is_none());
        let refusals = [
            Error::invalid_config("endpoint session queue capacity must be greater than zero"),
            Error::invalid_argument("invalid local Network-Number-Is NPDU"),
            Error::unsupported("source Audit requires the session's link"),
            Error::LimitReached(
                Refusal::new("A router needs 1 to {} ports, not {}").with_values(255, 256),
            ),
            // Persisted state (#1833), damaged or a backend's failure: the
            // base class too, as the untyped text it replaces was.
            Error::from(
                PersistenceError::new(PersistenceErrorKind::Damaged, "checksum mismatch")
                    .in_format("Notification Class file"),
            ),
            Error::from(PersistenceError::new(
                PersistenceErrorKind::BackendFailed,
                "persistence panicked during a save",
            )),
        ];
        for error in refusals {
            let text = error.to_string();
            let raised = to_py_err(error);
            assert!(
                raised.value(py).get_type().is(py.get_type::<BacnetError>()),
                "{text}"
            );
            assert_eq!(raised.value(py).str().unwrap().to_str().unwrap(), text);
        }
    });
}

/// A request the client or its link cut off (#2617, #1833) raises the base
/// exception with the error's own text, which says whether it had gone out
/// and so whether the device may have carried it out, as a cut-off
/// `write_bdt` or `delete_fdt_entry` may have changed the table: it is one
/// request's outcome, not a state. The closed refusals that look like it
/// (a closed seat, an endpoint's shutdown before anything was sent) are
/// states, and each raises its own text.
#[test]
fn a_cut_off_request_says_whether_it_went_out() {
    Python::initialize();
    Python::attach(|py| {
        let message = |sent| {
            let cut_off = Error::RequestCutOff { sent };
            let text = cut_off.to_string();
            let raised = to_py_err(cut_off);
            assert!(raised.value(py).get_type().is(py.get_type::<BacnetError>()));
            let shown = raised.value(py).str().unwrap().to_str().unwrap().to_owned();
            assert_eq!(shown, text);
            shown
        };
        assert!(message(true).contains("may have carried it out"));
        assert!(message(false).contains("did not get it"));
        let class = state_error_type(py).unwrap().bind(py);
        for text in [
            "the client's seat has closed: its node or the client stopped",
            "endpoint shutdown",
        ] {
            let raised = to_py_err(Error::state(Lifecycle::Closed, text));
            assert!(raised.value(py).get_type().is(class));
            assert_eq!(raised.value(py).str().unwrap().to_str().unwrap(), text);
        }
    });
}

/// A device the table can't place for a call by instance (#1833) raises the
/// lookup error, naming the instance, as a routed address with no known
/// router does.
#[test]
fn an_unknown_device_raises_the_lookup_error() {
    Python::initialize();
    Python::attach(|py| {
        let raised = to_py_err(Error::UnknownDevice {
            device: "1234".into(),
        });
        let value = raised.value(py);
        assert!(value.get_type().is(lookup_error_type(py).unwrap().bind(py)));
        let instance: Option<u32> = value.getattr("instance").unwrap().extract().unwrap();
        assert_eq!(instance, Some(1234));
    });
}

/// The bindings' own lifecycle checks (#2819) raise `BacnetStateError`
/// with the text they always raised and the state they found, so
/// `except RuntimeError`, `except BacnetError` and `e.state` all work; a
/// routing-table query whose client stopped under it is one of them.
#[test]
fn the_bindings_own_lifecycle_checks_raise_the_state_error() {
    Python::initialize();
    Python::attach(|py| {
        let class = state_error_type(py).unwrap().bind(py);
        let checks = [
            (
                lifecycle_error(
                    Lifecycle::NotRunning,
                    "client not started — use 'async with'",
                ),
                "client not started — use 'async with'",
                "not_running",
            ),
            (
                lifecycle_error(Lifecycle::AlreadyStarted, "router already started"),
                "router already started",
                "already_started",
            ),
            (
                lifecycle_error(
                    Lifecycle::Stopping,
                    "client closing — await stop() before starting new work",
                ),
                "client closing — await stop() before starting new work",
                "stopping",
            ),
            (
                lifecycle_error(Lifecycle::Closed, String::from("endpoint closed")),
                "endpoint closed",
                "closed",
            ),
            (
                routing_table_err(bacnet_client::client::RoutingTableError::ClientStopped),
                "client stopped during routing_table()",
                "closed",
            ),
        ];
        for (raised, text, state) in checks {
            let value = raised.value(py);
            assert!(value.get_type().is(class), "{text}");
            assert!(value.is_instance_of::<BacnetError>(), "{text}");
            assert!(
                value.is_instance_of::<pyo3::exceptions::PyRuntimeError>(),
                "{text}"
            );
            assert_eq!(value.str().unwrap().to_str().unwrap(), text);
            let named: String = value.getattr("state").unwrap().extract().unwrap();
            assert_eq!(named, state, "{text}");
        }
    });
}
