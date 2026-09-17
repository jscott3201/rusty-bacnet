//! Bound SC NPDU ingress admission (RB-11, Refs #518).
//!
//! Per-origin + aggregate admission at the SC NPDU hand-off, mirroring the
//! network-layer receive-queue contract
//! (`bacnet-network/src/layer_admission.rs`): **Closed > fairness > Full**
//! precedence, insert-only-on-success per-source accounting, and saturating
//! count-only drop counters. The hub broadcast budgets (`sc_hub`) are
//! untouched: this bounds NPDU queue slots, not broadcast relay, and no
//! equivalent bucket is layered twice.
//!
//! Fairness key: `(ingress path, source bytes)` — the originating VMAC on the
//! hub path, the peer VMAC on the direct path (listener and hub merge). The
//! VMAC is a **scheduling key, never identity**: it carries no authentication
//! semantics and must not be treated as a principal (see
//! [`TransportProvenance`](crate::port::TransportProvenance) for the actual
//! relay/peer assertions). Only `Encapsulated-NPDU` admissions are charged:
//! unicast- and broadcast-shaped NPDUs share one per-origin quota, while
//! heartbeat, disconnect, advertisement, address-resolution, control,
//! proprietary and unknown frames never reach this gate (they are consumed or
//! NAKed earlier) and never touch the quota or the counters. Fairness drops
//! never consult or consume the rejection NAK budget.
//!
//! Numbers: the network layer allows 16 queued APDUs per source in a
//! 256-item queue (a 1:16 ratio). Scaled to the 64-item SC NPDU queue this
//! gives **4 per origin in 64 aggregate**. Tradeoff, measured in
//! `rb11_npdu_fairness_tests::bursty_source_capped_while_legitimate_origins_progress`:
//! one bursty origin is capped at 4/64 slots (6%) while fifteen legitimate
//! origins still fill the remaining 60 and progress; a larger quota would let
//! one origin take a larger share of the small queue, a smaller one would
//! drop legitimate back-to-back NPDUs (e.g. a finite segmented burst) under a
//! merely slow consumer. Draining always releases quota, so a live consumer
//! never deadlocks a legitimate source.
//!
//! Lifecycle: the per-source map is bounded by construction — entries exist
//! only for queued items (`live_entries <= current_depth <= CAPACITY`, checked
//! by [`AdmissionState::debug_check`] mirroring `assert_source_depth`).
//! Insertion happens only on successful admission; dequeues are observed
//! lazily by reconciling against the channel depth before every admission, so
//! no receiver hook, timer, or background task is needed and no await is
//! added before the hand-off. Receiver drop/close observations clear the map
//! (the channel is gone, so every later attempt counts Closed); connection
//! teardown and the direct idle-timeout end the owning task as the outer
//! bound. Attacker VMAC churn cannot grow the map beyond queued items: fresh
//! keys are inserted only alongside a queued item, and churned keys for
//! drained items are reaped by reconciliation.
//!
//! Locking: one short synchronous critical section per admission (a
//! `std::sync::Mutex`, never held across an await or a blocking call), so the
//! connection-lock discipline is unchanged and a slow application still
//! cannot block heartbeat/disconnect parsing — delivery stays
//! `try_send`/drop-arriving. Concurrent direct connections serialize on the
//! same section; the post-send reconcile keeps the map exact under races in
//! the conservative direction (a recycled slot may count one extra fairness
//! drop, never an unbounded entry).

use std::collections::{HashMap, VecDeque};
#[cfg(any(test, feature = "sc-tls"))]
use std::net::SocketAddr;
use std::sync::Mutex as StdMutex;

use bytes::Bytes;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use bacnet_types::error::Error;
use bacnet_types::MacAddr;

use super::{data_attributes, ScTransport, WebSocketPort};
use crate::port::{ReceivedNpdu, TransportProvenance};
use crate::sc_frame::{ScMessage, Vmac, BROADCAST_VMAC};

/// SC NPDU queue capacity this admission is scaled to.
///
/// Must match the hub (`ScTransport::start`) and direct-listener
/// (`DirectListener::start`) NPDU channel capacities; enforced by a
/// `debug_assert` on every admission.
pub(crate) const SC_NPDU_QUEUE_CAPACITY: usize = 64;

