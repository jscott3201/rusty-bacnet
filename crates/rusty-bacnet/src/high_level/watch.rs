//! [`PyWatch`]: what `await device.watch(points)` returns (#2561), a
//! device's points watched as an async iterator of `WatchUpdate`s, over a
//! `bacnet-ops` watch hub of the watch's own ([`Hub::stream`]).
//!
//! Each watch runs its own hub, with its own `lifetime` and
//! `poll_interval`, and its own range of subscriber process identifiers.
//! The hub holds the high-level client while it runs, so the watch never
//! takes a call's lease between steps: its client keeps it
//! ([`crate::client::WatchTie`]) and ends it before a stop takes the
//! low-level client. A watch ends on `close()` (or leaving `async with`),
//! the Python object going away, or its client's stop, whichever comes
//! first. Its iterator ends at once, for good, raising
//! `StopAsyncIteration`; then its hub cancels the subscriptions (best
//! effort, waiting up to [`END_WAIT`]), stops its tasks, and lets go of the
//! client. A restarted client doesn't revive it: watch again.
//!
//! Updates go through the event iterators' receiver ([`LossyReceiver`])
//! from the hub's mailbox, which keeps the latest update of each point for
//! a consumer that falls behind ([`PyWatch::folded`] counts what it folded
//! away). The read is cancel-safe: the mailbox gives an update up only on
//! the poll that returns it, and an update whose `__anext__` was cancelled
//! on its way to Python goes back into the receiver's slot and comes out
//! first, so none is lost or handed out twice.

use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use bacnet_ops::PropertyRef;
use bacnet_ops::target::AddressStyle;
use bacnet_ops::watch::hub::{
    Backoff, Hub, Lifetime, MIN_POLL, MissingDevice, POLL_FLOOR, Reads, WatchOptions,
    renewal_margin,
};
use bacnet_ops::watch::{MAX_WATCHED, Stream, Update, Watch};
use pyo3::exceptions::{PyStopAsyncIteration, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyFloat, PyInt, PyTuple};
use tokio::sync::broadcast::error::RecvError;

use super::OpsDevice;
use super::device::Core;
use super::errors::ops_err;
use super::specs::property_item;
use super::watch_update::PyWatchUpdate;
use crate::app_transport::AppTransport;
use crate::client::{END_WAIT, WatchTie};
use crate::event_stream::{EventSource, LossyReceiver};
use crate::py_async::{future_into_py, future_into_py_or_put_back};
use crate::types::{PyObjectIdentifier, PyPropertyIdentifier};

#[cfg(test)]
#[path = "watch_tests.rs"]
mod tests;

/// A watch's hub.
pub(crate) type OpsHub = Hub<AppTransport>;

/// The longest `poll_interval` taken, in seconds: an hour.
const MAX_POLL_SECS: f64 = 3600.0;

/// Why a watch's iterator ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ended {
    /// `close()`, `async with`, or the Python object went away.
    Closed,
    /// Its client stopped.
    Stopped,
}

impl Ended {
    fn error(self) -> PyErr {
        PyStopAsyncIteration::new_err(match self {
            Self::Closed => "the watch ended: it was closed",
            Self::Stopped => "the watch ended: the client stopped",
        })
    }
}

/// A watch's updates, as the event iterators' receiver reads them.
pub(crate) struct Updates {
    watch: Watch,
    folded: Arc<AtomicU64>,
}

impl EventSource<Box<Update>> for Updates {
    /// The next update; the mailbox gives one up only on the poll that
    /// returns it, so a dropped read loses none.
    async fn recv(&mut self) -> Result<Box<Update>, RecvError> {
        let next = std::future::poll_fn(|cx| Pin::new(&mut self.watch).poll_next(cx)).await;
        self.folded.store(self.watch.folded(), Ordering::Relaxed);
        next.map(Box::new).ok_or(RecvError::Closed)
    }
}

/// What a watch shares with its client: its hub, until the watch ends, and
/// its stream of updates.
pub(crate) struct WatchShared {
    /// Taken once the hub has shut down; held across that shutdown, so a
    /// second end waits for the first.
    hub: tokio::sync::Mutex<Option<OpsHub>>,
    stream: LossyReceiver<Box<Update>, Updates>,
    /// Why the iterator ended, once it has; a step's future keeps this
    /// alone.
    ended: Arc<OnceLock<Ended>>,
    folded: Arc<AtomicU64>,
}

