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
//! [`TransportProvenance`] for the actual
//! relay/peer assertions). Only `Encapsulated-NPDU` admissions are charged:
//! unicast- and broadcast-shaped NPDUs share one per-origin quota, while
//! heartbeat, disconnect, advertisement, address-resolution, control,
//! proprietary and unknown frames never reach this gate (they are consumed or
//! NAKed earlier) and never touch the quota or the counters. Fairness drops
//! never consult or consume the rejection NAK budget.
//!
//! Numbers: **32 per origin in 256 aggregate** (#1712). The quota sits above
//! what a server admits from one peer at once (16 confirmed requests, 1 DCC
//! recovery request and 8 unconfirmed ones; see `docs/request-admission.md`),
//! so a single busy peer meets the server's prompt Abort rather than a drop
//! here, which its requests would only show as APDU timeouts. It also holds
//! the replies to that many requests from one client to one server. The
//! receive loop admits frames that arrive together without yielding, so a
//! burst from one peer lands before the network layer drains any of it: the
//! earlier 4-of-64 dropped the fifth NPDU of such a burst, and a client with
//! ten requests in flight to one server stalled. Tradeoff, measured in
//! `rb11_npdu_fairness_tests::bursty_source_capped_while_legitimate_origins_progress`:
//! one bursty origin is capped at 32/256 slots (12.5%) while seven
//! legitimate origins still fill the remaining 224 and progress; a larger
//! quota would let one origin take a larger share of the queue, a smaller
//! one drops a busy peer's legitimate concurrency. Draining always releases
//! quota, so a live consumer never deadlocks a legitimate source.
//!
//! Network-layer messages also pass a per-source rate limit first
//! ([`ScControlRateLimit`], `control_rate`, #518), so the precedence is
//! **Closed > control rate > fairness > Full**. APDUs are never charged.
//!
//! Drops: control-rate, fairness and Full drops share one [`WarnThrottle`] per admission
//! instance, so one transport or listener warns at most once per
//! [`WARNING_INTERVAL`](crate::warn_throttle::WARNING_INTERVAL), with the
//! number dropped since its last warning; the rest log at debug. Closed drops
//! only happen as the receiver goes away and stay at debug.
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
//! drop, never an unbounded entry). A drop then takes the warning throttle's
//! own short lock, after the accounting lock is released; the two never nest.

use std::collections::{HashMap, VecDeque};
use std::fmt;
#[cfg(any(test, feature = "sc-tls"))]
use std::net::SocketAddr;
use std::sync::Mutex as StdMutex;

use bytes::Bytes;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use bacnet_types::error::Error;
use bacnet_types::MacAddr;

use super::control_rate::{is_network_message, ControlRate, ControlSource, ScControlRateLimit};
use super::{data_attributes, ScTransport, WebSocketPort};
use crate::port::{ReceivedNpdu, TransportProvenance};
use crate::sc_frame::{ScMessage, Vmac, BROADCAST_VMAC};
use crate::warn_throttle::WarnThrottle;

/// SC NPDU queue capacity: the aggregate admission cap.
///
/// The hub transport (`ScTransport::start`) and the direct listener
/// (`DirectListener::start`) size their NPDU channels with it; a
/// `debug_assert` on every admission checks that. Like the B/IP channel, it
/// holds 256 NPDUs.
pub(crate) const SC_NPDU_QUEUE_CAPACITY: usize = 256;

/// Default per-origin queued-NPDU quota: above a server's whole per-peer
/// request budget (25 handlers), and one eighth of the queue (#1712).
pub(crate) const DEFAULT_NPDU_PER_ORIGIN_LIMIT: usize = 32;

/// Startup-validated SC NPDU admission limits (builder-style).
///
/// Only the per-origin quota is configurable; the aggregate cap is the fixed
/// 256-item NPDU channel. The default quota of 32 lets one busy peer keep as
/// many requests or replies queued as a server would admit from it, while
/// one origin holds at most 12.5% of the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ScNpduAdmissionPolicy {
    /// Maximum queued, unconsumed NPDUs per `(ingress path, source)` key.
    pub per_origin_limit: usize,
    /// Rate limit on network-layer messages per `(ingress path, source)` key.
    pub control_rate: ScControlRateLimit,
}

