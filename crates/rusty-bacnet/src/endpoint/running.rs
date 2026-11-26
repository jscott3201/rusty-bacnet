//! A running endpoint session and the full client on its link (#2617).
//!
//! Every Python endpoint runs its session over [`AppTransport`], the type a
//! standalone `BACnetClient`'s link has, so the session's full client is a
//! Python `BACnetClient`: `await endpoint.client()` returns one, on the
//! endpoint's one address, with every method the standalone class has.
//!
//! One client per session: while it runs, `endpoint.client()` returns the
//! same object; once its `stop()` has finished, a fresh one. The session
//! lock orders `client()` against close, and close stops the session, then
//! the client (the `stop` of its lifecycle `Session` impl). `client()`
//! shares that lock, as server calls do, and takes the client slot's own
//! lock for no await, so it never waits for a server call in flight.
//!
//! The session owns its full server (#2617), and `endpoint.server()` hands
//! out a Python `BACnetServer` that borrows it through the session lock on
//! each call ([`crate::server::ServerHost`]); the session's stop stops it.

use std::future::Future;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bacnet_endpoint::session::EndpointSession;
use bacnet_types::error::Lifecycle;
use bacnet_types::sync::MutexExt;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;

use crate::app_transport::AppTransport;
use crate::client::{BACnetClient, ClientInner, Liveness};
use crate::endpoint::common::not_running;
use crate::errors::{lifecycle_error, to_py_err};
use crate::server::{BACnetServer, SeatedHost};

#[cfg(test)]
#[path = "running_tests.rs"]
mod tests;

/// An endpoint's session: over [`AppTransport`], so its full client is a
/// Python `BACnetClient`'s.
pub(crate) type Session = EndpointSession<AppTransport>;

/// The runs started in this process, so each has its own number.
static RUNS: AtomicU64 = AtomicU64::new(0);

/// A running session, and the client `endpoint.client()` last handed out.
pub(crate) struct Running {
    session: Session,
    /// Under a lock of its own, so `endpoint.client()` shares the session
    /// lock with the server's calls rather than waiting for them all. It is
    /// held for no await.
    client: std::sync::Mutex<Option<Seated>>,
    /// This run's number: a server handle names the run it came from, so
    /// one kept past a close never reaches a later start's server.
    run: u64,
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
            client: std::sync::Mutex::new(None),
            run: RUNS.fetch_add(1, Ordering::Relaxed) + 1,
        }
    }

    /// The session's full server, when this is run `run` and the server
    /// runs: what an endpoint's `BACnetServer` calls.
    pub(crate) fn server_of_run(
        &self,
        run: u64,
    ) -> Option<&bacnet_server::server::BACnetServer<AppTransport>> {
        (self.run == run).then(|| self.session.server()).flatten()
    }

    /// The client to hand out: the one now running, or `candidate`, filled
    /// with the session's full client. A client still stopping refuses,
    /// since it may hold the session's one seat.
    fn client(
        &self,
        candidate: Arc<Py<BACnetClient>>,
        state: ClientInner,
    ) -> PyResult<Arc<Py<BACnetClient>>> {
        let mut slot = self.client.locked();
        if let Some(seated) = slot.as_ref() {
            match seated.state.liveness() {
                Liveness::Started => return Ok(Arc::clone(&seated.object)),
                Liveness::Stopping => {
                    return Err(lifecycle_error(
                        Lifecycle::Stopping,
                        "the endpoint's client is stopping: await its stop() before asking \
                         for another",
                    ));
                }
                // Its stop has not finished: another stop() may still.
                Liveness::Failed(cause) => {
                    return Err(lifecycle_error(
                        Lifecycle::Stopping,
                        format!(
                            "the endpoint's last client failed to stop ({cause}): await its \
                             stop() again before asking for another"
                        ),
                    ));
                }
                Liveness::Stopped => {}
            }
        }
        let core = self.session.client().map_err(to_py_err)?;
        let own_instance = self.session.identity().map(|identity| identity.instance());
        if !state.seat(core, own_instance) {
            return Err(lifecycle_error(
                Lifecycle::AlreadyStarted,
                "the endpoint's new client is already started",
            ));
        }
        *slot = Some(Seated {
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
            // Its database goes: the next start builds the objects again
            // from their registrations (#2766).
            return Err(to_py_err(error.into()));
        }
        Ok(())
    }

    /// Stop the session, then close the client on its link.
    ///
    /// The session stops with its full client alive (#2617): its one stop
    /// closes the roles, ends the client's tasks, fails its open requests
    /// and stops the link. Closing the Python client then ends its
    /// iterators and handles and drops the Rust client. A client whose own
    /// stop fails keeps its state; the error says the link is released.
    async fn stop(&mut self) -> PyResult<()> {
        let stopped = self.session.stop().await.map(drop).map_err(to_py_err);
        let seated = self.client.get_mut_locked().take();
        let closed = match seated {
            Some(seated) => crate::client::close(seated.state).await.map_err(|error| {
                PyRuntimeError::new_err(format!(
                    "the endpoint stopped and released its link, but its client failed to \
                     stop ({error}); that client keeps its own state until its stop() succeeds"
                ))
            }),
            None => Ok(()),
        };
        stopped.and(closed)
    }
}

