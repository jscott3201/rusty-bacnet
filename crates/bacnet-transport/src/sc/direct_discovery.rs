//! Opt-in direct-connection discovery and NPDU-over-direct with hub fallback.
//!
//! When enabled, unicast sends consult a bounded URI cache keyed by
//! destination VMAC. On a miss the transport issues one Address-Resolution
//! request through the hub, waits for the matching Address-Resolution-ACK
//! by message ID, caches the returned URIs, dials a direct peer, runs the
//! Connect-Request into Connect-Accept handshake with the existing hub
//! validation, and sends the NPDU over the direct WebSocket with both
//! address parameters omitted. Any failure at any stage falls back to the
//! existing hub send path.
//! Disabled (the default) leaves the hub send path byte-identical.
//!
//! Source grounding paraphrases the local Standard 135-2020 Annex AB: a
//! node may request a peer's direct URIs with an Address-Resolution message
//! sent through the hub, the peer answers with its URIs or an empty list
//! when it accepts direct connections but knows none, and a node that does
//! not accept direct connections answers with an unsupported-function NAK;
//! direct URIs may otherwise be statically configured, and when none are
//! configured they may be requested through the hub. Only unicast addressed
//! to the direct peer travels over a direct connection; all other traffic
//! uses the hub. Direct sends omit both address parameters while hub sends
//! carry both. A direct WebSocket carries NPDUs only after its Connect
//! handshake completes; strict peers that require the handshake otherwise
//! fall back to hub delivery. When a node initiates direct connections, the timing of
//! initiation and re-initiation is a local matter, and a failed URI attempt
//! still leaves hub delivery available. Response messages copy the causing
//! message ID so an ACK can be matched to its request, and the wait budget
//! is a local matter. This module reuses the transport's existing connect
//! timeout for that wait and introduces no new global deadline type.
//!
//! Asymmetry is intentional and documented: this transport dials out to
//! direct peers only. Inbound direct connections stay refused — solicited
//! Advertisements always report accept-direct 0 — so a peer's direct dial
//! to this node is out of scope. Hub, failover, reconnect, and accept-side
//! behavior are unchanged.
//!
//! Cache policy (owner-local): at most [`DIRECT_URI_CACHE_MAX_ENTRIES`]
//! VMAC entries; each entry lives [`DIRECT_URI_CACHE_TTL`] from insertion.
//! Reads lazily expire entries. Inserts evict the oldest inserted VMAC first
//! (FIFO) while over cap. Empty URI lists (peer accepts direct but knows no
//! URIs, or an unsupported-function NAK) are cached as empty so later sends
//! within TTL go straight to the hub without another request. Timeouts and
//! transport failures are not cached.
//!
//! Redial backoff (owner-local): each dial/handshake/send failure on a URI
//! records exponential backoff (`200ms, 400ms, 800ms, ...` capped at 5s via
//! [`redial_backoff_delay`]). Sends skip URIs still inside their backoff
//! window and fall back to hub delivery. Success clears the URI entry. At
//! most [`DIRECT_REDIAL_MAX_ENTRIES`] URIs are tracked (FIFO eviction).
//! Annex AB leaves initiation/re-initiation timing to the local node, so
//! these bounds are local policy, not wire conformance.
//!
//! Connection reuse (owner-local): at most [`DIRECT_POOL_MAX_ENTRIES`]
//! handshaked direct connections are pooled, one per destination VMAC
//! (FIFO eviction while over cap). Each pooled entry lives
//! [`DIRECT_POOL_IDLE_TTL`] from last successful use; reads lazily expire.
//! A single reusable connection per VMAC covers the need — discovery already
//! yields one URI list per VMAC and sends are per-VMAC — so no general pool
//! is introduced. Pool teardown is dropping `DirectShared` with the
//! transport (lifecycle precedent); no background task or new timer type is
//! added. All waits reuse `connect_timeout_ms`; expiry uses `Instant` checks
//! like the URI cache.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{oneshot, Mutex};

use bacnet_types::error::Error;
use bytes::BytesMut;

use crate::port::DataAttribute;
use crate::sc_frame::{
    address_resolution_message_error, decode_sc_bvlc_result, encode_sc_message, is_valid_wss_uri,
    ScBvlcResult, ScFunction, ScMessage, Vmac, BROADCAST_VMAC,
};

