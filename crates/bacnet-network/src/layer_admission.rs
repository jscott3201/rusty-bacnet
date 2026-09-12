use super::*;
use bacnet_encoding::npdu::decode_npdu;
use std::sync::Mutex;
use std::task::Poll;
use tokio::sync::mpsc;
use tracing::{debug, warn};

const QUEUE_CAPACITY: usize = 256;

/// A consistent, count-only snapshot of one NetworkLayer receive queue.
///
/// Counters belong to one receiver, not to a source MAC or to the entire layer.
/// Separate APDU/control snapshots are not an atomic snapshot of both queues.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct QueueAdmissionSnapshot {
    /// Items currently queued, excluding items already returned to the consumer.
    pub current_depth: usize,
    /// Greatest queued depth since this receiver was created (at most 256).
    pub high_water: usize,
    /// Arriving items dropped because the queue was full; saturates at `u64::MAX`.
    pub full_drops: u64,
    /// Arriving items dropped because the receiver was closed; saturates at `u64::MAX`.
    ///
    /// The first such APDU ends dispatch; the first such control disables that
    /// stream. Later discarded controls are not queue-admission attempts.
    /// Dropping already queued items with the receiver is not an admission drop.
    pub closed_drops: u64,
}

/// Cloneable snapshot handle obtained from an [`AdmissionReceiver`].
///
/// This handle keeps only counters alive, not the channel or dispatch task.
/// It remains readable after the receiver or NetworkLayer is dropped. Dropping
/// the receiver sets depth to zero; closing it preserves depth until drained.
#[derive(Debug, Clone, Default)]
pub struct QueueAdmissionCounters(Arc<Mutex<QueueAdmissionSnapshot>>);

impl QueueAdmissionCounters {
    /// Read this queue's depth, high-water mark, and admission-drop totals.
    pub fn snapshot(&self) -> QueueAdmissionSnapshot {
        *self.0.lock().expect("queue admission counters poisoned")
    }
}

/// Opt-in, one-consumer receiver with exact queue-depth accounting.
///
/// Created by [`NetworkLayer::start_with_admission`] or
/// [`NetworkLayer::enable_network_control_receiver_with_admission`]. The queue
/// holds 256 items; full admission drops the arriving item, never an older one.
/// APDU and control queues have independent capacities and counters. Neither
/// full queue waits for its consumer or blocks dispatch to the other queue.
///
/// This wrapper deliberately does not expose the underlying receiver: all
/// dequeues must update the snapshot. Use the existing NetworkLayer methods
/// when a plain `mpsc::Receiver` is required instead.
#[derive(Debug)]
pub struct AdmissionReceiver<T> {
    rx: mpsc::Receiver<T>,
    counters: QueueAdmissionCounters,
}

impl<T> AdmissionReceiver<T> {
    /// Obtain an independently owned snapshot handle for this queue.
    pub fn counters(&self) -> QueueAdmissionCounters {
        self.counters.clone()
    }

    /// Receive the next item, or `None` once closed and drained.
    ///
    /// Cancellation safe: dropping a pending receive does not remove an item
    /// or alter its depth. No counter lock is held while waiting.
    pub async fn recv(&mut self) -> Option<T> {
        std::future::poll_fn(|cx| {
            let mut snapshot = self
                .counters
                .0
                .lock()
                .expect("queue admission counters poisoned");
            let result = self.rx.poll_recv(cx);
            if matches!(result, Poll::Ready(Some(_))) {
                snapshot.current_depth -= 1;
            }
            result
        })
        .await
    }

    /// Receive immediately, distinguishing an empty queue from a closed one.
    pub fn try_recv(&mut self) -> Result<T, mpsc::error::TryRecvError> {
        let mut snapshot = self
            .counters
            .0
            .lock()
            .expect("queue admission counters poisoned");
        let result = self.rx.try_recv();
        if result.is_ok() {
            snapshot.current_depth -= 1;
        }
        result
    }

    /// Stop admission while retaining already queued items for draining.
    ///
    /// The next arriving APDU ends dispatch, or the next control disables its
    /// stream, exactly as closing the corresponding legacy receiver does.
    pub fn close(&mut self) {
        let _snapshot = self
            .counters
            .0
            .lock()
            .expect("queue admission counters poisoned");
        self.rx.close();
    }
}

