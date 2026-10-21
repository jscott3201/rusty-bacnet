//! Concurrent operation leases and cancellation-safe ownership of client shutdown.

use super::{ClientInner, CoreClient};
use crate::errors::to_py_err;
use bacnet_transport::sc::ScEvent;
use bacnet_types::error::Error;
use pyo3::exceptions::PyRuntimeError;
use pyo3::PyResult;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, MutexGuard, PoisonError};
use tokio::sync::{broadcast, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard};
use tokio::task::JoinHandle;

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;

/// One binding owner, including an independently owned shutdown task.
#[derive(Default)]
pub(super) struct ClientState {
    client: RwLock<Option<CoreClient>>,
    closing: AtomicBool,
    shutdown: Mutex<Option<JoinHandle<Result<(), Error>>>>,
    /// An SC client's event receiver, subscribed while it was built, so it
    /// holds the first `Connected` (#1908). The first `connection_events()`
    /// call takes it. A leaf lock: never held across an await.
    first_sc_events: StdMutex<Option<broadcast::Receiver<ScEvent>>>,
}

impl ClientState {
    /// Keep the receiver an SC build subscribed, for the first
    /// `connection_events()` caller. Called before the client is published.
    pub(super) fn keep_first_sc_events(&self, events: broadcast::Receiver<ScEvent>) {
        *self.first_sc_events() = Some(events);
    }

    /// The build's receiver, once; `None` after the first call.
    pub(super) fn take_first_sc_events(&self) -> Option<broadcast::Receiver<ScEvent>> {
        self.first_sc_events().take()
    }

    fn first_sc_events(&self) -> MutexGuard<'_, Option<broadcast::Receiver<ScEvent>>> {
        self.first_sc_events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

fn closing() -> pyo3::PyErr {
    PyRuntimeError::new_err("client closing — await stop() before starting new work")
}

/// Hold shared ownership through a native operation, without serializing readers.
pub(super) async fn started(inner: &ClientInner) -> PyResult<RwLockReadGuard<'_, CoreClient>> {
    if inner.closing.load(Ordering::Acquire) {
        return Err(closing());
    }
    let guard = inner.client.read().await;
    if inner.closing.load(Ordering::Acquire) {
        return Err(closing());
    }
    RwLockReadGuard::try_map(guard, Option::as_ref)
        .map_err(|_| PyRuntimeError::new_err("client not started — use 'async with'"))
}

/// Keep entry exclusive through preparation and publication. A dropped startup
/// future drops this guard and its still-private builder without publishing.
pub(super) async fn starting(
    inner: &ClientInner,
) -> PyResult<RwLockWriteGuard<'_, Option<CoreClient>>> {
    if inner.closing.load(Ordering::Acquire) {
        return Err(closing());
    }
    let guard = inner.client.write().await;
    if inner.closing.load(Ordering::Acquire) {
        return Err(closing());
    }
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
    if let Some(client) = client.as_mut() {
        client.stop().await?;
    }
    *client = None;
    // A receiver nobody took ends with the client; a restart subscribes anew.
    drop(state.take_first_sc_events());
    Ok(())
}