/// Default per-origin queued-NPDU quota: the network-layer 16/256 ratio
/// scaled to the 64-item SC queue.
pub(crate) const DEFAULT_NPDU_PER_ORIGIN_LIMIT: usize = 4;

/// Startup-validated SC NPDU admission limits (builder-style).
///
/// Only the per-origin quota is configurable; the aggregate cap is the fixed
/// 64-item NPDU channel. See the [module](self) docs for the ratio and the
/// measured tradeoff behind the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScNpduAdmissionPolicy {
    /// Maximum queued, unconsumed NPDUs per `(ingress path, source)` key.
    pub per_origin_limit: usize,
}

impl Default for ScNpduAdmissionPolicy {
    fn default() -> Self {
        Self {
            per_origin_limit: DEFAULT_NPDU_PER_ORIGIN_LIMIT,
        }
    }
}

impl ScNpduAdmissionPolicy {
    /// Check the limits before any transport I/O or state change.
    ///
    /// A zero quota would deadlock all NPDU delivery and a quota above the
    /// queue capacity could never bind before the aggregate cap, so both are
    /// rejected rather than coerced.
    pub fn validate(&self) -> Result<(), Error> {
        if self.per_origin_limit == 0 || self.per_origin_limit > SC_NPDU_QUEUE_CAPACITY {
            return Err(Error::OutOfRange(format!(
                "BACnet/SC NPDU per-origin limit must be in 1..={SC_NPDU_QUEUE_CAPACITY}, got {}",
                self.per_origin_limit
            )));
        }
        Ok(())
    }
}

/// Saturating lifetime NPDU drop counts for one SC NPDU queue.
///
/// Count-only diagnostics mirroring [`ScHubBroadcastDropCounts`](crate::sc_hub)
/// and the network-layer `QueueAdmissionSnapshot`: each admission drop
/// increments exactly one field in **Closed > fairness > Full** order, values
/// saturate at `u64::MAX`, and no field is ever an input to an admission
/// decision. Readable after stop; clones observe the same queue.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ScNpduDropCounts {
    /// NPDUs dropped by the per-origin quota (evaluated before Full even when
    /// the queue has room). Queue-wide total, never per-source.
    pub fairness_drops: u64,
    /// NPDUs passing the quota but arriving at a full queue.
    pub full_drops: u64,
    /// NPDUs dropped because the receiver was closed or dropped; takes
    /// precedence over fairness and Full.
    pub closed_drops: u64,
}

/// Which NPDU hand-off admitted the item. Part of the fairness key so a
/// direct peer never consumes hub-path quota and vice versa.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ScIngressPath {
    /// Hub-relayed path (`ScTransport` hub hand-off).
    Hub,
    /// Direct path (listener hand-off and the hub-side direct merge).
    Direct,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AdmissionKey {
    path: ScIngressPath,
    source: MacAddr,
}

#[derive(Debug, Default)]
struct AdmissionState {
    /// Sources in admission order; length tracks the channel depth.
    queued: VecDeque<AdmissionKey>,
    /// Live per-source counts; entries exist only for queued items.
    counts: HashMap<AdmissionKey, usize>,
    drops: ScNpduDropCounts,
}

impl AdmissionState {
    fn count_for(&self, key: &AdmissionKey) -> usize {
        self.counts.get(key).copied().unwrap_or(0)
    }

    /// Reap keys for items the application already dequeued.
    ///
    /// The channel is FIFO and `queued` mirrors admission order, so drained
    /// items are always the oldest keys. Keeps `live_entries <= depth`.
    fn reconcile(&mut self, tx: &mpsc::Sender<ReceivedNpdu>) {
        let actual = tx.max_capacity().saturating_sub(tx.capacity());
        while self.queued.len() > actual {
            let Some(oldest) = self.queued.pop_front() else {
                break;
            };
            if let Some(count) = self.counts.get_mut(&oldest) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    self.counts.remove(&oldest);
                }
            }
        }
    }

    fn push(&mut self, key: AdmissionKey) {
        *self.counts.entry(key.clone()).or_default() += 1;
        self.queued.push_back(key);
    }

    fn note_closed(&mut self) {
        self.drops.closed_drops = self.drops.closed_drops.saturating_add(1);
        self.queued.clear();
        self.counts.clear();
    }

    fn debug_check(&self, per_origin_limit: usize) {
        debug_assert!(self.counts.len() <= self.queued.len());
        debug_assert!(self.queued.len() <= SC_NPDU_QUEUE_CAPACITY);
        debug_assert_eq!(self.counts.values().sum::<usize>(), self.queued.len());
        debug_assert!(self
            .counts
            .values()
            .all(|count| (1..=per_origin_limit).contains(count)));
    }
}

