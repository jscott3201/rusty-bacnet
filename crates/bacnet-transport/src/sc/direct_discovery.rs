//! Optional URI discovery; established direct routes are selected independently.
//!
//! A unicast first uses current UUID/VMAC membership, including accepted peers
//! when discovery is disabled. Without an established route, enabled discovery
//! consults the bounded URI cache or sends Address-Resolution through the Hub.
//! Built-in TLS dials authenticate the peer and admit application traffic in
//! both directions. Custom factories are send-only and cannot mint identity.
//! Connect completes before publication; direct frames omit both VMAC fields.
//! Broadcast always uses the Hub.
//!
//! Predial/connect failure leaves Hub routing available. A full current-direct
//! queue returns a capacity error. A possibly started write never retries via
//! another socket. Only definitely unstarted work on a retired route permits a
//! fresh routing decision. Original-response capabilities never use fallback.
//!
//! Local bounds: 32 URI-cache entries, five-minute insertion TTL, FIFO eviction;
//! empty URI results are cached, transport failure/timeouts are not. Up to 32
//! URI backoffs grow from 200ms to 5s. The outbound owner permits 16 established
//! peers, 16 pending dials and 32 physical sockets including retirement. Socket
//! workers expire after 60s binary/write inactivity and observe remote close.
//! Each socket has one writer, 64 shared ordinary/reply queue slots and at most
//! one active bounded write. Ready reads and writes alternate; TLS Ping/Pong
//! consume one read turn without renewing the idle deadline. Stop aborts and
//! joins owned workers; disable/drop abort them. Accepted listener lifetime is
//! independent of discovery, but registered transport teardown seals it too.
//!
//! These discovery/cache/idle deadlines are local policy. Annex AB.4.2 and
//! AB.6.2 ground established direct selection and the handshake/address shape.

use super::direct_membership::{DirectMembership, DirectRole, Refusal};
pub(crate) use super::direct_pool::DirectPool;
use super::direct_pool::PooledDirect;
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::{
    Arc, Mutex as StdMutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

use tokio::sync::{Mutex, oneshot};

use bacnet_types::error::Error;
use bytes::BytesMut;

use crate::port::DataAttribute;
use crate::sc_frame::{
    BROADCAST_VMAC, ScBvlcResult, ScFunction, ScMessage, Vmac, address_resolution_message_error,
    decode_sc_bvlc_result, encode_sc_message, is_valid_wss_uri,
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
///
/// Traffic in either direction refreshes it; heartbeats do not, so keep-alive
/// alone never holds an unused link open (#2127).
pub(crate) const DIRECT_POOL_IDLE_TTL: Duration = Duration::from_secs(60);

/// The node's timing that a link it dials inherits: the connect wait and
/// the heartbeat `(interval, timeout)`, all in milliseconds.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DirectTiming {
    pub(crate) connect_timeout_ms: u64,
    pub(crate) heartbeat_ms: (u64, u64),
}

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

pub(crate) use super::direct_socket::DirectDialer;

/// One Address-Resolution request waiting for its answer.
struct Waiter {
    /// Per-request tag: after a u16 wrap a newer request may reuse the same
    /// message ID, and only the request that inserted an entry removes it.
    seq: u64,
    /// The node asked. Only an answer the hub relays from this node counts.
    dest: Vmac,
    answer: oneshot::Sender<Answer>,
}

/// How an Address-Resolution request ended.
#[derive(Debug, PartialEq)]
enum Answer {
    /// The node asked answered: its URIs, or none (an empty ACK or its NAK).
    /// Cached.
    Uris(Vec<String>),
    /// The hub itself refused the request. Fall back to hub delivery now
    /// and cache nothing: the refusal says nothing about the node.
    HubRefused,
}

/// Address-Resolution waiters keyed by message ID. Every access is short
/// and synchronous, so the lock is never held across an await.
#[derive(Default)]
struct Waiters {
    next_seq: u64,
    by_id: HashMap<u16, Waiter>,
}

/// Lock `waiters` the same way everywhere, poisoned or not.
///
/// No critical section can panic part-way through a map update: the
/// allocator only builds requests and probes the map before its one
/// insert, and the rest are single map operations. A poisoned lock still
/// holds a consistent map, so take it rather than panic, in a Drop least
/// of all.
fn lock_waiters(waiters: &StdMutex<Waiters>) -> std::sync::MutexGuard<'_, Waiters> {
    waiters
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Owns one [`Waiters`] entry for the life of a `discover_via_hub` call.
///
/// Dropping it removes the entry if it is still this request's, so an
/// answer, a send error, a timeout and a cancelled caller all clean up the
/// same way. An entry the answer already took, or a newer request's entry
/// under the same message ID, is left alone.
struct PendingEntry<'a> {
    waiters: &'a StdMutex<Waiters>,
    message_id: u16,
    seq: u64,
}

