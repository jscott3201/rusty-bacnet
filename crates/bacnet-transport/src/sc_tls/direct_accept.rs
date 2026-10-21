//! Opt-in direct-connection listener (accept side, Refs #615).
//!
//! Disabled by default: no socket is bound unless [`DirectListener::start`]
//! runs. Enabled, the listener binds, accepts TLS peers with node
//! operational credentials, selects only the direct subprotocol, runs the
//! Connect-Request into Connect-Accept exchange with the existing hub
//! validation, and delivers inbound direct NPDUs as [`ReceivedNpdu`].
//!
//! Source grounding paraphrases the local Standard 135-2020 Annex AB as
//! located through prior direct slices (Annex AB, printed pp1377-1410): a
//! direct connection is established with the direct subprotocol upgrade
//! followed by the Connect identity and length exchange before NPDU
//! traffic, and the initiating peer waits for the matching Accept under its
//! existing connect wait. Only the request plus accept exchange is required
//! inbound. A direct WebSocket carries unicast NPDUs with both address
//! parameters omitted after the handshake; initiation and re-initiation
//! timing stays a local matter. TLS uses the same operational-certificate
//! policy as the dial path with the roles reversed (explicit CA trust,
//! mandatory peer verification, TLS 1.3 only). Peers without a trusted
//! operational certificate fail the TLS handshake before any BACnet
//! exchange. Response messages are never answered, per the response rule.
//!
//! Owner-local bounds (not wire conformance): at most
//! [`DIRECT_ACCEPT_MAX_ESTABLISHED_PEERS`] established accepted peers, the
//! same number of pending handshakes and twice that many physical sockets.
//! TCP accepts beyond pending/physical capacity are dropped. Each connection must
//! complete its Connect handshake within the configured connect timeout and
//! is closed after [`DIRECT_ACCEPT_IDLE_TIMEOUT`] without an inbound frame
//! by default. No hub, failover, discovery, or dial-out behavior changes.

use std::net::SocketAddr;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;

use crate::sc::direct_membership::{DirectMembership, DirectRole, Membership, Refusal};
use bacnet_types::error::Error;
use bytes::{Bytes, BytesMut};
use futures_util::StreamExt;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::port::ReceivedNpdu;
use crate::sc::npdu_admission::{
    ScNpduAdmission, ScNpduAdmissionPolicy, ScNpduDropCounts, SC_NPDU_QUEUE_CAPACITY,
};
use crate::sc_frame::{
    decode_sc_message, encode_sc_message, validate_connect_request, ScFunction, ScMessage, Vmac,
    BACNET_SC_DIRECT_SUBPROTOCOL,
};
use crate::warn_throttle::WarnThrottle;

use super::ScNodeTlsConfig;

/// Default maximum established accepted direct peers (owner-local bound).
/// Pending handshakes are separately capped at this limit, and physical
/// accepted sockets (including retiring generations) at twice this limit.
pub const DIRECT_ACCEPT_MAX_ESTABLISHED_PEERS: usize = 16;

/// Idle timeout for an accepted direct connection (owner-local policy).
///
/// A connection with no inbound WebSocket frame for this long is closed
/// with a normal Close. Annex AB leaves initiation timing local; this
/// bound only reclaims idle sockets.
pub const DIRECT_ACCEPT_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// NPDU receive channel capacity for the listener.
///
/// The hub transport's capacity, which the shared admission checks; delivery
/// uses `try_send` and drops on full with a throttled warning, never blocking
/// the accept loop.
const DIRECT_ACCEPT_NPDU_CHANNEL_CAPACITY: usize = SC_NPDU_QUEUE_CAPACITY;

/// Default Connect-handshake wait for an accepted peer (owner-local).
const DIRECT_ACCEPT_DEFAULT_CONNECT_TIMEOUT_MS: u64 = 10_000;

