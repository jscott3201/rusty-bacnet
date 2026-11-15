//! Bounded raw-frame observation, sharing the event bridge's cancellation delivery.

use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::Instant;

use bacnet_transport::tap::{FrameTapReceiver, TapDirection, TapPeer, TappedFrame};
use pyo3::exceptions::{PyStopAsyncIteration, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use tokio::sync::{Notify, broadcast::error::RecvError};

use crate::event_stream::{EventSource, LossyReceiver, Lost};

static EPOCH: OnceLock<Instant> = OnceLock::new();

/// Initialize the common capture-time origin before subscribing.
pub(crate) fn capacity(capacity: usize) -> PyResult<usize> {
    if !(1..=65_536).contains(&capacity) {
        return Err(PyValueError::new_err(
            "capacity must be between 1 and 65536 frames",
        ));
    }
    EPOCH.get_or_init(Instant::now);
    Ok(capacity)
}

/// One transport observation. Its timestamp belongs to capture, not iteration.
#[pyclass(name = "Frame", module = "rusty_bacnet", frozen)]
pub(crate) struct PyFrame {
    frame: TappedFrame,
    local_address: String,
    missed_before: u64,
}

#[pymethods]
impl PyFrame {
    /// Monotonic seconds from this module's first frame subscription.
    /// Comparable across subscriptions, not to wall time or Python's clock origin.
    #[getter]
    fn timestamp(&self) -> f64 {
        self.frame
            .monotonic_timestamp
            .duration_since(*EPOCH.get().expect("subscription epoch"))
            .as_secs_f64()
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

    /// The link's display name, such as `"BACnet/IP"` or `"MS/TP"`.
    #[getter]
    fn link(&self) -> &'static str {
        self.frame.link.name()
    }

    /// The observing node's link address when the subscription was created.
    #[getter]
    fn local_address(&self) -> &str {
        &self.local_address
    }

    /// UDP source/destination, SC hub name, MS/TP station or Ethernet MAC.
    /// This is the link peer, not the original source in a forwarded frame.
    #[getter]
    fn peer(&self) -> String {
        match self.frame.peer {
            TapPeer::Udp(address) => address.to_string(),
            TapPeer::ScHub(bacnet_transport::sc::ScActiveHub::Primary) => "primary".into(),
            TapPeer::ScHub(bacnet_transport::sc::ScActiveHub::Failover) => "failover".into(),
            TapPeer::Mstp(station) => station.to_string(),
            TapPeer::Ethernet(mac) => hex_address(&mac),
            _ => "other".into(),
        }
    }

    /// Raw link-layer octets, including malformed received datagrams.
    #[getter]
    fn octets<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.frame.bytes)
    }

    /// Frames this subscription missed just before this one: dropped by its
    /// full queue, or lost on the way to Python with the gap such a frame
    /// carried. A cancelled read's redelivery keeps the value.
    #[getter]
    fn missed_before(&self) -> u64 {
        self.missed_before
    }
}

pub(crate) fn hex_address(mac: &[u8]) -> String {
    mac.iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(":")
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
        local_address: String,
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
    local_address: String,
}

/// What a read's future needs to turn a tapped frame into a Python
/// `Frame`. It holds the source and the stream's lost count, never the
/// subscription, so a pending read doesn't keep a dropped iterator open.
struct Converter {
    source: FrameSource,
    lost: Lost,
    local_address: String,
}

impl Converter {
    fn frame(&self, frame: TappedFrame) -> PyFrame {
        let missed_before = self
            .source
            .0
            .missed_before(frame.missed_before, self.lost.get());
        PyFrame {
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
mod tests {
    use super::*;
    use std::future::Future;
    use std::task::{Context, Waker};

    #[tokio::test(start_paused = true)]
    async fn owner_close_releases_receiver_and_wakes_pending_read() {
        let tap = bacnet_transport::tap::FrameTap::new();
        let owner = FrameSubscriptions::default();
        let iterator = owner.subscribe(tap.subscribe(), "local".into());
        let (pending, _hook) = iterator.subscription.stream.next();
        tokio::pin!(pending);
        assert!(
            pending
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        assert_eq!(tap.counters().subscribers, 1);
        owner.close();
        assert_eq!(tap.counters().subscribers, 0);
        assert!(pending.await.is_none());
        let fresh = owner.subscribe(tap.subscribe(), "new generation".into());
        assert!(iterator.subscription.stream.next().0.await.is_none());
        assert_eq!(tap.counters().subscribers, 1);
        drop(fresh);
        assert_eq!(tap.counters().subscribers, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn cancelling_native_wait_keeps_subscription_until_iterator_drop() {
        let tap = bacnet_transport::tap::FrameTap::new();
        let owner = FrameSubscriptions::default();
        let iterator = owner.subscribe(tap.subscribe(), "local".into());
        {
            let (pending, _hook) = iterator.subscription.stream.next();
            tokio::pin!(pending);
            assert!(
                pending
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop()))
                    .is_pending()
            );
        }
        assert_eq!(tap.counters().subscribers, 1);
        drop(iterator);
        assert_eq!(tap.counters().subscribers, 0);
    }
}