impl Drop for PendingEntry<'_> {
    fn drop(&mut self) {
        let mut waiters = lock_waiters(self.waiters);
        if waiters
            .by_id
            .get(&self.message_id)
            .is_some_and(|waiter| waiter.seq == self.seq)
        {
            waiters.by_id.remove(&self.message_id);
        }
    }
}

/// Shared opt-in direct discovery state.
///
/// This optional owner does not own accepted membership. Without a dialer,
/// discovery cannot establish a new outbound route; existing routes still work.
pub(crate) struct DirectShared<W: WebSocketPort> {
    cache: Mutex<DirectUriCache>,
    pending: StdMutex<Waiters>,
    dialer: Mutex<Option<DirectDialer<W>>>,
    backoff: Mutex<RedialBackoff>,
    pool: StdMutex<DirectPool>,
    enabled: AtomicBool,
    workers: StdMutex<Vec<tokio::task::JoinHandle<()>>>,
    membership: Arc<DirectMembership>,
    pending_dials: Arc<Semaphore>,
    physical: Arc<Semaphore>,
    #[cfg(feature = "sc-tls")]
    intake: StdMutex<Option<super::direct_pool::DirectIntake>>,
}

impl<W: WebSocketPort> DirectShared<W> {
    pub(crate) fn new(membership: Arc<DirectMembership>) -> Self {
        Self {
            cache: Mutex::new(DirectUriCache::new()),
            pending: StdMutex::new(Waiters::default()),
            dialer: Mutex::new(None),
            backoff: Mutex::new(RedialBackoff::new()),
            pool: StdMutex::new(DirectPool::new()),
            enabled: AtomicBool::new(true),
            workers: StdMutex::new(Vec::new()),
            membership,
            pending_dials: Arc::new(Semaphore::new(DIRECT_POOL_MAX_ENTRIES)),
            physical: Arc::new(Semaphore::new(DIRECT_POOL_MAX_ENTRIES * 2)),
            #[cfg(feature = "sc-tls")]
            intake: StdMutex::new(None),
        }
    }
}

impl<W: WebSocketPort> super::ScTransport<W> {
    /// Configure on-demand URI discovery before start (default OFF).
    ///
    /// Established matching direct peers are used even when disabled. Without
    /// one, enabled discovery may query the Hub and dial; broadcasts stay on
    /// the Hub. Failed discovery/Connect permits Hub fallback, but saturation
    /// or an uncertain started write returns an error without duplicate send.
    /// This never starts a listener. Disabling a running transport retires its
    /// outbound workers and discards the cache/dialer, preserving accepted peers.
    pub fn with_direct_discovery(mut self, enabled: bool) -> Self {
        if enabled {
            if self.direct.is_none() {
                self.direct = Some(Arc::new(DirectShared::new(self.direct_membership.clone())));
            }
        } else {
            if let Some(shared) = self.direct.take() {
                shared.disable();
            }
        }
        self
    }

