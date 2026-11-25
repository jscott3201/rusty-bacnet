//! An opt-in, bounded and lossy tap on the link-layer frames a transport
//! sends and receives (#957).
//!
//! B/IP, B/IPv6, BACnet/SC, MS/TP and Ethernet each own a [`FrameTap`],
//! reached with their `frame_tap()` method (through
//! `BACnetClient::transport()` on a running client). [`FrameTap::subscribe`]
//! returns a [`FrameTapReceiver`] that gets a [`TappedFrame`] for every frame
//! the transport sends, and every frame it receives, control traffic
//! included:
//!
//! - B/IP and B/IPv6: every BVLL message, as the UDP datagram's payload. A
//!   received one appears if the transport takes it: what a B/IP broadcast
//!   listener reads only when the node keeps it.
//! - BACnet/SC: every BVLC-SC message on the hub connection, as the
//!   WebSocket message's payload.
//! - MS/TP: every frame, preamble to CRC, Token and Poll For Master
//!   included, that the station sends or hears on the line: other
//!   stations' traffic too, since the bus is shared. Frames are found with
//!   the master node's inter-octet timing: a frame the node gives up, for a
//!   silence longer than T_frame_abort in the middle of it or because it
//!   started to send, is not reported.
//! - Ethernet: every 802.3 frame, from the destination MAC to the padding,
//!   as the packet socket sends or reads it (the network card adds and
//!   strips the frame check sequence). XID and TEST commands and their
//!   responses are included. A received frame appears if the transport
//!   takes it: one addressed to this station or to the broadcast, and not
//!   sent from this station's MAC, whatever the node then does with it.
//!
//! # What it is not
//!
//! The tap sees what this process sends and receives, inside the process.
//! On B/IP, B/IPv6, BACnet/SC and Ethernet that is this node's own traffic
//! only, so it does not replace a wire capture. For BACnet/SC it is the plaintext:
//! frames are taken before TLS encrypts them on the way out and after it
//! decrypts them on the way in.
//!
//! # Order
//!
//! A receiver gets frames in the order the transport hands them over, and
//! a reply never comes ahead of the request that drew it (#2519). On B/IP,
//! B/IPv6, BACnet/SC and Ethernet one task sends while another receives, so
//! a send is handed over just before the socket writes it, and a received
//! frame just after the read: an answer that comes back while the send has
//! yet to return still follows it. Frames that cross on the wire, neither
//! answering the other, may come either way round. A sent frame is reported
//! even if its write then fails or is given up; the sender learns of the
//! failure as before. On MS/TP one task writes and reads the line, so a
//! frame is reported once written, before the station reads on, and a frame
//! whose write fails is not reported.
//!
//! # Cost and loss
//!
//! With nobody subscribed, a frame costs the transport one atomic load and
//! nothing is copied. With subscribers, each frame is copied once and the
//! copy shared among them. Each receiver has its own bounded queue
//! ([`DEFAULT_TAP_CAPACITY`] unless chosen with
//! [`FrameTap::subscribe_with_capacity`]). The transport never waits for a
//! receiver: a frame that finds a receiver's queue full is dropped for that
//! receiver and counted, in [`FrameTapReceiver::dropped`] and in the
//! tap-wide [`FrameTapCounters::dropped`]. The next frame queued to that
//! receiver carries the size of the gap in front of it, in
//! [`TappedFrame::missed_before`], so a reader can tell where frames went
//! missing; the queue keeps the oldest frames and loses the newest.
//!
//! ```
//! use bacnet_transport::tap::FrameTap;
//!
//! let tap = FrameTap::new();
//! assert_eq!(tap.counters().subscribers, 0);
//! let mut frames = tap.subscribe_with_capacity(8);
//! assert_eq!(tap.counters().subscribers, 1);
//! assert!(frames.try_recv().is_none());
//! drop(frames);
//! assert_eq!(tap.counters().subscribers, 0);
//! ```

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::task::{Context, Poll};
use std::time::{Instant, SystemTime};

use bacnet_types::data_link::DataLink;
use bacnet_types::sync::MutexExt;
use bytes::Bytes;
use tokio::sync::mpsc;

use crate::sc::ScActiveHub;

