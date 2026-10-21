//! An opt-in, bounded and lossy tap on the link-layer frames a transport
//! sends and receives (#957).
//!
//! B/IP, B/IPv6, BACnet/SC and MS/TP each own a [`FrameTap`], reached with
//! their `frame_tap()` method (through `BACnetClient::transport()` on a
//! running client). [`FrameTap::subscribe`] returns a [`FrameTapReceiver`]
//! that gets a [`TappedFrame`] for every frame the transport sends, and
//! every frame it receives, control traffic included:
//!
//! - B/IP and B/IPv6: every BVLL message, as the UDP datagram's payload. A
//!   received one appears if the transport takes it: what a B/IP broadcast
//!   listener reads only when the node keeps it.
//! - BACnet/SC: every BVLC-SC message on the hub connection, as the
//!   WebSocket message's payload.
//! - MS/TP: every frame, preamble to CRC, Token and Poll For Master
//!   included, that the station sends or hears on the line: other
//!   stations' traffic too, since the bus is shared. Frames are found from
//!   the octets alone, without the master node's inter-octet timing, so one
//!   the node aborted for a gap can still appear.
//!
//! # What it is not
//!
//! The tap sees what this process sends and receives, inside the process.
//! On B/IP, B/IPv6 and BACnet/SC that is this node's own traffic only, so
//! it does not replace a wire capture. For BACnet/SC it is the plaintext:
//! frames are taken before TLS encrypts them on the way out and after it
//! decrypts them on the way in.
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
//! tap-wide [`FrameTapCounters::dropped`].
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
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::SystemTime;

use bacnet_types::data_link::DataLink;
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
    /// Sent by this node.
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
}

/// One link-layer frame a transport sent or received.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TappedFrame {
    /// Wall-clock time the transport handed the frame to the tap: just
    /// after the send returned, or just after the receive did.
    pub timestamp: SystemTime,
    /// Sent or received.
    pub direction: TapDirection,
    /// The data link the frame is on.
    pub link: DataLink,
    /// The other end.
    pub peer: TapPeer,
    /// The frame as the link carries it: a BVLL message for B/IP and
    /// B/IPv6, a BVLC-SC message for BACnet/SC, a whole frame for MS/TP.
    pub bytes: Bytes,
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

    #[cold]
    #[inline(never)]
    fn publish(&self, direction: TapDirection, link: DataLink, peer: TapPeer, bytes: &[u8]) {
        let frame = TappedFrame {
            timestamp: SystemTime::now(),
            direction,
            link,
            peer,
            bytes: Bytes::copy_from_slice(bytes),
        };
        let mut sinks = self.lock_sinks();
        sinks.retain(|sink| match sink.tx.try_send(frame.clone()) {
            Ok(()) => {
                self.shared.delivered.fetch_add(1, Ordering::Relaxed);
                true
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                sink.dropped.fetch_add(1, Ordering::Relaxed);
                self.shared.dropped.fetch_add(1, Ordering::Relaxed);
                true
            }
            // The receiver is gone; its drop already counted it out.
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        });
    }

    fn lock_sinks(&self) -> std::sync::MutexGuard<'_, Vec<Sink>> {
        self.shared
            .sinks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
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

    /// Frames dropped for this receiver because its queue was full.
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
