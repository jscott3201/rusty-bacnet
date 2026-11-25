//! Bounded raw-frame observation, sharing the event bridge's cancellation delivery.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::task::{Context, Poll};
use std::time::{Instant, SystemTime};

use bacnet_transport::tap::{FrameTapReceiver, TapDirection, TapPeer, TappedFrame};
use bacnet_types::data_link::DataLink;
use bacnet_types::sync::MutexExt;
use pyo3::exceptions::{PyStopAsyncIteration, PyValueError};
use pyo3::intern;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use tokio::sync::{Notify, broadcast::error::RecvError};

use crate::event_stream::{EventSource, LossyReceiver, Lost};

/// Check a `frames(capacity=...)` argument.
pub(crate) fn capacity(capacity: usize) -> PyResult<usize> {
    if !(1..=65_536).contains(&capacity) {
        return Err(PyValueError::new_err(
            "capacity must be between 1 and 65536 frames",
        ));
    }
    Ok(capacity)
}

/// A reading of the tap's capture clock (`Instant`) paired with one of
/// Python's `time.monotonic_ns()`, so a capture instant converts to
/// `time.monotonic()` seconds by its offset from the pair.
///
/// Both read the same OS clock on Linux (`CLOCK_MONOTONIC`), on macOS
/// (`mach_absolute_time`, which `CLOCK_UPTIME_RAW` is) and on Windows from
/// Python 3.13 (`QueryPerformanceCounter`), so the pair stays in step. On
/// Windows, Python 3.11 and 3.12 read `GetTickCount64` instead (see
/// `Frame.monotonic`'s docs).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Clock {
    instant: Instant,
    python_ns: i64,
}

/// The process's pair, measured once, by the first `frames()` call.
static CLOCK: OnceLock<Clock> = OnceLock::new();

impl Clock {
    /// The process's pair, measuring it on the first call. Called from a
    /// `frames()` method, on the Python thread that called it.
    pub(crate) fn get(py: Python<'_>) -> PyResult<Self> {
        if let Some(clock) = CLOCK.get() {
            return Ok(*clock);
        }
        let monotonic_ns = py
            .import(intern!(py, "time"))?
            .getattr(intern!(py, "monotonic_ns"))?;
        let before = Instant::now();
        let python_ns: i64 = monotonic_ns.call0()?.extract()?;
        let after = Instant::now();
        // Python read its clock somewhere between the two instants.
        let measured = Self {
            instant: before + (after - before) / 2,
            python_ns,
        };
        Ok(*CLOCK.get_or_init(|| measured))
    }

    /// A pair for tests that don't run Python: the instant now as zero.
    #[cfg(test)]
    pub(crate) fn fixed() -> Self {
        Self {
            instant: Instant::now(),
            python_ns: 0,
        }
    }

    /// `at` in `time.monotonic()` seconds. The offset is exact in
    /// nanoseconds and the conversion to a float never decreases, so later
    /// instants never read earlier.
    pub(crate) fn seconds(self, at: Instant) -> f64 {
        let nanos = |duration: std::time::Duration| {
            i128::try_from(duration.as_nanos()).unwrap_or(i128::MAX)
        };
        let offset = match at.checked_duration_since(self.instant) {
            Some(after) => nanos(after),
            None => -nanos(self.instant.duration_since(at)),
        };
        i128::from(self.python_ns).saturating_add(offset) as f64 / 1e9
    }
}

/// `at` as POSIX seconds, negative before 1970.
pub(crate) fn posix_seconds(at: SystemTime) -> f64 {
    match at.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(since) => since.as_secs_f64(),
        Err(before) => -before.duration().as_secs_f64(),
    }
}

/// One transport observation. Its times belong to capture, not iteration.
#[pyclass(name = "Frame", module = "rusty_bacnet", frozen)]
pub(crate) struct PyFrame {
    frame: TappedFrame,
    local_address: Arc<str>,
    missed_before: u64,
    monotonic: f64,
}

