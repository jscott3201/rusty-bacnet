//! [`PyWatch`]: what `await device.watch(points)` returns (#2561, #2603),
//! a device's points watched as an async iterator of `WatchUpdate`s, over
//! the high-level client's watch hubs ([`bacnet_ops::Client::watch_with`]).
//!
//! A watch takes its settings (`lifetime`, `poll_interval`, `subscribe`)
//! from its client's `watch_*` keywords unless `Device.watch` names
//! others, and watches with equal settings share one hub, so a point two
//! of them hold is one subscription. The hubs hold the high-level client
//! while a watch on them lives, so the watch never takes a call's lease
//! between steps: its client keeps it ([`crate::client::WatchTie`]), and
//! a stop ends every watch and shuts every hub before it takes the
//! low-level client. A watch ends on `close()` (or leaving `async with`),
//! the Python object going away, or its client's stop, whichever comes
//! first. Its iterator ends at once, for good, raising
//! `StopAsyncIteration`; then its points go, the cancels of those no other
//! watch holds waited for up to [`END_WAIT`]. A restarted client doesn't
//! revive it: watch again.
//!
//! `add` and `remove` change the points while another task iterates. Once
//! `remove` has run, no `__anext__` begun after hands out a removed point:
//! the hub's mailbox drops it, an update put back by a cancelled
//! `__anext__` is dropped from the receiver's slot, and one on its way back
//! then is dropped as it comes ([`Pruned`]), each told by its mark
//! ([`WatchControl::holds`]).
//!
//! Updates go through the event iterators' receiver ([`LossyReceiver`])
//! from the hub's mailbox, which keeps the latest update of each point for
//! a consumer that falls behind ([`PyWatch::folded`] counts what it folded
//! away). The read is cancel-safe: the mailbox gives an update up only on
//! the poll that returns it, and an update whose `__anext__` was cancelled
//! on its way to Python goes back into the receiver's slot and comes out
//! first, so none is lost or handed out twice.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use bacnet_ops::target::AddressStyle;
use bacnet_ops::watch::hub::{Lifetime, MAX_POLL, POLL_FLOOR};
use bacnet_ops::watch::{
    Binding, MAX_WATCHED, Point, Update, Watch, WatchControl, WatchError, WatchSettings, Watched,
};
use bacnet_ops::{OpsError, PropertyRef};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Lifecycle;
use bacnet_types::sync::MutexExt;
use pyo3::exceptions::{PyRuntimeError, PyStopAsyncIteration, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyFloat, PyInt, PyTuple};
use tokio::sync::broadcast::error::RecvError;

use super::device::Core;
use super::errors::ops_err;
use super::specs::{property_item, refuse_text};
use super::watch_update::PyWatchUpdate;
use crate::client::{END_WAIT, WatchTie};
use crate::errors::lifecycle_error;
use crate::event_stream::{EventSource, LossyReceiver};
use crate::py_async::{PutBack, Via, future_into_py, future_into_py_or_put_back};
use crate::types::{PyObjectIdentifier, PyPropertyIdentifier};

#[cfg(test)]
#[path = "watch_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "watch_prune_tests.rs"]
mod prune_tests;

/// Why a watch's iterator ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ended {
    /// `close()`, `async with`, or the Python object went away.
    Closed,
    /// Its client stopped.
    Stopped,
}

impl Ended {
    fn text(self) -> &'static str {
        match self {
            Self::Closed => "the watch ended: it was closed",
            Self::Stopped => "the watch ended: the client stopped",
        }
    }

    fn error(self) -> PyErr {
        PyStopAsyncIteration::new_err(self.text())
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
        let next = self.watch.next().await;
        self.folded.store(self.watch.folded(), Ordering::Relaxed);
        next.map(Box::new).ok_or(RecvError::Closed)
    }
}

/// What a watch shares with its client: its control, and its stream of
/// updates.
pub(crate) struct WatchShared {
    control: WatchControl,
    stream: LossyReceiver<Box<Update>, Updates>,
    /// Why the iterator ended, once it has; a step's future keeps this
    /// alone.
    ended: Arc<OnceLock<Ended>>,
    folded: Arc<AtomicU64>,
    /// Held across an end, so a second end waits for the first.
    ending: tokio::sync::Mutex<()>,
}