/// Opt-in direct-listener configuration (default off: no listener exists).
///
/// Construct with [`DirectAcceptConfig::new`] and pass to
/// [`DirectListener::start`]. The local VMAC must be neither all-zero nor
/// broadcast and the device UUID must be nonzero; startup rejects reserved
/// identity before binding, like the hub transport.
#[derive(Clone, Debug)]
pub struct DirectAcceptConfig {
    bind_addr: SocketAddr,
    local_vmac: Vmac,
    device_uuid: [u8; 16],
    tls: ScNodeTlsConfig,
    connect_timeout: Duration,
    idle_timeout: Duration,
    max_established_peers: usize,
    npdu_admission_policy: ScNpduAdmissionPolicy,
    max_bvlc_length: u16,
    max_apdu_length: u16,
}

impl DirectAcceptConfig {
    pub(crate) fn matches_identity(&self, vmac: Vmac, uuid: [u8; 16]) -> bool {
        self.local_vmac == vmac && self.device_uuid == uuid
    }

    /// Configure an opt-in direct listener.
    ///
    /// `bind_addr` is the local TCP address to bind (e.g.
    /// `127.0.0.1:0` for a loopback test port). `local_vmac` and
    /// `device_uuid` are advertised in Connect-Accept exactly as
    /// configured; the caller provisions and durably reuses the UUID.
    /// `tls` supplies both the server identity and the mandatory
    /// operational-client trust.
    pub fn new(
        bind_addr: SocketAddr,
        local_vmac: Vmac,
        device_uuid: [u8; 16],
        tls: ScNodeTlsConfig,
    ) -> Self {
        Self {
            bind_addr,
            local_vmac,
            device_uuid,
            tls,
            connect_timeout: Duration::from_millis(DIRECT_ACCEPT_DEFAULT_CONNECT_TIMEOUT_MS),
            idle_timeout: DIRECT_ACCEPT_IDLE_TIMEOUT,
            max_established_peers: DIRECT_ACCEPT_MAX_ESTABLISHED_PEERS,
            npdu_admission_policy: ScNpduAdmissionPolicy::default(),
            max_bvlc_length: crate::sc_limits::DEFAULT_MAX_BVLC_LENGTH,
            max_apdu_length: crate::sc_limits::LOCAL_RECEIVE_NPDU_CAPACITY,
        }
    }

    /// Set the Connect-handshake wait (builder-style).
    pub fn with_connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    /// Set the per-connection idle timeout (builder-style).
    pub fn with_idle_timeout(mut self, timeout: Duration) -> Self {
        self.idle_timeout = timeout;
        self
    }

    /// Set the established accepted-peer limit M (builder-style).
    ///
    /// Pending handshakes are bounded by M; physical sockets, including
    /// retiring replacements, by 2M. Startup rejects overflow before binding.
    ///
    /// Values above zero are honored; zero is replaced with one so the
    /// listener can always make progress for its first peer.
    pub fn with_max_established_peers(mut self, max: usize) -> Self {
        self.max_established_peers = max.max(1);
        self
    }

    /// Set the per-origin queued-NPDU quota (builder-style).
    ///
    /// Defaults to 32 per peer VMAC in the 256-item listener queue, as on the
    /// hub path (#1712). [`DirectListener::start`] validates
    /// the limit before binding and rejects zero or above-capacity values;
    /// the aggregate cap stays the fixed channel capacity. The peer VMAC is
    /// a scheduling key, never identity.
    pub fn with_npdu_per_origin_limit(mut self, limit: usize) -> Self {
        self.npdu_admission_policy.per_origin_limit = limit;
        self
    }

    /// Set the per-peer limit on inbound network-layer messages
    /// (builder-style; default [`ScControlRateLimit::default`](crate::sc::ScControlRateLimit::default), #518).
    /// Each verified peer certificate has one budget, whatever VMAC it uses
    /// and however often it reconnects. A message over budget is dropped silently and counted in
    /// `control_rate_drops`; APDUs are never charged. [`DirectListener::start`]
    /// validates it before binding.
    pub fn with_control_rate_limit(mut self, limit: crate::sc::ScControlRateLimit) -> Self {
        self.npdu_admission_policy.control_rate = limit;
        self
    }
}

