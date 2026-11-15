//! Bounded raw-frame observation, sharing the event bridge's cancellation delivery.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::{Instant, SystemTime};

use bacnet_transport::tap::{FrameTapReceiver, TapDirection, TapPeer, TappedFrame};
use bacnet_types::data_link::DataLink;
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
    fn seconds(self, at: Instant) -> f64 {
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
fn posix_seconds(at: SystemTime) -> f64 {
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
    mac.iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(":")
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
fn peer_text(peer: TapPeer) -> String {
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

struct SourceState {
    receiver: Mutex<Option<FrameTapReceiver>>,
    closed: Notify,
    /// Preserve the final count after the receiver is released.
    final_missed: std::sync::atomic::AtomicU64,
    /// What the last frame converted reported, for the next one.
    carry: Mutex<Carry>,
}

/// The gap the last converted frame reported, and the stream's lost count
/// when it was converted. A leaf lock: held only for the arithmetic.
#[derive(Default)]
struct Carry {
    reported: u64,
    lost: u64,
}

impl SourceState {
    fn receiver(&self) -> MutexGuard<'_, Option<FrameTapReceiver>> {
        self.receiver.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The `missed_before` of a fresh frame whose queue gap is `queue_gap`,
    /// when the stream has lost `lost` deliveries so far.
    ///
    /// One delivery is in flight at a time, and a frame is converted only
    /// once the delivery before it settled, so a loss counted since the
    /// last conversion lost that last frame: the frame and the gap it
    /// reported both fall in front of this one. The sum of `missed_before`
    /// over the frames Python received, plus whatever was lost after the
    /// last of them, is then the iterator's `missed`.
    fn missed_before(&self, queue_gap: u64, lost: u64) -> u64 {
        let mut carry = self.carry.lock().unwrap_or_else(PoisonError::into_inner);
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
                .store(receiver.dropped(), std::sync::atomic::Ordering::Relaxed);
        }
        self.closed.notify_waiters();
    }

    fn missed(&self) -> u64 {
        self.receiver().as_ref().map_or_else(
            || self.final_missed.load(std::sync::atomic::Ordering::Relaxed),
            FrameTapReceiver::dropped,
        )
    }
}

#[derive(Clone)]
struct FrameSource(Arc<SourceState>);

impl EventSource<TappedFrame> for FrameSource {
    async fn recv(&mut self) -> Result<TappedFrame, RecvError> {
        let closed = self.0.closed.notified();
        tokio::pin!(closed);
        closed.as_mut().enable();
        tokio::select! {
            biased;
            () = &mut closed => Err(RecvError::Closed),
            frame = std::future::poll_fn(|cx| {
                match self.0.receiver().as_mut() {
                    Some(receiver) => receiver.poll_recv(cx),
                    None => std::task::Poll::Ready(None),
                }
            }) => frame.ok_or(RecvError::Closed),
        }
    }
}

struct Subscription {
    stream: LossyReceiver<TappedFrame, FrameSource>,
    source: FrameSource,
}

impl Subscription {
    fn close(&self) {
        self.stream.close();
        self.source.0.close();
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.close();
    }
}

/// An owner's weak subscriptions: observers never keep the owner alive.
#[derive(Default)]
pub(crate) struct FrameSubscriptions(Mutex<Vec<Weak<Subscription>>>);

impl FrameSubscriptions {
    pub(crate) fn subscribe(
        &self,
        receiver: FrameTapReceiver,
        local_address: Arc<str>,
        clock: Clock,
    ) -> PyFrameIterator {
        let source = FrameSource(Arc::new(SourceState {
            receiver: Mutex::new(Some(receiver)),
            closed: Notify::new(),
            final_missed: Default::default(),
            carry: Mutex::default(),
        }));
        let subscription = Arc::new(Subscription {
            stream: LossyReceiver::from_source(source.clone()),
            source,
        });
        let mut subscriptions = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        subscriptions.retain(|old| old.strong_count() != 0);
        subscriptions.push(Arc::downgrade(&subscription));
        PyFrameIterator {
            subscription,
            local_address,
            clock,
        }
    }

    /// End the current generation; a later owner restart may subscribe anew.
    pub(crate) fn close(&self) {
        let subscriptions =
            std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner));
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
    subscription: Arc<Subscription>,
    local_address: Arc<str>,
    clock: Clock,
}

/// What a read's future needs to turn a tapped frame into a Python
/// `Frame`. It holds the source and the stream's lost count, never the
/// subscription, so a pending read doesn't keep a dropped iterator open.
struct Converter {
    source: FrameSource,
    lost: Lost,
    local_address: Arc<str>,
    clock: Clock,
}

impl Converter {
    fn frame(&self, frame: TappedFrame) -> PyFrame {
        let missed_before = self
            .source
            .0
            .missed_before(frame.missed_before, self.lost.get());
        PyFrame {
            monotonic: self.clock.seconds(frame.monotonic_timestamp),
            frame,
            local_address: self.local_address.clone(),
            missed_before,
        }
    }
}

impl PyFrameIterator {
    fn converter(&self) -> Converter {
        Converter {
            source: self.subscription.source.clone(),
            lost: self.subscription.stream.lost(),
            local_address: self.local_address.clone(),
            clock: self.clock,
        }
    }
}

#[pymethods]
impl PyFrameIterator {
    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let (next, put_back) = self.subscription.stream.next();
        let convert = self.converter();
        crate::py_async::future_into_py_or_put_back(
            py,
            async move {
                next.await
                    .map(|next| next.map(|frame| convert.frame(frame)))
                    .ok_or_else(|| PyStopAsyncIteration::new_err("frame observation ended"))
            },
            put_back,
        )
    }

    /// Frames lost so far, live: dropped by the full queue (which keeps
    /// the oldest frames) or lost on the way to Python, including any lost
    /// after the last frame read. The count remains readable after close.
    #[getter]
    fn missed(&self) -> u64 {
        self.subscription.source.0.missed() + self.subscription.stream.missed()
    }

    /// Stop observation, discard queued frames and wake pending reads.
    /// An already admitted native delivery may still complete its Python Future.
    /// This does not stop the owner or cancel its protocol requests.
    fn aclose<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let subscription = self.subscription.clone();
        crate::py_async::future_into_py(py, async move {
            subscription.close();
            Ok(())
        })
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
