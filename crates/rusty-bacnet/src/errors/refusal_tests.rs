//! The local refusals (#1833) raise the base exception with their own text,
//! as the untyped error they replace did, whose text led with its prefix.

use super::*;
use bacnet_types::error::{Lifecycle, Refusal};

/// Every kind of local refusal, and every lifecycle state, is a plain
/// `BacnetError` (not a timeout, a transport error or a lookup error) whose
/// message is the refusal's text alone.
#[test]
fn local_refusals_raise_the_base_exception_with_their_text() {
    Python::initialize();
    Python::attach(|py| {
        let refusals = [
            Error::state(
                Lifecycle::AlreadyStarted,
                "router application port already started",
            ),
            Error::state(Lifecycle::NotRunning, "endpoint session is not running"),
            Error::state(Lifecycle::Stopping, "endpoint ingress is not running"),
            Error::state(Lifecycle::NotConnected, "not connected"),
            Error::state(Lifecycle::Closed, "endpoint shutdown"),
            Error::invalid_config("endpoint session queue capacity must be greater than zero"),
            Error::invalid_argument("invalid local Network-Number-Is NPDU"),
            Error::unsupported("source Audit requires the session's link"),
            Error::LimitReached(
                Refusal::new("A router needs 1 to {} ports, not {}").with_values(255, 256),
            ),
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
        // The untyped error they replace took the same arm.
        let untyped = to_py_err(Error::Encoding("endpoint shutdown".into()));
        assert!(
            untyped
                .value(py)
                .get_type()
                .is(py.get_type::<BacnetError>())
        );
        assert_eq!(
            untyped.value(py).str().unwrap().to_str().unwrap(),
            "encoding error: endpoint shutdown"
        );
    });
}

/// A request the client's stop cut off (#2617), now a closed refusal
/// (#1833), raises the base exception with the message it raised as text;
/// every other closed refusal, a closed seat's included, raises its own
/// text, and so does the untyped text, which no client sends any more.
#[test]
fn a_cut_off_request_keeps_its_message() {
    Python::initialize();
    Python::attach(|py| {
        let message = |error: Error| {
            let raised = to_py_err(error);
            assert!(raised.value(py).get_type().is(py.get_type::<BacnetError>()));
            raised.value(py).str().unwrap().to_str().unwrap().to_owned()
        };
        let cut_off = Error::state(Lifecycle::Closed, "TSM response channel closed");
        assert!(bacnet_client::client::is_request_cut_off(&cut_off));
        assert_eq!(message(cut_off), REQUEST_CUT_OFF);
        for (other, text) in [
            (
                Error::state(
                    Lifecycle::Closed,
                    "the client's seat has closed: its node or the client stopped",
                ),
                "the client's seat has closed: its node or the client stopped",
            ),
            (
                Error::state(Lifecycle::Stopping, "TSM response channel closed"),
                "TSM response channel closed",
            ),
            (
                Error::Encoding("TSM response channel closed".into()),
                "encoding error: TSM response channel closed",
            ),
        ] {
            assert_eq!(message(other), text);
        }
    });
}