impl WatchShared {
    /// `watch`, of `hub`'s points.
    pub(crate) fn new(hub: OpsHub, watch: Watch) -> Self {
        let folded = Arc::new(AtomicU64::new(0));
        let updates = Updates {
            watch,
            folded: Arc::clone(&folded),
        };
        Self {
            hub: tokio::sync::Mutex::new(Some(hub)),
            stream: LossyReceiver::from_source(updates),
            ended: Arc::new(OnceLock::new()),
            folded,
        }
    }

    /// End the iterator, for `why` unless it has ended already: a pending
    /// or later `__anext__` raises `StopAsyncIteration`. Sends nothing.
    pub(crate) fn close_stream(&self, why: Ended) {
        let _ = self.ended.set(why);
        self.stream.close();
    }

    /// End the watch: its iterator, then its hub, which cancels the
    /// subscriptions it holds, waiting up to `wait` for the cancels (those
    /// not answered by then lapse with their lifetime), stops its tasks and
    /// lets go of the client. Once; a later end waits for an earlier one.
    pub(crate) async fn end(&self, why: Ended, wait: Duration) {
        self.close_stream(why);
        let mut hub = self.hub.lock().await;
        if let Some(running) = hub.as_ref() {
            running.shutdown(wait).await;
        }
        *hub = None;
    }

    /// Why the iterator ended, once it has.
    pub(crate) fn ended_why(&self) -> Option<Ended> {
        self.ended.get().copied()
    }

    /// Whether the hub is still held: until an end has shut it down.
    #[cfg(test)]
    pub(crate) async fn has_hub(&self) -> bool {
        self.hub.lock().await.is_some()
    }
}

/// The settings `lifetime` and `poll_interval` give, checked when the call
/// is made.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Settings {
    lifetime: Lifetime,
    poll_every: Duration,
}

impl Settings {
    /// `lifetime`: an int of seconds, 1 or more, 300 when None;
    /// `poll_interval`: an int or float of seconds, 1 to 3600, 10 when
    /// None. A bool is neither.
    pub(super) fn new(
        lifetime: Option<&Bound<'_, PyAny>>,
        poll_interval: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let lifetime = match lifetime.filter(|lifetime| !lifetime.is_none()) {
            Some(lifetime) => lifetime_arg(lifetime)?,
            None => Lifetime::DEFAULT,
        };
        let poll_every = match poll_interval.filter(|interval| !interval.is_none()) {
            Some(interval) => poll_arg(interval)?,
            None => MIN_POLL,
        };
        Ok(Self {
            lifetime,
            poll_every,
        })
    }

    /// The hub's settings: subscriptions of the lifetime, renewed early
    /// enough to cover `request_budget` twice; the device's polled points
    /// read together every `poll_every`, backing off to a minute (or the
    /// interval, if longer) when it stops answering; a point whose
    /// subscription went unanswered or whose renewal failed polled for 30
    /// s, doubling to 5 minutes, before it is subscribed again; each point
    /// read for its value, Status_Flags, Units, name and state texts; every
    /// change signalled, the watch's mailbox folding them.
    pub(super) fn options(self, request_budget: Duration) -> WatchOptions {
        let poll_every = self.poll_every;
        WatchOptions {
            lifetime: self.lifetime,
            renewal_margin: renewal_margin(self.lifetime.secs(), request_budget),
            poll_every,
            poll_floor: POLL_FLOOR,
            poll_backoff: Backoff {
                first: poll_every,
                longest: poll_every.max(Duration::from_secs(60)),
            },
            retry_cov: Backoff {
                first: Duration::from_secs(30),
                longest: Duration::from_secs(300),
            },
            missing: MissingDevice::Poll,
            reads: Reads::value(),
            coalesce: Duration::ZERO,
            max_points: MAX_WATCHED,
            ..WatchOptions::default()
        }
    }
}

