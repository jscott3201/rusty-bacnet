//! Fresh Python error snapshots borrow the retained native cleanup report.
use super::lifecycle::{CloseError, StartError};
use super::records::{PyRouterCleanupFailure, RemoteReason};
use bacnet_network::router::{RouterStopError, RouterStopFailure};
use bacnet_types::error::{Error, RemoteError};
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::PyTuple;

pyo3::create_exception!(
    rusty_bacnet,
    BacnetRouterCloseError,
    crate::errors::BacnetError
);

pub(super) fn start_error(error: StartError<Error, RouterStopError>, networks: &[u16]) -> PyErr {
    match error {
        StartError::AlreadyStarted => PyRuntimeError::new_err("router already started"),
        StartError::Cancelled => {
            PyRuntimeError::new_err("router startup cancelled by close or disposal")
        }
        StartError::Operation(error) => crate::errors::to_py_err(error),
        StartError::Cleanup(error) => close_error(&error, networks),
    }
}

pub(super) fn close_error(error: &CloseError<RouterStopError>, networks: &[u16]) -> PyErr {
    let failures = match error {
        CloseError::Stop(error) => error
            .failures()
            .iter()
            .map(|failure| match failure {
                RouterStopFailure::Transport {
                    network_number,
                    error,
                } => {
                    let (error_class, error_code) = match error {
                        Error::Remote(RemoteError::Error { class, code })
                        | Error::Structured { class, code, .. } => (Some(*class), Some(*code)),
                        _ => (None, None),
                    };
                    PyRouterCleanupFailure {
                        network_number: Some(*network_number),
                        port_index: networks
                            .iter()
                            .position(|network| network == network_number),
                        kind: "transport",
                        message: error.to_string(),
                        errno: match error {
                            Error::Transport(io) => io.raw_os_error(),
                            _ => None,
                        },
                        error_class,
                        error_code,
                        reason: match error {
                            Error::Remote(RemoteError::Reject { reason }) => {
                                Some(RemoteReason::Reject(*reason))
                            }
                            Error::Remote(RemoteError::Abort { reason }) => {
                                Some(RemoteReason::Abort(*reason))
                            }
                            _ => None,
                        },
                        worker: None,
                    }
                }
                RouterStopFailure::Worker {
                    network_number,
                    worker,
                    detail,
                } => PyRouterCleanupFailure {
                    network_number: *network_number,
                    port_index: network_number
                        .and_then(|n| networks.iter().position(|network| *network == n)),
                    kind: "worker",
                    message: detail.clone(),
                    errno: None,
                    error_class: None,
                    error_code: None,
                    reason: None,
                    worker: Some(worker),
                },
            })
            .collect::<Vec<_>>(),
        CloseError::WorkerEnded => vec![PyRouterCleanupFailure {
            network_number: None,
            port_index: None,
            kind: "worker",
            message: "router lifecycle worker ended unexpectedly".into(),
            errno: None,
            error_class: None,
            error_code: None,
            reason: None,
            worker: Some("router lifecycle"),
        }],
    };
    let error = BacnetRouterCloseError::new_err(
        "router cleanup failed; the retained failures prevent restart",
    );
    let converted = crate::py_async::attach(|py| {
        let records = failures
            .into_iter()
            .map(|failure| Py::new(py, failure))
            .collect::<PyResult<Vec<_>>>()?;
        error
            .value(py)
            .setattr("failures", PyTuple::new(py, records)?)?;
        Ok(())
    });
    match converted {
        Ok(()) => error,
        Err(error) => error,
    }
}

/// A body exception stays primary; the owner still retains a cleanup failure.
pub(super) fn context_outcome(
    outcome: Result<(), CloseError<RouterStopError>>,
    body_error: bool,
    networks: &[u16],
) -> PyResult<()> {
    match outcome {
        Err(error) if !body_error => Err(close_error(&error, networks)),
        _ => Ok(()),
    }
}
