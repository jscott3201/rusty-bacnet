//! A running endpoint session and the full client on its link (#2617).
//!
//! Every Python endpoint runs its session over [`AppTransport`], the type a
//! standalone `BACnetClient`'s link has, so the session's full client is a
//! Python `BACnetClient`: `await endpoint.client()` returns one, on the
//! endpoint's one address, with every method the standalone class has.
//!
//! One client per session: while it runs, `endpoint.client()` returns the
//! same object; once its `stop()` has finished, a fresh one. The session
//! lock orders `client()` against close, and close stops the client first
//! (the `stop` of its lifecycle `Session` impl).

use std::future::Future;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use bacnet_endpoint::session::EndpointSession;
use pyo3::exceptions::{PyNotImplementedError, PyRuntimeError};
use pyo3::prelude::*;

use crate::app_transport::AppTransport;
use crate::client::{BACnetClient, ClientInner, Liveness};
use crate::errors::to_py_err;

/// An endpoint's session: over [`AppTransport`], so its full client is a
/// Python `BACnetClient`'s.
pub(crate) type Session = EndpointSession<AppTransport>;

/// A running session, and the client `endpoint.client()` last handed out.
pub(crate) struct Running {
    session: Session,
    client: Option<Seated>,
}

/// The Python client on the session's link, and its state, which the
/// endpoint reads without the interpreter.
struct Seated {
    object: Arc<Py<BACnetClient>>,
    state: ClientInner,
}

impl Running {
    pub(crate) fn new(session: Session) -> Self {
        Self {
            session,
            client: None,
        }
    }

    /// The client to hand out: the one now running, or `candidate`, filled
    /// with the session's full client. A client still stopping refuses,
    /// since it may hold the session's one seat.
    fn client(
        &mut self,
        candidate: Arc<Py<BACnetClient>>,
        state: ClientInner,
    ) -> PyResult<Arc<Py<BACnetClient>>> {
        if let Some(seated) = &self.client {
            match seated.state.liveness() {
                Liveness::Started => return Ok(Arc::clone(&seated.object)),
                Liveness::Stopping => {
                    return Err(PyRuntimeError::new_err(
                        "the endpoint's client is stopping: await its stop() before asking \
                         for another",
                    ));
                }
                Liveness::Stopped => {}
            }
        }
        let core = self.session.full_client().map_err(to_py_err)?;
        if !state.seat(core) {
            return Err(PyRuntimeError::new_err(
                "the endpoint's new client is already started",
            ));
        }
        self.client = Some(Seated {
            object: Arc::clone(&candidate),
            state,
        });
        Ok(candidate)
    }
}

impl Deref for Running {
    type Target = Session;

    fn deref(&self) -> &Session {
        &self.session
    }
}

impl DerefMut for Running {
    fn deref_mut(&mut self) -> &mut Session {
        &mut self.session
    }
}

/// Session cleanup is awaited by the private lifecycle worker, never its waiter.
impl super::lifecycle::Session for Running {
    type Error = PyErr;

    async fn start(&mut self) -> PyResult<()> {
        if let Err(error) = self.session.start().await {
            // Start errors before role setup leave no spawned transport workers;
            // role-setup errors already clean up. Settle Running before dropping.
            if self.session.is_running() {
                let _ = self.session.stop().await;
            }
            return Err(to_py_err(error));
        }
        Ok(())
    }

    /// Stop the client on the link, then the session.
    ///
    /// The stop order lives here alone. Until a transport's stop works
    /// through a shared reference (`refactor/transport-shared-stop`), a
    /// session can't stop a link its full client still holds: the first
    /// stop closes the roles, ends the client's tasks and fails its open
    /// requests, then fails. Closing the Python client then drops the Rust
    /// one, and a second stop finishes. Once a stop works with the client
    /// alive, this is: stop the session, then close the client, and the
    /// second stop goes.
    async fn stop(&mut self) -> PyResult<()> {
        let first = self.session.stop().await;
        let Some(seated) = self.client.take() else {
            return first.map(drop).map_err(to_py_err);
        };
        let closed = crate::client::close(seated.state).await;
        let stopped = match first {
            Ok(_) => Ok(()),
            Err(_) => self.session.stop().await.map(drop).map_err(to_py_err),
        };
        stopped.and(closed)
    }
}

/// The full client's seat, as an endpoint's `status()` shows it: the APDUs
/// handed to it, and those dropped at once because its queue was full,
/// never waited for (and the same for network controls). The counts cover
/// the session's whole run, every client on it.
#[derive(Clone, Copy)]
pub(crate) struct SeatCounts {
    apdus: u64,
    full_drops: u64,
    control_full_drops: u64,
}

impl SeatCounts {
    /// Put the counts in a `status()` dict.
    pub(crate) fn add_to(self, dict: &Bound<'_, pyo3::types::PyDict>) -> PyResult<()> {
        dict.set_item("client_seat_apdus", self.apdus)?;
        dict.set_item("client_seat_full_drops", self.full_drops)?;
        dict.set_item("client_seat_control_full_drops", self.control_full_drops)?;
        Ok(())
    }
}

impl Running {
    /// The seat's counts now.
    pub(crate) fn seat_counts(&self) -> SeatCounts {
        let counters = self.session.client_seat_counters();
        SeatCounts {
            apdus: counters.apdus,
            full_drops: counters.full_drops,
            control_full_drops: counters.control_full_drops,
        }
    }
}

/// What `await endpoint.client()` awaits on an endpoint whose link takes
/// `transport`'s addresses (`"bip"`, `"sc"` or `"mstp"`): the
/// `BACnetClient` on the endpoint's link. An endpoint with a source Audit
/// Reporter raises NotImplementedError at the call: its client arrives when
/// Audit moves to the full client (#2617 step 3).
pub(crate) fn client(
    py: Python<'_>,
    session: Arc<tokio::sync::Mutex<Option<Running>>>,
    transport: &'static str,
    audited: bool,
) -> PyResult<impl Future<Output = PyResult<Py<BACnetClient>>> + Send + 'static> {
    if audited {
        return Err(PyNotImplementedError::new_err(
            "an endpoint with a source Audit Reporter has no client yet: its requests would \
             not be audited. The client on an audited endpoint arrives when source Audit \
             moves to the full client (#2617 step 3)",
        ));
    }
    // Made here, with the interpreter attached, and filled under the
    // session lock; unused when a client already runs.
    let candidate = Py::new(py, BACnetClient::seated(transport))?;
    let state = candidate.borrow(py).state();
    let candidate = Arc::new(candidate);
    Ok(async move {
        let chosen = {
            let mut guard = session.lock().await;
            let running = guard.as_mut().ok_or_else(|| {
                PyRuntimeError::new_err("endpoint not started — use 'async with' or await start()")
            })?;
            running.client(Arc::clone(&candidate), state)?
        };
        crate::py_async::attach(move |py| {
            let object = chosen.clone_ref(py);
            // An unused candidate goes here, with the interpreter attached.
            drop(candidate);
            Ok(object)
        })
    })
}
