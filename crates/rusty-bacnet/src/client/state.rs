//! Concurrent operation leases and cancellation-safe ownership of client shutdown.

use super::{ClientInner, CoreClient};
use crate::errors::{lifecycle_error, to_py_err};
use crate::high_level::{OpsClient, OpsDevice};
use bacnet_transport::sc::ScEvent;
use bacnet_types::error::{Error, Lifecycle};
use bacnet_types::sync::MutexExt;
use pyo3::PyResult;
use pyo3::exceptions::PyRuntimeError;
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::sync::{Mutex, Notify, RwLock, RwLockReadGuard, RwLockWriteGuard, broadcast};
use tokio::task::JoinHandle;

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "watches_tests.rs"]
mod watches_tests;

#[cfg(test)]
impl ClientState {
    /// Another reference to the started low-level client, so a test can
    /// make its stop fail as a stray reference would.
    pub(crate) async fn hold_core_for_test(&self) -> Option<Arc<CoreClient>> {
        let slot = self.client.read().await;
        slot.as_ref().map(|started| Arc::clone(&started.core))
    }
}

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
    /// `policy`, and whose handles ask for a bare instance in `scope`.
    pub(super) fn new(
        core: CoreClient,
        policy: bacnet_ops::WritePolicy,
        scope: bacnet_ops::ResolveScope,
    ) -> Self {
        let core = Arc::new(core);
        let high = crate::high_level::client(Arc::clone(&core), policy, scope);
        Self {
            core,
            high: Some(high),
        }
    }

    /// This client, its watches taking `settings` unless a watch names
    /// others. Their renewals leave room for the low-level client's own
    /// timers, which the high-level client reads from it.
    pub(super) fn with_watches(mut self, settings: bacnet_ops::WatchSettings) -> Self {
        self.high = self.high.map(|high| high.with_watch_settings(settings));
        self
    }

    /// Stop the low-level client, once the high-level one has let go of
    /// it. A failed stop leaves the client closed to new operations (see
    /// [`close`]), so the high-level client isn't needed again. With
    /// `settle`, a watch was made since the last stop: the tasks its hub
    /// aborted may hold the client a moment longer, so the stop waits up
    /// to [`SETTLE`](super::watches::SETTLE) for them to go.
    async fn stop(&mut self, settle: bool) -> Result<(), Error> {
        self.high = None;
        if settle {
            let until = tokio::time::Instant::now() + super::watches::SETTLE;
            while Arc::get_mut(&mut self.core).is_none() && tokio::time::Instant::now() < until {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        }
        let core = Arc::get_mut(&mut self.core).ok_or_else(|| {
            Error::limit_reached("cannot stop BACnetClient while high-level references remain")
        })?;
        core.stop().await
    }
}

/// One binding owner, including an independently owned shutdown task.
#[derive(Default)]
pub(crate) struct ClientState {
    pub(super) frames: crate::frames::FrameSubscriptions,
    /// The live watches, each holding the high-level client through its
    /// hub until it ends: a stop ends them before it takes the low-level
    /// client (#2561).
    pub(super) watches: super::watches::Watches,
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
    /// The high-level handles of the Python `Device`s made on the started
    /// client, by [`HandleId`] (#2561). Each holds the high-level client,
    /// so a stop empties this before it takes the low-level one, and a
    /// Python `Device` keeps only its id and what it was named by. An entry
    /// is added only under a read lease and the map emptied only under the
    /// stop's exclusive hold, so every entry belongs to the client now
    /// started. A std mutex, held only to look up, add or remove: a
    /// `Device`'s drop removes its entry without waiting.
    handles: std::sync::Mutex<HashMap<HandleId, OpsDevice>>,
    next_handle: AtomicU64,
    /// A client on an endpoint's link (#2617), which its endpoint starts
    /// and which can't be started again once stopped.
    seated: bool,
    /// The lookups in flight for routed stations, one per station (#2617).
    pub(super) route_misses: super::routed::StationLookups,
    /// Why the last stop failed, until a stop is tried again: an endpoint
    /// asked for a client then says so (#2617).
    stop_failure: std::sync::Mutex<Option<String>>,
    /// The device instance of the endpoint this client is seated on, set
    /// at the seat: its I-Am arrivals leave out that device's own I-Am come
    /// back, as the endpoint's do (#2612).
    own_instance: std::sync::OnceLock<u32>,
}

/// Where a client on an endpoint's link stands, for the endpoint deciding
/// whether `endpoint.client()` hands it out again (#2617).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Liveness {
    /// Started: calls go through.
    Started,
    /// A stop is admitted or running: the Rust client may still hold the
    /// endpoint's seat.
    Stopping,
    /// The last stop failed, for this reason: the client stays closed to
    /// calls and may still hold the seat until a stop is tried again.
    Failed(String),
    /// Stopped: the Rust client, and with it the seat, is gone.
    Stopped,
}

