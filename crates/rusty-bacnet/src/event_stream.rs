//! The receiving end of a Rust broadcast stream behind a Python async
//! iterator: SC connection events (#1829), I-Haves and network reports
//! (#1850), and COV notifications.
//!
//! The Rust senders never wait for a receiver: one that falls behind the
//! channel's capacity loses the oldest items. The Python iterators keep
//! going past such a gap and count what was lost in `missed`, so a slow
//! Python consumer costs only its own items, never the sender's progress.
//! Holding one holds no sender, so once the owner stops and its senders
//! drop, the next item is the end of the iteration.
//!
//! An item whose `__anext__` future was cancelled while the item was being
//! delivered, as a timed-out `asyncio.wait_for` does, comes back through
//! [`PutBack`] into a one-item slot that the next `__anext__` returns first
//! (#1858). Deliveries go one at a time: while one is in flight, from the
//! channel to its asyncio future or back into the slot, the next `next()`
//! waits, so a later item can't overtake one on its way back, and neither
//! can the end of the stream. A delivery left unsettled for
//! [`STALL_WARNING`], as when its event loop stopped with the `set_result`
//! callback still queued, raises one `RuntimeWarning`; the read keeps
//! waiting, since dropping the item would break the order.
//!
//! `missed` counts the items lost: to lag, and to a delivery dropped before
//! its item reached a future or the slot (an aborted task, a failed
//! conversion, a closed exit gate or a closed loop that dropped the queued
//! callback).

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex, MutexGuard, PoisonError};
use std::time::Duration;

use pyo3::IntoPyObjectExt;
use pyo3::prelude::*;
use tokio::sync::broadcast::{self, error::RecvError};
use tokio::sync::{Mutex, Notify};

use crate::py_async::{PutBack, Via};

/// How long a read waits behind an unsettled delivery before it warns.
#[cfg(not(test))]
pub(crate) const STALL_WARNING: Duration = Duration::from_secs(5);
/// Short in tests, so a stalled loop shows up quickly.
#[cfg(test)]
pub(crate) const STALL_WARNING: Duration = Duration::from_millis(300);

#[cfg(test)]
#[path = "event_stream_tests.rs"]
mod tests;

/// A broadcast receiver shared by one Python iterator's pending
/// `__anext__` futures, with its redelivery slot and a running count of
/// the items lost.
pub(crate) struct LossyReceiver<T> {
    rx: Arc<Mutex<broadcast::Receiver<T>>>,
    slot: Arc<Redelivery>,
}

/// The one-item slot for an item a cancelled `__anext__` could not take.
#[derive(Default)]
pub(crate) struct Redelivery {
    /// A leaf lock: held only to swap the item, never across an await or a
    /// call into Python.
    item: StdMutex<Option<Py<PyAny>>>,
    /// Wakes a waiting `next()` when the slot fills or a delivery settles.
    filled: Notify,
    missed: AtomicU64,
    /// Deliveries handed out by `next()` and not yet settled: at most one,
    /// since `next()` hands out none while this is above zero.
    in_flight: AtomicUsize,
    /// Set once the stall warning has gone out, so it goes out once.
    warned: AtomicBool,
    /// Which put-back and settle paths ran, for the tests.
    #[cfg(test)]
    trace: Trace,
}

/// Counts of the delivery paths taken, so tests can tell which one ran
/// instead of trusting their timing.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct Trace {
    /// Put back by the binding thread: the future was done before delivery.
    pub(crate) before_delivery: AtomicU64,
    /// Put back by the `set_result` callback on the loop.
    pub(crate) on_loop: AtomicU64,
    /// Put back because the loop was closed.
    pub(crate) closed_loop: AtomicU64,
    /// Settled with the item in its future.
    pub(crate) delivered: AtomicU64,
    /// `set_result` callbacks queued on the loop.
    pub(crate) queued: AtomicU64,
}

impl Redelivery {
    fn item(&self) -> MutexGuard<'_, Option<Py<PyAny>>> {
        self.item.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn take(&self) -> Option<Py<PyAny>> {
        self.item().take()
    }