/// A `lifetime` argument: an int of seconds, 1 or more.
fn lifetime_arg(lifetime: &Bound<'_, PyAny>) -> PyResult<Lifetime> {
    if !lifetime.is_instance_of::<PyInt>() || lifetime.is_instance_of::<PyBool>() {
        return Err(PyTypeError::new_err(format!(
            "lifetime is an int of seconds, not {}",
            lifetime.get_type().name()?
        )));
    }
    lifetime
        .extract::<u32>()
        .ok()
        .and_then(Lifetime::new)
        .ok_or_else(|| {
            PyValueError::new_err(format!(
                "lifetime is 1 to {} seconds, not {lifetime}: a watch's subscriptions always \
                 lapse",
                u32::MAX
            ))
        })
}

/// A `poll_interval` argument: an int or float of seconds, 1 to 3600.
fn poll_arg(interval: &Bound<'_, PyAny>) -> PyResult<Duration> {
    let number = interval.is_instance_of::<PyInt>() || interval.is_instance_of::<PyFloat>();
    if !number || interval.is_instance_of::<PyBool>() {
        return Err(PyTypeError::new_err(format!(
            "poll_interval is a number of seconds, not {}",
            interval.get_type().name()?
        )));
    }
    let secs: f64 = interval.extract()?;
    let floor = POLL_FLOOR.as_secs_f64();
    if !(floor..=MAX_POLL_SECS).contains(&secs) {
        return Err(PyValueError::new_err(format!(
            "poll_interval is {floor} to {MAX_POLL_SECS} seconds, not {secs}"
        )));
    }
    Ok(Duration::from_secs_f64(secs))
}

/// A `points` argument: an iterable of what `read_many` takes, each point
/// once, in order. Not one str; at least one point, at most
/// [`MAX_WATCHED`]; no property that stands for many.
pub(super) fn points_arg(points: &Bound<'_, PyAny>) -> PyResult<Vec<PropertyRef>> {
    super::points::refuse_text(points, "points", "['ai:1', 'ao:2 priority-array[8]']")?;
    let mut wanted: Vec<PropertyRef> = Vec::new();
    for item in points.try_iter()? {
        let point = property_item(&item?)?;
        super::device::many_refused(point.property)?;
        if !wanted.contains(&point) {
            wanted.push(point);
        }
        if wanted.len() > MAX_WATCHED {
            return Err(PyValueError::new_err(format!(
                "a watch takes at most {MAX_WATCHED} points"
            )));
        }
    }
    if wanted.is_empty() {
        return Err(PyValueError::new_err(
            "points is empty: name a point to watch",
        ));
    }
    Ok(wanted)
}

/// Watch `wanted` on the device `core` names, under `settings`: the
/// handle, held with the client's lease while the hub is made and the
/// client keeps it.
pub(super) async fn open(
    core: Arc<Core>,
    wanted: Vec<PropertyRef>,
    settings: Settings,
) -> PyResult<PyWatch> {
    let budget = core.tie().request_budget();
    let keeper = Arc::clone(&core);
    core.run(move |device: OpsDevice, style| async move {
        let points = wanted
            .iter()
            .map(|point| device.point(point))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| ops_err(error, style))?;
        // Inside the runtime, as the hub's tasks need; nothing awaited
        // from here until the client keeps the watch.
        let hub = Hub::new(device.client().clone(), settings.options(budget))
            .map_err(PyValueError::new_err)?;
        let watch = hub
            .stream(points)
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        let shared = Arc::new(WatchShared::new(hub, watch));
        let tie = keeper.tie().keep_watch(Arc::clone(&shared));
        Ok(PyWatch {
            shared,
            tie,
            address: device.address().styled(style),
            points: wanted,
            settings,
            style,
        })
    })
    .await
}

/// One `__anext__` over `stream`, as an asyncio future: the next update,
/// or StopAsyncIteration, for `why`, once the watch has ended. An update
/// whose future was cancelled on its way goes back into the receiver's
/// slot, and the next step hands it out first.
fn step<'py, T, S, R>(
    py: Python<'py>,
    stream: &LossyReceiver<T, S>,
    why: impl FnOnce() -> Ended + Send + 'static,
    fresh: impl FnOnce(T) -> R + Send + 'static,
) -> PyResult<Bound<'py, PyAny>>
where
    T: Send + 'static,
    S: EventSource<T> + 'static,
    R: for<'a> IntoPyObject<'a> + Send + 'static,
{
    let (next, put_back) = stream.next();
    future_into_py_or_put_back(
        py,
        async move {
            match next.await {
                Some(next) => Ok(next.map(fresh)),
                None => Err(why().error()),
            }
        },
        put_back,
    )
}

