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
//! [`DIRECT_ACCEPT_MAX_CONNECTIONS`] concurrent accepted connections;
//! further TCP accepts are dropped while at cap. Each connection must
//! complete its Connect handshake within the configured connect timeout and
//! is closed after [`DIRECT_ACCEPT_IDLE_TIMEOUT`] without an inbound frame
//! by default. No hub, failover, discovery, or dial-out behavior changes.

use std::net::SocketAddr;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;

use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::port::{DataAttribute, ReceivedNpdu};
use crate::sc_frame::{
    decode_sc_message, encode_sc_message, first_must_understand_destination_option_marker,
    validate_connect_request, ScFunction, ScMessage, Vmac, BACNET_SC_DIRECT_SUBPROTOCOL,
    BROADCAST_VMAC,
};

use super::ScNodeTlsConfig;

/// Maximum concurrent accepted direct connections (owner-local bound).
///
/// Further TCP accepts are dropped while at cap; the peer observes a
/// refused connection and may retry later under its own local timing.
pub const DIRECT_ACCEPT_MAX_CONNECTIONS: usize = 16;

/// Idle timeout for an accepted direct connection (owner-local policy).
///
/// A connection with no inbound WebSocket frame for this long is closed
/// with a normal Close. Annex AB leaves initiation timing local; this
/// bound only reclaims idle sockets.
pub const DIRECT_ACCEPT_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// NPDU receive channel capacity for the listener.
///
/// Matches the hub transport's bounded channel; delivery uses `try_send`
/// and drops on full with a diagnostic, never blocking the accept loop.
const DIRECT_ACCEPT_NPDU_CHANNEL_CAPACITY: usize = 64;

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
    max_connections: usize,
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
            max_connections: DIRECT_ACCEPT_MAX_CONNECTIONS,
            max_bvlc_length: crate::sc_limits::DEFAULT_MAX_BVLC_LENGTH,
            max_apdu_length: 1476,
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

    /// Set the concurrent-connection cap (builder-style).
    ///
    /// Values above zero are honored; zero is replaced with one so the
    /// listener can always make progress for its first peer.
    pub fn with_max_connections(mut self, max: usize) -> Self {
        self.max_connections = max.max(1);
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
        let listener = TcpListener::bind(config.bind_addr)
            .await
            .map_err(|e| Error::Encoding(format!("direct accept bind failed: {e}")))?;
        let local_addr = listener.local_addr().map_err(|e| {
            Error::Encoding(format!("direct accept could not read local address: {e}"))
        })?;
        let (npdu_tx, npdu_rx) = mpsc::channel(DIRECT_ACCEPT_NPDU_CHANNEL_CAPACITY);
        let (shutdown, shutdown_rx) = watch::channel(false);
        let active = Arc::new(AtomicUsize::new(0));
        let task = tokio::spawn(accept_loop(
            listener,
            config,
            npdu_tx,
            shutdown_rx,
            Arc::clone(&active),
            ListenerStopped(shutdown.clone()),
        ));
        debug!("BACnet/SC direct listener on {local_addr}");
        Ok((
            Self {
                local_addr,
                accept_task: Some(task),
                shutdown,
                active,
            },
            npdu_rx,
        ))
    }

    /// The address the listener is bound to.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Number of currently active accepted connections (for tests).
    pub fn active_connections(&self) -> usize {
        self.active.load(Ordering::Relaxed)
    }

    /// Stop admission and await accept-loop cleanup.
    ///
    /// Signals accepted connections to close; they exit on their next
    /// frame or shutdown poll. This is forceful local shutdown, not the
    /// BACnet Disconnect sequence.
    pub async fn stop(&mut self) {
        self.shutdown.send_replace(true);
        if let Some(task) = self.accept_task.take() {
            task.abort();
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

async fn accept_loop(
    listener: TcpListener,
    config: DirectAcceptConfig,
    npdu_tx: mpsc::Sender<ReceivedNpdu>,
    mut shutdown: watch::Receiver<bool>,
    active: Arc<AtomicUsize>,
    _stopped: ListenerStopped,
) {
    loop {
        let accepted = tokio::select! {
            biased;
            _ = shutdown.changed() => break,
            accepted = listener.accept() => accepted,
        };
        let (tcp, peer_addr) = match accepted {
            Ok(v) => v,
            Err(e) => {
                warn!("direct accept error: {e}");
                continue;
            }
        };
        let Some(_guard) = AcceptGuard::acquire(&active, config.max_connections) else {
            warn!("direct accept at cap, refusing {peer_addr}");
            drop(tcp);
            continue;
        };
        let guard = _guard;
        let peer_config = config.clone();
        let peer_tx = npdu_tx.clone();
        let mut peer_shutdown = shutdown.clone();
        let peer_active = Arc::clone(&active);
        tokio::spawn(async move {
            let _guard = guard;
            let _active = peer_active;
            tokio::select! {
                _ = peer_shutdown.changed() => {},
                _ = serve_connection(tcp, peer_addr, peer_config, peer_tx) => {},
            }
        });
    }
    drop(listener);
}

async fn serve_connection(
    tcp: tokio::net::TcpStream,
    peer_addr: SocketAddr,
    config: DirectAcceptConfig,
    npdu_tx: mpsc::Sender<ReceivedNpdu>,
) {
    let tls_stream =
        match tokio::time::timeout(config.connect_timeout, config.tls.acceptor().accept(tcp)).await
        {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => {
                warn!("direct TLS handshake failed for {peer_addr}: {e}");
                return;
            }
            Err(_) => {
                debug!("direct TLS handshake timed out for {peer_addr}");
                return;
            }
        };
    let ws_stream = match tokio::time::timeout(
        config.connect_timeout,
        tokio_tungstenite::accept_hdr_async_with_config(
            tls_stream,
            direct_subprotocol_response,
            Some(crate::sc_limits::websocket(config.max_bvlc_length as usize)),
        ),
    )
    .await
    {
        Ok(Ok(ws)) => ws,
        Ok(Err(e)) => {
            warn!("direct WebSocket upgrade failed for {peer_addr}: {e}");
            return;
        }
        Err(_) => {
            debug!("direct WebSocket upgrade timed out for {peer_addr}");
            return;
        }
    };
    let (mut write, mut read) = ws_stream.split();
    let peer_vmac = match serve_handshake(&mut write, &mut read, &config, peer_addr).await {
        Some(vmac) => vmac,
        None => return,
    };
    serve_npdu_loop(
        &mut write, &mut read, &config, peer_addr, peer_vmac, &npdu_tx,
    )
    .await;
}

async fn serve_handshake<W>(
    write: &mut W,
    read: &mut W::Read,
    config: &DirectAcceptConfig,
    peer_addr: SocketAddr,
) -> Option<Vmac>
where
    W: DirectWs,
{
    let data = match tokio::time::timeout(config.connect_timeout, read.next_data()).await {
        Ok(Some(Ok(data))) => data,
        Ok(Some(Err(e))) => {
            warn!("direct handshake recv error from {peer_addr}: {e}");
            return None;
        }
        Ok(None) => return None,
        Err(_) => {
            debug!("direct handshake timed out for {peer_addr}");
            return None;
        }
    };
    if data.len() > config.max_bvlc_length as usize {
        warn!("direct handshake frame exceeds local Max-BVLC-Length, closing {peer_addr}");
        return None;
    }
    let msg = match decode_sc_message(&data) {
        Ok(msg) => msg,
        Err(e) => {
            warn!("direct handshake decode error from {peer_addr}: {e}");
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
    if peer_vmac == config.local_vmac {
        let nak = duplicate_vmac_nak(msg.message_id);
        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &nak);
        let _ = write.send_data(&buf).await;
        return None;
    }
    let accept = build_connect_accept(msg.message_id, config);
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &accept);
    if write.send_data(&buf).await.is_err() {
        return None;
    }
    debug!("direct handshake accepted {peer_addr} vmac={peer_vmac:02x?}");
    Some(peer_vmac)
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

fn duplicate_vmac_nak(message_id: u16) -> ScMessage {
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    let class = ErrorClass::COMMUNICATION.to_raw().to_be_bytes();
    let code = ErrorCode::NODE_DUPLICATE_VMAC.to_raw().to_be_bytes();
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

async fn serve_npdu_loop<W>(
    write: &mut W,
    read: &mut W::Read,
    config: &DirectAcceptConfig,
    peer_addr: SocketAddr,
    peer_vmac: Vmac,
    npdu_tx: &mpsc::Sender<ReceivedNpdu>,
) where
    W: DirectWs,
{
    loop {
        let next = tokio::time::timeout(config.idle_timeout, read.next_data()).await;
        let data = match next {
            Ok(Some(Ok(data))) => data,
            Ok(Some(Err(e))) => {
                warn!("direct recv error from {peer_addr}: {e}");
                return;
            }
            Ok(None) => return,
            Err(_) => {
                debug!("direct idle timeout, closing {peer_addr}");
                let _ = write.send_close().await;
                return;
            }
        };
        if data.len() > config.max_bvlc_length as usize {
            warn!("direct frame exceeds local Max-BVLC-Length, dropping from {peer_addr}");
            continue;
        }
        let msg = match decode_sc_message(&data) {
            Ok(msg) => msg,
            Err(e) => {
                warn!("direct decode error from {peer_addr}: {e}");
                continue;
            }
        };
        match msg.function {
            ScFunction::EncapsulatedNpdu => {
                match direct_must_understand_decision(&msg, &data) {
                    DirectMuDecision::Pass => {}
                    DirectMuDecision::Drop => continue,
                    DirectMuDecision::Nak(nak) => {
                        let mut buf = BytesMut::new();
                        encode_sc_message(&mut buf, &nak);
                        if write.send_data(&buf).await.is_err() {
                            warn!("direct destination-option NAK send error for {peer_addr}");
                        }
                        continue;
                    }
                }
                if let Some(npdu) = direct_npdu(&msg, config) {
                    let received = ReceivedNpdu {
                        npdu,
                        source_mac: MacAddr::from_slice(&peer_vmac),
                        link_layer_group: false,
                        data_attributes: msg
                            .data_options
                            .iter()
                            .map(|option| DataAttribute {
                                option_type: option.option_type,
                                must_understand: option.must_understand,
                                data: option.data.clone(),
                            })
                            .collect(),
                        reply_tx: None,
                    };
                    if npdu_tx.try_send(received).is_err() {
                        warn!("direct NPDU channel full, dropping from {peer_addr}");
                    }
                }
            }
            ScFunction::DisconnectRequest => {
                let ack = ScMessage {
                    function: ScFunction::DisconnectAck,
                    message_id: msg.message_id,
                    originating_vmac: None,
                    destination_vmac: None,
                    dest_options: Vec::new(),
                    data_options: Vec::new(),
                    payload: Bytes::new(),
                };
                let mut buf = BytesMut::new();
                encode_sc_message(&mut buf, &ack);
                let _ = write.send_data(&buf).await;
                return;
            }
            ScFunction::DisconnectAck => return,
            _ => continue,
        }
    }
}

/// Must-Understand Destination Option decision for an inbound direct NPDU.
///
/// Hub/node parity (`sc/data_attributes.rs` +
/// `sc/rejection.rs::unsupported_must_understand_destination_option`): an
/// Encapsulated-NPDU carrying any Must-Understand Destination Option is
/// never delivered. Unicast-shaped frames answer with a connection-local
/// BVLC-Result NAK (`COMMUNICATION`/`HEADER_NOT_UNDERSTOOD` carrying the
/// wire marker); broadcast-shaped frames drop silently. A frame whose wire
/// marker cannot be recovered also drops silently without a NAK, matching
/// the hub gate. Runs before the direct shape/payload gates so an MU
/// option is never lost to an earlier silent drop.
enum DirectMuDecision {
    /// No unsupported MU Destination Option: continue through the direct gates.
    Pass,
    /// Drop without delivery and without a NAK.
    Drop,
    /// Drop without delivery after sending this NAK on the direct socket.
    Nak(ScMessage),
}

fn direct_must_understand_decision(msg: &ScMessage, wire: &[u8]) -> DirectMuDecision {
    if msg.function != ScFunction::EncapsulatedNpdu {
        return DirectMuDecision::Pass;
    }
    if msg
        .dest_options
        .iter()
        .all(|option| !option.must_understand)
    {
        return DirectMuDecision::Pass;
    }
    if msg.destination_vmac == Some(BROADCAST_VMAC) {
        return DirectMuDecision::Drop;
    }
    match first_must_understand_destination_option_marker(wire) {
        Some(marker) => DirectMuDecision::Nak(direct_must_understand_nak(
            msg.message_id,
            marker,
            msg.originating_vmac,
        )),
        None => {
            warn!("direct NPDU with unsupported Destination Option lost its wire marker, dropping");
            DirectMuDecision::Drop
        }
    }
}

/// Connection-local BVLC-Result NAK for an unsupported MU Destination Option.
///
/// Mirrors `sc/data_attributes.rs::build_bvlc_result_nak` for the direct
/// socket: the NAK answers on the same connection, so a well-formed direct
/// NPDU (both VMACs omitted) yields a peer-addressed NAK with neither
/// address parameter, exactly like the hub mapping with an absent origin.
fn direct_must_understand_nak(
    message_id: u16,
    error_header_marker: u8,
    destination_vmac: Option<Vmac>,
) -> ScMessage {
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    let class = ErrorClass::COMMUNICATION.to_raw().to_be_bytes();
    let code = ErrorCode::HEADER_NOT_UNDERSTOOD.to_raw().to_be_bytes();
    ScMessage {
        function: ScFunction::Result,
        message_id,
        originating_vmac: None,
        destination_vmac,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(vec![
            ScFunction::EncapsulatedNpdu.to_raw(),
            0x01,
            error_header_marker,
            class[0],
            class[1],
            code[0],
            code[1],
        ]),
    }
}

/// Direct NPDU admission: unicast only with both addresses omitted.
///
/// Returns the NPDU bytes when the frame is a well-formed direct
/// Encapsulated-NPDU within local limits; otherwise `None` and the frame
/// is dropped without delivery or state change.
fn direct_npdu(msg: &ScMessage, config: &DirectAcceptConfig) -> Option<Bytes> {
    if msg.originating_vmac.is_some() || msg.destination_vmac.is_some() {
        return None;
    }
    if msg.payload.is_empty() {
        return None;
    }
    if msg.payload.len() > config.max_apdu_length as usize {
        return None;
    }
    Some(msg.payload.clone())
}

/// Minimal WebSocket surface used by the acceptor handshake and NPDU loop.
trait DirectWs {
    type Read: DirectWsRead;
    async fn send_data(&mut self, data: &[u8]) -> Result<(), ()>;
    async fn send_close(&mut self) -> Result<(), ()>;
}

trait DirectWsRead {
    async fn next_data(&mut self) -> Option<Result<Vec<u8>, String>>;
}

type TlsWsStream =
    tokio_tungstenite::WebSocketStream<tokio_rustls::server::TlsStream<tokio::net::TcpStream>>;

impl DirectWs
    for futures_util::stream::SplitSink<TlsWsStream, tokio_tungstenite::tungstenite::Message>
{
    type Read = futures_util::stream::SplitStream<TlsWsStream>;
    async fn send_data(&mut self, data: &[u8]) -> Result<(), ()> {
        self.send(tokio_tungstenite::tungstenite::Message::Binary(
            data.to_vec().into(),
        ))
        .await
        .map_err(|_| ())
    }
    async fn send_close(&mut self) -> Result<(), ()> {
        self.send(tokio_tungstenite::tungstenite::Message::Close(None))
            .await
            .map_err(|_| ())
    }
}

impl DirectWsRead for futures_util::stream::SplitStream<TlsWsStream> {
    async fn next_data(&mut self) -> Option<Result<Vec<u8>, String>> {
        loop {
            match self.next().await {
                Some(Ok(tokio_tungstenite::tungstenite::Message::Binary(data))) => {
                    return Some(Ok(data.to_vec()));
                }
                Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))) => return None,
                Some(Ok(
                    tokio_tungstenite::tungstenite::Message::Ping(_)
                    | tokio_tungstenite::tungstenite::Message::Pong(_),
                )) => continue,
                Some(Ok(_)) => return Some(Err("non-binary".into())),
                Some(Err(e)) => return Some(Err(e.to_string())),
                None => return None,
            }
        }
    }
}

#[cfg(test)]
#[path = "direct_accept_tests.rs"]
mod direct_accept_tests;