/// Opt-in direct-connection listener.
///
/// Created by [`DirectListener::start`]; dropping aborts the accept loop
/// and signals accepted connections to close. Use [`DirectListener::stop`]
/// to await accept-loop cleanup. Accepted NPDUs arrive on the returned
/// channel as [`ReceivedNpdu`] with `source_mac` set to the peer VMAC
/// learned in its Connect-Request and `link_layer_group` always false
/// (direct connections carry unicast only).
pub struct DirectListener {
    local_addr: SocketAddr,
    accept_task: Option<JoinHandle<()>>,
    shutdown: watch::Sender<bool>,
    active: Arc<AtomicUsize>,
    #[cfg(test)]
    pending: Arc<AtomicUsize>,
    #[cfg(test)]
    membership: Arc<DirectMembership>,
    npdu_admission: Arc<ScNpduAdmission>,
}

/// Invalidate registrations even if the accept task is cancelled before its
/// first poll, exits unexpectedly, or unwinds. No separate monitor task.
struct ListenerStopped(watch::Sender<bool>);

impl Drop for ListenerStopped {
    fn drop(&mut self) {
        self.0.send_replace(true);
    }
}

impl DirectListener {
    pub(crate) fn shutdown_signal(&self) -> watch::Sender<bool> {
        self.shutdown.clone()
    }

    pub(crate) fn shutdown_status(&self) -> watch::Receiver<bool> {
        self.shutdown.subscribe()
    }

    /// Start an opt-in direct listener.
    ///
    /// Binds `config.bind_addr`, begins accepting direct peers on a
    /// background task, and returns the listener plus the inbound NPDU
    /// receiver. No hub, failover, discovery, or dial-out state changes.
    /// Without this call no socket exists and inbound direct dials are
    /// refused, preserving current default behavior bit-for-bit.
    pub async fn start(
        config: DirectAcceptConfig,
    ) -> Result<(Self, mpsc::Receiver<ReceivedNpdu>), Error> {
        Self::start_shared(config, Arc::new(DirectMembership::default())).await
    }

    pub(crate) async fn start_shared(
        config: DirectAcceptConfig,
        membership: Arc<DirectMembership>,
    ) -> Result<(Self, mpsc::Receiver<ReceivedNpdu>), Error> {
        config.max_established_peers.checked_mul(2).ok_or_else(|| {
            Error::Encoding("direct accepted-peer limit overflows physical socket capacity".into())
        })?;
        if config.device_uuid == [0; 16] {
            return Err(Error::Encoding(
                "direct accept device UUID is all-zero".into(),
            ));
        }
        if config.local_vmac == [0; 6] || config.local_vmac == [0xFF; 6] {
            return Err(Error::Encoding(
                "direct accept VMAC is zero or broadcast".into(),
            ));
        }
        config.npdu_admission_policy.validate()?;
        let npdu_admission = Arc::new(ScNpduAdmission::new(config.npdu_admission_policy));
        let listener = TcpListener::bind(config.bind_addr)
            .await
            .map_err(|e| Error::Encoding(format!("direct accept bind failed: {e}")))?;
        let local_addr = listener.local_addr().map_err(|e| {
            Error::Encoding(format!("direct accept could not read local address: {e}"))
        })?;
        let (npdu_tx, npdu_rx) = mpsc::channel(DIRECT_ACCEPT_NPDU_CHANNEL_CAPACITY);
        let (shutdown, shutdown_rx) = watch::channel(false);
        let active = Arc::new(AtomicUsize::new(0));
        let pending = Arc::new(AtomicUsize::new(0));
        let task = tokio::spawn(accept_loop(AcceptLoop {
            listener,
            config,
            shutdown: shutdown_rx,
            active: Arc::clone(&active),
            stopped: ListenerStopped(shutdown.clone()),
            pending: Arc::clone(&pending),
            shared: ListenerShared {
                npdu_tx,
                npdu_admission: Arc::clone(&npdu_admission),
                membership: Arc::clone(&membership),
                warnings: Arc::default(),
            },
        }));
        debug!("BACnet/SC direct listener on {local_addr}");
        Ok((
            Self {
                local_addr,
                accept_task: Some(task),
                shutdown,
                active,
                #[cfg(test)]
                pending,
                #[cfg(test)]
                membership,
                npdu_admission,
            },
            npdu_rx,
        ))
    }