impl WatchShared {
    /// `watch`'s stream and control.
    pub(crate) fn new(watch: Watch) -> Self {
        let folded = Arc::new(AtomicU64::new(0));
        let control = watch.control();
        let updates = Updates {
            watch,
            folded: Arc::clone(&folded),
        };
        Self {
            control,
            stream: LossyReceiver::from_source(updates),
            ended: Arc::new(OnceLock::new()),
            folded,
            ending: tokio::sync::Mutex::new(()),
        }
    }

    /// End the iterator, for `why` unless it has ended already: a pending
    /// or later `__anext__` raises `StopAsyncIteration`. Sends nothing.
    pub(crate) fn close_stream(&self, why: Ended) {
        let _ = self.ended.set(why);
        self.stream.close();
    }

    /// End the watch: its iterator, then its points, waiting up to `wait`
    /// for the cancels of those no other watch holds (those not answered
    /// by then lapse with their lifetime). A later end waits for an earlier
    /// one.
    pub(crate) async fn end(&self, why: Ended, wait: Duration) {
        self.close_stream(why);
        let _ending = self.ending.lock().await;
        self.control.close(wait).await;
    }

    /// Why the iterator ended, once it has.
    pub(crate) fn ended_why(&self) -> Option<Ended> {
        self.ended.get().copied()
    }

    /// Stop watching `points` (as the hub keys them) in this watch: the
    /// count it held. The hub's mailbox drops a change of theirs waiting,
    /// then an update of theirs a cancelled `__anext__` put back is dropped
    /// from the receiver's slot; one put back later is dropped as it comes
    /// ([`Pruned`]). With the GIL held, so no Python code runs between.
    fn remove_points(&self, py: Python<'_>, points: &[Point]) -> Result<usize, WatchError> {
        let count = self.control.remove(points.iter())?;
        self.stream
            .purge(|object| Pruned::stale(py, &self.control, object));
        Ok(count)
    }

    /// Whether the watch still holds points.
    #[cfg(test)]
    pub(crate) fn watching(&self) -> bool {
        !self.control.points().is_empty()
    }
}

/// What `Device.watch`'s keywords set, checked when the call is made: each
/// `None` takes the client's.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(super) struct Overrides {
    lifetime: Option<Lifetime>,
    poll_every: Option<Duration>,
    subscribe: Option<bool>,
}

impl Overrides {
    /// `lifetime`: an int of seconds, 1 to 28800; `poll_interval`: an int
    /// or float of seconds, 1 to 3600; `subscribe`: a bool. None, or a
    /// keyword not given, takes the client's.
    pub(super) fn new(
        lifetime: Option<&Bound<'_, PyAny>>,
        poll_interval: Option<&Bound<'_, PyAny>>,
        subscribe: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        Ok(Self {
            lifetime: given(lifetime)
                .map(|value| lifetime_arg("lifetime", value))
                .transpose()?,
            poll_every: given(poll_interval)
                .map(|value| poll_arg("poll_interval", value))
                .transpose()?,
            subscribe: given(subscribe)
                .map(|value| subscribe_arg("subscribe", value))
                .transpose()?,
        })
    }

    /// `defaults`, with what these set in their place.
    pub(super) fn over(self, defaults: &WatchSettings) -> WatchSettings {
        let mut settings = defaults.clone();
        if let Some(lifetime) = self.lifetime {
            settings = settings.with_lifetime(lifetime);
        }
        if let Some(every) = self.poll_every {
            settings = settings.with_poll_every(every);
        }
        if let Some(subscribe) = self.subscribe {
            settings = settings.with_subscribe(subscribe);
        }
        settings
    }
}

/// `BACnetClient`'s `watch_lifetime` keyword (300 when not given): an int
/// of seconds, 1 to 28800.
pub(crate) fn watch_lifetime_arg(value: &Bound<'_, PyAny>) -> PyResult<u32> {
    lifetime_arg("watch_lifetime", value).map(Lifetime::secs)
}

/// `BACnetClient`'s `watch_poll_interval` keyword (10.0 when not given): an
/// int or float of seconds, 1 to 3600.
pub(crate) fn watch_poll_arg(value: &Bound<'_, PyAny>) -> PyResult<f64> {
    poll_arg("watch_poll_interval", value).map(|every| every.as_secs_f64())
}

/// `BACnetClient`'s `watch_subscribe` keyword (True when not given): a
/// bool.
pub(crate) fn watch_subscribe_arg(value: &Bound<'_, PyAny>) -> PyResult<bool> {
    subscribe_arg("watch_subscribe", value)
}

