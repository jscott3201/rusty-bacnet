//! Concurrent operation leases and cancellation-safe ownership of client shutdown.

use super::{ClientInner, CoreClient};
use crate::errors::to_py_err;
use crate::high_level::OpsClient;
use bacnet_transport::sc::ScEvent;
use bacnet_types::error::Error;
use pyo3::PyResult;
use pyo3::exceptions::PyRuntimeError;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Mutex, Notify, RwLock, RwLockReadGuard, RwLockWriteGuard, broadcast};
use tokio::task::JoinHandle;

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;

/// A started client: the low-level client, and the high-level client over
/// it that the priority calls go through (#2561), which shares it, its
/// discovery table and one set of request limits.
pub(super) struct Started {
    core: Arc<CoreClient>,
    /// `None` only once a stop has begun: the high-level client lets go of
    /// the low-level one first, so the stop can own it.
    high: Option<OpsClient>,
}

impl Started {
    /// `core` and a high-level client over it whose writes go under
    /// `policy`.
    pub(super) fn new(core: CoreClient, policy: bacnet_ops::WritePolicy) -> Self {
        let core = Arc::new(core);
        let high = crate::high_level::client(Arc::clone(&core), policy);
        Self {
            core,
            high: Some(high),
        }
    }

    /// Stop the low-level client, once the high-level one has let go of
    /// it. A failed stop leaves the client closed to new operations (see
    /// [`close`]), so the high-level client isn't needed again.
    async fn stop(&mut self) -> Result<(), Error> {
        self.high = None;
        let core = Arc::get_mut(&mut self.core).ok_or_else(|| {
            Error::Encoding("cannot stop BACnetClient while high-level references remain".into())
        })?;
        core.stop().await
    }
}

/// One binding owner, including an independently owned shutdown task.
#[derive(Default)]
pub(super) struct ClientState {
    client: RwLock<Option<Started>>,
    closing: AtomicBool,
    /// Woken when close is admitted, so an operation or entry queued behind
    /// a pending entry's exclusive hold fails at once instead of waiting
    /// for that entry to finish (#2030).
    closing_admitted: Notify,
    shutdown: Mutex<Option<JoinHandle<Result<(), Error>>>>,
    /// An SC client's event receiver, subscribed while it was built, so it
    /// holds the first `Connected` (#1908). The first `connection_events()`
    /// call takes it.
    first_sc_events: crate::sc_events::FirstScEvents,
}

impl ClientState {
    /// Keep the receiver an SC build subscribed, for the first
    /// `connection_events()` caller. Called before the client is published.
    pub(super) fn keep_first_sc_events(&self, events: broadcast::Receiver<ScEvent>) {
        self.first_sc_events.keep(events);
    }

    /// The build's receiver, once; `None` after the first call.
    pub(super) fn take_first_sc_events(&self) -> Option<broadcast::Receiver<ScEvent>> {
        self.first_sc_events.take()
    }
}

fn closing() -> pyo3::PyErr {
    PyRuntimeError::new_err("client closing — await stop() before starting new work")
}

/// Take `guard`, a lock on the client slot, unless close is admitted first
/// or meanwhile. A pending entry holds the slot exclusively until it
/// finishes, and close waits for it; an operation queued behind it still
/// learns of the close at once (#2030).
///
/// When the close wins, the queued lock future is dropped in that same
/// poll, leaving the lock's queue, and the arm awaits nothing, so no
/// unpolled waiter can sit on a granted guard (#1494).
async fn admitted<G>(inner: &ClientInner, guard: impl Future<Output = G>) -> PyResult<G> {
    let admitted = inner.closing_admitted.notified();
    tokio::pin!(admitted);
    // Registered before the flag is read, so a close admitted after the
    // read still wakes this waiter.
    admitted.as_mut().enable();
    if inner.closing.load(Ordering::Acquire) {
        return Err(closing());
    }
    tokio::select! {
        biased;
        () = &mut admitted => Err(closing()),
        guard = guard => {
            if inner.closing.load(Ordering::Acquire) {
                Err(closing())
            } else {
                Ok(guard)
            }
        }
    }
}

fn not_started() -> pyo3::PyErr {
    PyRuntimeError::new_err("client not started — use 'async with'")
}

/// Hold shared ownership through a native operation, without serializing readers.
pub(super) async fn started(inner: &ClientInner) -> PyResult<RwLockReadGuard<'_, CoreClient>> {
    let guard = admitted(inner, inner.client.read()).await?;
    RwLockReadGuard::try_map(guard, |slot| slot.as_ref().map(|started| &*started.core))
        .map_err(|_| not_started())
}

/// [`started`] for a call through the high-level client: the same lease,
/// so a stop waits for the call, and its handles let go of the client with
/// it.
pub(super) async fn high_level(inner: &ClientInner) -> PyResult<RwLockReadGuard<'_, OpsClient>> {
    let guard = admitted(inner, inner.client.read()).await?;
    RwLockReadGuard::try_map(guard, |slot| slot.as_ref().and_then(|s| s.high.as_ref()))
        .map_err(|_| not_started())
}

/// Keep entry exclusive through preparation and publication. A dropped startup
/// future drops this guard and its still-private builder without publishing.
pub(super) async fn starting(
    inner: &ClientInner,
) -> PyResult<RwLockWriteGuard<'_, Option<Started>>> {
    let guard = admitted(inner, inner.client.write()).await?;
    if guard.is_some() {
        return Err(PyRuntimeError::new_err("client already started"));
    }
    Ok(guard)
}

/// Close admission, drain read leases, then join native teardown. The retained
/// task survives cancellation of its Python waiter, including while core stop
/// temporarily owns lower-layer joins. A later close joins that same task.
pub(super) async fn close(inner: ClientInner) -> PyResult<()> {
    let mut shutdown = inner.shutdown.lock().await;
    if shutdown.is_none() {
        // No await separates admission closure from storing the task. A waiter
        // cancelled before this mutex is acquired cannot leave a closing flag.
        inner.closing.store(true, Ordering::Release);
        inner.closing_admitted.notify_waiters();
        let state = Arc::clone(&inner);
        *shutdown = Some(tokio::spawn(stop_native(state)));
    }
    // Await by reference: cancellation releases the mutex but retains the join.
    let result = shutdown.as_mut().expect("shutdown task installed").await;
    *shutdown = None;
    match result {
        Ok(Ok(())) => {
            inner.closing.store(false, Ordering::Release);
            Ok(())
        }
        Ok(Err(error)) => Err(to_py_err(error)),
        Err(error) => Err(PyRuntimeError::new_err(format!(
            "client shutdown task failed: {error}"
        ))),
    }
}

/// Keep the native client present until its teardown successfully completes.
async fn stop_native(state: ClientInner) -> Result<(), Error> {
    let mut client = state.client.write().await;
    if let Some(started) = client.as_mut() {
        started.stop().await?;
    }
    *client = None;
    // A receiver nobody took ends with the client; a restart subscribes anew.
    drop(state.take_first_sc_events());
    Ok(())
}
