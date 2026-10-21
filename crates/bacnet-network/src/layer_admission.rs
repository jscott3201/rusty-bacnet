use super::ingress::{ApduReceiver, Ingress};
use super::*;
use std::collections::HashMap;
use std::sync::Mutex;
use std::task::Poll;
use tokio::sync::mpsc;

const QUEUE_CAPACITY: usize = 256;
const APDU_SOURCE_CAPACITY: usize = 16;

// NetworkLayer has one unnamed transport; router MACs are scoped to the
// ingress port's network number, never the routed NPDU SNET/SADR.
type AdmissionSource = (Option<u16>, MacAddr);

fn apdu_source(apdu: &ReceivedApdu) -> AdmissionSource {
    (apdu.ingress_network, apdu.source_mac.clone())
}

/// A consistent, count-only snapshot of one network-layer receive queue.
///
/// Counters belong to one receiver, not to a source MAC. A router's local queue
/// is shared by all its ports. No payloads, source identities or per-source
/// totals are exposed. The three drop totals saturate at `u64::MAX` and attribute
/// each admission drop once, in **Closed > fairness > Full** order.
/// Separate APDU/control snapshots are not an atomic snapshot of both queues.
/// See the [receive-queue contract](crate::layer#receive-queue-admission) and
/// [`QueueAdmissionCounters::snapshot`] for receiver kinds and lifetime.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct QueueAdmissionSnapshot {
    /// Items currently queued (at most 256), excluding items already returned by
    /// [`AdmissionReceiver::recv`] or [`AdmissionReceiver::try_recv`]. Close and
    /// stop preserve depth until draining; dropping the receiver sets it to zero.
    pub current_depth: usize,
    /// Greatest queued depth since this receiver was created (at most 256).
    /// Draining, closing, stopping or dropping the receiver does not reset it.
    pub high_water: usize,
    /// Arriving items dropped because the 256-item queue was full, after Closed
    /// and any per-source fairness check; saturates at `u64::MAX`.
    /// A fairness drop does not also increment this total.
    pub full_drops: u64,
    /// Arriving APDUs dropped by the tracked queue's quota of 16 per source:
    /// complete source MAC bytes for NetworkLayer, (ingress port network number,
    /// source MAC bytes) for routers, never routed NPDU SNET/SADR. See the
    /// [source-key definitions](crate::layer#source-keys-and-drop-precedence).
    /// Saturates at `u64::MAX`. Closed takes precedence; otherwise evaluated
    /// before global Full, even when the queue has room. Always zero for control
    /// queues, which have no per-source quota. This is a queue-wide total, not
    /// a per-source counter.
    pub fairness_drops: u64,
    /// Arriving items dropped because the receiver was closed or dropped;
    /// saturates at `u64::MAX`. Takes precedence over fairness and Full.
    ///
    /// In NetworkLayer, the first such APDU ends dispatch; the first such control
    /// disables that stream. Later discarded controls are not admission attempts.
    /// Routers keep forwarding and count each closed local-admission attempt.
    /// Dropping already queued items with the receiver is not an admission drop.
    pub closed_drops: u64,
}

/// Cloneable snapshot handle obtained from an [`AdmissionReceiver`].
///
/// This handle keeps only accounting alive, not payloads, the channel or dispatch task.
/// It remains readable after the receiver, layer, or router is dropped. Dropping
/// the receiver sets depth to zero; closing it preserves depth until drained.
/// High-water/drop totals are retained, and later Closed admission attempts may
/// still increase the Closed total. All clones observe the same queue; snapshots
/// expose counts only, not source identities or per-source totals. See the
/// [receive-queue contract](crate::layer#receive-queue-admission).
#[derive(Debug, Clone, Default)]
pub struct QueueAdmissionCounters(Arc<Mutex<QueueAdmissionState>>);

#[derive(Debug, Default)]
struct QueueAdmissionState {
    snapshot: QueueAdmissionSnapshot,
    // Used only by tracked APDU queues. Insert after successful
    // admission, remove on the last dequeue: live entries <= depth <= 256.
    queued_by_source: HashMap<AdmissionSource, usize>,
}

impl QueueAdmissionState {
    fn dequeued(&mut self, source: Option<AdmissionSource>) {
        self.snapshot.current_depth -= 1;
        if let Some(ref source) = source {
            let count = self
                .queued_by_source
                .get_mut(source)
                .expect("queued APDU source must be accounted");
            *count -= 1;
            if *count == 0 {
                self.queued_by_source.remove(source);
            }
            self.assert_source_depth();
        }
    }