impl ClientState {
    /// The state of a client on an endpoint's link (#2617), empty until
    /// [`Self::seat`] puts the endpoint's full client in it.
    pub(super) fn seated() -> Self {
        Self {
            seated: true,
            ..Self::default()
        }
    }

    /// Start this seated state on `core`, the endpoint session's full
    /// client, with the high-level client over it that the standalone one
    /// has: the default write policy, and a bare instance asked for on this
    /// network alone. Called once, before the Python object is handed out,
    /// so the slot is never contended; `false` if it was already filled.
    /// `own_instance` is the endpoint's device instance, when it has one.
    pub(crate) fn seat(&self, core: CoreClient, own_instance: Option<u32>) -> bool {
        match self.client.try_write() {
            Ok(mut slot) if slot.is_none() => {
                *slot = Some(Started::new(
                    core,
                    bacnet_ops::WritePolicy::default(),
                    bacnet_ops::ResolveScope::Local,
                ));
                if let Some(instance) = own_instance {
                    let _ = self.own_instance.set(instance);
                }
                true
            }
            _ => false,
        }
    }

    /// The device instance of the endpoint this client is seated on;
    /// `None` for a standalone client, which sends no I-Am.
    pub(crate) fn own_instance(&self) -> Option<u32> {
        self.own_instance.get().copied()
    }

    /// Record why the last stop failed, or that a new stop is under way.
    fn set_stop_failure(&self, failure: Option<String>) {
        *self.stop_failure.locked() = failure;
    }

    /// Where this client stands; see [`Liveness`]. Sends nothing and waits
    /// for nothing.
    pub(crate) fn liveness(&self) -> Liveness {
        if self.closing.load(Ordering::Acquire) {
            let failure = self.stop_failure.locked().clone();
            return failure.map_or(Liveness::Stopping, Liveness::Failed);
        }
        match self.client.try_read() {
            Ok(slot) if slot.is_some() => Liveness::Started,
            Ok(_) => Liveness::Stopped,
            // A writer holds or awaits the slot: a stop has begun.
            Err(_) => Liveness::Stopping,
        }
    }
}

/// What names one Python `Device`'s handle among its client's.
pub(crate) type HandleId = u64;

/// A high-level handle held through one call: the handle, then the lease
/// that keeps the client started, dropped in that order.
pub(crate) struct Held<'a> {
    /// The handle. A clone of the client's own, so its place is shared.
    pub(crate) device: OpsDevice,
    _lease: RwLockReadGuard<'a, OpsClient>,
}

impl ClientState {
    /// An id for a new Python `Device`.
    pub(crate) fn new_handle(&self) -> HandleId {
        self.next_handle.fetch_add(1, Ordering::Relaxed)
    }

    /// What `read` makes of the handle `id` names, if the client now
    /// started has made it. `read` runs under the handles' mutex and gets
    /// no clone to keep: outside a lease, a clone alive while a stop takes
    /// the low-level client would fail that stop.
    pub(crate) fn with_handle<R>(
        &self,
        id: HandleId,
        read: impl FnOnce(&OpsDevice) -> R,
    ) -> Option<R> {
        let handles = self.handles.locked();
        handles.get(&id).map(read)
    }

    /// Forget the handle `id` names: its Python `Device` is gone.
    pub(crate) fn drop_handle(&self, id: HandleId) {
        let dropped = {
            let mut handles = self.handles.locked();
            handles.remove(&id)
        };
        drop(dropped);
    }

    /// Drop every handle, outside the mutex: a stop's first step.
    fn drop_handles(&self) {
        let dropped = {
            let mut handles = self.handles.locked();
            std::mem::take(&mut *handles)
        };
        drop(dropped);
    }
}