/// Outcome of one admission attempt. Rejected variants carry the arriving
/// item back so the caller drops it after the accounting lock is released.
#[derive(Debug)]
pub(crate) enum AdmissionOutcome {
    Admitted,
    FairnessDrop(ReceivedNpdu),
    FullDrop(ReceivedNpdu),
    ClosedDrop(ReceivedNpdu),
}

fn saturating_inc(counter: &mut u64) {
    *counter = counter.saturating_add(1);
}

/// Shared per-queue SC NPDU admission.
///
/// One instance guards one NPDU channel: the hub transport shares a single
/// instance between its hub hand-off and its direct merge (same channel),
/// while each direct listener owns one shared across its accepted
/// connections. All methods are synchronous; no await is added before the
/// hand-off.
#[derive(Debug)]
pub(crate) struct ScNpduAdmission {
    policy: ScNpduAdmissionPolicy,
    state: StdMutex<AdmissionState>,
}

impl ScNpduAdmission {
    pub(crate) fn new(policy: ScNpduAdmissionPolicy) -> Self {
        Self {
            policy,
            state: StdMutex::new(AdmissionState::default()),
        }
    }

    pub(crate) fn drop_counts(&self) -> ScNpduDropCounts {
        self.state
            .lock()
            .expect("SC NPDU admission state poisoned")
            .drops
    }

    /// Core gate: Closed > fairness > Full. Holds only the short accounting
    /// lock across the synchronous `try_send`; never across an await.
    pub(crate) fn admit(
        &self,
        tx: &mpsc::Sender<ReceivedNpdu>,
        path: ScIngressPath,
        source: &MacAddr,
        item: ReceivedNpdu,
    ) -> AdmissionOutcome {
        let mut state = self.state.lock().expect("SC NPDU admission state poisoned");
        if tx.is_closed() {
            state.note_closed();
            return AdmissionOutcome::ClosedDrop(item);
        }
        debug_assert_eq!(tx.max_capacity(), SC_NPDU_QUEUE_CAPACITY);
        state.reconcile(tx);
        state.debug_check(self.policy.per_origin_limit);
        let key = AdmissionKey {
            path,
            source: source.clone(),
        };
        if state.count_for(&key) >= self.policy.per_origin_limit.max(1) {
            saturating_inc(&mut state.drops.fairness_drops);
            return AdmissionOutcome::FairnessDrop(item);
        }
        match tx.try_send(item) {
            Ok(()) => {
                state.push(key);
                // A concurrent dequeue may have landed after the pre-check
                // read; trim oldest-first so the map never exceeds depth.
                state.reconcile(tx);
                state.debug_check(self.policy.per_origin_limit);
                AdmissionOutcome::Admitted
            }
            Err(mpsc::error::TrySendError::Full(item)) => {
                saturating_inc(&mut state.drops.full_drops);
                AdmissionOutcome::FullDrop(item)
            }
            Err(mpsc::error::TrySendError::Closed(item)) => {
                state.note_closed();
                AdmissionOutcome::ClosedDrop(item)
            }
        }
    }

    /// Hub-relayed hand-off: build the verified-relayed envelope and admit
    /// under the hub-path key. `reply_tx` stays `None`, so drops release no
    /// reply hazard and send no wire rejection.
    pub(crate) fn admit_hub_relayed(
        &self,
        tx: &mpsc::Sender<ReceivedNpdu>,
        msg: &ScMessage,
        npdu: Bytes,
        source: Vmac,
    ) {
        let source_mac = MacAddr::from_slice(&source);
        let item = ReceivedNpdu {
            npdu,
            source_mac: source_mac.clone(),
            link_layer_group: msg.destination_vmac == Some(BROADCAST_VMAC),
            data_attributes: data_attributes::from_data_options(msg),
            provenance: TransportProvenance::verified_relayed_origin(),
            reply_tx: None,
        };
        match self.admit(tx, ScIngressPath::Hub, &source_mac, item) {
            AdmissionOutcome::Admitted => {}
            AdmissionOutcome::FairnessDrop(dropped) => {
                debug!("SC transport: per-origin NPDU quota exceeded, dropping incoming message");
                drop(dropped);
            }
            AdmissionOutcome::FullDrop(dropped) => {
                warn!("SC transport: NPDU channel full, dropping incoming message");
                drop(dropped);
            }
            AdmissionOutcome::ClosedDrop(dropped) => {
                debug!("SC transport: NPDU receiver closed, dropping incoming message");
                drop(dropped);
            }
        }
    }