    fn assert_source_depth(&self) {
        debug_assert!(self.queued_by_source.len() <= self.snapshot.current_depth);
        debug_assert!(self.snapshot.current_depth <= QUEUE_CAPACITY);
        debug_assert_eq!(
            self.queued_by_source.values().sum::<usize>(),
            self.snapshot.current_depth
        );
        debug_assert!(self
            .queued_by_source
            .values()
            .all(|count| (1..=APDU_SOURCE_CAPACITY).contains(count)));
    }
}

impl QueueAdmissionCounters {
    /// Read this queue's depth, high-water mark, and admission-drop totals.
    ///
    /// Returns a consistent copy of all [`QueueAdmissionSnapshot`] fields under
    /// one accounting lock, without resetting them. Separate calls for APDU and
    /// control queues are not jointly atomic. Counts do not expose payloads,
    /// source identities or per-source totals; drop precedence and saturation
    /// follow the [receive-queue contract](crate::layer#receive-queue-admission).
    /// The handle remains readable after the receiver, layer, or router is
    /// dropped; a snapshot need not be final while dispatch can still attempt
    /// admission. A default handle has zero counts and is not attached to a queue.
    pub fn snapshot(&self) -> QueueAdmissionSnapshot {
        self.0
            .lock()
            .expect("queue admission counters poisoned")
            .snapshot
    }
}

/// Opt-in, one-consumer receiver with exact queue-depth accounting.
///
/// Created by [`NetworkLayer::start_with_admission`] or
/// [`NetworkLayer::enable_network_control_receiver_with_admission`], or by
/// [`RouterOptions::track_admission`](crate::router::RouterOptions::track_admission) or
/// [`RouterOptions::network_control_receiver_with_admission`](crate::router::RouterOptions::network_control_receiver_with_admission). The queue
/// holds 256 items; tracked APDUs additionally have a quota of 16 queued items per
/// source MAC (NetworkLayer) or (ingress port network number, source MAC) (router).
/// Controls have no per-source quota. **Closed > fairness > Full** determines
/// drop attribution; an admission drop releases the arriving payload, metadata
/// and reply sender without reply bytes or a wire rejection, never evicting an
/// older item. See the [receive-queue contract](crate::layer#receive-queue-admission)
/// for exact keys, the raw/tracked matrix, queue independence and lifecycle.
///
/// [`Self::close`] retains queued items for draining; dropping this receiver
/// discards them and sets depth to zero without counting admission drops.
/// High-water/drop totals survive through an owned [`Self::counters`] handle.
///
/// This wrapper deliberately does not expose the underlying receiver: all
/// dequeues must update the snapshot. Use [`NetworkLayer::start`] for the
/// raw [`ApduReceiver`], a router started without
/// [`RouterOptions::track_admission`](crate::router::RouterOptions::track_admission)
/// for a plain `mpsc::Receiver`, or
/// [`NetworkLayer::enable_network_control_receiver`] for raw controls.
#[derive(Debug)]
pub struct AdmissionReceiver<T> {
    rx: mpsc::Receiver<T>,
    counters: QueueAdmissionCounters,
    // Only tracked APDU constructors install source accounting. The same
    // extractor is used on admission and every dequeue; no public trait bound.
    source_key: Option<fn(&T) -> AdmissionSource>,
}

impl<T> AdmissionReceiver<T> {
    // Crate-internal construction keeps the generic sender and accounting in
    // this module while letting the router hand out a raw receiver as well.
    pub(crate) fn channel(
        track_depth: bool,
    ) -> (
        AdmissionSender<T>,
        mpsc::Receiver<T>,
        QueueAdmissionCounters,
    ) {
        let (tx, rx) = AdmissionSender::channel(track_depth);
        let counters = tx.counters.clone();
        (tx, rx, counters)
    }

    pub(crate) fn from_parts(rx: mpsc::Receiver<T>, counters: QueueAdmissionCounters) -> Self {
        Self {
            rx,
            counters,
            source_key: None,
        }
    }

    /// Obtain an independently owned snapshot handle for this queue.
    ///
    /// The clone shares accounting without retaining the receiver, payloads or
    /// dispatch task, and remains readable after receiver/layer/router drop.
    /// See [`QueueAdmissionCounters::snapshot`] and the
    /// [receive-queue contract](crate::layer#receive-queue-admission).
    pub fn counters(&self) -> QueueAdmissionCounters {
        self.counters.clone()
    }