/// The two seats, as an endpoint's `status()` shows them: the APDUs handed
/// to the full client and to the full server, and those dropped at once
/// because a seat's queue was full, never waited for (and the same for the
/// client's network controls). The counts cover the session's whole run,
/// every client on it.
#[derive(Clone, Copy)]
pub(crate) struct SeatCounts {
    apdus: u64,
    full_drops: u64,
    control_full_drops: u64,
    server_apdus: u64,
    server_full_drops: u64,
}

impl SeatCounts {
    /// Put the counts in a `status()` dict.
    pub(crate) fn add_to(self, dict: &Bound<'_, pyo3::types::PyDict>) -> PyResult<()> {
        dict.set_item("client_seat_apdus", self.apdus)?;
        dict.set_item("client_seat_full_drops", self.full_drops)?;
        dict.set_item("client_seat_control_full_drops", self.control_full_drops)?;
        dict.set_item("server_seat_apdus", self.server_apdus)?;
        dict.set_item("server_seat_full_drops", self.server_full_drops)?;
        Ok(())
    }
}

impl Running {
    /// The seats' counts now.
    pub(crate) fn seat_counts(&self) -> SeatCounts {
        let counters = self.session.client_seat_counters();
        let server = self.session.server_seat_counters();
        SeatCounts {
            apdus: counters.apdus,
            full_drops: counters.full_drops,
            control_full_drops: counters.control_full_drops,
            server_apdus: server.apdus,
            server_full_drops: server.full_drops,
        }
    }
}

/// What `await endpoint.server()` awaits (#2617): a `BACnetServer` on the
/// full server the endpoint's session owns, on a `transport` link
/// (`"bip"`, `"sc"` or `"mstp"`), for the endpoint's `device_instance`.
/// Each call makes a new handle on the run's one server. It names this
/// run, so after a close it raises rather than reach a later start's
/// server. Made with the interpreter attached once the session lock is
/// let go.
pub(crate) async fn server(
    session: Arc<tokio::sync::RwLock<Option<Running>>>,
    transport: &'static str,
    device_instance: u32,
) -> PyResult<Py<BACnetServer>> {
    let run = {
        let guard = session.read().await;
        let running = guard.as_ref().ok_or_else(|| {
            not_running("endpoint not started — use 'async with' or await start()")
        })?;
        // This run's server has stopped and won't start again: a handle's
        // calls raise the same once the run is over.
        running.server_of_run(running.run).ok_or_else(|| {
            lifecycle_error(Lifecycle::Closed, "the endpoint's server is not running")
        })?;
        running.run
    };
    let host = SeatedHost {
        session: Arc::downgrade(&session),
        run,
    };
    crate::py_async::attach(move |py| {
        Py::new(py, BACnetServer::seated(host, transport, device_instance))
    })
}

/// What `await endpoint.client()` awaits on an endpoint whose link takes
/// `transport`'s addresses (`"bip"`, `"sc"` or `"mstp"`): the
/// `BACnetClient` on the endpoint's link. On an endpoint with a source
/// Audit Reporter, the session audits its reads and writes (#2617).
pub(crate) fn client(
    py: Python<'_>,
    session: Arc<tokio::sync::RwLock<Option<Running>>>,
    transport: &'static str,
) -> PyResult<impl Future<Output = PyResult<Py<BACnetClient>>> + Send + 'static> {
    // Made here, with the interpreter attached, and filled under the
    // session lock; unused when a client already runs.
    let candidate = Py::new(py, BACnetClient::seated(transport))?;
    let state = candidate.borrow(py).state();
    let candidate = Arc::new(candidate);
    Ok(async move {
        let chosen = {
            let guard = session.read().await;
            let running = guard.as_ref().ok_or_else(|| {
                not_running("endpoint not started — use 'async with' or await start()")
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