/// A fresh update, made a `WatchUpdate` when it reaches Python.
struct Fresh(Box<Update>, AddressStyle);

impl<'py> IntoPyObject<'py> for Fresh {
    type Target = PyWatchUpdate;
    type Output = Bound<'py, PyWatchUpdate>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> PyResult<Bound<'py, PyWatchUpdate>> {
        Bound::new(py, PyWatchUpdate::new(py, &self.0, self.1)?)
    }
}

/// A watch of a device's points: what `await device.watch(points)`
/// returns. An async iterator of `WatchUpdate`s, each point's latest value
/// and how its watch goes; `async with` closes it on the way out. Frozen;
/// compares by identity and refuses `copy` and `pickle`.
#[pyclass(name = "Watch", module = "rusty_bacnet", frozen)]
pub(crate) struct PyWatch {
    shared: Arc<WatchShared>,
    tie: WatchTie,
    address: String,
    points: Vec<PropertyRef>,
    settings: Settings,
    style: AddressStyle,
}

#[pymethods]
impl PyWatch {
    /// Where the device answered when the watch was made, as text
    /// `BACnetClient.device` reads back: every point is at this address.
    #[getter]
    fn address(&self) -> &str {
        &self.address
    }

    /// The points watched, in the order named, each once, as
    /// `(ObjectIdentifier, PropertyIdentifier, array_index)` tuples:
    /// Present_Value for a point named by its object alone.
    #[getter]
    fn points<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        let points = self.points.iter().map(|point| {
            let object = PyObjectIdentifier::from_rust(point.object);
            let property = PyPropertyIdentifier {
                inner: point.property,
            };
            (object, property, point.index)
        });
        PyTuple::new(py, points)
    }

    /// Each subscription's lifetime, in seconds.
    #[getter]
    fn lifetime(&self) -> u32 {
        self.settings.lifetime.secs()
    }

    /// The least time between two polls of the device, in seconds.
    #[getter]
    fn poll_interval(&self) -> f64 {
        self.settings.poll_every.as_secs_f64()
    }

    /// Changes folded into a later update of the same point because the
    /// watch hadn't handed the earlier one out yet, as of the last update
    /// handed out.
    #[getter]
    fn folded(&self) -> u64 {
        self.shared.folded.load(Ordering::Relaxed)
    }

    /// True once the watch has ended: closed, or its client stopped.
    #[getter]
    fn closed(&self) -> bool {
        self.shared.ended_why().is_some()
    }

    fn __len__(&self) -> usize {
        self.points.len()
    }

    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let ended = Arc::clone(&self.shared.ended);
        let style = self.style;
        let why = move || ended.get().copied().unwrap_or(Ended::Closed);
        step(py, &self.shared.stream, why, move |update| {
            Fresh(update, style)
        })
    }

    /// End the watch: the iterator ends at once, then its subscriptions
    /// are cancelled. The await waits up to 5 s for the cancels; any the
    /// device hasn't answered by then lapse with their lifetime. A
    /// cancelled await leaves the cancels running. Closing again does
    /// nothing.
    fn close<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.tie.close();
        let shared = Arc::clone(&self.shared);
        future_into_py(py, async move {
            shared.end(Ended::Closed, END_WAIT).await;
            Ok(())
        })
    }

    fn __aenter__<'py>(slf: Bound<'py, Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let watch = slf.unbind();
        future_into_py(py, async move { Ok(watch) })
    }

    /// Close the watch, as `close()`.
    #[pyo3(signature = (_exc_type=None, _exc_val=None, _exc_tb=None))]
    fn __aexit__<'py>(
        &self,
        py: Python<'py>,
        _exc_type: Option<Bound<'py, PyAny>>,
        _exc_val: Option<Bound<'py, PyAny>>,
        _exc_tb: Option<Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.close(py)
    }

    fn __repr__(&self) -> String {
        let state = match self.shared.ended_why() {
            Some(_) => "closed",
            None => "open",
        };
        format!(
            "Watch({}, points={}, {state})",
            self.address,
            self.points.len()
        )
    }

    /// Refuses `copy` and `pickle`: a watch on a live client. See
    /// [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}