/// A client's watch settings from its `watch_*` keywords, each checked as
/// it was taken.
pub(crate) fn client_settings(
    lifetime: u32,
    poll_interval: f64,
    subscribe: bool,
) -> PyResult<WatchSettings> {
    let lifetime =
        Lifetime::new(lifetime).map_err(|error| PyValueError::new_err(error.to_string()))?;
    Ok(WatchSettings::default()
        .with_lifetime(lifetime)
        .with_poll_every(Duration::from_secs_f64(poll_interval))
        .with_subscribe(subscribe))
}

/// `value`, unless it is None or not given.
fn given<'a, 'py>(value: Option<&'a Bound<'py, PyAny>>) -> Option<&'a Bound<'py, PyAny>> {
    value.filter(|value| !value.is_none())
}

/// A lifetime argument `name`: an int of seconds, 1 to 28800.
fn lifetime_arg(name: &str, lifetime: &Bound<'_, PyAny>) -> PyResult<Lifetime> {
    if !lifetime.is_instance_of::<PyInt>() || lifetime.is_instance_of::<PyBool>() {
        return Err(PyTypeError::new_err(format!(
            "{name} is an int of seconds, not {}",
            lifetime.get_type().name()?
        )));
    }
    lifetime
        .extract::<u32>()
        .ok()
        .and_then(|secs| Lifetime::new(secs).ok())
        .ok_or_else(|| {
            PyValueError::new_err(format!(
                "{name} is 1 to {} seconds, not {lifetime}: a watch's subscriptions always lapse, \
                 and no device need take one longer than 8 hours",
                Lifetime::MAX.secs()
            ))
        })
}

/// A poll interval argument `name`: an int or float of seconds, 1 to 3600.
fn poll_arg(name: &str, interval: &Bound<'_, PyAny>) -> PyResult<Duration> {
    let number = interval.is_instance_of::<PyInt>() || interval.is_instance_of::<PyFloat>();
    if !number || interval.is_instance_of::<PyBool>() {
        return Err(PyTypeError::new_err(format!(
            "{name} is a number of seconds, not {}",
            interval.get_type().name()?
        )));
    }
    let secs: f64 = interval.extract()?;
    let (floor, ceiling) = (POLL_FLOOR.as_secs_f64(), MAX_POLL.as_secs_f64());
    if !(floor..=ceiling).contains(&secs) {
        return Err(PyValueError::new_err(format!(
            "{name} is {floor} to {ceiling} seconds, not {secs}"
        )));
    }
    Ok(Duration::from_secs_f64(secs))
}

/// A subscribe argument `name`: a bool.
fn subscribe_arg(name: &str, subscribe: &Bound<'_, PyAny>) -> PyResult<bool> {
    if !subscribe.is_instance_of::<PyBool>() {
        return Err(PyTypeError::new_err(format!(
            "{name} is a bool, not {}",
            subscribe.get_type().name()?
        )));
    }
    subscribe.extract()
}

/// A `points` argument: an iterable of what `read_many` takes, each point
/// once, in order. Not one str; at least one point, at most
/// [`MAX_WATCHED`]; no property that stands for many.
pub(super) fn points_arg(points: &Bound<'_, PyAny>) -> PyResult<Vec<PropertyRef>> {
    refuse_text(points, "points", "['ai:1', 'ao:2 priority-array[8]']")?;
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

/// `wanted` as the point the hub watches of the device `target` names (its
/// address, or its instance followed): Present_Value by its object, as
/// `Device::point` names it.
fn point_of(target: &Binding, wanted: &PropertyRef) -> Point {
    let watched = match (wanted.property, wanted.index) {
        (PropertyIdentifier::PRESENT_VALUE, None) => Watched::Object,
        (property, index) => Watched::Property { property, index },
    };
    Point {
        device: target.clone(),
        object: wanted.object,
        watched,
    }
}

/// Watch `wanted` on the device `core` names, under the client's settings
/// with `overrides` in their place: the handle, held with the client's
/// lease while the watch is made and the client keeps it.
pub(super) async fn open(
    core: Arc<Core>,
    wanted: Vec<PropertyRef>,
    overrides: Overrides,
) -> PyResult<PyWatch> {
    let keeper = Arc::clone(&core);
    core.run(move |device, style| async move {
        let points = wanted
            .iter()
            .map(|point| device.point(point))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| ops_err(error, style))?;
        let settings = overrides.over(device.client().watch_settings());
        // Inside the runtime, as the hub's tasks need; nothing awaited
        // from here until the client keeps the watch.
        let watch = device
            .client()
            .watch_with(points, &settings)
            .map_err(|error| watch_err(error, style))?;
        let shared = Arc::new(WatchShared::new(watch));
        let tie = keeper.tie().keep_watch(Arc::clone(&shared));
        Ok(PyWatch {
            shared,
            tie,
            address: device.address().styled(style),
            target: device.binding(),
            points: Arc::new(Mutex::new(wanted)),
            settings,
            style,
        })
    })
    .await
}