#[pymethods]
impl PyFrame {
    /// Capture time in seconds on `time.monotonic()`'s clock and origin; it
    /// never decreases within a subscription. On Windows with Python 3.11
    /// or 3.12 it can sit up to one 10-16 ms step of that clock away.
    #[getter]
    fn monotonic(&self) -> f64 {
        self.monotonic
    }

    /// Capture time as POSIX seconds, from the wall clock, as `time.time()`.
    #[getter]
    fn timestamp(&self) -> f64 {
        posix_seconds(self.frame.timestamp)
    }

    /// `"inbound"` or `"outbound"` (`"other"` for a future direction).
    #[getter]
    fn direction(&self) -> &'static str {
        match self.frame.direction {
            TapDirection::Inbound => "inbound",
            TapDirection::Outbound => "outbound",
            _ => "other",
        }
    }

    /// The link's `transport=` token (see [`link_token`]).
    #[getter]
    fn link(&self) -> &'static str {
        link_token(self.frame.link)
    }

    /// This node's address on the link, as the subscription began, in the
    /// form [`peer`](Self::peer) uses (see [`local_address`]).
    #[getter]
    fn local_address(&self) -> &str {
        &self.local_address
    }

    /// The other end on the link (see [`peer_text`]): the next hop, not a
    /// forwarded frame's original source.
    #[getter]
    fn peer(&self) -> String {
        peer_text(self.frame.peer)
    }

    /// Raw link-layer octets, including malformed received datagrams.
    #[getter]
    fn octets<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.frame.bytes)
    }

    /// Short: never the octets, which may be long.
    fn __repr__(&self) -> String {
        format!(
            "Frame(direction='{}', link='{}', peer='{}', length={})",
            self.direction(),
            self.link(),
            self.peer(),
            self.frame.bytes.len()
        )
    }

    /// Frames this subscription missed just before this one: dropped by its
    /// full queue, or lost on the way to Python with the gap such a frame
    /// carried. A cancelled read's redelivery keeps the value.
    #[getter]
    fn missed_before(&self) -> u64 {
        self.missed_before
    }
}

/// A MAC in lowercase colon hex, as SC addresses are written.
fn hex_address(mac: &[u8]) -> String {
    bacnet_ops::octets::hex_sep(mac, ":")
}

/// `link` as the `transport=` argument of `BACnetClient`, `BACnetServer`
/// and the endpoints spells it: `"bip"`, `"ipv6"`, `"sc"` or `"mstp"`. A
/// link no Python owner runs (Ethernet, the in-process loopback, or one
/// added later) is `"other"`.
pub(crate) fn link_token(link: DataLink) -> &'static str {
    match link {
        DataLink::Bip => "bip",
        DataLink::Bip6 => "ipv6",
        DataLink::Sc => "sc",
        DataLink::Mstp => "mstp",
        _ => "other",
    }
}

/// A frame's peer, written as the Python API writes that link's addresses:
/// the UDP socket address on B/IP (`"ip:port"`) and on B/IPv6
/// (`"[ip]:port"`, the form `resolve_vmac` takes, not a VMAC); on SC the
/// hub connection the message went over, `"primary"` or `"failover"` as
/// `ScEvent.hub` names it (the VMACs are in the BVLC-SC header); the MS/TP
/// station in decimal; an Ethernet MAC in colon hex.
pub(crate) fn peer_text(peer: TapPeer) -> String {
    match peer {
        TapPeer::Udp(address) => address.to_string(),
        TapPeer::ScHub(hub) => crate::sc_events::hub_name(hub).into(),
        TapPeer::Mstp(station) => station.to_string(),
        TapPeer::Ethernet(mac) => hex_address(&mac),
        _ => "other".into(),
    }
}