use super::{ScConnection, ScConnectionState, WebSocketPort};

/// Maximum cached direct-connection URI entries (owner-local bound).
pub(crate) const DIRECT_URI_CACHE_MAX_ENTRIES: usize = 32;

/// Time-to-live for cached URI entries (owner-local policy).
pub(crate) const DIRECT_URI_CACHE_TTL: Duration = Duration::from_secs(300);

/// Initial redial backoff after the first consecutive direct failure
/// (owner-local policy; Annex AB leaves re-initiation timing local).
pub(crate) const DIRECT_REDIAL_INITIAL_BACKOFF: Duration = Duration::from_millis(200);

/// Maximum backoff between redials to the same URI (owner-local cap).
pub(crate) const DIRECT_REDIAL_MAX_BACKOFF: Duration = Duration::from_secs(5);

/// Maximum URIs tracked in the redial backoff table (owner-local bound).
pub(crate) const DIRECT_REDIAL_MAX_ENTRIES: usize = 32;

/// Maximum pooled handshaked direct connections (owner-local bound).
///
/// One entry per destination VMAC; FIFO eviction while over cap.
pub(crate) const DIRECT_POOL_MAX_ENTRIES: usize = 16;

/// Idle TTL for pooled direct connections (owner-local policy).
pub(crate) const DIRECT_POOL_IDLE_TTL: Duration = Duration::from_secs(60);

/// Backoff delay for `consecutive_failures` (1-indexed) direct failures.
///
/// Exponential `200ms, 400ms, 800ms, ...` capped at 5s. Pure and
/// time-virtualized: tests assert progression without sleeping.
pub(crate) fn redial_backoff_delay(consecutive_failures: u32) -> Duration {
    let shift = consecutive_failures.saturating_sub(1).min(5);
    DIRECT_REDIAL_INITIAL_BACKOFF
        .saturating_mul(1u32 << shift)
        .min(DIRECT_REDIAL_MAX_BACKOFF)
}

#[derive(Debug, Clone)]
struct BackoffEntry {
    consecutive_failures: u32,
    not_before: Instant,
}

/// Bounded per-URI redial backoff table with FIFO eviction.
///
/// `is_backed_off` is a pure `Instant` comparison (no timers); failures
/// advance the exponential delay, successes remove the entry.
pub(crate) struct RedialBackoff {
    entries: HashMap<String, BackoffEntry>,
    order: VecDeque<String>,
}

impl RedialBackoff {
    pub(crate) fn new() -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn is_backed_off(&self, uri: &str, now: Instant) -> bool {
        self.entries
            .get(uri)
            .is_some_and(|entry| now < entry.not_before)
    }

    pub(crate) fn record_failure(&mut self, uri: String, now: Instant) {
        let failures = self
            .entries
            .get(&uri)
            .map(|entry| entry.consecutive_failures.saturating_add(1))
            .unwrap_or(1);
        if !self.entries.contains_key(&uri) {
            self.order.push_back(uri.clone());
        }
        let delay = redial_backoff_delay(failures);
        self.entries.insert(
            uri,
            BackoffEntry {
                consecutive_failures: failures,
                not_before: now + delay,
            },
        );
        while self.entries.len() > DIRECT_REDIAL_MAX_ENTRIES {
            match self.order.pop_front() {
                Some(oldest) => {
                    self.entries.remove(&oldest);
                }
                None => break,
            }
        }
    }

    pub(crate) fn record_success(&mut self, uri: &str) {
        if self.entries.remove(uri).is_some() {
            self.order.retain(|existing| existing != uri);
        }
    }
}

#[derive(Debug)]
pub(crate) struct PooledDirect<W> {
    ws: Arc<W>,
    uri: String,
    peer_max_bvlc_length: u16,
    peer_max_apdu_length: u16,
    idle_deadline: Instant,
}

impl<W> Clone for PooledDirect<W> {
    fn clone(&self) -> Self {
        Self {
            ws: Arc::clone(&self.ws),
            uri: self.uri.clone(),
            peer_max_bvlc_length: self.peer_max_bvlc_length,
            peer_max_apdu_length: self.peer_max_apdu_length,
            idle_deadline: self.idle_deadline,
        }
    }
}