    /// The address the listener is bound to.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Physical accepted sockets, including pending and retiring generations.
    /// May reach twice the configured established accepted-peer limit.
    pub fn active_connections(&self) -> usize {
        self.active.load(Ordering::Relaxed)
    }

    /// Read this listener's NPDU drop counts, including after [`Self::stop`].
    ///
    /// Count-only and saturating: per-origin fairness drops, aggregate-full
    /// drops, and closed-receiver drops in Closed > fairness > Full order.
    /// No per-VMAC statistics or wire response is created.
    pub fn npdu_drop_counts(&self) -> ScNpduDropCounts {
        self.npdu_admission.drop_counts()
    }

    /// Stop admission and await accept-loop cleanup.
    ///
    /// Signals accepted connections to close; they exit on their next
    /// frame or shutdown poll. This is forceful local shutdown, not the
    /// BACnet Disconnect sequence.
    pub async fn stop(&mut self) {
        self.shutdown.send_replace(true);
        if let Some(task) = self.accept_task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for DirectListener {
    fn drop(&mut self) {
        self.shutdown.send_replace(true);
        if let Some(task) = self.accept_task.take() {
            task.abort();
        }
    }
}

struct AcceptGuard {
    active: Arc<AtomicUsize>,
}

impl AcceptGuard {
    fn acquire(active: &Arc<AtomicUsize>, cap: usize) -> Option<Self> {
        let mut current = active.load(Ordering::Relaxed);
        loop {
            if current >= cap {
                return None;
            }
            match active.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    return Some(Self {
                        active: Arc::clone(active),
                    });
                }
                Err(observed) => current = observed,
            }
        }
    }
}

impl Drop for AcceptGuard {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::Relaxed);
    }
}

#[allow(clippy::result_large_err)]
fn direct_subprotocol_response(
    request: &tokio_tungstenite::tungstenite::handshake::server::Request,
    mut response: tokio_tungstenite::tungstenite::handshake::server::Response,
) -> Result<
    tokio_tungstenite::tungstenite::handshake::server::Response,
    tokio_tungstenite::tungstenite::handshake::server::ErrorResponse,
> {
    let offered = request
        .headers()
        .get("Sec-WebSocket-Protocol")
        .and_then(|v| v.to_str().ok())
        .map(|s| {
            s.split(',')
                .any(|p| p.trim() == BACNET_SC_DIRECT_SUBPROTOCOL)
        })
        .unwrap_or(false);
    if !offered {
        return Err(tokio_tungstenite::tungstenite::http::Response::builder()
            .status(tokio_tungstenite::tungstenite::http::StatusCode::BAD_REQUEST)
            .body(Some(format!(
                "BACnet/SC direct requires WebSocket subprotocol {BACNET_SC_DIRECT_SUBPROTOCOL}"
            )))
            .expect("static WebSocket error response is valid"));
    }
    response.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        BACNET_SC_DIRECT_SUBPROTOCOL
            .parse()
            .expect("static direct subprotocol is a valid header value"),
    );
    Ok(response)
}

/// Everything the accept loop owns for the life of the listener.
struct AcceptLoop {
    listener: TcpListener,
    config: DirectAcceptConfig,
    shutdown: watch::Receiver<bool>,
    active: Arc<AtomicUsize>,
    /// Signals listener stop to the handle when the loop ends.
    stopped: ListenerStopped,
    pending: Arc<AtomicUsize>,
    shared: ListenerShared,
}