    /// Receive the next item, or `None` once closed and drained.
    ///
    /// Returning an item releases one depth slot and, for tracked APDUs, one
    /// source-key slot before the consumer can retain or process it. `None`
    /// changes no counts. The key and limits follow the
    /// [receive-queue contract](crate::layer#receive-queue-admission).
    /// Cancellation safe: dropping a pending receive does not remove an item
    /// or alter its accounting. No counter lock is held while waiting.
    pub async fn recv(&mut self) -> Option<T> {
        std::future::poll_fn(|cx| {
            let mut state = self
                .counters
                .0
                .lock()
                .expect("queue admission counters poisoned");
            let result = self.rx.poll_recv(cx);
            if let Poll::Ready(Some(item)) = &result {
                state.dequeued(self.source_key.map(|source_key| source_key(item)));
            }
            result
        })
        .await
    }

    /// Receive immediately, releasing depth and source-key slots as in [`Self::recv`].
    ///
    /// A closed queue can still return queued items. Returns `Empty` when no item
    /// is currently available but admission remains open, or `Disconnected` once
    /// closed and drained. Errors do not change counts. See the
    /// [receive-queue contract](crate::layer#receive-queue-admission).
    pub fn try_recv(&mut self) -> Result<T, mpsc::error::TryRecvError> {
        let mut state = self
            .counters
            .0
            .lock()
            .expect("queue admission counters poisoned");
        let result = self.rx.try_recv();
        if let Ok(item) = &result {
            state.dequeued(self.source_key.map(|source_key| source_key(item)));
        }
        result
    }

    /// Stop admission while retaining already queued items for draining.
    ///
    /// Depth and source-key slots remain until [`Self::recv`] or [`Self::try_recv`]
    /// dequeues an item, or the receiver is dropped. Close itself does not count
    /// as a drop or reset high-water/drop totals. Later attempts count as Closed,
    /// taking precedence over fairness and Full.
    ///
    /// In NetworkLayer, the next APDU admission attempt ends dispatch, or the next
    /// control admission attempt disables only that stream. Routers count every
    /// closed local admission attempt and keep forwarding/handling controls.
    /// These match closing the corresponding raw receiver. See the
    /// [receive-queue contract](crate::layer#close-drop-stop-and-observation).
    pub fn close(&mut self) {
        let _snapshot = self
            .counters
            .0
            .lock()
            .expect("queue admission counters poisoned");
        self.rx.close();
    }
}

impl AdmissionReceiver<ReceivedApdu> {
    pub(crate) fn from_apdu_parts(
        rx: mpsc::Receiver<ReceivedApdu>,
        counters: QueueAdmissionCounters,
    ) -> Self {
        Self {
            rx,
            counters,
            source_key: Some(apdu_source),
        }
    }
}

impl<T> Drop for AdmissionReceiver<T> {
    fn drop(&mut self) {
        let mut state = self
            .counters
            .0
            .lock()
            .expect("queue admission counters poisoned");
        self.rx.close();
        state.snapshot.current_depth = 0;
        state.queued_by_source = HashMap::new();
        // The receiver's queued items (including reply senders) drop after
        // this guard is released. Closing first prevents concurrent admission.
    }
}

/// Shared non-waiting admission for raw and tracked 256-item queues.
///
/// Both modes count Full/Closed internally; only tracked receivers expose
/// snapshots and release depth/source slots on dequeue. Tracked APDU sends use
/// the 16-per-key quota and Closed > fairness > Full precedence. Generic/control
/// sends and raw APDUs have no quota. See the public
/// [receive-queue contract](crate::layer#receive-queue-admission).
pub(crate) struct AdmissionSender<T> {
    tx: mpsc::Sender<T>,
    counters: QueueAdmissionCounters,
    // Legacy receivers dequeue outside this module, so only opt-in receivers
    // expose depth/high-water accounting. Admission drops are counted in both.
    track_depth: bool,
}