impl<T> Drop for AdmissionReceiver<T> {
    fn drop(&mut self) {
        let mut snapshot = self
            .counters
            .0
            .lock()
            .expect("queue admission counters poisoned");
        self.rx.close();
        snapshot.current_depth = 0;
        // The receiver's queued items (including reply senders) drop after
        // this guard is released. Closing first prevents concurrent admission.
    }
}

pub(super) struct AdmissionSender<T> {
    tx: mpsc::Sender<T>,
    counters: QueueAdmissionCounters,
    // Legacy receivers dequeue outside this module, so only opt-in receivers
    // expose depth/high-water accounting. Admission drops are counted in both.
    track_depth: bool,
}

impl<T> AdmissionSender<T> {
    fn channel(track_depth: bool) -> (Self, mpsc::Receiver<T>) {
        let (tx, rx) = mpsc::channel(QUEUE_CAPACITY);
        (
            Self {
                tx,
                counters: QueueAdmissionCounters::default(),
                track_depth,
            },
            rx,
        )
    }

    fn try_send(&self, item: T) -> Result<(), mpsc::error::TrySendError<T>> {
        // Serialize the admission and dequeue accounting, not asynchronous
        // waiting. Each queue has its own short critical section; counters do
        // not govern admission and no lock survives a poll/try operation.
        let mut snapshot = self
            .counters
            .0
            .lock()
            .expect("queue admission counters poisoned");
        let result = self.tx.try_send(item);
        match &result {
            Ok(()) if self.track_depth => {
                snapshot.current_depth += 1;
                snapshot.high_water = snapshot.high_water.max(snapshot.current_depth);
            }
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                snapshot.full_drops = snapshot.full_drops.saturating_add(1);
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                snapshot.closed_drops = snapshot.closed_drops.saturating_add(1);
            }
        }
        result
    }
}