/// Why a watch wasn't made, as Python raises it: RuntimeError when every
/// range of subscriber process identifiers on the client is held, one for
/// each set of watch settings in use (or still cancelling); else what any
/// call raises.
fn watch_err(error: OpsError, style: AddressStyle) -> PyErr {
    match error {
        OpsError::Watch(WatchError::Hubs { max }) => PyRuntimeError::new_err(format!(
            "{max} watches with different settings live on one client, each on a hub with its \
             own range of subscriber process identifiers: close one, or watch with the settings \
             of one already live"
        )),
        other => ops_err(other, style),
    }
}

/// The put-back hook of a watch's `__anext__`: an update whose point was
/// removed since it was taken is dropped, not put back, so no later
/// `__anext__` hands it out.
struct Pruned {
    inner: Arc<dyn PutBack>,
    control: WatchControl,
}

impl Pruned {
    /// Whether `object`, an update the watch handed out, is of a point it
    /// no longer holds where it did.
    fn stale(py: Python<'_>, control: &WatchControl, object: &Py<PyAny>) -> bool {
        object
            .bind(py)
            .cast::<PyWatchUpdate>()
            .is_ok_and(|update| !control.holds(update.get().mark()))
    }
}

impl PutBack for Pruned {
    fn put_back(&self, py: Python<'_>, value: Py<PyAny>, via: Via) {
        if !Self::stale(py, &self.control, &value) {
            self.inner.put_back(py, value, via);
        }
    }

    fn settled(&self) {
        self.inner.settled();
    }

    #[cfg(test)]
    fn queued(&self) {
        self.inner.queued();
    }
}

/// One `__anext__` over `stream`, as an asyncio future: the next update,
/// or StopAsyncIteration, for `why`, once the watch has ended. An update
/// whose future was cancelled on its way goes back into the receiver's
/// slot through the hook `wrap` makes of the slot's, and the next step
/// hands it out first.
fn step<'py, T, S, R>(
    py: Python<'py>,
    stream: &LossyReceiver<T, S>,
    why: impl FnOnce() -> Ended + Send + 'static,
    fresh: impl FnOnce(T) -> R + Send + 'static,
    wrap: impl FnOnce(Arc<dyn PutBack>) -> Arc<dyn PutBack>,
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
        wrap(put_back),
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
/// and how its watch goes; `async with` closes it on the way out; `add`
/// and `remove` change its points. Frozen; compares by identity and
/// refuses `copy` and `pickle`.
#[pyclass(name = "Watch", module = "rusty_bacnet", frozen)]
pub(crate) struct PyWatch {
    shared: Arc<WatchShared>,
    tie: WatchTie,
    address: String,
    /// What the points are bound to: the handle's address, or the instance
    /// it follows (#2795).
    target: Binding,
    /// The points watched, in the order named, added ones last. Held
    /// across each change of the hub's, so the two agree.
    points: Arc<Mutex<Vec<PropertyRef>>>,
    settings: WatchSettings,
    style: AddressStyle,
}

impl PyWatch {
    /// Why a change was refused: the watch has ended (`BacnetStateError`,
    /// state closed, still a RuntimeError; #2819), or what any call raises.
    fn changed_err(ended: Option<Ended>, error: WatchError, style: AddressStyle) -> PyErr {
        match error {
            WatchError::Closed | WatchError::Shut => lifecycle_error(
                Lifecycle::Closed,
                ended.map_or("the watch ended", Ended::text),
            ),
            other => ops_err(OpsError::Watch(other), style),
        }
    }
}