/// This node's own address on `link`, in the form [`peer_text`] writes the
/// link's peers: the bound UDP socket address on B/IP and B/IPv6
/// (`socket`), the MS/TP station in decimal, and on SC (its VMAC) or any
/// other link the MAC in colon hex. `None` when the transport has none.
pub(crate) fn local_address(
    link: DataLink,
    mac: &[u8],
    socket: Option<SocketAddr>,
) -> Option<Arc<str>> {
    let text = match link {
        DataLink::Bip | DataLink::Bip6 => socket?.to_string(),
        DataLink::Mstp => match mac {
            [station] => station.to_string(),
            _ => return None,
        },
        _ if mac.is_empty() => return None,
        _ => hex_address(mac),
    };
    Some(text.into())
}

/// `transport`'s own UDP socket address on B/IP and B/IPv6, which its
/// peers see as the source of what it sends; `None` on other links.
pub(crate) fn link_socket(transport: &bacnet_transport::any::AnyTransport) -> Option<SocketAddr> {
    use bacnet_transport::port::TransportPort;
    match transport.data_link()? {
        DataLink::Bip => bacnet_transport::bvll::decode_bip_mac(transport.local_mac())
            .ok()
            .map(|(ip, port)| SocketAddr::from((ip, port))),
        DataLink::Bip6 => transport
            .bip6_transport()
            .and_then(|bip6| bip6.local_address())
            .map(SocketAddr::V6),
        _ => None,
    }
}

/// What a subscription reads: the tap's receiver, or a stream built over
/// one (`i_am_arrivals.rs`). Its items carry the gap in front of them.
pub(crate) trait TapStream: Send + 'static {
    /// What each read yields.
    type Item: Send + 'static;
    /// Poll for the next item; cancel safe.
    fn poll_next(&mut self, cx: &mut Context<'_>) -> Poll<Option<Self::Item>>;
    /// Frames lost to the full queue so far.
    fn dropped(&self) -> u64;
    /// Detach and synchronize with an in-flight publication before counting the last drop.
    fn close(self) -> u64;
}

impl TapStream for FrameTapReceiver {
    type Item = TappedFrame;

    fn poll_next(&mut self, cx: &mut Context<'_>) -> Poll<Option<TappedFrame>> {
        self.poll_recv(cx)
    }

    fn dropped(&self) -> u64 {
        FrameTapReceiver::dropped(self)
    }

    fn close(self) -> u64 {
        FrameTapReceiver::close(self)
    }
}

struct SourceState<R> {
    receiver: Mutex<Option<R>>,
    closed: Notify,
    /// Preserve the final count after the receiver is released.
    final_missed: std::sync::atomic::AtomicU64,
    /// What the last item converted reported, for the next one.
    carry: Mutex<Carry>,
}

/// The gap the last converted item reported, and the stream's lost count
/// when it was converted. A leaf lock: held only for the arithmetic.
#[derive(Default)]
struct Carry {
    reported: u64,
    lost: u64,
}

impl<R: TapStream> SourceState<R> {
    fn receiver(&self) -> MutexGuard<'_, Option<R>> {
        self.receiver.locked()
    }

    /// The `missed_before` of a fresh item whose queue gap is `queue_gap`,
    /// when the stream has lost `lost` deliveries so far.
    ///
    /// One delivery is in flight at a time, and an item is converted only
    /// once the delivery before it settled, so a loss counted since the
    /// last conversion lost that last item: the item and the gap it
    /// reported both fall in front of this one. The sum of `missed_before`
    /// over the items Python received, plus whatever was lost after the
    /// last of them, is then the iterator's `missed`.
    fn missed_before(&self, queue_gap: u64, lost: u64) -> u64 {
        let mut carry = self.carry.locked();
        let lost_since = lost.saturating_sub(carry.lost);
        let carried = if lost_since == 0 {
            0
        } else {
            lost_since.saturating_add(carry.reported)
        };
        let reported = queue_gap.saturating_add(carried);
        *carry = Carry { reported, lost };
        reported
    }

    fn close(&self) {
        if let Some(receiver) = self.receiver().take() {
            self.final_missed
                .store(receiver.close(), std::sync::atomic::Ordering::Relaxed);
        }
        self.closed.notify_waiters();
    }

    fn missed(&self) -> u64 {
        self.receiver().as_ref().map_or_else(
            || self.final_missed.load(std::sync::atomic::Ordering::Relaxed),
            R::dropped,
        )
    }
}

