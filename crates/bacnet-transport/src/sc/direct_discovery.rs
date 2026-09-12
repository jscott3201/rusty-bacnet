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
}

impl<W: WebSocketPort> DirectShared<W> {
    pub(crate) fn new() -> Self {
        Self {
            cache: Mutex::new(DirectUriCache::new()),
            pending: Mutex::new(HashMap::new()),
            dialer: Mutex::new(None),
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

    /// Dial each candidate URI in order, run the Connect handshake, and send
    /// the NPDU over the first direct connection that completes it, omitting
    /// both address parameters.
    ///
    /// Returns `Ok(())` on the first successful direct send. Returns `Err`
    /// when no dialer is configured, every URI fails, the handshake is
    /// rejected or times out, or the hub connection is no longer usable for
    /// ID allocation; the caller must fall back to hub delivery. Never
    /// mutates hub connection state besides consuming fresh message IDs,
    /// and never touches hub failover or reconnect state.
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
        // Materialize one direct frame per attempt under the hub ID counter
        // so direct and hub messages share unique IDs. The hub state itself
        // is only read, never transitioned, here. Each attempt first runs
        // the shared Connect-Request into Connect-Accept handshake on an
        // ephemeral probe carrying the hub connection's node identity; any
        // handshake failure drops the dialed socket and tries the next URI.
        for uri in uris {
            let dial_wait = Duration::from_millis(connect_timeout_ms.max(1));
            let direct_ws = match tokio::time::timeout(dial_wait, dialer(uri.clone())).await {
                Ok(Ok(ws)) => ws,
                _ => continue,
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
                Ok(Ok(())) => return Ok(()),
                _ => continue,
            }
        }
        Err(())
    }
}