impl<T> Clone for AdmissionSender<T> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            counters: self.counters.clone(),
            track_depth: self.track_depth,
        }
    }
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

    pub(crate) fn try_send(&self, item: T) -> Result<(), mpsc::error::TrySendError<T>> {
        self.try_send_from(item, None)
    }

    fn try_send_from(
        &self,
        item: T,
        source: Option<AdmissionSource>,
    ) -> Result<(), mpsc::error::TrySendError<T>> {
        if !self.track_depth {
            // A raw receiver dequeues without accounting and has no quota, so
            // only a refusal is counted, and only a refusal takes the lock: an
            // admitted item costs the channel alone (#1713).
            let result = self.tx.try_send(item);
            if let Err(error) = &result {
                let mut state = self
                    .counters
                    .0
                    .lock()
                    .expect("queue admission counters poisoned");
                let drops = match error {
                    mpsc::error::TrySendError::Full(_) => &mut state.snapshot.full_drops,
                    mpsc::error::TrySendError::Closed(_) => &mut state.snapshot.closed_drops,
                };
                *drops = drops.saturating_add(1);
            }
            return result;
        }
        // Serialize the admission and dequeue accounting, not asynchronous
        // waiting. Each queue has its own short critical section, and no lock
        // survives a poll/try operation. Closed keeps its lifecycle semantics;
        // otherwise a source over quota takes precedence over global Full.
        let mut state = self
            .counters
            .0
            .lock()
            .expect("queue admission counters poisoned");
        if !self.tx.is_closed()
            && source.as_ref().is_some_and(|source| {
                state.queued_by_source.get(source).copied().unwrap_or(0) >= APDU_SOURCE_CAPACITY
            })
        {
            state.snapshot.fairness_drops = state.snapshot.fairness_drops.saturating_add(1);
            // Reuse the caller's drop-arriving path, but not its Full counter.
            return Err(mpsc::error::TrySendError::Full(item));
        }
        let result = self.tx.try_send(item);
        match &result {
            Ok(()) if self.track_depth => {
                state.snapshot.current_depth += 1;
                state.snapshot.high_water =
                    state.snapshot.high_water.max(state.snapshot.current_depth);
                if let Some(source) = source {
                    *state.queued_by_source.entry(source).or_default() += 1;
                    state.assert_source_depth();
                }
            }
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                state.snapshot.full_drops = state.snapshot.full_drops.saturating_add(1);
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                state.snapshot.closed_drops = state.snapshot.closed_drops.saturating_add(1);
            }
        }
        result
    }
}

impl AdmissionSender<ReceivedApdu> {
    pub(crate) fn try_send_apdu(
        &self,
        apdu: ReceivedApdu,
    ) -> Result<(), mpsc::error::TrySendError<()>> {
        // Raw receivers cannot release source counts: legacy ingress neither
        // clones keys nor enforces a quota.
        let source = self.track_depth.then(|| apdu_source(&apdu));
        // Both dispatchers drop arriving APDUs on failure. Release the owned
        // item after the accounting lock is released; only Closed affects the
        // NetworkLayer dispatch lifecycle, not the returned payload.
        self.try_send_from(apdu, source)
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => mpsc::error::TrySendError::Full(()),
                mpsc::error::TrySendError::Closed(_) => mpsc::error::TrySendError::Closed(()),
            })
    }
}