impl<T: TransportPort + 'static> NetworkLayer<T> {
    /// Enable the one-consumer decoded network-control stream.
    ///
    /// This must be called before [`Self::start`] and at most once. Dropping
    /// the returned receiver disables control delivery without affecting APDU
    /// ingress. The queue holds 256 items and drops the arriving control when
    /// full. Use [`Self::enable_network_control_receiver_with_admission`] to
    /// observe admission counters alongside receive operations.
    pub fn enable_network_control_receiver(
        &mut self,
    ) -> Result<mpsc::Receiver<ReceivedNetworkControl>, Error> {
        self.enable_network_control(false).map(|(rx, _)| rx)
    }

    /// Enable the control stream with queue-admission snapshots.
    ///
    /// This has the same before-start, one-consumer contract and errors as
    /// [`Self::enable_network_control_receiver`]; the two methods are alternatives.
    /// Controls discarded without opting into either stream are not admission
    /// drops. Ingress sequencing still advances before every attempted control
    /// admission, including Full and Closed drops.
    pub fn enable_network_control_receiver_with_admission(
        &mut self,
    ) -> Result<AdmissionReceiver<ReceivedNetworkControl>, Error> {
        let (rx, counters) = self.enable_network_control(true)?;
        Ok(AdmissionReceiver { rx, counters })
    }

    fn enable_network_control(
        &mut self,
        track_depth: bool,
    ) -> Result<
        (
            mpsc::Receiver<ReceivedNetworkControl>,
            QueueAdmissionCounters,
        ),
        Error,
    > {
        if self.dispatch_task.is_some() {
            return Err(Error::Encoding(
                "network-control receiver must be enabled before NetworkLayer::start".into(),
            ));
        }
        if self.network_control_tx.is_some() {
            return Err(Error::Encoding(
                "network-control receiver is already enabled".into(),
            ));
        }
        let (tx, rx) = AdmissionSender::channel(track_depth);
        let counters = tx.counters.clone();
        self.network_control_tx = Some(tx);
        Ok((rx, counters))
    }

    /// Start the network layer. Returns a receiver for incoming APDUs.
    ///
    /// This starts the underlying transport and spawns a dispatch task that
    /// decodes incoming NPDUs and extracts APDUs. The queue holds 256 items;
    /// full admission drops the arriving APDU and its owned reply channel and
    /// metadata without blocking control delivery. A closed APDU receiver ends
    /// dispatch on the next APDU admission attempt.
    /// Use [`Self::start_with_admission`] to observe queue-admission snapshots.
    pub async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedApdu>, Error> {
        self.start_dispatch(false).await.map(|(rx, _)| rx)
    }

    /// Start with an APDU receiver that also exposes admission snapshots.
    ///
    /// This is an alternative to [`Self::start`], with the same transport and
    /// dispatch lifecycle. Stopping the layer closes admission but leaves
    /// queued items available to drain. Dropping the receiver discards those
    /// items and releases their reply channels without sending reply bytes.
    ///
    /// ```no_run
    /// # async fn example() -> Result<(), bacnet_types::error::Error> {
    /// use bacnet_network::layer::NetworkLayer;
    /// use bacnet_transport::loopback::LoopbackTransport;
    /// let (transport, _peer) = LoopbackTransport::pair(vec![1], vec![2]);
    /// let mut network = NetworkLayer::new(transport);
    /// let mut controls = network.enable_network_control_receiver_with_admission()?;
    /// let mut apdus = network.start_with_admission().await?;
    /// let apdu_counters = apdus.counters();
    /// let control_counters = controls.counters();
    /// tokio::select! {
    ///     _ = apdus.recv() => {},
    ///     _ = controls.recv() => {},
    /// }
    /// let _snapshots = (apdu_counters.snapshot(), control_counters.snapshot());
    /// network.stop().await?;
    /// # Ok(()) }
    /// ```
    pub async fn start_with_admission(&mut self) -> Result<AdmissionReceiver<ReceivedApdu>, Error> {
        let (rx, counters) = self.start_dispatch(true).await?;
        Ok(AdmissionReceiver { rx, counters })
    }

    async fn start_dispatch(
        &mut self,
        track_depth: bool,
    ) -> Result<(mpsc::Receiver<ReceivedApdu>, QueueAdmissionCounters), Error> {
        let mut npdu_rx = self.transport.start().await?;
        let mut network_control_tx = self.network_control_tx.take();
        let network_control_ingress_sequence = Arc::clone(&self.network_control_ingress_sequence);
        let (apdu_tx, apdu_rx) = AdmissionSender::channel(track_depth);
        let counters = apdu_tx.counters.clone();

        let dispatch_task = tokio::spawn(async move {
            while let Some(received) = npdu_rx.recv().await {
                match decode_npdu(received.npdu.clone()) {
                    Ok(npdu) => {
                        if npdu.is_network_message {
                            if let Some(tx) = network_control_tx.as_ref() {
                                let ingress_sequence = next_ingress_sequence(
                                    network_control_ingress_sequence.as_ref(),
                                );
                                let control = ReceivedNetworkControl {
                                    npdu,
                                    source_mac: received.source_mac,
                                    link_layer_group: received.link_layer_group,
                                    data_attributes: received.data_attributes,
                                    ingress_sequence,
                                };
                                match tx.try_send(control) {
                                    Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => {}
                                    Err(mpsc::error::TrySendError::Closed(_)) => {
                                        network_control_tx = None;
                                        debug!("Network-control receiver closed; resuming discard behavior");
                                    }
                                }
                            } else {
                                debug!(
                                    message_type = npdu.message_type,
                                    "Ignoring network layer message (non-router mode)"
                                );
                            }
                            continue;
                        }

                        // Non-routing node: discard messages with a specific DNET.
                        if let Some(ref dest) = npdu.destination {
                            if dest.network != 0xFFFF {
                                debug!(
                                    dnet = dest.network,
                                    "Discarding routed message (non-router)"
                                );
                                continue;
                            }
                        }

                        let source_network = npdu.source.clone();
                        let is_group =
                            is_group_delivery(received.link_layer_group, npdu.destination.as_ref());
                        let apdu = ReceivedApdu {
                            apdu: npdu.payload,
                            source_mac: received.source_mac,
                            source_network,
                            link_layer_group: received.link_layer_group,
                            is_group,
                            data_attributes: received.data_attributes,
                            reply_tx: received.reply_tx,
                        };

                        match apdu_tx.try_send(apdu) {
                            Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => {}
                            Err(mpsc::error::TrySendError::Closed(_)) => break,
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "Failed to decode NPDU");
                    }
                }
            }
        });

        self.dispatch_task = Some(dispatch_task);
        Ok((apdu_rx, counters))
    }
}