    /// Raise one `RuntimeWarning` for a read held up behind a delivery that
    /// has not settled, from a blocking thread so no runtime worker waits
    /// for the GIL.
    fn warn_stalled(&self) {
        if self.warned.swap(true, Ordering::AcqRel) {
            return;
        }
        tokio::task::spawn_blocking(|| {
            let _ = crate::py_async::attach(|py| {
                PyErr::warn(
                    py,
                    &py.get_type::<pyo3::exceptions::PyRuntimeWarning>(),
                    c"an event iterator's read is held up behind an earlier event that has not \
                      reached its asyncio future, whose event loop may have stopped: read each \
                      iterator from one running event loop",
                    0,
                )
            });
        });
    }

    fn put_back(&self, value: Py<PyAny>) {
        let refused = {
            let mut item = self.item();
            match *item {
                None => {
                    *item = Some(value);
                    None
                }
                Some(_) => Some(value),
            }
        };
        match refused {
            None => self.filled.notify_one(),
            // The slot keeps the older item and the newer one is dropped.
            // With one delivery in flight at a time the slot is empty here,
            // so this only backs up that rule. Dropped after the lock is
            // released, with the GIL held.
            Some(value) => {
                drop(value);
                self.missed.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// One `__anext__`'s delivery: armed once its `next()` hands out an item,
/// and settled once that item reached the asyncio future or went back into
/// the slot, or, failing both, when the last handle on it drops.
struct Delivery {
    slot: Arc<Redelivery>,
    armed: AtomicBool,
    /// Whether the item went back into the slot, for the tests' trace.
    #[cfg(test)]
    put: AtomicBool,
}

impl Delivery {
    /// Count the item handed out as in flight. Runs under the receiver lock.
    fn arm(&self) {
        self.slot.in_flight.fetch_add(1, Ordering::AcqRel);
        self.armed.store(true, Ordering::Release);
    }

    /// Settle an armed delivery, once; an unarmed one has nothing to settle.
    fn settle(&self) {
        if self.armed.swap(false, Ordering::AcqRel) {
            self.slot.in_flight.fetch_sub(1, Ordering::AcqRel);
            self.slot.filled.notify_one();
        }
    }
}

impl PutBack for Delivery {
    fn put_back(&self, _py: Python<'_>, value: Py<PyAny>, via: Via) {
        #[cfg(test)]
        {
            self.put.store(true, Ordering::Relaxed);
            let trace = &self.slot.trace;
            match via {
                Via::BeforeDelivery => &trace.before_delivery,
                Via::OnLoop => &trace.on_loop,
                Via::ClosedLoop => &trace.closed_loop,
            }
            .fetch_add(1, Ordering::Relaxed);
        }
        let _ = via;
        self.slot.put_back(value);
    }

    fn settled(&self) {
        #[cfg(test)]
        if self.armed.load(Ordering::Acquire) && !self.put.load(Ordering::Relaxed) {
            self.slot.trace.delivered.fetch_add(1, Ordering::Relaxed);
        }
        self.settle();
    }

    #[cfg(test)]
    fn queued(&self) {
        self.slot.trace.queued.fetch_add(1, Ordering::Relaxed);
    }
}

impl Drop for Delivery {
    /// An item dropped on its way (an aborted task, a failed conversion, a
    /// closed exit gate, a closed loop that dropped the queued callback)
    /// counts as missed and settles here, so the count never stays raised.
    fn drop(&mut self) {
        if self.armed.load(Ordering::Acquire) {
            self.slot.missed.fetch_add(1, Ordering::Relaxed);
        }
        self.settle();
    }
}

/// What `next()` yields: an item fresh from the channel, or one a cancelled
/// `__anext__` put back, already a Python object.
pub(crate) enum Next<T> {
    /// From the channel.
    Fresh(T),
    /// From the redelivery slot.
    Again(Py<PyAny>),
}

impl<T> Next<T> {
    /// Convert a fresh item; a put-back one is already converted.
    pub(crate) fn map<U>(self, f: impl FnOnce(T) -> U) -> Next<U> {
        match self {
            Self::Fresh(item) => Next::Fresh(f(item)),
            Self::Again(object) => Next::Again(object),
        }
    }
}

impl<'py, T: IntoPyObject<'py>> IntoPyObject<'py> for Next<T> {
    type Target = PyAny;
    type Output = Bound<'py, PyAny>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        match self {
            Self::Fresh(item) => item.into_bound_py_any(py),
            Self::Again(object) => Ok(object.into_bound(py)),
        }
    }
}

impl<T: Clone + Send + 'static> LossyReceiver<T> {
    pub(crate) fn new(rx: broadcast::Receiver<T>) -> Self {
        Self {
            rx: Arc::new(Mutex::new(rx)),
            slot: Arc::default(),
        }
    }