struct TapSource<R>(Arc<SourceState<R>>);

impl<R> Clone for TapSource<R> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<R: TapStream> EventSource<R::Item> for TapSource<R> {
    async fn recv(&mut self) -> Result<R::Item, RecvError> {
        let closed = self.0.closed.notified();
        tokio::pin!(closed);
        closed.as_mut().enable();
        tokio::select! {
            biased;
            () = &mut closed => Err(RecvError::Closed),
            item = std::future::poll_fn(|cx| {
                match self.0.receiver().as_mut() {
                    Some(receiver) => receiver.poll_next(cx),
                    None => Poll::Ready(None),
                }
            }) => item.ok_or(RecvError::Closed),
        }
    }
}

/// One iterator's subscription: its stream of deliveries over the source.
pub(crate) struct Subscription<R: TapStream> {
    stream: LossyReceiver<R::Item, TapSource<R>>,
    source: TapSource<R>,
}

impl<R: TapStream> Subscription<R> {
    fn close(&self) {
        self.stream.close();
        self.source.0.close();
    }

    /// Items lost so far, live: dropped by the full queue or lost on the
    /// way to Python, including any lost after the last item read. It
    /// stays readable after close.
    pub(crate) fn missed(&self) -> u64 {
        self.source.0.missed() + self.stream.missed()
    }

    /// The next delivery, as [`Self::anext`] takes it: for tests.
    #[cfg(test)]
    pub(crate) fn stream_next(
        &self,
    ) -> (
        impl std::future::Future<Output = Option<crate::event_stream::Next<R::Item>>>
        + Send
        + 'static
        + use<R>,
        Arc<dyn crate::py_async::PutBack>,
    ) {
        self.stream.next()
    }

    /// What a read's future needs to report a fresh item's gap.
    pub(crate) fn gaps(&self) -> Gaps<R> {
        Gaps {
            source: self.source.clone(),
            lost: self.stream.lost(),
        }
    }

    /// `__anext__`: the next item, converted by `convert` with the gap in
    /// front of it, or `StopAsyncIteration` naming `what` once it ends.
    pub(crate) fn anext<'py, T>(
        &self,
        py: Python<'py>,
        what: &'static str,
        convert: impl FnOnce(R::Item, &Gaps<R>) -> T + Send + 'static,
    ) -> PyResult<Bound<'py, PyAny>>
    where
        T: for<'a> IntoPyObject<'a> + Send + 'static,
    {
        let (next, put_back) = self.stream.next();
        let gaps = self.gaps();
        crate::py_async::future_into_py_or_put_back(
            py,
            async move {
                next.await
                    .map(|next| next.map(|item| convert(item, &gaps)))
                    .ok_or_else(|| PyStopAsyncIteration::new_err(format!("{what} ended")))
            },
            put_back,
        )
    }

    /// `aclose`: stop observing, discard queued items and wake pending
    /// reads. The owner and its requests are left alone.
    pub(crate) fn aclose<'py>(self: &Arc<Self>, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let subscription = Arc::clone(self);
        crate::py_async::future_into_py(py, async move {
            subscription.close();
            Ok(())
        })
    }
}

impl<R: TapStream> Drop for Subscription<R> {
    fn drop(&mut self) {
        self.close();
    }
}

/// A read's view of the gaps: the source and the stream's lost count, never
/// the subscription, so a pending read doesn't keep a dropped iterator open.
pub(crate) struct Gaps<R> {
    source: TapSource<R>,
    lost: Lost,
}