impl Default for ScNpduAdmissionPolicy {
    fn default() -> Self {
        Self {
            per_origin_limit: DEFAULT_NPDU_PER_ORIGIN_LIMIT,
            control_rate: ScControlRateLimit::default(),
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
        self.control_rate.validate()
    }
}

/// Saturating lifetime NPDU drop counts for one SC NPDU queue.
///
/// Count-only diagnostics mirroring the hub's `ScHubBroadcastDropCounts`
/// and the network-layer `QueueAdmissionSnapshot`: each admission drop
/// increments exactly one field in **Closed > control rate > fairness > Full** order, values
/// saturate at `u64::MAX`, and no field is ever an input to an admission
/// decision. Readable after stop; clones observe the same queue.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ScNpduDropCounts {
    /// Network-layer messages dropped by the per-source control-message rate
    /// limit, before taking a queue slot (#518). Queue-wide total.
    pub control_rate_drops: u64,
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

#[derive(Debug)]
struct AdmissionState {
    /// Sources in admission order; length tracks the channel depth.
    queued: VecDeque<AdmissionKey>,
    /// Live per-source counts; entries exist only for queued items.
    counts: HashMap<AdmissionKey, usize>,
    /// Network-layer message buckets, bounded on their own.
    control: ControlRate,
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
    ControlRateDrop(ReceivedNpdu),
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
    /// Throttles the warning for fairness and Full drops, shared by every
    /// hand-off into this queue.
    drop_warning: WarnThrottle,
}

/// Identity, address and reply channel of a direct-connect peer whose NPDU is
/// being admitted.
#[cfg(any(test, feature = "sc-tls"))]
pub(crate) struct DirectPeer {
    /// Peer VMAC (the fairness key on the direct path).
    pub(crate) vmac: Vmac,
    /// Peer socket address, for drop logging.
    pub(crate) addr: SocketAddr,
    /// Verified TLS identity of the peer.
    pub(crate) identity: crate::port::DirectScIdentity,
    /// Handle for replying on the direct connection, when available.
    pub(crate) response: Option<crate::port::DirectResponse>,
}

impl ScNpduAdmission {
    pub(crate) fn new(policy: ScNpduAdmissionPolicy) -> Self {
        Self {
            policy,
            state: StdMutex::new(AdmissionState {
                queued: VecDeque::new(),
                counts: HashMap::new(),
                control: ControlRate::new(policy.control_rate),
                drops: ScNpduDropCounts::default(),
            }),
            drop_warning: WarnThrottle::default(),
        }
    }

    pub(crate) fn drop_counts(&self) -> ScNpduDropCounts {
        self.state
            .lock()
            .expect("SC NPDU admission state poisoned")
            .drops
    }

    /// Core gate: Closed > control rate > fairness > Full. Holds only the
    /// short accounting lock across the synchronous `try_send`; never across
    /// an await. `charge_control` is false only for an item another queue's
    /// admission already charged.
    pub(crate) fn admit(
        &self,
        tx: &mpsc::Sender<ReceivedNpdu>,
        path: ScIngressPath,
        source: &MacAddr,
        item: ReceivedNpdu,
        charge_control: bool,
    ) -> AdmissionOutcome {
        let mut state = self.state.lock().expect("SC NPDU admission state poisoned");
        if tx.is_closed() {
            state.note_closed();
            return AdmissionOutcome::ClosedDrop(item);
        }
        debug_assert_eq!(tx.max_capacity(), SC_NPDU_QUEUE_CAPACITY);
        state.reconcile(tx);
        state.debug_check(self.policy.per_origin_limit);
        // A verified direct peer spends its certificate's budget, so a
        // VMAC change or reconnect doesn't reset it; anything else spends
        // its ingress path and VMAC's.
        let control_source = || match item.provenance.principal() {
            Some(principal) => ControlSource::Principal(principal),
            None => ControlSource::Vmac(path, source.clone()),
        };
        if charge_control
            && is_network_message(&item.npdu)
            && !state
                .control
                .admit(control_source(), tokio::time::Instant::now())
        {
            saturating_inc(&mut state.drops.control_rate_drops);
            return AdmissionOutcome::ControlRateDrop(item);
        }
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
            direct_response: None,
            npdu,
            source_mac: source_mac.clone(),
            link_layer_group: msg.destination_vmac == Some(BROADCAST_VMAC),
            data_attributes: data_attributes::from_data_options(msg),
            provenance: TransportProvenance::verified_relayed_origin(),
            reply_tx: None,
        };
        let outcome = self.admit(tx, ScIngressPath::Hub, &source_mac, item, true);
        self.settle(outcome, "hub", None);
    }

    /// Hub-side direct merge: the listener already admitted this item into
    /// its own queue; re-admit into the hub queue under the direct-path key
    /// so bursty direct peers cannot starve hub-path origins (or vice versa).
    /// The listener's admission already charged its control-message rate.
    pub(crate) fn admit_merged_direct(&self, tx: &mpsc::Sender<ReceivedNpdu>, item: ReceivedNpdu) {
        let source = item.source_mac.clone();
        let outcome = self.admit(tx, ScIngressPath::Direct, &source, item, false);
        self.settle(outcome, "merged direct", None);
    }