    /// Items lost so far: to lag, or dropped on the way to a read.
    pub(crate) fn missed(&self) -> u64 {
        self.slot.missed.load(Ordering::Relaxed)
    }

    /// The next item, with the put-back hook for that same delivery: hand
    /// both to `future_into_py_or_put_back`.
    ///
    /// The future yields the item in the redelivery slot, else the next one
    /// from the channel; `None` once every sender is gone and the slot is
    /// empty. A lag adds the count lost to `missed` and goes on to the
    /// oldest item still held. It first waits for the delivery before it to
    /// settle, so an item on its way back into the slot comes before any
    /// later item and before the end.
    ///
    /// Dropping the future before it yields loses no item and leaves
    /// nothing in flight: the lock, `recv` and `notified` are cancel-safe,
    /// and the slot is taken, and the delivery armed, only on the poll that
    /// returns. The lock only orders this iterator's own concurrent
    /// `__anext__` calls; no Python thread waits on it.
    pub(crate) fn next(
        &self,
    ) -> (
        impl std::future::Future<Output = Option<Next<T>>> + Send + 'static + use<T>,
        Arc<dyn PutBack>,
    ) {
        let rx = Arc::clone(&self.rx);
        let delivery = Arc::new(Delivery {
            slot: Arc::clone(&self.slot),
            armed: AtomicBool::new(false),
            #[cfg(test)]
            put: AtomicBool::new(false),
        });
        let hook: Arc<dyn PutBack> = delivery.clone();
        let next = async move {
            let slot = &delivery.slot;
            let mut rx = rx.lock().await;
            loop {
                // Registered before the checks, so a settle or put-back
                // between a check and the await still wakes this.
                let wake = slot.filled.notified();
                tokio::pin!(wake);
                wake.as_mut().enable();
                if slot.in_flight.load(Ordering::Acquire) > 0 {
                    // Bounded only to warn: the item ahead must come first.
                    if tokio::time::timeout(STALL_WARNING, wake.as_mut())
                        .await
                        .is_err()
                    {
                        slot.warn_stalled();
                        wake.await;
                    }
                    continue;
                }
                if let Some(object) = slot.take() {
                    delivery.arm();
                    return Some(Next::Again(object));
                }
                tokio::select! {
                    biased;
                    () = &mut wake => {}
                    received = rx.recv() => match received {
                        Ok(item) => {
                            delivery.arm();
                            return Some(Next::Fresh(item));
                        }
                        Err(RecvError::Lagged(lost)) => {
                            slot.missed.fetch_add(lost, Ordering::Relaxed);
                        }
                        Err(RecvError::Closed) => {
                            let object = slot.take()?;
                            delivery.arm();
                            return Some(Next::Again(object));
                        }
                    },
                }
            }
        };
        (next, hook)
    }
}

/// `StopAsyncIteration`, for a stream whose owner has stopped.
pub(crate) fn ended(what: &str) -> pyo3::PyErr {
    pyo3::exceptions::PyStopAsyncIteration::new_err(format!("{what} ended: the client stopped"))
}