async fn accept_loop(state: AcceptLoop) {
    let AcceptLoop {
        listener,
        config,
        mut shutdown,
        active,
        stopped: _stopped,
        pending,
        shared,
    } = state;
    let mut peers = tokio::task::JoinSet::new();
    loop {
        let accepted = tokio::select! {
            biased;
            _ = shutdown.changed() => break,
            Some(_) = peers.join_next(), if !peers.is_empty() => continue,
            accepted = listener.accept() => accepted,
        };
        let (tcp, peer_addr) = match accepted {
            Ok(v) => v,
            Err(e) => {
                warn!("direct accept error: {e}");
                continue;
            }
        };
        let Some(_guard) = AcceptGuard::acquire(&active, config.max_established_peers * 2) else {
            crate::warn_throttled!(
                shared.warnings.connection,
                "direct accept at cap, refusing {peer_addr}"
            );
            drop(tcp);
            continue;
        };
        let Some(pending_guard) = AcceptGuard::acquire(&pending, config.max_established_peers)
        else {
            drop(tcp);
            continue;
        };
        let guard = _guard;
        crate::sc_tls::disable_nagle(&tcp);
        let peer_config = config.clone();
        let peer_shared = shared.clone();
        let mut peer_shutdown = shutdown.clone();
        let peer_active = Arc::clone(&active);
        peers.spawn(async move {
            let _guard = guard;
            let _active = peer_active;
            tokio::select! {
                biased;
                _ = async { if !*peer_shutdown.borrow_and_update() { let _ = peer_shutdown.changed().await; } } => {},
                _ = serve_connection(tcp, peer_addr, peer_config, peer_shared, pending_guard) => {},
            }
        });
    }
    drop(listener);
    peers.shutdown().await;
}

// The chain comes only from rustls after a successful verifying handshake.
pub(super) fn verified_leaf_sha256(
    chain: Option<&[rustls::pki_types::CertificateDer<'_>]>,
) -> Option<[u8; 32]> {
    let leaf = chain?.first()?;
    Some(
        aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, leaf.as_ref())
            .as_ref()
            .try_into()
            .expect("SHA-256 output length"),
    )
}

// Each stage's future is boxed, once per connection, as in the hub's
// serve_connection: unboxed, the TLS accept, the tokio-tungstenite upgrade and
// the peer loop all put their whole futures in this debug-build frame (#953).
async fn serve_connection(
    tcp: tokio::net::TcpStream,
    peer_addr: SocketAddr,
    config: DirectAcceptConfig,
    shared: ListenerShared,
    pending: AcceptGuard,
) {
    let ListenerShared {
        npdu_tx,
        npdu_admission,
        membership,
        warnings,
    } = shared;
    let tls_deadline = tokio::time::Instant::now() + config.connect_timeout;
    let tls_stream = match super::boxed(|| {
        let accept = config.tls.acceptor().accept(tcp).into_fallible();
        tokio::time::timeout_at(tls_deadline, accept)
    })
    .await
    {
        Ok(Ok(s)) => s,
        Ok(Err((e, tcp))) => {
            crate::warn_throttled!(
                warnings.connection,
                "direct TLS handshake failed for {peer_addr}: {e}"
            );
            crate::tls_reject::close_after_alert(tcp, tls_deadline).await;
            return;
        }
        Err(_) => {
            debug!("direct TLS handshake timed out for {peer_addr}");
            return;
        }
    };
    let Some(leaf_sha256) = verified_leaf_sha256(tls_stream.get_ref().1.peer_certificates()) else {
        warn!("direct TLS peer has no verified leaf certificate");
        return;
    };
    let ws_stream = match super::boxed(|| {
        tokio::time::timeout(
            config.connect_timeout,
            tokio_tungstenite::accept_hdr_async_with_config(
                tls_stream,
                direct_subprotocol_response,
                Some(crate::sc_limits::websocket(config.max_bvlc_length as usize)),
            ),
        )
    })
    .await
    {
        Ok(Ok(ws)) => ws,
        Ok(Err(e)) => {
            crate::warn_throttled!(
                warnings.connection,
                "direct WebSocket upgrade failed for {peer_addr}: {e}"
            );
            return;
        }
        Err(_) => {
            debug!("direct WebSocket upgrade timed out for {peer_addr}");
            return;
        }
    };
    let (mut write, mut read) = ws_stream.split();
    let (member, _limits) = match super::boxed(|| {
        tokio::time::timeout(
            config.connect_timeout,
            serve_handshake(
                &mut write,
                &mut read,
                &config,
                peer_addr,
                &membership,
                &warnings,
            ),
        )
    })
    .await
    {
        Ok(Some(member)) => member,
        _ => return,
    };
    drop(pending);
    let identity = crate::port::DirectScIdentity::verified(leaf_sha256, member.generation);
    // Socket task owns retirement even if a short-lived send admission holds
    // another strong reference. Capabilities themselves keep only a Weak.
    let _retire = RetireMember(&member);
    let response = crate::port::DirectResponse::new(&member, identity);
    let mut responses = member.take_writes();
    super::boxed(|| {
        serve_npdu_loop(
            &mut write,
            &mut read,
            &config,
            AdmittedDirectPeer {
                address: peer_addr,
                member: &member,
                identity,
                response,
                warnings: &warnings,
            },
            &npdu_tx,
            &npdu_admission,
            &mut responses,
        )
    })
    .await;
}

