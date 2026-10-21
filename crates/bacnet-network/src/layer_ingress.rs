//! Turning a transport's received NPDUs into APDUs and network controls, in
//! the task that reads them (#1713).
//!
//! [`NetworkLayer::start`](super::NetworkLayer::start) returns an
//! [`ApduReceiver`], which does this inside its own `recv`: there is no
//! dispatch task between the transport and the application, and no second
//! queue. [`NetworkLayer::start_with_admission`](super::NetworkLayer::start_with_admission)
//! runs the same [`Ingress`] in a dispatch task in front of its tracked queue.

use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::task::{ready, Context, Poll};

use bacnet_router_core::node::Node;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::warn_throttle::WarnThrottle;
use std::sync::{Mutex, MutexGuard, PoisonError};

use tokio::sync::mpsc;
use tracing::debug;

use super::receive::{self, Inbound, NodeTotals};
use super::{next_ingress_sequence, AdmissionSender, ReceivedApdu, ReceivedNetworkControl};

/// A layer's receive state: the router core's node, which applies the
/// receive rules (#1795), and where network controls go.
pub(crate) struct Ingress {
    node: Node,
    totals: Arc<NodeTotals>,
    drop_warning: WarnThrottle,
    controls: Option<AdmissionSender<ReceivedNetworkControl>>,
    control_sequence: Arc<AtomicU64>,
}

impl std::fmt::Debug for Ingress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ingress")
            .field("controls", &self.controls.is_some())
            .finish_non_exhaustive()
    }
}

impl Ingress {
    pub(crate) fn new(
        totals: Arc<NodeTotals>,
        controls: Option<AdmissionSender<ReceivedNetworkControl>>,
        control_sequence: Arc<AtomicU64>,
    ) -> Self {
        Self {
            node: Node::new(),
            totals,
            drop_warning: WarnThrottle::default(),
            controls,
            control_sequence,
        }
    }

    /// The APDU `received` carries, if it carries one. A network message
    /// goes to the control receiver, if there is one, without waiting; a
    /// frame the node drops is counted. Nothing here awaits.
    pub(crate) fn take(&mut self, mut received: ReceivedNpdu) -> Option<ReceivedApdu> {
        // Moved, not cloned: nothing below reads the frame again.
        let frame = std::mem::take(&mut received.npdu);
        let inbound = receive::admit(
            &mut self.node,
            &self.totals,
            &self.drop_warning,
            &received.source_mac,
            received.link_layer_group,
            frame,
            self.controls.is_some(),
        );
        match inbound {
            Inbound::Apdu {
                apdu,
                source_network,
                is_group,
                global_broadcast,
            } => Some(ReceivedApdu {
                router_response: None,
                direct_response: received.direct_response,
                apdu,
                source_mac: received.source_mac,
                ingress_network: None,
                source_network,
                link_layer_group: received.link_layer_group,
                is_group,
                global_broadcast,
                data_attributes: received.data_attributes,
                provenance: received.provenance,
                reply_tx: received.reply_tx,
            }),
            Inbound::Control(Some(npdu)) => {
                let tx = self.controls.as_ref()?;
                let control = ReceivedNetworkControl {
                    npdu,
                    source_mac: received.source_mac,
                    link_layer_group: received.link_layer_group,
                    data_attributes: received.data_attributes,
                    provenance: received.provenance,
                    ingress_sequence: next_ingress_sequence(&self.control_sequence),
                };
                if let Err(mpsc::error::TrySendError::Closed(_)) = tx.try_send(control) {
                    self.controls = None;
                    debug!("Network-control receiver closed; resuming discard behavior");
                }
                None
            }
            Inbound::Control(None) | Inbound::Dropped => None,
        }
    }
}

/// The transport queue an [`ApduReceiver`] takes from, shared with its
/// layer so [`NetworkLayer::stop`](super::NetworkLayer::stop) can close it.
/// Never locked across an await.
pub(crate) type SharedNpduQueue = Arc<Mutex<mpsc::Receiver<ReceivedNpdu>>>;

/// The APDUs a [`NetworkLayer`](super::NetworkLayer) receives, from
/// [`NetworkLayer::start`](super::NetworkLayer::start).
///
/// Each [`recv`](Self::recv) takes the transport's next NPDUs and decodes
/// them in the caller's task until one carries an APDU. Network messages met
/// on the way go to the control receiver, if one was enabled, so **controls
/// flow only while this receiver is polled**: while its consumer is busy in
/// an inline await they wait in the transport's queue, which adds latency
/// but cannot deadlock. That queue is the only buffer: under overload it
/// bounds memory, and the transport counts the NPDUs it drops. See the
/// [receive-queue contract](super#receive-queue-admission).
///
/// [`NetworkLayer::stop`](super::NetworkLayer::stop), or dropping the layer,
/// closes the transport's queue: the receiver yields what was queued before
/// then, then `None`, even when the transport keeps its sender alive.
#[derive(Debug)]
pub struct ApduReceiver {
    npdu_rx: SharedNpduQueue,
    ingress: Ingress,
}

impl ApduReceiver {
    pub(crate) fn new(npdu_rx: SharedNpduQueue, ingress: Ingress) -> Self {
        Self { npdu_rx, ingress }
    }

    fn queue(&self) -> MutexGuard<'_, mpsc::Receiver<ReceivedNpdu>> {
        self.npdu_rx.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Receive the next APDU, or `None` once the transport's queue is closed,
    /// by a stop or [`Self::close`], and every NPDU in it has been taken.
    ///
    /// Cancel safe, so it can sit in a `select!`: the only wait is the
    /// transport queue's own receive, and an NPDU taken from it is decoded
    /// and handed on before this returns or waits again. Dropping the future
    /// loses nothing, and it holds no lock while it waits.
    pub async fn recv(&mut self) -> Option<ReceivedApdu> {
        std::future::poll_fn(|cx| self.poll_recv(cx)).await
    }

    /// Poll for the next APDU, as [`Self::recv`] does.
    pub fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Option<ReceivedApdu>> {
        let mut queue = self.npdu_rx.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            let Some(received) = ready!(queue.poll_recv(cx)) else {
                return Poll::Ready(None);
            };
            if let Some(apdu) = self.ingress.take(received) {
                return Poll::Ready(Some(apdu));
            }
        }
    }

    /// Receive an APDU the transport has already queued, without waiting.
    /// `Empty` means none is queued now; `Disconnected`, that the queue is
    /// closed and drained.
    pub fn try_recv(&mut self) -> Result<ReceivedApdu, mpsc::error::TryRecvError> {
        let mut queue = self.npdu_rx.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            let received = queue.try_recv()?;
            if let Some(apdu) = self.ingress.take(received) {
                return Ok(apdu);
            }
        }
    }

    /// Stop the transport handing up more NPDUs, keeping those it already
    /// queued for [`Self::recv`] to drain.
    pub fn close(&mut self) {
        self.queue().close();
    }

    /// Whether the transport can no longer hand up NPDUs: the layer stopped,
    /// or this receiver was closed. Queued ones may remain.
    pub fn is_closed(&self) -> bool {
        self.queue().is_closed()
    }
}

/// Close `queue`, if its receiver still exists, so its stream ends once
/// drained. Run by the layer's stop and drop.
pub(crate) fn close_queue(queue: &std::sync::Weak<Mutex<mpsc::Receiver<ReceivedNpdu>>>) {
    if let Some(queue) = queue.upgrade() {
        queue.lock().unwrap_or_else(PoisonError::into_inner).close();
    }
}