/// Bounded per-VMAC pool of handshaked direct connections.
///
/// Single reusable connection per destination VMAC with lazy idle expiry;
/// inserts evict the oldest VMAC first (FIFO) while over cap.
pub(crate) struct DirectPool<W> {
    entries: HashMap<Vmac, PooledDirect<W>>,
    order: VecDeque<Vmac>,
}

impl<W> DirectPool<W> {
    pub(crate) fn new() -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn get(&mut self, vmac: &Vmac, now: Instant) -> Option<PooledDirect<W>> {
        let expired = match self.entries.get(vmac) {
            Some(entry) if now < entry.idle_deadline => return Some(entry.clone()),
            Some(_) => true,
            None => return None,
        };
        if expired {
            self.entries.remove(vmac);
            self.order.retain(|existing| existing != vmac);
        }
        None
    }

    pub(crate) fn insert(&mut self, vmac: Vmac, pooled: PooledDirect<W>) {
        if self.entries.contains_key(&vmac) {
            self.order.retain(|existing| existing != &vmac);
        }
        self.order.push_back(vmac);
        self.entries.insert(vmac, pooled);
        while self.entries.len() > DIRECT_POOL_MAX_ENTRIES {
            match self.order.pop_front() {
                Some(oldest) => {
                    self.entries.remove(&oldest);
                }
                None => break,
            }
        }
    }

    pub(crate) fn remove(&mut self, vmac: &Vmac) {
        if self.entries.remove(vmac).is_some() {
            self.order.retain(|existing| existing != vmac);
        }
    }

    pub(crate) fn refresh(&mut self, vmac: &Vmac, now: Instant) {
        if let Some(entry) = self.entries.get_mut(vmac) {
            entry.idle_deadline = now + DIRECT_POOL_IDLE_TTL;
        }
    }

    #[cfg(test)]
    pub(crate) fn insert_test_entry(&mut self, vmac: Vmac, ws: Arc<W>, uri: String, now: Instant) {
        let pooled = PooledDirect {
            ws,
            uri,
            peer_max_bvlc_length: 1476,
            peer_max_apdu_length: 1476,
            idle_deadline: now + DIRECT_POOL_IDLE_TTL,
        };
        self.insert(vmac, pooled);
    }
}

#[derive(Debug, Clone)]
struct CacheEntry {
    uris: Vec<String>,
    inserted: Instant,
}

/// Bounded FIFO URI cache with lazy TTL expiry.
///
/// Eviction is documented here, not inferred: inserts that would exceed
/// [`DIRECT_URI_CACHE_MAX_ENTRIES`] drop the oldest inserted VMAC first.
/// Expired entries are removed on read and never returned.
pub(crate) struct DirectUriCache {
    entries: HashMap<Vmac, CacheEntry>,
    order: VecDeque<Vmac>,
}

impl DirectUriCache {
    pub(crate) fn new() -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    #[allow(dead_code)]
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Return fresh URIs for `vmac`, lazily expiring stale entries.
    pub(crate) fn get(&mut self, vmac: &Vmac, now: Instant) -> Option<Vec<String>> {
        let fresh = match self.entries.get(vmac) {
            Some(entry) if now.duration_since(entry.inserted) < DIRECT_URI_CACHE_TTL => {
                Some(entry.uris.clone())
            }
            Some(_) => None,
            None => return None,
        };
        match fresh {
            Some(uris) => Some(uris),
            None => {
                self.entries.remove(vmac);
                None
            }
        }
    }

    /// Insert `uris` for `vmac`, evicting oldest inserts while over cap.
    pub(crate) fn insert(&mut self, vmac: Vmac, uris: Vec<String>, now: Instant) {
        if self.entries.contains_key(&vmac) {
            self.order.retain(|existing| existing != &vmac);
        }
        self.order.push_back(vmac);
        self.entries.insert(
            vmac,
            CacheEntry {
                uris,
                inserted: now,
            },
        );
        while self.entries.len() > DIRECT_URI_CACHE_MAX_ENTRIES {
            match self.order.pop_front() {
                Some(oldest) => {
                    self.entries.remove(&oldest);
                }
                None => break,
            }
        }
    }
}