#[pymethods]
impl PyWatch {
    /// Where the device answered when the watch was made, as text
    /// `BACnetClient.device` reads back. A watch of a handle made from an
    /// address keeps every point there; one of a handle made from an
    /// instance follows the device, and each update's `address` says where
    /// it answers now.
    #[getter]
    fn address(&self) -> &str {
        &self.address
    }

    /// True when the watch follows its device's instance from address to
    /// address (a handle made from an instance), False when it watches one
    /// address.
    #[getter]
    fn follows(&self) -> bool {
        matches!(self.target, Binding::Follows { .. })
    }

    /// The points watched, in the order named, added ones last, each once,
    /// as `(ObjectIdentifier, PropertyIdentifier, array_index)` tuples:
    /// Present_Value for a point named by its object alone.
    #[getter]
    fn points<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        let points: Vec<_> = self
            .points
            .locked()
            .iter()
            .map(|point| {
                let object = PyObjectIdentifier::from_rust(point.object);
                let property = PyPropertyIdentifier {
                    inner: point.property,
                };
                (object, property, point.index)
            })
            .collect();
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

    /// Whether points are subscribed for COV where the device takes it;
    /// False polls every point.
    #[getter]
    fn subscribe(&self) -> bool {
        self.settings.subscribe
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
        self.points.locked().len()
    }

    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let ended = Arc::clone(&self.shared.ended);
        let style = self.style;
        let control = self.shared.control.clone();
        let why = move || ended.get().copied().unwrap_or(Ended::Closed);
        let prune = move |inner| -> Arc<dyn PutBack> { Arc::new(Pruned { inner, control }) };
        step(
            py,
            &self.shared.stream,
            why,
            move |update| Fresh(update, style),
            prune,
        )
    }

    /// Watch `points` too, of the watch's device (at its address, or
    /// following it, as the watch does), each in what `read_many` takes;
    /// those watched already are left as they are.
    /// Done before this returns, as `remove` is, so an `add` and a
    /// `remove` take effect in the order called, awaited or not.
    /// Awaitable: the count newly watched. Each new point is read, then
    /// subscribed (or polled) under the watch's settings; one another
    /// watch with the same settings holds joins its subscription.
    /// Raised when the call is made: TypeError or ValueError for a bad
    /// argument, RuntimeError once the watch has ended or when the points
    /// would take the watch's hub past 4096 (nothing is added then).
    fn add<'py>(&self, py: Python<'py>, points: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let wanted = points_arg(points)?;
        let added: Vec<Point> = wanted.iter().map(|p| point_of(&self.target, p)).collect();
        let count = {
            // Held across the hub's change and the list's, so the two agree.
            let mut list = self.points.locked();
            let control = &self.shared.control;
            // In the runtime: a new device's scheduler is a task.
            let count = crate::py_async::in_runtime(|| control.add(added))
                .map_err(|error| Self::changed_err(self.shared.ended_why(), error, self.style))?;
            for point in wanted {
                if !list.contains(&point) {
                    list.push(point);
                }
            }
            count
        };
        future_into_py(py, async move { Ok(count) })
    }

    /// Stop watching `points` in this watch, each in what `read_many`
    /// takes. Done before this returns: no `__anext__` begun after hands
    /// out one of them, an update already waiting included; one already
    /// waiting may still get an update it took before. Awaitable: the
    /// count removed. A point no other watch holds is cancelled on the
    /// wire afterwards, best effort, without delaying this. Raised:
    /// TypeError or ValueError for a bad argument, RuntimeError once the
    /// watch has ended.
    fn remove<'py>(
        &self,
        py: Python<'py>,
        points: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let wanted = points_arg(points)?;
        let removed: Vec<Point> = wanted.iter().map(|p| point_of(&self.target, p)).collect();
        let count = {
            let mut list = self.points.locked();
            let count = self
                .shared
                .remove_points(py, &removed)
                .map_err(|error| Self::changed_err(self.shared.ended_why(), error, self.style))?;
            list.retain(|point| !wanted.contains(point));
            count
        };
        future_into_py(py, async move { Ok(count) })
    }

    /// End the watch: the iterator ends at once, then its points go, the
    /// subscriptions no other watch holds cancelled. The await waits up to
    /// 5 s for the cancels; any the device hasn't answered by then lapse
    /// with their lifetime. A cancelled await leaves the cancels running.
    /// Closing again does nothing.
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
            self.points.locked().len()
        )
    }

    /// Refuses `copy` and `pickle`: a watch on a live client. See
    /// [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}