    /// Configure an untrusted, send-only direct factory before start.
    ///
    /// Implies discovery. Candidate URIs are tried in ACK order until Connect
    /// succeeds. Application NPDUs received from this adapter are discarded;
    /// even a closure returning `TlsWebSocket` cannot manufacture verified
    /// ingress identity or response authority. Use `with_direct_tls`
    /// with `sc-tls` for the built-in authenticated bidirectional path.
    /// Disabling discovery discards this factory and its current connections.
    pub fn with_custom_direct_dialer<F, Fut>(mut self, dialer: F) -> Self
    where
        F: Fn(String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<W, Error>> + Send + 'static,
    {
        let shared = match self.direct.take() {
            Some(shared) => shared,
            None => Arc::new(DirectShared::new(self.direct_membership.clone())),
        };
        if let Ok(mut slot) = shared.dialer.try_lock() {
            *slot = Some(DirectDialer::Custom(Arc::new(move |uri: String| {
                Box::pin(dialer(uri)) as Pin<Box<dyn Future<Output = Result<W, Error>> + Send>>
            })));
        }
        self.direct = Some(shared);
        self
    }

    /// Before start, enable direct discovery using the built-in authenticated TLS
    /// adapter. Only this path admits outbound application NPDUs with verified
    /// leaf/incarnation evidence and an original-socket response capability.
    #[cfg(feature = "sc-tls")]
    pub fn with_direct_tls(mut self, config: crate::sc_tls::ScNodeTlsConfig) -> Self {
        self = self.with_direct_discovery(true);
        if let Some(shared) = &self.direct {
            *shared
                .dialer
                .try_lock()
                .expect("builder has no active dial") = Some(DirectDialer::Tls(config));
        }
        self
    }

    #[cfg(all(test, feature = "sc-tls"))]
    pub(crate) fn direct_route_for_test(
        &self,
        vmac: &Vmac,
    ) -> Option<super::direct_egress::DirectEgress> {
        self.direct_membership.route(vmac)
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
    pub(super) fn disable(&self) {
        // Same lock as publication seals in-flight dials before clearing owners.
        let mut pool = self.pool.lock().unwrap();
        self.enabled.store(false, Ordering::Release);
        self.pending_dials.close();
        pool.clear();
        for task in self.workers.lock().unwrap().iter() {
            task.abort();
        }
    }

    /// Seal and retire every link like [`Self::disable`], but give each
    /// worker until `deadline` to send its Disconnect-Request and close
    /// (#2520); one still sending then is aborted.
    pub(super) async fn shutdown(&self, deadline: tokio::time::Instant) {
        {
            let mut pool = self.pool.lock().unwrap();
            self.enabled.store(false, Ordering::Release);
            self.pending_dials.close();
            pool.clear();
        }
        let workers = std::mem::take(&mut *self.workers.lock().unwrap());
        for mut worker in workers {
            if tokio::time::timeout_at(deadline, &mut worker)
                .await
                .is_err()
            {
                worker.abort();
                let _ = worker.await;
            }
        }
        #[cfg(feature = "sc-tls")]
        self.intake.lock().unwrap().take();
    }

    #[cfg(feature = "sc-tls")]
    pub(crate) fn set_intake(
        &self,
        tx: tokio::sync::mpsc::Sender<crate::port::ReceivedNpdu>,
        admission: Arc<super::npdu_admission::ScNpduAdmission>,
    ) {
        *self.intake.lock().unwrap() = Some(super::direct_pool::DirectIntake { tx, admission });
    }

    pub(super) async fn cached_uris(&self, vmac: &Vmac) -> Option<Vec<String>> {
        let now = Instant::now();
        self.cache.lock().await.get(vmac, now)
    }

    async fn cache_insert(&self, vmac: Vmac, uris: Vec<String>) {
        let now = Instant::now();
        self.cache.lock().await.insert(vmac, uris, now);
    }

    #[cfg(all(test, feature = "sc-tls"))]
    fn pooled_get(&self, vmac: &Vmac, now: Instant) -> Option<PooledDirect> {
        self.pool.lock().unwrap().get(vmac, now)
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
        self.pool.lock().unwrap().len()
    }

    /// Wake the pending discovery matching an inbound hub message, if any.
    ///
    /// Well-formed ACKs complete with their parsed URI list; a BVLC-Result
    /// NAK for Address-Resolution completes with an empty list so the sender
    /// falls back to hub delivery (and caches the empty result). Malformed
    /// ACKs, unmatched IDs, and unrelated functions stay silent and keep
    /// waiting until the sender's timeout. Never changes connection state.
    ///
    /// An answer must also come from the node asked. Both the ACK and the
    /// NAK are the destination node's (AB.3.3), and the hub names the node a
    /// relayed message came from as its originating VMAC (AB.5.3.2). An ACK
    /// or NAK naming another node is ignored: IDs count up from 1, and any
    /// node on the hub could otherwise guess one. An ACK with no originating
    /// VMAC is ignored too. A NAK with none came from the connection peer,
    /// the hub itself (AB.3.1.2); no other node can produce one, since the
    /// hub stamps the origin on everything it relays. It ends the wait at
    /// once and caches nothing; the caller then takes the hub path. (The
    /// connection itself treats such a hub NAK as fatal, so that path
    /// currently meets a closed hub link rather than waiting out the
    /// connect timeout first.)
    pub(super) fn fulfill_from_hub_message(&self, msg: &ScMessage) {
        let answer = match msg.function {
            ScFunction::AddressResolutionAck => {
                if address_resolution_message_error(msg).is_some() {
                    return;
                }
                let Some(uris) = parse_ack_uris(&msg.payload) else {
                    return;
                };
                Answer::Uris(uris)
            }
            ScFunction::Result => {
                let Ok(ScBvlcResult::Nak { result_for, .. }) = decode_sc_bvlc_result(msg) else {
                    return;
                };
                if result_for != ScFunction::AddressResolution {
                    return;
                }
                match msg.originating_vmac {
                    None => Answer::HubRefused,
                    Some(_) => Answer::Uris(Vec::new()),
                }
            }
            _ => return,
        };
        let waiter = {
            let mut waiters = lock_waiters(&self.pending);
            let accepted = waiters.by_id.get(&msg.message_id).is_some_and(|waiter| {
                match answer {
                    // Only an origin-less NAK yields this; see above.
                    Answer::HubRefused => true,
                    Answer::Uris(_) => msg.originating_vmac == Some(waiter.dest),
                }
            });
            if !accepted {
                return;
            }
            waiters.by_id.remove(&msg.message_id)
        };
        if let Some(waiter) = waiter {
            let _ = waiter.answer.send(answer);
        }
    }

    /// Issue one Address-Resolution request through the hub and wait for the
    /// ACK with the same message ID, bounded by the connect timeout.
    ///
    /// Returns `Some(uris)` on a matched ACK or NAK (empty when the peer
    /// knows no URIs or does not support direct connections). Returns `None`
    /// on any transport failure, timeout or refusal by the hub itself; the
    /// caller must fall back to hub delivery and must not cache the miss. Successful results are cached
    /// before returning, including empty lists.
    pub(super) async fn discover_via_hub<H: WebSocketPort>(
        &self,
        dest: Vmac,
        ws: &Arc<H>,
        conn: &Arc<Mutex<ScConnection>>,
        connect_timeout_ms: u64,
    ) -> Option<Vec<String>> {
        let (entry, request_bytes, receiver) = {
            let mut c = conn.lock().await;
            if c.state != ScConnectionState::Connected {
                return None;
            }
            // Allocate an ID unused by other pending discoveries so an ACK
            // cannot complete the wrong waiter after u16 wrap, and insert
            // under the same lock so no other request can claim it between.
            let mut request = c.build_address_resolution_request(dest);
            let mut waiters = lock_waiters(&self.pending);
            let mut attempts = 0;
            while waiters.by_id.contains_key(&request.message_id) && attempts < u16::MAX as usize {
                request = c.build_address_resolution_request(dest);
                attempts += 1;
            }
            if waiters.by_id.contains_key(&request.message_id) {
                return None;
            }
            let (answer, receiver) = oneshot::channel();
            let seq = waiters.next_seq;
            waiters.next_seq = seq.wrapping_add(1);
            waiters
                .by_id
                .insert(request.message_id, Waiter { seq, dest, answer });
            drop(waiters);
            // Nothing between the insert and here can panic. From here on
            // every exit, cancellation included, removes the entry.
            let entry = PendingEntry {
                waiters: &self.pending,
                message_id: request.message_id,
                seq,
            };
            let mut buf = BytesMut::new();
            encode_sc_message(&mut buf, &request);
            (entry, buf.freeze().to_vec(), receiver)
        };
        if ws.send(&request_bytes).await.is_err() {
            return None;
        }
        let wait = Duration::from_millis(connect_timeout_ms.max(1));
        let Ok(Ok(Answer::Uris(uris))) = tokio::time::timeout(wait, receiver).await else {
            return None;
        };
        drop(entry);
        self.cache_insert(dest, uris.clone()).await;
        Some(uris)
    }

    /// Establish a bounded direct route when no established peer is usable.
    /// After queue admission, uncertain writes never trigger another attempt.
    pub(super) async fn try_direct_uris(
        &self,
        uris: &[String],
        dest: Vmac,
        npdu: &[u8],
        attributes: &[DataAttribute],
        conn: &Arc<Mutex<ScConnection>>,
        timing: DirectTiming,
    ) -> Result<(), super::direct_egress::DirectSendError> {
        use super::direct_egress::DirectSendError as Failure;
        let DirectTiming {
            connect_timeout_ms,
            heartbeat_ms,
        } = timing;
        if !self.enabled.load(Ordering::Acquire) || dest == BROADCAST_VMAC {
            return Err(Failure::Unavailable);
        }
        if let Some(route) = self.membership.route(&dest) {
            return route.send_npdu(npdu, attributes).await;
        }
        let dialer = self
            .dialer
            .lock()
            .await
            .clone()
            .ok_or(Failure::Unavailable)?;
        let candidates = self.eligible_uris(uris, Instant::now()).await;
        let _pending = self
            .pending_dials
            .clone()
            .try_acquire_owned()
            .map_err(|_| Failure::Unavailable)?;
        let wait = Duration::from_millis(connect_timeout_ms.max(1));
        for uri in candidates {
            let physical = self
                .physical
                .clone()
                .try_acquire_owned()
                .map_err(|_| Failure::Unavailable)?;
            let ws = match tokio::time::timeout(wait, dialer.dial(uri.clone())).await {
                Ok(Ok(ws)) => ws,
                _ => {
                    self.note_direct_failure(&uri).await;
                    continue;
                }
            };
            let probe = {
                let c = conn.lock().await;
                if c.state != ScConnectionState::Connected {
                    return Err(Failure::Unavailable);
                }
                Arc::new(Mutex::new(c.connect_probe()))
            };
            if let Err(error) =
                super::handshake::perform_handshake(&ws, &probe, None, connect_timeout_ms.max(1))
                    .await
            {
                super::handshake::close_failed_connect(&ws, &error).await;
                self.note_direct_failure(&uri).await;
                continue;
            }
            // A connected socket this dial gives up from here on ends in
            // order: Disconnect-Request, then close 1000 (#2520).
            let goodbye = super::direct_pool::LinkEnd::Retired;
            let identity = {
                let p = probe.lock().await;
                (p.hub_vmac == Some(dest)).then(|| {
                    p.hub_device_uuid.map(|uuid| {
                        (
                            (p.hub_max_bvlc_length, p.hub_max_npdu_length),
                            (p.max_bvlc_length, p.max_apdu_length),
                            uuid,
                            p.device_uuid,
                            p.local_vmac,
                        )
                    })
                })
            };
            let Some(identity) = identity else {
                goodbye.say_goodbye(&ws, wait).await;
                continue;
            };
            let Some((limits, local_limits, uuid, local_uuid, local_vmac)) = identity else {
                goodbye.say_goodbye(&ws, wait).await;
                return Err(Failure::Unavailable);
            };
            let mut socket = Some(ws);
            let pooled: Result<PooledDirect, Failure> = 'pooled: {
                let mut pool = self.pool.lock().unwrap();
                if !self.enabled.load(Ordering::Acquire) {
                    break 'pooled Err(Failure::Unavailable);
                }
                pool.prune(Instant::now());
                let mut reservation = self.membership.reserve(
                    uuid,
                    dest,
                    local_uuid,
                    local_vmac,
                    DirectRole::Outbound,
                    DIRECT_POOL_MAX_ENTRIES,
                );
                if matches!(reservation, Err(Refusal::Resources)) {
                    pool.evict_oldest();
                    reservation = self.membership.reserve(
                        uuid,
                        dest,
                        local_uuid,
                        local_vmac,
                        DirectRole::Outbound,
                        DIRECT_POOL_MAX_ENTRIES,
                    );
                }
                let Ok(reservation) = reservation else {
                    break 'pooled Err(Failure::Unavailable);
                };
                let member = reservation.commit_with_limits(limits, wait);
                pool.prune(Instant::now());
                let (pooled, worker) = PooledDirect::start(
                    socket.take().expect("the socket goes to its worker once"),
                    member,
                    physical,
                    #[cfg(feature = "sc-tls")]
                    self.intake.lock().unwrap().clone(),
                    local_limits,
                    heartbeat_ms,
                );
                let mut workers = self.workers.lock().unwrap();
                workers.retain(|task| !task.is_finished());
                workers.push(worker);
                pool.insert(dest, pooled.clone());
                Ok(pooled)
            };
            let pooled = match (pooled, socket) {
                (Ok(pooled), _) => pooled,
                (Err(failure), Some(ws)) => {
                    goodbye.say_goodbye(&ws, wait).await;
                    return Err(failure);
                }
                (Err(failure), None) => return Err(failure),
            };
            let result = pooled.member.egress.send_npdu(npdu, attributes).await;
            if result.is_ok() {
                self.note_direct_success(&uri).await;
            }
            return result;
        }
        Err(Failure::Unavailable)
    }
}

impl<W: WebSocketPort> Drop for DirectShared<W> {
    fn drop(&mut self) {
        self.disable();
    }
}

#[cfg(all(test, feature = "sc-tls"))]
#[path = "direct_membership_tls_tests.rs"]
mod direct_membership_tls_tests;

#[cfg(test)]
#[path = "direct_dial_close_tests.rs"]
mod direct_dial_close_tests;

#[cfg(test)]
#[path = "direct_discovery_pending_tests.rs"]
mod direct_discovery_pending_tests;