/// Dial-one-URI factory supplied by the owner.
///
/// Production wiring captures an [`crate::sc_tls::ScNodeTlsConfig`] clone and
/// calls `TlsWebSocket::connect_direct`; tests supply a loopback peer.
/// The factory must be cancellation-safe: a dropped future must not leave a
/// half-published connection behind.
pub(crate) type DirectDialer<W> =
    Arc<dyn Fn(String) -> Pin<Box<dyn Future<Output = Result<W, Error>> + Send>> + Send + Sync>;

/// Shared opt-in direct discovery state.
///
/// `Some` in the transport means enabled; `None` (the default) means the hub
/// path runs unchanged. The dialer is optional: enabled without a dialer
/// always falls back to the hub path.
pub(crate) struct DirectShared<W: WebSocketPort> {
    cache: Mutex<DirectUriCache>,
    pending: Mutex<HashMap<u16, oneshot::Sender<Vec<String>>>>,
    dialer: Mutex<Option<DirectDialer<W>>>,
    backoff: Mutex<RedialBackoff>,
    pool: Mutex<DirectPool<W>>,
}

impl<W: WebSocketPort> DirectShared<W> {
    pub(crate) fn new() -> Self {
        Self {
            cache: Mutex::new(DirectUriCache::new()),
            pending: Mutex::new(HashMap::new()),
            dialer: Mutex::new(None),
            backoff: Mutex::new(RedialBackoff::new()),
            pool: Mutex::new(DirectPool::new()),
        }
    }
}

impl<W: WebSocketPort> super::ScTransport<W> {
    /// Opt in to on-demand direct discovery for unicast sends (default OFF).
    ///
    /// Disabled (the default) leaves the hub send path byte-identical: no
    /// cache consult, no Address-Resolution request, no direct dial. Enabled
    /// consults the bounded URI cache on each unicast; on a miss it issues
    /// one Address-Resolution request through the hub, caches the ACK URIs,
    /// dials direct, and sends the NPDU over direct with both address
    /// parameters omitted. Any direct-stage failure falls back to hub
    /// delivery. Broadcasts always use the hub path.
    ///
    /// Dial-out only: this flag never enables inbound direct acceptance.
    /// Solicited Advertisements keep accept-direct 0. Hub, failover, and
    /// reconnect behavior are unchanged. The ACK wait reuses the configured
    /// connect timeout; no new global deadline type is introduced.
    pub fn with_direct_discovery(mut self, enabled: bool) -> Self {
        if enabled {
            if self.direct.is_none() {
                self.direct = Some(Arc::new(DirectShared::new()));
            }
        } else {
            self.direct = None;
        }
        self
    }

    /// Supply the direct-dial factory used after discovery (implies opt-in).
    ///
    /// The closure receives one candidate URI string and returns a connected
    /// direct WebSocket. It is tried in ACK order until one dial and send
    /// succeeds; every failure falls back to hub delivery. Calling this
    /// enables discovery; call [`Self::with_direct_discovery`] with `false`
    /// afterwards to disable again (which drops the dialer and cache).
    /// Production callers capture a TLS config clone and call
    /// `TlsWebSocket::connect_direct` inside the closure.
    pub fn with_direct_dialer<F, Fut>(mut self, dialer: F) -> Self
    where
        F: Fn(String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<W, Error>> + Send + 'static,
    {
        let shared = match self.direct.take() {
            Some(shared) => shared,
            None => Arc::new(DirectShared::new()),
        };
        if let Ok(mut slot) = shared.dialer.try_lock() {
            *slot = Some(Arc::new(move |uri: String| {
                Box::pin(dialer(uri)) as Pin<Box<dyn Future<Output = Result<W, Error>> + Send>>
            }));
        }
        self.direct = Some(shared);
        self
    }

    pub(super) fn direct_shared(&self) -> Option<Arc<DirectShared<W>>> {
        self.direct.clone()
    }

    #[cfg(test)]
    pub(crate) async fn direct_shared_test_pool_len(&self) -> Option<usize> {
        match self.direct.clone() {
            Some(shared) => Some(shared.test_pool_len().await),
            None => None,
        }
    }

    #[cfg(test)]
    pub(crate) async fn direct_shared_test_is_backed_off(&self, uri: &str) -> bool {
        match self.direct.clone() {
            Some(shared) => shared.test_is_backed_off(uri).await,
            None => false,
        }
    }
}

/// Split an Address-Resolution-ACK payload into validated URI strings.
///
/// Empty payloads are valid and yield an empty list. Any structural fault
/// yields `None` so the caller falls back to hub delivery without caching.
pub(crate) fn parse_ack_uris(payload: &[u8]) -> Option<Vec<String>> {
    if payload.is_empty() {
        return Some(Vec::new());
    }
    let text = core::str::from_utf8(payload).ok()?;
    if text.starts_with(' ') || text.ends_with(' ') || text.contains("  ") {
        return None;
    }
    let mut out = Vec::new();
    for token in text.split(' ') {
        if !is_valid_wss_uri(token) {
            return None;
        }
        out.push(token.to_owned());
    }
    Some(out)
}

impl<W: WebSocketPort> DirectShared<W> {
    pub(super) async fn cached_uris(&self, vmac: &Vmac) -> Option<Vec<String>> {
        let now = Instant::now();
        self.cache.lock().await.get(vmac, now)
    }