impl<R: TapStream> Gaps<R> {
    /// The `missed_before` of a fresh item whose queue gap is `queue_gap`.
    pub(crate) fn missed_before(&self, queue_gap: u64) -> u64 {
        self.source.0.missed_before(queue_gap, self.lost.get())
    }
}

/// A subscription as its owner holds it, to end it.
trait Close: Send + Sync {
    fn close(&self);
}

impl<R: TapStream> Close for Subscription<R> {
    fn close(&self) {
        Subscription::close(self);
    }
}

/// An owner's weak subscriptions, to frames and to streams over them:
/// observers never keep the owner alive.
#[derive(Default)]
pub(crate) struct FrameSubscriptions(Mutex<Vec<Weak<dyn Close>>>);

impl FrameSubscriptions {
    pub(crate) fn subscribe(
        &self,
        receiver: FrameTapReceiver,
        local_address: Arc<str>,
        clock: Clock,
    ) -> PyFrameIterator {
        PyFrameIterator {
            subscription: self.open(receiver),
            local_address,
            clock,
        }
    }

    /// A subscription to `stream`, which the owner's close ends.
    pub(crate) fn open<R: TapStream>(&self, stream: R) -> Arc<Subscription<R>> {
        let source = TapSource(Arc::new(SourceState {
            receiver: Mutex::new(Some(stream)),
            closed: Notify::new(),
            final_missed: Default::default(),
            carry: Mutex::default(),
        }));
        let subscription = Arc::new(Subscription {
            stream: LossyReceiver::from_source(source.clone()),
            source,
        });
        let weak: Weak<dyn Close> = Arc::downgrade(&subscription) as Weak<dyn Close>;
        let mut subscriptions = self.0.locked();
        subscriptions.retain(|old| old.strong_count() != 0);
        subscriptions.push(weak);
        subscription
    }

    /// End the current generation; a later owner restart may subscribe anew.
    pub(crate) fn close(&self) {
        let subscriptions = std::mem::take(&mut *self.0.locked());
        for subscription in subscriptions.into_iter().filter_map(|weak| weak.upgrade()) {
            subscription.close();
        }
    }
}

impl Drop for FrameSubscriptions {
    fn drop(&mut self) {
        self.close();
    }
}

/// A bounded subscription to the observing owner's existing transport.
#[pyclass(name = "FrameIterator", module = "rusty_bacnet", frozen)]
pub(crate) struct PyFrameIterator {
    subscription: Arc<Subscription<FrameTapReceiver>>,
    local_address: Arc<str>,
    clock: Clock,
}

impl PyFrameIterator {
    /// What a read's future needs to turn a tapped frame into a Python
    /// `Frame`. It keeps nothing of the iterator.
    fn converter(
        &self,
    ) -> impl FnOnce(TappedFrame, &Gaps<FrameTapReceiver>) -> PyFrame + Send + 'static + use<> {
        let local_address = self.local_address.clone();
        let clock = self.clock;
        move |frame, gaps| PyFrame {
            missed_before: gaps.missed_before(frame.missed_before),
            monotonic: clock.seconds(frame.monotonic_timestamp),
            frame,
            local_address,
        }
    }
}

#[pymethods]
impl PyFrameIterator {
    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.subscription
            .anext(py, "frame observation", self.converter())
    }

    /// Frames lost so far, live: dropped by the full queue (which keeps
    /// the oldest frames) or lost on the way to Python, including any lost
    /// after the last frame read. The count remains readable after close.
    #[getter]
    fn missed(&self, py: Python<'_>) -> u64 {
        // The GIL is released: a read in progress holds the receiver.
        py.detach(|| self.subscription.missed())
    }

    /// Stop observation, discard queued frames and wake pending reads.
    /// An already admitted native delivery may still complete its Python Future.
    /// This does not stop the owner or cancel its protocol requests.
    fn aclose<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.subscription.aclose(py)
    }

    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyFrame>()?;
    m.add_class::<PyFrameIterator>()?;
    Ok(())
}

#[cfg(test)]
#[path = "frames_tests.rs"]
mod tests;