/// The queue length of a receiver from [`FrameTap::subscribe`]: frames it
/// can fall behind by before new ones are dropped for it.
pub const DEFAULT_TAP_CAPACITY: usize = 1024;

/// Which way a tapped frame went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TapDirection {
    /// Received by this node.
    Inbound,
    /// Handed to the socket by this node to send: on MS/TP, written to the
    /// line. See [Order](crate::tap#order) for a send that then fails.
    Outbound,
}

/// The other end of a tapped frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TapPeer {
    /// B/IP and B/IPv6: the datagram's source for an inbound frame, its
    /// destination (a broadcast or multicast address included) for an
    /// outbound one.
    Udp(SocketAddr),
    /// BACnet/SC: the hub connection the message went over.
    ScHub(ScActiveHub),
    /// MS/TP: the frame's source station for an inbound frame, its
    /// destination (255 for broadcast) for an outbound one.
    Mstp(u8),
    /// Ethernet: the frame's source MAC for an inbound frame, its
    /// destination MAC (the all-ones broadcast included) for an outbound
    /// one.
    Ethernet([u8; 6]),
}

/// One link-layer frame a transport sent or received, as one receiver got
/// it: every receiver gets its own copy, which differs from the others'
/// only in [`missed_before`](Self::missed_before).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TappedFrame {
    /// Wall-clock time the transport handed the frame to the tap: just
    /// before the socket write for a sent frame (on MS/TP, just after it),
    /// just after the read for a received one. See [Order](crate::tap#order).
    pub timestamp: SystemTime,
    /// Monotonic time at the same capture point as [`Self::timestamp`].
    /// Use this for elapsed times: wall-clock adjustments cannot reorder it.
    pub monotonic_timestamp: Instant,
    /// Sent or received.
    pub direction: TapDirection,
    /// The data link the frame is on.
    pub link: DataLink,
    /// The other end.
    pub peer: TapPeer,
    /// The frame as the link carries it: a BVLL message for B/IP and
    /// B/IPv6, a BVLC-SC message for BACnet/SC, a whole frame for MS/TP,
    /// and a whole 802.3 frame, headers and padding, for Ethernet.
    pub bytes: Bytes,
    /// Frames this receiver lost to its full queue since the frame queued
    /// to it before this one (since it subscribed, for its first frame):
    /// the gap just ahead of this frame. Frames lost after the last one
    /// queued show only in [`FrameTapReceiver::dropped`] until another
    /// frame is queued, so `dropped` is the sum of this over the frames
    /// queued so far plus that trailing gap.
    pub missed_before: u64,
}

/// A snapshot of a tap's counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct FrameTapCounters {
    /// Receivers subscribed now.
    pub subscribers: usize,
    /// Frames queued to a receiver, one count per receiver.
    pub delivered: u64,
    /// Frames dropped because a receiver's queue was full, one count per
    /// receiver.
    pub dropped: u64,
}

/// One receiver's side of the tap, as the transport sees it.
struct Sink {
    tx: mpsc::Sender<TappedFrame>,
    dropped: Arc<AtomicU64>,
    /// Frames dropped for this receiver since the last one queued to it,
    /// for the next frame's [`TappedFrame::missed_before`]. Read and written
    /// only under the sinks lock, in [`FrameTap::publish`].
    gap: u64,
}

#[derive(Default)]
struct Shared {
    /// Live receivers: the fast path reads only this.
    subscribers: AtomicUsize,
    sinks: Mutex<Vec<Sink>>,
    delivered: AtomicU64,
    dropped: AtomicU64,
}

/// A transport's frame tap. Clones share one tap.
#[derive(Clone, Default)]
pub struct FrameTap {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for FrameTap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameTap")
            .field("counters", &self.counters())
            .finish()
    }
}

impl FrameTap {
    /// A tap nobody is subscribed to.
    pub fn new() -> Self {
        Self::default()
    }

    /// Subscribe with a queue of [`DEFAULT_TAP_CAPACITY`] frames.
    pub fn subscribe(&self) -> FrameTapReceiver {
        self.subscribe_with_capacity(DEFAULT_TAP_CAPACITY)
    }