    async fn cache_insert(&self, vmac: Vmac, uris: Vec<String>) {
        let now = Instant::now();
        self.cache.lock().await.insert(vmac, uris, now);
    }

    async fn pooled_get(&self, vmac: &Vmac, now: Instant) -> Option<PooledDirect<W>> {
        self.pool.lock().await.get(vmac, now)
    }

    async fn pooled_insert(
        &self,
        vmac: Vmac,
        ws: Arc<W>,
        uri: String,
        peer_max_bvlc_length: u16,
        peer_max_apdu_length: u16,
        now: Instant,
    ) {
        let pooled = PooledDirect {
            ws,
            uri,
            peer_max_bvlc_length,
            peer_max_apdu_length,
            idle_deadline: now + DIRECT_POOL_IDLE_TTL,
        };
        self.pool.lock().await.insert(vmac, pooled);
    }

    async fn pooled_remove(&self, vmac: &Vmac) {
        self.pool.lock().await.remove(vmac);
    }

    async fn pooled_refresh(&self, vmac: &Vmac, now: Instant) {
        self.pool.lock().await.refresh(vmac, now);
    }

    /// URIs still eligible for dial (ACK order preserved).
    async fn eligible_uris(&self, uris: &[String], now: Instant) -> Vec<String> {
        let guard = self.backoff.lock().await;
        uris.iter()
            .filter(|uri| !guard.is_backed_off(uri, now))
            .cloned()
            .collect()
    }

    async fn note_direct_failure(&self, uri: &str) {
        let now = Instant::now();
        self.backoff
            .lock()
            .await
            .record_failure(uri.to_owned(), now);
    }

    async fn note_direct_success(&self, uri: &str) {
        self.backoff.lock().await.record_success(uri);
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) async fn test_backoff_len(&self) -> usize {
        self.backoff.lock().await.len()
    }

    #[cfg(test)]
    pub(crate) async fn test_is_backed_off(&self, uri: &str) -> bool {
        let guard = self.backoff.lock().await;
        guard.is_backed_off(uri, Instant::now())
    }

    #[cfg(test)]
    pub(crate) async fn test_pool_len(&self) -> usize {
        self.pool.lock().await.len()
    }

    /// Wake the pending discovery matching an inbound hub message, if any.
    ///
    /// Well-formed ACKs complete with their parsed URI list; a BVLC-Result
    /// NAK for Address-Resolution completes with an empty list so the sender
    /// falls back to hub delivery (and caches the empty result). Malformed
    /// ACKs, unmatched IDs, and unrelated functions stay silent and keep
    /// waiting until the sender's timeout. Never changes connection state.
    pub(super) async fn fulfill_from_hub_message(&self, msg: &ScMessage) {
        match msg.function {
            ScFunction::AddressResolutionAck => {
                if address_resolution_message_error(msg).is_some() {
                    return;
                }
                let Some(uris) = parse_ack_uris(&msg.payload) else {
                    return;
                };
                let sender = self.pending.lock().await.remove(&msg.message_id);
                if let Some(sender) = sender {
                    let _ = sender.send(uris);
                }
            }
            ScFunction::Result => {
                let Ok(ScBvlcResult::Nak { result_for, .. }) = decode_sc_bvlc_result(msg) else {
                    return;
                };
                if result_for != ScFunction::AddressResolution {
                    return;
                }
                let sender = self.pending.lock().await.remove(&msg.message_id);
                if let Some(sender) = sender {
                    let _ = sender.send(Vec::new());
                }
            }
            _ => {}
        }
    }