async fn serve_handshake<W>(
    write: &mut W,
    read: &mut W::Read,
    config: &DirectAcceptConfig,
    peer_addr: SocketAddr,
    membership: &Arc<DirectMembership>,
    warnings: &DirectWarnings,
) -> Option<(Arc<Membership>, (u16, u16))>
where
    W: DirectWs,
{
    let data = match tokio::time::timeout(config.connect_timeout, read.next_data()).await {
        Ok(Some(Ok(data))) => data,
        Ok(Some(Err(e))) => {
            crate::warn_throttled!(
                warnings.connection,
                "direct handshake recv error from {peer_addr}: {e}"
            );
            return None;
        }
        Ok(None) => return None,
        Err(_) => {
            debug!("direct handshake timed out for {peer_addr}");
            return None;
        }
    };
    if data.len() > config.max_bvlc_length as usize {
        crate::warn_throttled!(
            warnings.connection,
            "direct handshake frame exceeds local Max-BVLC-Length, closing {peer_addr}"
        );
        return None;
    }
    let msg = match decode_sc_message(&data) {
        Ok(msg) => msg,
        Err(e) => {
            crate::warn_throttled!(
                warnings.connection,
                "direct handshake decode error from {peer_addr}: {e}"
            );
            return None;
        }
    };
    if msg.function != ScFunction::ConnectRequest {
        debug!("direct handshake expected Connect-Request from {peer_addr}, closing");
        return None;
    }
    match validate_connect_request(&msg, &data) {
        Ok(()) => {}
        Err(Some(nak)) => {
            let _ = write.send_data(&nak).await;
            return None;
        }
        Err(None) => return None,
    }
    let mut peer_vmac = [0u8; 6];
    peer_vmac.copy_from_slice(&msg.payload[0..6]);
    let mut peer_uuid = [0; 16];
    peer_uuid.copy_from_slice(&msg.payload[6..22]);
    let reservation = match membership.reserve(
        peer_uuid,
        peer_vmac,
        config.device_uuid,
        config.local_vmac,
        DirectRole::Accepted,
        config.max_established_peers,
    ) {
        Ok(reservation) => reservation,
        Err(refusal) => {
            let nak = admission_nak(msg.message_id, refusal);
            let mut buf = BytesMut::new();
            encode_sc_message(&mut buf, &nak);
            let _ = write.send_data(&buf).await;
            return None;
        }
    };
    let accept = build_connect_accept(msg.message_id, config);
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &accept);
    if write.send_data(&buf).await.is_err() {
        return None;
    }
    debug!("direct handshake accepted {peer_addr} vmac={peer_vmac:02x?}");
    // No await between successful Accept and publication: cancellation cannot
    // expose a successful but unregistered contender at this boundary.
    Some((
        reservation.commit_with_limits(
            (
                u16::from_be_bytes([msg.payload[22], msg.payload[23]]),
                u16::from_be_bytes([msg.payload[24], msg.payload[25]]),
            ),
            config.connect_timeout,
        ),
        (
            u16::from_be_bytes([msg.payload[22], msg.payload[23]]),
            u16::from_be_bytes([msg.payload[24], msg.payload[25]]),
        ),
    ))
}