    /// Subscribe with a queue of `capacity` frames (at least one). The
    /// receiver gets frames from now on, until it is dropped.
    pub fn subscribe_with_capacity(&self, capacity: usize) -> FrameTapReceiver {
        let (tx, rx) = mpsc::channel(capacity.max(1));
        let dropped = Arc::new(AtomicU64::new(0));
        let mut sinks = self.lock_sinks();
        sinks.retain(|sink| !sink.tx.is_closed());
        sinks.push(Sink {
            tx,
            dropped: dropped.clone(),
            gap: 0,
        });
        drop(sinks);
        self.shared.subscribers.fetch_add(1, Ordering::Release);
        FrameTapReceiver {
            rx,
            dropped,
            shared: Arc::downgrade(&self.shared),
        }
    }

    /// The tap's counts so far.
    pub fn counters(&self) -> FrameTapCounters {
        FrameTapCounters {
            subscribers: self.shared.subscribers.load(Ordering::Relaxed),
            delivered: self.shared.delivered.load(Ordering::Relaxed),
            dropped: self.shared.dropped.load(Ordering::Relaxed),
        }
    }

    /// Whether anyone is subscribed: the only work a frame costs without.
    #[inline]
    pub(crate) fn is_active(&self) -> bool {
        self.shared.subscribers.load(Ordering::Relaxed) != 0
    }

    /// Hand a frame to every receiver, never waiting. `peer` is only
    /// worked out, and `bytes` only copied, when someone is subscribed.
    #[inline]
    pub(crate) fn record(
        &self,
        direction: TapDirection,
        link: DataLink,
        peer: impl FnOnce() -> TapPeer,
        bytes: &[u8],
    ) {
        if self.is_active() {
            self.publish(direction, link, peer(), bytes);
        }
    }

    /// Hand a frame to the receivers as a transport would: for tests in
    /// other crates (feature `test-support`), which can't build a
    /// [`TappedFrame`] or reach a transport's own calls.
    #[cfg(any(test, feature = "test-support"))]
    pub fn record_for_test(
        &self,
        direction: TapDirection,
        link: DataLink,
        peer: TapPeer,
        bytes: &[u8],
    ) {
        self.record(direction, link, || peer, bytes);
    }