    /// Issue one Address-Resolution request through the hub and wait for the
    /// ACK with the same message ID, bounded by the connect timeout.
    ///
    /// Returns `Some(uris)` on a matched ACK or NAK (empty when the peer
    /// knows no URIs or does not support direct connections). Returns `None`
    /// on any transport failure or timeout; the caller must fall back to hub
    /// delivery and must not cache the miss. Successful results are cached
    /// before returning, including empty lists.
    pub(super) async fn discover_via_hub(
        &self,
        dest: Vmac,
        ws: &Arc<W>,
        conn: &Arc<Mutex<ScConnection>>,
        connect_timeout_ms: u64,
    ) -> Option<Vec<String>> {
        let (message_id, request_bytes) = {
            let mut c = conn.lock().await;
            if c.state != ScConnectionState::Connected {
                return None;
            }
            // Allocate an ID unused by other pending discoveries so an ACK
            // cannot complete the wrong waiter after u16 wrap.
            let mut request = c.build_address_resolution_request(dest);
            let guard = self.pending.lock().await;
            let mut attempts = 0;
            while guard.contains_key(&request.message_id) && attempts < u16::MAX as usize {
                request = c.build_address_resolution_request(dest);
                attempts += 1;
            }
            if guard.contains_key(&request.message_id) {
                return None;
            }
            let message_id = request.message_id;
            let mut buf = BytesMut::new();
            encode_sc_message(&mut buf, &request);
            (message_id, buf.freeze().to_vec())
        };
        let (sender, receiver) = oneshot::channel();
        {
            let mut guard = self.pending.lock().await;
            // A concurrent waiter cannot hold our ID: we reserved it above
            // under the same pending lock ordering (connection then pending).
            // If insertion still collides, fall back to hub delivery.
            if guard.contains_key(&message_id) {
                return None;
            }
            guard.insert(message_id, sender);
        }
        let send_result = ws.send(&request_bytes).await;
        if send_result.is_err() {
            self.pending.lock().await.remove(&message_id);
            return None;
        }
        let wait = Duration::from_millis(connect_timeout_ms.max(1));
        let uris = match tokio::time::timeout(wait, receiver).await {
            Ok(Ok(uris)) => uris,
            _ => {
                self.pending.lock().await.remove(&message_id);
                return None;
            }
        };
        self.cache_insert(dest, uris.clone()).await;
        Some(uris)
    }