    /// Direct socket hand-off: build the verified-direct envelope and admit under
    /// the direct-path key. Never touches the rejection NAK budget.
    #[cfg(any(test, feature = "sc-tls"))]
    pub(crate) fn admit_direct_peer(
        &self,
        tx: &mpsc::Sender<ReceivedNpdu>,
        msg: &ScMessage,
        npdu: Bytes,
        peer: DirectPeer,
    ) {
        let DirectPeer {
            vmac: peer,
            addr: peer_addr,
            identity,
            response: direct_response,
        } = peer;
        let source_mac = MacAddr::from_slice(&peer);
        let item = ReceivedNpdu {
            npdu,
            source_mac: source_mac.clone(),
            link_layer_group: false,
            data_attributes: data_attributes::from_data_options(msg),
            provenance: TransportProvenance::verified_direct_peer(identity),
            direct_response,
            reply_tx: None,
        };
        let outcome = self.admit(tx, ScIngressPath::Direct, &source_mac, item, true);
        self.settle(outcome, "direct peer", Some(&peer_addr));
    }

    /// Log an admission outcome and drop a rejected item, after the
    /// accounting lock is released. `handoff` names the hand-off and `peer`
    /// a direct peer's socket address.
    ///
    /// Fairness and Full drops warn at most once per
    /// [`WARNING_INTERVAL`](crate::warn_throttle::WARNING_INTERVAL) for this
    /// queue, with the number dropped since the last warning, so a busy peer
    /// over its quota shows in the log without flooding it; the rest log at
    /// debug. The counters count every drop either way. Closed drops log at
    /// debug: they only happen as the receiver goes away.
    fn settle(
        &self,
        outcome: AdmissionOutcome,
        handoff: &'static str,
        peer: Option<&dyn fmt::Display>,
    ) {
        let peer = peer.map(tracing::field::display);
        let (item, reason) = match outcome {
            AdmissionOutcome::Admitted => return,
            AdmissionOutcome::ControlRateDrop(item) => (item, "control-message rate"),
            AdmissionOutcome::FairnessDrop(item) => (item, "per-origin quota"),
            AdmissionOutcome::FullDrop(item) => (item, "queue full"),
            AdmissionOutcome::ClosedDrop(item) => {
                debug!(
                    handoff,
                    peer, "SC transport: NPDU receiver closed, dropping an incoming NPDU"
                );
                drop(item);
                return;
            }
        };
        drop(item);
        match self.drop_warning.note() {
            Some(dropped) => warn!(
                dropped,
                reason,
                handoff,
                peer,
                "SC transport: NPDU admission dropped incoming NPDUs since the last warning"
            ),
            None => debug!(
                reason,
                handoff, peer, "SC transport: NPDU admission dropped an incoming NPDU"
            ),
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
    /// Defaults to 32 of the 256-item SC queue, enough for one busy peer's
    /// requests or replies (#1712); 4 restores the stricter guard used before.
    /// [`TransportPort::start`](crate::port::TransportPort::start)
    /// validates the limit before any I/O or state change and rejects zero
    /// or above-capacity values; the aggregate cap stays the fixed channel
    /// capacity. Applies jointly to hub-relayed and merged-direct NPDUs,
    /// keyed by ingress path so neither starves the other.
    pub fn with_npdu_per_origin_limit(mut self, limit: usize) -> Self {
        self.npdu_admission_policy.per_origin_limit = limit;
        self
    }

    /// Set the per-source limit on inbound network-layer messages
    /// (builder-style; default [`ScControlRateLimit::default`], #518).
    ///
    /// Each originating VMAC on the hub path, and each direct peer's verified
    /// certificate whatever VMAC it uses, gets its own bucket; a message over budget is dropped silently and counted in
    /// [`ScNpduDropCounts::control_rate_drops`]. APDUs are never charged.
    /// [`TransportPort::start`](crate::port::TransportPort::start) validates
    /// it before any I/O.
    pub fn with_control_rate_limit(mut self, limit: ScControlRateLimit) -> Self {
        self.npdu_admission_policy.control_rate = limit;
        self
    }

    /// Read this transport's NPDU drop counts, including after
    /// [`TransportPort::stop`](crate::port::TransportPort::stop).
    ///
    /// Count-only and saturating: control-message rate drops, per-origin
    /// fairness drops, aggregate-full drops, and closed-receiver drops in
    /// Closed > control rate > fairness > Full order.
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
#[path = "npdu_admission_tests.rs"]
mod tests;