    /// Stage the moment a publisher has met a full queue and not yet
    /// counted it, for tests in other crates (feature `test-support`).
    ///
    /// Holds the sinks lock as [`Self::record`] does, offers the frame to
    /// every receiver, and runs `while_in_flight` before it counts the
    /// rejections. A receiver that closes inside `while_in_flight` has to
    /// wait for the count to finish.
    #[cfg(any(test, feature = "test-support"))]
    pub fn record_with_overflow_in_flight_for_test(
        &self,
        direction: TapDirection,
        link: DataLink,
        peer: TapPeer,
        bytes: &[u8],
        while_in_flight: impl FnOnce(&OverflowInFlight),
    ) {
        let mut sinks = self.lock_sinks();
        let frame = TappedFrame {
            timestamp: SystemTime::now(),
            monotonic_timestamp: Instant::now(),
            direction,
            link,
            peer,
            bytes: Bytes::copy_from_slice(bytes),
            missed_before: 0,
        };
        let mut full = Vec::new();
        let mut rejected = Vec::new();
        for (index, sink) in sinks.iter_mut().enumerate() {
            let mut copy = frame.clone();
            copy.missed_before = sink.gap;
            match sink.tx.try_send(copy) {
                Ok(()) => sink.gap = 0,
                Err(mpsc::error::TrySendError::Full(_)) => {
                    sink.gap = sink.gap.saturating_add(1);
                    full.push(sink.tx.clone());
                    rejected.push(index);
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {}
            }
        }
        while_in_flight(&OverflowInFlight { full });
        for index in rejected {
            sinks[index].dropped.fetch_add(1, Ordering::Relaxed);
            self.shared.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[cold]
    #[inline(never)]
    fn publish(&self, direction: TapDirection, link: DataLink, peer: TapPeer, bytes: &[u8]) {
        let bytes = Bytes::copy_from_slice(bytes);
        // A rejected frame is counted while this lock is held: a receiver's
        // `close` takes the lock after sealing its queue, so its final count
        // includes a rejection in flight. Keep the count under the lock;
        // `record_with_overflow_in_flight_for_test` mirrors this loop.
        let mut sinks = self.lock_sinks();
        // Stamped under the lock, so that frames two threads report at once
        // are queued in the order of their timestamps.
        let frame = TappedFrame {
            timestamp: SystemTime::now(),
            monotonic_timestamp: Instant::now(),
            direction,
            link,
            peer,
            bytes,
            missed_before: 0,
        };
        sinks.retain_mut(|sink| {
            // Each receiver gets its own copy (the octets are shared), with
            // the gap in front of it.
            let mut copy = frame.clone();
            copy.missed_before = sink.gap;
            match sink.tx.try_send(copy) {
                Ok(()) => {
                    sink.gap = 0;
                    self.shared.delivered.fetch_add(1, Ordering::Relaxed);
                    true
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    sink.gap = sink.gap.saturating_add(1);
                    sink.dropped.fetch_add(1, Ordering::Relaxed);
                    self.shared.dropped.fetch_add(1, Ordering::Relaxed);
                    true
                }
                // The receiver is gone; its drop already counted it out.
                Err(mpsc::error::TrySendError::Closed(_)) => false,
            }
        });
    }

    fn lock_sinks(&self) -> std::sync::MutexGuard<'_, Vec<Sink>> {
        self.shared.sinks.locked()
    }
}

/// Full-queue rejections a publisher has seen and not yet counted: see
/// [`FrameTap::record_with_overflow_in_flight_for_test`].
#[cfg(any(test, feature = "test-support"))]
pub struct OverflowInFlight {
    full: Vec<mpsc::Sender<TappedFrame>>,
}

#[cfg(any(test, feature = "test-support"))]
impl OverflowInFlight {
    /// Block until every receiver that met the full queue has sealed it
    /// (closed it or been dropped), then hold the publisher's lock for a
    /// further 25 ms. Panics if no receiver seals within ten seconds.
    pub fn wait_until_sealed(&self) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !self.full.iter().all(mpsc::Sender::is_closed) {
            assert!(
                std::time::Instant::now() < deadline,
                "a receiver never sealed its queue"
            );
            std::thread::yield_now();
        }
        // A close that does not wait for the count reads it now. Give it
        // 25 ms to; one that waits stays blocked, so this never fails a
        // correct close.
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

/// A subscription to a [`FrameTap`]. Dropping it unsubscribes.
pub struct FrameTapReceiver {
    rx: mpsc::Receiver<TappedFrame>,
    dropped: Arc<AtomicU64>,
    /// Weak, so that the queue closes once the transport and its tap are
    /// gone.
    shared: Weak<Shared>,
}

impl std::fmt::Debug for FrameTapReceiver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameTapReceiver")
            .field("queued", &self.rx.len())
            .field("dropped", &self.dropped())
            .finish()
    }
}

impl FrameTapReceiver {
    /// Poll for the next frame without keeping a mutable borrow across an
    /// await. Like [`Self::recv`], cancellation before readiness loses no frame.
    pub fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Option<TappedFrame>> {
        self.rx.poll_recv(cx)
    }

    /// The next frame, waiting for one. After the transport stops this
    /// keeps waiting, since it may start again; it returns `None` once the
    /// transport that owns the tap is dropped and the queue is empty.
    pub async fn recv(&mut self) -> Option<TappedFrame> {
        self.rx.recv().await
    }

    /// The next frame if one is queued, without waiting.
    pub fn try_recv(&mut self) -> Option<TappedFrame> {
        self.rx.try_recv().ok()
    }

    /// Unsubscribe, discard queued frames and return the final overflow count.
    /// This includes a full-queue rejection that a publisher was still counting
    /// when the receiver closed.
    pub fn close(mut self) -> u64 {
        self.rx.close();
        if let Some(shared) = self.shared.upgrade() {
            // A publisher may have observed Full just before the queue closed.
            // Its increment completes under this lock before the final snapshot.
            shared.sinks.locked().retain(|sink| !sink.tx.is_closed());
        }
        self.dropped()
    }

    /// Frames dropped for this receiver because its queue was full, so far:
    /// the gaps the queued frames' [`TappedFrame::missed_before`] report,
    /// and any frames lost after the last one queued.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl Drop for FrameTapReceiver {
    fn drop(&mut self) {
        if let Some(shared) = self.shared.upgrade() {
            shared.subscribers.fetch_sub(1, Ordering::Release);
        }
    }
}

#[cfg(test)]
#[path = "tap_tests.rs"]
mod tests;