    /// Send over a pooled handshaked connection or dial each eligible URI in
    /// order, run the Connect handshake, pool the success, and send the NPDU
    /// over direct with both address parameters omitted.
    ///
    /// Pooled reuse is attempted first: a fresh idle entry for `dest` sends
    /// one direct frame bounded by the stored peer limits. Pool hit refreshes
    /// the idle deadline; pool send failure evicts without recording redial
    /// backoff (a broken reuse is not URI health) and falls through to
    /// redial. URIs inside their backoff window are skipped; every skipped
    /// or failed attempt must fall back to hub delivery.
    ///
    /// Returns `Ok(())` on the first successful direct send (pooled or fresh
    /// dial). Returns `Err` when no dialer is configured, the pool misses and
    /// every eligible URI fails or is backed off, the handshake is rejected
    /// or times out, or the hub connection is no longer usable for ID
    /// allocation; the caller must fall back to hub delivery. Never mutates
    /// hub connection state besides consuming fresh message IDs, and never
    /// touches hub failover or reconnect state. Locks are never held across
    /// dial/handshake/send awaits; concurrent sends may duplicate a dial in
    /// the race window (last-writer-wins pool insert) but never deadlock.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn try_direct_uris(
        &self,
        uris: &[String],
        dest: Vmac,
        npdu: &[u8],
        data_attributes: &[DataAttribute],
        conn: &Arc<Mutex<ScConnection>>,
        hub_max_apdu_length: u16,
        connect_timeout_ms: u64,
    ) -> Result<(), ()> {
        let dialer = self.dialer.lock().await.clone().ok_or(())?;
        if uris.is_empty() || dest == BROADCAST_VMAC {
            return Err(());
        }
        if npdu.len() > hub_max_apdu_length as usize {
            return Err(());
        }
        // Pooled reuse: one handshaked connection per destination VMAC.
        // Snapshot under the pool lock, then release before any await.
        let now = Instant::now();
        if let Some(pooled) = self.pooled_get(&dest, now).await {
            if npdu.len() <= pooled.peer_max_apdu_length as usize {
                let direct_msg = {
                    let mut c = conn.lock().await;
                    if c.state != ScConnectionState::Connected {
                        return Err(());
                    }
                    c.build_direct_encapsulated_npdu(npdu, data_attributes).ok()
                };
                if let Some(direct_msg) = direct_msg {
                    let mut buf = BytesMut::new();
                    encode_sc_message(&mut buf, &direct_msg);
                    if buf.len() <= pooled.peer_max_bvlc_length as usize {
                        let send_wait = Duration::from_millis(connect_timeout_ms.max(1));
                        match tokio::time::timeout(send_wait, pooled.ws.send(&buf)).await {
                            Ok(Ok(())) => {
                                let refresh_now = Instant::now();
                                self.pooled_refresh(&dest, refresh_now).await;
                                self.note_direct_success(&pooled.uri).await;
                                return Ok(());
                            }
                            _ => {
                                self.pooled_remove(&dest).await;
                            }
                        }
                    }
                }
            }
        }
        // Redial with per-URI backoff: skip URIs still inside their window,
        // preserving ACK order. All-backed-off means immediate hub fallback.
        let candidates = self.eligible_uris(uris, Instant::now()).await;
        if candidates.is_empty() {
            return Err(());
        }
        // Materialize one direct frame per attempt under the hub ID counter
        // so direct and hub messages share unique IDs. The hub state itself
        // is only read, never transitioned, here. Each attempt first runs
        // the shared Connect-Request into Connect-Accept handshake on an
        // ephemeral probe carrying the hub connection's node identity; any
        // handshake failure records backoff, drops the dialed socket, and
        // tries the next eligible URI. Size mismatches against freshly
        // learned peer limits are per-NPDU, not URI health, so they skip
        // without recording backoff.
        for uri in &candidates {
            let dial_wait = Duration::from_millis(connect_timeout_ms.max(1));
            let direct_ws = match tokio::time::timeout(dial_wait, dialer(uri.clone())).await {
                Ok(Ok(ws)) => ws,
                _ => {
                    self.note_direct_failure(uri).await;
                    continue;
                }
            };
            let probe = {
                let c = conn.lock().await;
                if c.state != ScConnectionState::Connected {
                    return Err(());
                }
                Arc::new(Mutex::new(c.connect_probe()))
            };
            let handshake_wait = connect_timeout_ms.max(1);
            if super::handshake::perform_handshake(&direct_ws, &probe, None, handshake_wait)
                .await
                .is_err()
            {
                self.note_direct_failure(uri).await;
                continue;
            }
            let (peer_max_bvlc_length, peer_max_apdu_length) = {
                let p = probe.lock().await;
                (p.hub_max_bvlc_length, p.hub_max_apdu_length)
            };
            if npdu.len() > peer_max_apdu_length as usize {
                continue;
            }
            let direct_msg = {
                let mut c = conn.lock().await;
                if c.state != ScConnectionState::Connected {
                    return Err(());
                }
                match c.build_direct_encapsulated_npdu(npdu, data_attributes) {
                    Ok(msg) => msg,
                    Err(_) => return Err(()),
                }
            };
            let mut buf = BytesMut::new();
            encode_sc_message(&mut buf, &direct_msg);
            if buf.len() > peer_max_bvlc_length as usize {
                continue;
            }
            let send_wait = Duration::from_millis(connect_timeout_ms.max(1));
            match tokio::time::timeout(send_wait, direct_ws.send(&buf)).await {
                Ok(Ok(())) => {
                    self.note_direct_success(uri).await;
                    self.pooled_insert(
                        dest,
                        Arc::new(direct_ws),
                        uri.clone(),
                        peer_max_bvlc_length,
                        peer_max_apdu_length,
                        Instant::now(),
                    )
                    .await;
                    return Ok(());
                }
                _ => {
                    self.note_direct_failure(uri).await;
                    continue;
                }
            }
        }
        Err(())
    }
}