impl<T: TransportPort + 'static> NetworkLayer<T> {
    /// Enable the one-consumer decoded network-control stream with a raw receiver.
    ///
    /// Call before either [`Self::start`] or [`Self::start_with_admission`], at
    /// most once across the two control-enable alternatives. Returns an encoding
    /// error if the layer has started or a control receiver is already enabled.
    /// With [`Self::start`], controls arrive only while its [`ApduReceiver`] is
    /// polled; with [`Self::start_with_admission`], whenever they arrive.
    /// The separate 256-item control queue has no per-source quota: Closed takes
    /// precedence over Full, with Full/Closed counted internally but no snapshot
    /// or depth/high-water tracking. Failed admission drops the control and its
    /// metadata without waiting for a consumer or generating reply bytes/wire
    /// rejections; APDU admission remains independent.
    ///
    /// Closing retains queued controls; dropping discards them. The first later
    /// control admission attempt disables only this stream, then controls resume
    /// discard behavior. Use [`Self::enable_network_control_receiver_with_admission`]
    /// for counters, and see the [receive-queue contract](crate::layer#receive-queue-admission)
    /// for the raw/tracked matrix, sequencing and stop/drain semantics.
    pub fn enable_network_control_receiver(
        &mut self,
    ) -> Result<mpsc::Receiver<ReceivedNetworkControl>, Error> {
        self.enable_network_control(false).map(|(rx, _)| rx)
    }

    /// Enable the control stream with queue-admission snapshots.
    ///
    /// This has the same before-start, one-consumer contract and errors as
    /// [`Self::enable_network_control_receiver`]; the two methods are alternatives.
    /// The separate 256-item queue tracks depth/high-water and Full/Closed drops
    /// through [`AdmissionReceiver::counters`]. It has no per-source quota, so
    /// fairness drops are always zero and precedence is Closed > Full.
    /// Drop-arriving, ownership release, no-reply/no-wire-rejection and
    /// close/drop/stop behavior follow the shared
    /// [receive-queue contract](crate::layer#receive-queue-admission).
    ///
    /// Controls discarded without opting into either stream are not admission
    /// drops. [`Self::network_control_ingress_sequence`] advances before every
    /// attempted control admission, including Full and the first Closed drop,
    /// but not for later controls once that stream is disabled.
    pub fn enable_network_control_receiver_with_admission(
        &mut self,
    ) -> Result<AdmissionReceiver<ReceivedNetworkControl>, Error> {
        let (rx, counters) = self.enable_network_control(true)?;
        Ok(AdmissionReceiver::from_parts(rx, counters))
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
        if self.started {
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

    /// Start the network layer. Returns the receiver for incoming APDUs.
    ///
    /// This starts the underlying transport. The [`ApduReceiver`] decodes the
    /// transport's NPDUs in the task that polls it, with no dispatch task or
    /// second queue between them (#1713): the transport's queue is the only
    /// buffer and bounds memory under overload, and the transport counts what
    /// it drops. Network messages reach an opted-in control receiver only
    /// while the APDU receiver is polled. Receive is cancel safe.
    ///
    /// Close retains the transport's queued NPDUs for draining; drop discards
    /// them. [`Self::stop`] after start leaves them drainable too. Use
    /// [`Self::start_with_admission`] for a tracked queue with snapshots and
    /// per-source fairness, whose controls flow independently; see the
    /// [receive-queue contract](crate::layer#receive-queue-admission).
    pub async fn start(&mut self) -> Result<ApduReceiver, Error> {
        if self.started {
            return Err(Error::Encoding("NetworkLayer is already started".into()));
        }
        let npdu_rx = self.transport.start().await?;
        self.started = true;
        // Taken once the transport knows its own addresses.
        self.local_broadcasts = self.transport.local_broadcast_destinations();
        let npdu_rx = Arc::new(Mutex::new(npdu_rx));
        self.receiver_queue = Arc::downgrade(&npdu_rx);
        Ok(ApduReceiver::new(npdu_rx, self.ingress()))
    }

    /// The receive state for this layer's start: its totals and controls.
    fn ingress(&mut self) -> Ingress {
        Ingress::new(
            Arc::clone(&self.node_totals),
            self.network_control_tx.take(),
            Arc::clone(&self.network_control_ingress_sequence),
        )
    }

    /// Start with an APDU receiver that also exposes admission snapshots.
    ///
    /// This is an alternative to [`Self::start`]. It runs a dispatch task that
    /// decodes into its own queue, so controls flow whether or not this
    /// receiver is polled. [`Self::stop`] closes admission but leaves
    /// queued items available to drain. Dropping the receiver discards those
    /// items and releases their reply channels without sending reply bytes.
    ///
    /// Unlike the raw receiver, this tracks depth/high-water and drop totals via
    /// [`AdmissionReceiver::counters`] and caps each complete source MAC byte
    /// value at 16 queued, unconsumed APDUs in the 256-item queue. The single
    /// transport uses no port key and ignores routed NPDU SNET/SADR for quotas.
    /// Dequeuing releases one source slot even if the consumer retains the APDU.
    /// **Closed > fairness > Full** selects one drop reason, releasing the
    /// arriving payload, metadata and reply sender without sending reply bytes
    /// or a wire rejection. Control queues have separate capacity and no quota.
    /// For the router's port-scoped key, raw/tracked matrix, and full
    /// close/drop/stop and counter-lifetime rules, see the
    /// [receive-queue contract](crate::layer#receive-queue-admission).
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
        let (rx, counters) = self.start_dispatch().await?;
        Ok(AdmissionReceiver::from_apdu_parts(rx, counters))
    }

    async fn start_dispatch(
        &mut self,
    ) -> Result<(mpsc::Receiver<ReceivedApdu>, QueueAdmissionCounters), Error> {
        if self.started {
            return Err(Error::Encoding("NetworkLayer is already started".into()));
        }
        let mut npdu_rx = self.transport.start().await?;
        self.started = true;
        // Taken once the transport knows its own addresses.
        self.local_broadcasts = self.transport.local_broadcast_destinations();
        let mut ingress = self.ingress();
        let (apdu_tx, apdu_rx) = AdmissionSender::channel(true);
        let counters = apdu_tx.counters.clone();

        let dispatch_task = tokio::spawn(async move {
            while let Some(received) = npdu_rx.recv().await {
                let Some(apdu) = ingress.take(received) else {
                    continue;
                };
                match apdu_tx.try_send_apdu(apdu) {
                    Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => {}
                    Err(mpsc::error::TrySendError::Closed(_)) => break,
                }
            }
        });

        self.dispatch_task = Some(dispatch_task);
        Ok((apdu_rx, counters))
    }
}

#[cfg(test)]
#[path = "layer_fairness_tests.rs"]
mod fairness_tests;