    /// Hub-side direct merge: the listener already admitted this item into
    /// its own queue; re-admit into the hub queue under the direct-path key
    /// so bursty direct peers cannot starve hub-path origins (or vice versa).
    pub(crate) fn admit_merged_direct(&self, tx: &mpsc::Sender<ReceivedNpdu>, item: ReceivedNpdu) {
        let source = item.source_mac.clone();
        match self.admit(tx, ScIngressPath::Direct, &source, item) {
            AdmissionOutcome::Admitted => {}
            AdmissionOutcome::FairnessDrop(dropped) => {
                debug!("SC transport: per-origin NPDU quota exceeded, dropping direct message");
                drop(dropped);
            }
            AdmissionOutcome::FullDrop(dropped) => {
                warn!("SC transport: NPDU channel full, dropping direct message");
                drop(dropped);
            }
            AdmissionOutcome::ClosedDrop(dropped) => {
                debug!("SC transport: NPDU receiver closed, dropping direct message");
                drop(dropped);
            }
        }
    }

    /// Listener hand-off: build the verified-direct envelope and admit under
    /// the direct-path key. Never touches the rejection NAK budget.
    #[cfg(any(test, feature = "sc-tls"))]
    pub(crate) fn admit_direct_peer(
        &self,
        tx: &mpsc::Sender<ReceivedNpdu>,
        msg: &ScMessage,
        npdu: Bytes,
        peer: Vmac,
        peer_addr: SocketAddr,
    ) {
        let source_mac = MacAddr::from_slice(&peer);
        let item = ReceivedNpdu {
            npdu,
            source_mac: source_mac.clone(),
            link_layer_group: false,
            data_attributes: data_attributes::from_data_options(msg),
            provenance: TransportProvenance::verified_direct_peer(),
            reply_tx: None,
        };
        match self.admit(tx, ScIngressPath::Direct, &source_mac, item) {
            AdmissionOutcome::Admitted => {}
            AdmissionOutcome::FairnessDrop(dropped) => {
                debug!("direct per-origin NPDU quota exceeded, dropping from {peer_addr}");
                drop(dropped);
            }
            AdmissionOutcome::FullDrop(dropped) => {
                warn!("direct NPDU channel full, dropping from {peer_addr}");
                drop(dropped);
            }
            AdmissionOutcome::ClosedDrop(dropped) => {
                debug!("direct NPDU receiver closed, dropping from {peer_addr}");
                drop(dropped);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn test_live_entries(&self) -> (usize, usize) {
        let state = self.state.lock().expect("SC NPDU admission state poisoned");
        (state.counts.len(), state.queued.len())
    }

    #[cfg(test)]
    pub(crate) fn test_set_fairness_drops(&self, value: u64) {
        self.state
            .lock()
            .expect("SC NPDU admission state poisoned")
            .drops
            .fairness_drops = value;
    }
}

impl<W: WebSocketPort> ScTransport<W> {
    /// Set the per-origin queued-NPDU quota (builder-style).
    ///
    /// Defaults to [`DEFAULT_NPDU_PER_ORIGIN_LIMIT`]: the network-layer
    /// 16/256 ratio scaled to the 64-item SC queue. [`TransportPort::start`]
    /// validates the limit before any I/O or state change and rejects zero
    /// or above-capacity values; the aggregate cap stays the fixed channel
    /// capacity. Applies jointly to hub-relayed and merged-direct NPDUs,
    /// keyed by ingress path so neither starves the other.
    pub fn with_npdu_per_origin_limit(mut self, limit: usize) -> Self {
        self.npdu_admission_policy = ScNpduAdmissionPolicy {
            per_origin_limit: limit,
        };
        self
    }

    /// Read this transport's NPDU drop counts, including after
    /// [`TransportPort::stop`].
    ///
    /// Count-only and saturating: per-origin fairness drops, aggregate-full
    /// drops, and closed-receiver drops in Closed > fairness > Full order.
    /// No per-VMAC statistics or wire response is created. A never-started
    /// transport reports zero.
    pub fn npdu_drop_counts(&self) -> ScNpduDropCounts {
        self.npdu_admission
            .as_ref()
            .map(|admission| admission.drop_counts())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sc_frame::ScFunction;

    fn item(source: &MacAddr) -> ReceivedNpdu {
        ReceivedNpdu {
            npdu: Bytes::from_static(&[0x01, 0x00, 0x30]),
            source_mac: source.clone(),
            link_layer_group: false,
            data_attributes: Vec::new(),
            provenance: TransportProvenance::unverified(),
            reply_tx: None,
        }
    }

    fn hub_source(id: u8) -> (ScIngressPath, MacAddr) {
        (ScIngressPath::Hub, MacAddr::from_slice(&[id; 6]))
    }

    fn admitted(
        admission: &ScNpduAdmission,
        tx: &mpsc::Sender<ReceivedNpdu>,
        path: ScIngressPath,
        source: &MacAddr,
    ) {
        match admission.admit(tx, path, source, item(source)) {
            AdmissionOutcome::Admitted => {}
            outcome => panic!("expected admission, got {outcome:?}"),
        }
    }

    fn fairness_dropped(
        admission: &ScNpduAdmission,
        tx: &mpsc::Sender<ReceivedNpdu>,
        path: ScIngressPath,
        source: &MacAddr,
    ) {
        match admission.admit(tx, path, source, item(source)) {
            AdmissionOutcome::FairnessDrop(_) => {}
            outcome => panic!("expected fairness drop, got {outcome:?}"),
        }
    }

    fn full_dropped(
        admission: &ScNpduAdmission,
        tx: &mpsc::Sender<ReceivedNpdu>,
        path: ScIngressPath,
        source: &MacAddr,
    ) {
        match admission.admit(tx, path, source, item(source)) {
            AdmissionOutcome::FullDrop(_) => {}
            outcome => panic!("expected full drop, got {outcome:?}"),
        }
    }

    fn encapsulated(dest: Option<Vmac>) -> ScMessage {
        ScMessage {
            function: ScFunction::EncapsulatedNpdu,
            message_id: 0x1234,
            originating_vmac: None,
            destination_vmac: dest,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from_static(&[0x01, 0x00, 0x30]),
        }
    }

    #[test]
    fn policy_defaults_to_scaled_ratio_and_validates_bounds() {
        assert_eq!(
            ScNpduAdmissionPolicy::default().per_origin_limit,
            DEFAULT_NPDU_PER_ORIGIN_LIMIT
        );
        assert_eq!(DEFAULT_NPDU_PER_ORIGIN_LIMIT, 4);
        ScNpduAdmissionPolicy::default().validate().unwrap();
        for limit in [1, 4, 63, SC_NPDU_QUEUE_CAPACITY] {
            ScNpduAdmissionPolicy {
                per_origin_limit: limit,
            }
            .validate()
            .unwrap();
        }
        for limit in [0, SC_NPDU_QUEUE_CAPACITY + 1, usize::MAX] {
            let err = ScNpduAdmissionPolicy {
                per_origin_limit: limit,
            }
            .validate()
            .unwrap_err();
            assert!(err.to_string().contains("per-origin limit"), "{err}");
        }
    }

    #[test]
    fn single_source_flood_caps_at_quota_and_releases_on_drain() {
        let (tx, mut rx) = mpsc::channel(SC_NPDU_QUEUE_CAPACITY);
        let admission = ScNpduAdmission::new(ScNpduAdmissionPolicy::default());
        let (path, source) = hub_source(2);
        for _ in 0..DEFAULT_NPDU_PER_ORIGIN_LIMIT {
            admitted(&admission, &tx, path, &source);
        }
        assert_eq!(
            admission.test_live_entries(),
            (1, DEFAULT_NPDU_PER_ORIGIN_LIMIT)
        );
        fairness_dropped(&admission, &tx, path, &source);
        assert_eq!(admission.drop_counts().fairness_drops, 1);
        assert_eq!(admission.drop_counts().full_drops, 0);
        // Draining releases quota: the previously capped source progresses.
        rx.try_recv().unwrap();
        rx.try_recv().unwrap();
        admitted(&admission, &tx, path, &source);
        admitted(&admission, &tx, path, &source);
        fairness_dropped(&admission, &tx, path, &source);
        assert_eq!(admission.test_live_entries(), (1, 4));
        assert_eq!(admission.drop_counts().fairness_drops, 2);
    }

    #[test]
    fn sixteen_origins_compose_to_capacity_and_fairness_precedes_full() {
        let (tx, _rx) = mpsc::channel(SC_NPDU_QUEUE_CAPACITY);
        let admission = ScNpduAdmission::new(ScNpduAdmissionPolicy::default());
        for id in 0..16u8 {
            let (path, source) = hub_source(id);
            for _ in 0..DEFAULT_NPDU_PER_ORIGIN_LIMIT {
                admitted(&admission, &tx, path, &source);
            }
        }
        assert_eq!(admission.test_live_entries(), (16, 64));
        // Below quota but globally full: aggregate drop.
        let fresh = MacAddr::from_slice(&[0xFF; 6]);
        full_dropped(&admission, &tx, ScIngressPath::Hub, &fresh);
        // Over quota AND globally full: fairness takes precedence.
        let (path, capped) = hub_source(0);
        fairness_dropped(&admission, &tx, path, &capped);
        assert_eq!(
            admission.drop_counts(),
            ScNpduDropCounts {
                fairness_drops: 1,
                full_drops: 1,
                closed_drops: 0,
            }
        );
    }

    #[test]
    fn unicast_and_broadcast_share_one_quota_while_paths_stay_separate() {
        let (tx, rx) = mpsc::channel(SC_NPDU_QUEUE_CAPACITY);
        let admission = ScNpduAdmission::new(ScNpduAdmissionPolicy::default());
        let source: Vmac = [0x22; 6];
        let npdu = Bytes::from_static(&[0x01, 0x00, 0x30]);
        // Two unicast-shaped plus two broadcast-shaped hub NPDUs share the
        // single hub-path quota of 4; neither shape bypasses as "broadcast".
        for dest in [None, None, Some(BROADCAST_VMAC), Some(BROADCAST_VMAC)] {
            admission.admit_hub_relayed(&tx, &encapsulated(dest), npdu.clone(), source);
        }
        assert_eq!(rx.len(), 4);
        admission.admit_hub_relayed(&tx, &encapsulated(None), npdu.clone(), source);
        admission.admit_hub_relayed(
            &tx,
            &encapsulated(Some(BROADCAST_VMAC)),
            npdu.clone(),
            source,
        );
        assert_eq!(rx.len(), 4);
        assert_eq!(admission.drop_counts().fairness_drops, 2);
        assert_eq!(admission.drop_counts().full_drops, 0);
        // Identical bytes on the direct path are a separate key and progress.
        let peer_addr: SocketAddr = "127.0.0.1:1".parse().unwrap();
        admission.admit_direct_peer(&tx, &encapsulated(None), npdu.clone(), source, peer_addr);
        admission.admit_direct_peer(&tx, &encapsulated(None), npdu.clone(), source, peer_addr);
        assert_eq!(rx.len(), 6);
        assert_eq!(admission.drop_counts().fairness_drops, 2);
        assert_eq!(admission.test_live_entries(), (2, 6));
    }

    #[test]
    fn churn_cannot_grow_map_beyond_queued_items() {
        let (tx, mut rx) = mpsc::channel(SC_NPDU_QUEUE_CAPACITY);
        let admission = ScNpduAdmission::new(ScNpduAdmissionPolicy::default());
        for round in 0..3u16 {
            for id in 0..512u16 {
                let mut bytes = [0u8; 6];
                bytes[0..2].copy_from_slice(&round.to_be_bytes());
                bytes[2..4].copy_from_slice(&id.to_be_bytes());
                let source = MacAddr::from_slice(&bytes);
                let _ = admission.admit(&tx, ScIngressPath::Hub, &source, item(&source));
            }
            // Every queued item has at most one key; overflow is Full, never
            // fairness (each churned key appears once).
            assert_eq!(admission.test_live_entries(), (64, 64));
            assert_eq!(admission.drop_counts().fairness_drops, 0);
            assert_eq!(
                admission.drop_counts().full_drops,
                u64::from(round + 1) * 448
            );
            for _ in 0..32 {
                rx.try_recv().unwrap();
            }
            // Half-drained: the next admission reaps before charging.
            let fresh = MacAddr::from_slice(&[0xEE; 6]);
            admitted(&admission, &tx, ScIngressPath::Hub, &fresh);
            let (entries, queued) = admission.test_live_entries();
            assert!(entries <= queued, "{entries} entries vs {queued} queued");
            assert!(queued <= SC_NPDU_QUEUE_CAPACITY);
            for _ in 0..33 {
                rx.try_recv().unwrap();
            }
            // Fully drained: the next admission reaps every churned key, so
            // only the fresh key remains live.
            let fresh_tail = MacAddr::from_slice(&[0xEF; 6]);
            admitted(&admission, &tx, ScIngressPath::Hub, &fresh_tail);
            assert_eq!(admission.test_live_entries(), (1, 1));
            rx.try_recv().unwrap();
        }
    }

    #[test]
    fn close_and_drop_clear_map_and_count_closed_first() {
        for drop_receiver in [false, true] {
            let (tx, mut rx) = mpsc::channel(SC_NPDU_QUEUE_CAPACITY);
            let admission = ScNpduAdmission::new(ScNpduAdmissionPolicy::default());
            let (path, source) = hub_source(2);
            for _ in 0..DEFAULT_NPDU_PER_ORIGIN_LIMIT {
                admitted(&admission, &tx, path, &source);
            }
            assert_eq!(admission.test_live_entries(), (1, 4));
            if drop_receiver {
                drop(rx);
            } else {
                rx.close();
            }
            // Closed takes precedence over fairness even for a capped source.
            match admission.admit(&tx, path, &source, item(&source)) {
                AdmissionOutcome::ClosedDrop(_) => {}
                outcome => panic!("expected closed drop, got {outcome:?}"),
            }
            assert_eq!(admission.test_live_entries(), (0, 0));
            assert_eq!(
                admission.drop_counts(),
                ScNpduDropCounts {
                    closed_drops: 1,
                    ..Default::default()
                }
            );
        }
    }

    #[test]
    fn counters_saturate_without_affecting_depth_or_other_drops() {
        let (tx, _rx) = mpsc::channel(SC_NPDU_QUEUE_CAPACITY);
        let admission = ScNpduAdmission::new(ScNpduAdmissionPolicy::default());
        let (path, source) = hub_source(2);
        for _ in 0..DEFAULT_NPDU_PER_ORIGIN_LIMIT {
            admitted(&admission, &tx, path, &source);
        }
        admission.test_set_fairness_drops(u64::MAX - 1);
        fairness_dropped(&admission, &tx, path, &source);
        fairness_dropped(&admission, &tx, path, &source);
        assert_eq!(admission.drop_counts().fairness_drops, u64::MAX);
        assert_eq!(admission.drop_counts().full_drops, 0);
        assert_eq!(admission.drop_counts().closed_drops, 0);
        assert_eq!(
            admission.test_live_entries(),
            (1, DEFAULT_NPDU_PER_ORIGIN_LIMIT)
        );
    }

    #[tokio::test]
    async fn cancelled_recv_changes_no_counts() {
        let (tx, mut rx) = mpsc::channel(SC_NPDU_QUEUE_CAPACITY);
        let admission = ScNpduAdmission::new(ScNpduAdmissionPolicy::default());
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), rx.recv())
                .await
                .is_err()
        );
        assert_eq!(admission.drop_counts(), ScNpduDropCounts::default());
        assert_eq!(admission.test_live_entries(), (0, 0));
        let (path, source) = hub_source(2);
        admitted(&admission, &tx, path, &source);
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
                .await
                .expect("admitted item must arrive")
                .expect("channel open")
                .source_mac,
            source
        );
        assert_eq!(admission.test_live_entries(), (1, 1));
        // The dequeue is observed on the next admission, not by the recv.
        let other = MacAddr::from_slice(&[0x03; 6]);
        admitted(&admission, &tx, ScIngressPath::Hub, &other);
        assert_eq!(admission.test_live_entries(), (1, 1));
        assert_eq!(admission.drop_counts(), ScNpduDropCounts::default());
    }
}