/// The handle `id` names on the client now started, held with a lease for
/// one call. The first call after a start makes it with `make` (a stop
/// dropped the one before), and later calls share that one, so a following
/// handle keeps where its device went from call to call.
///
/// # Errors
///
/// The lease's (not started, or closing), and `make`'s.
pub(crate) async fn held<'a, F>(
    inner: &'a ClientInner,
    id: HandleId,
    make: impl FnOnce(OpsClient) -> F,
) -> PyResult<Held<'a>>
where
    F: Future<Output = PyResult<OpsDevice>>,
{
    let lease = high_level(inner).await?;
    // Under the lease a clone is safe: the stop waits for the lease.
    let device = match inner.with_handle(id, OpsDevice::clone) {
        Some(device) => device,
        None => {
            let made = make(lease.clone()).await?;
            let mut handles = inner.handles.locked();
            handles.entry(id).or_insert(made).clone()
        }
    };
    Ok(Held {
        device,
        _lease: lease,
    })
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
    lifecycle_error(
        Lifecycle::Stopping,
        "client closing — await stop() before starting new work",
    )
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

fn not_started(inner: &ClientInner) -> pyo3::PyErr {
    if inner.seated {
        // Its endpoint started it; once stopped it is done (#2617).
        return lifecycle_error(
            Lifecycle::Closed,
            "this client ran on an endpoint and is stopped, or its endpoint is: ask the \
             running endpoint for another with 'await endpoint.client()'",
        );
    }
    // A standalone client starts again after its stop.
    lifecycle_error(
        Lifecycle::NotRunning,
        "client not started — use 'async with'",
    )
}

/// Hold shared ownership through a native operation, without serializing readers.
pub(super) async fn started(inner: &ClientInner) -> PyResult<RwLockReadGuard<'_, CoreClient>> {
    let guard = admitted(inner, inner.client.read()).await?;
    RwLockReadGuard::try_map(guard, |slot| slot.as_ref().map(|started| &*started.core))
        .map_err(|_| not_started(inner))
}

/// [`started`] for a call through the high-level client: the same lease,
/// so a stop waits for the call, and its handles let go of the client with
/// it.
pub(crate) async fn high_level(inner: &ClientInner) -> PyResult<RwLockReadGuard<'_, OpsClient>> {
    let guard = admitted(inner, inner.client.read()).await?;
    RwLockReadGuard::try_map(guard, |slot| slot.as_ref().and_then(|s| s.high.as_ref()))
        .map_err(|_| not_started(inner))
}

/// Keep entry exclusive through preparation and publication. A dropped startup
/// future drops this guard and its still-private builder without publishing.
pub(super) async fn starting(
    inner: &ClientInner,
) -> PyResult<RwLockWriteGuard<'_, Option<Started>>> {
    let guard = admitted(inner, inner.client.write()).await?;
    if guard.is_some() {
        return Err(lifecycle_error(
            Lifecycle::AlreadyStarted,
            "client already started",
        ));
    }
    Ok(guard)
}

/// Close admission, drain read leases, then join native teardown. The retained
/// task survives cancellation of its Python waiter, including while core stop
/// temporarily owns lower-layer joins. A later close joins that same task.
/// An endpoint ends the client on its link this way too (#2617): dropping
/// the Rust client frees the endpoint's seat.
pub(crate) async fn close(inner: ClientInner) -> PyResult<()> {
    let mut shutdown = inner.shutdown.lock().await;
    if shutdown.is_none() {
        // No await separates admission closure from storing the task. A waiter
        // cancelled before this mutex is acquired cannot leave a closing flag.
        inner.closing.store(true, Ordering::Release);
        inner.closing_admitted.notify_waiters();
        inner.frames.close();
        // Watches' iterators end now; their hubs end once the leases drain.
        inner.close_watch_streams();
        // This try is a new one: an earlier failure no longer stands.
        inner.set_stop_failure(None);
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
        // stop_native records its failure itself, waiter or not; this
        // covers a shutdown task that isn't one.
        Ok(Err(error)) => {
            inner.set_stop_failure(Some(error.to_string()));
            Err(to_py_err(error))
        }
        Err(error) => {
            let message = format!("client shutdown task failed: {error}");
            inner.set_stop_failure(Some(message.clone()));
            Err(PyRuntimeError::new_err(message))
        }
    }
}

/// Keep the native client present until its teardown successfully completes.
async fn stop_native(state: ClientInner) -> Result<(), Error> {
    let mut client = state.client.write().await;
    // A frame subscription admitted before close may have finished while
    // close waited for its read lease. End those subscriptions too.
    state.frames.close();
    // The watches' hubs and the handles hold the high-level client: they
    // go first. A watch's iterator ends at once; the hubs' cancels get a
    // bounded wait.
    let high = client.as_ref().and_then(|started| started.high.clone());
    let settle = state.end_watches(super::watches::END_WAIT, high).await;
    state.drop_handles();
    if let Some(started) = client.as_mut()
        && let Err(error) = started.stop(settle).await
    {
        // Recorded here, so a stop() whose waiter was cancelled still
        // leaves the cause for an endpoint asked for a client (#2617).
        state.set_stop_failure(Some(error.to_string()));
        return Err(error);
    }
    *client = None;
    // A receiver nobody took ends with the client; a restart subscribes anew.
    drop(state.take_first_sc_events());
    Ok(())
}