fn build_connect_accept(message_id: u16, config: &DirectAcceptConfig) -> ScMessage {
    let mut payload = Vec::with_capacity(26);
    payload.extend_from_slice(&config.local_vmac);
    payload.extend_from_slice(&config.device_uuid);
    payload.extend_from_slice(&config.max_bvlc_length.to_be_bytes());
    payload.extend_from_slice(&config.max_apdu_length.to_be_bytes());
    ScMessage {
        function: ScFunction::ConnectAccept,
        message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(payload),
    }
}

fn admission_nak(message_id: u16, refusal: Refusal) -> ScMessage {
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    let (class, code) = match refusal {
        Refusal::DuplicateVmac => (ErrorClass::COMMUNICATION, ErrorCode::NODE_DUPLICATE_VMAC),
        Refusal::Resources | Refusal::Busy => (ErrorClass::RESOURCES, ErrorCode::OTHER),
    };
    let class = class.to_raw().to_be_bytes();
    let code = code.to_raw().to_be_bytes();
    ScMessage {
        function: ScFunction::Result,
        message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(vec![
            ScFunction::ConnectRequest.to_raw(),
            0x01,
            0x00,
            class[0],
            class[1],
            code[0],
            code[1],
        ]),
    }
}

/// Immutable context for this task, never looked up again by claimed VMAC.
struct AdmittedDirectPeer<'a> {
    address: SocketAddr,
    member: &'a Membership,
    identity: crate::port::DirectScIdentity,
    response: crate::port::DirectResponse,
    warnings: &'a DirectWarnings,
}

/// The throttles on one listener's warnings about its peers (#1950), shared
/// by all its connections, so a peer that floods bad frames or reconnects
/// over and over warns once per
/// [`WARNING_INTERVAL`](crate::warn_throttle::WARNING_INTERVAL) for each,
/// and the rest log at debug.
#[derive(Debug, Default)]
pub(super) struct DirectWarnings {
    /// Connections refused, or lost before the handshake completed.
    pub(super) connection: WarnThrottle,
    /// Frames dropped, and reads failed, on established connections.
    pub(super) frame: WarnThrottle,
}

/// What every connection of one listener shares.
#[derive(Clone)]
struct ListenerShared {
    npdu_tx: mpsc::Sender<ReceivedNpdu>,
    npdu_admission: Arc<ScNpduAdmission>,
    membership: Arc<DirectMembership>,
    warnings: Arc<DirectWarnings>,
}

struct RetireMember<'a>(&'a Membership);
impl Drop for RetireMember<'_> {
    fn drop(&mut self) {
        self.0.retire();
    }
}

use crate::sc::direct_receive::{direct_must_understand_decision, DirectMuDecision};
fn direct_npdu(msg: &ScMessage, config: &DirectAcceptConfig) -> Option<Bytes> {
    crate::sc::direct_receive::direct_npdu(msg, config.max_apdu_length)
}

#[path = "direct_response_loop.rs"]
mod response_loop;
use response_loop::serve_npdu_loop;

#[path = "direct_socket.rs"]
mod socket;
use socket::{DirectFrame, DirectWs, DirectWsRead};

#[cfg(test)]
#[path = "direct_accept_tests.rs"]
pub(crate) mod direct_accept_tests;

#[cfg(test)]
#[path = "rb08_direct_accept_provenance_tests.rs"]
mod rb08_direct_accept_provenance_tests;

#[cfg(test)]
#[path = "rb11_direct_accept_fairness_tests.rs"]
mod rb11_direct_accept_fairness_tests;
