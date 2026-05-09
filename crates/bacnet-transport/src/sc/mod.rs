//! BACnet/SC (Secure Connect) transport per ASHRAE 135-2020 Annex AB.
//!
//! Hub-and-spoke topology over WebSocket + TLS 1.3.
//! The actual WebSocket I/O is abstracted behind the [`WebSocketPort`] trait
//! so the connection state machine can be tested without a TLS stack.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::{Bytes, BytesMut};
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::port::{ReceivedNpdu, TransportPort};
use crate::sc_frame::{
    decode_sc_message, encode_sc_message, is_broadcast_vmac, ScFunction, ScMessage, Vmac,
    BROADCAST_VMAC,
};
use bacnet_types::error::Error;
use bacnet_types::MacAddr;

// ---------------------------------------------------------------------------
// WebSocket abstraction
// ---------------------------------------------------------------------------

/// Abstraction over a WebSocket connection for BACnet/SC.
///
/// Implementations wrap the platform WebSocket driver (e.g. `tokio-tungstenite`).
/// A loopback implementation is provided for testing.
pub trait WebSocketPort: Send + Sync + 'static {
    /// Send a binary WebSocket message.
    fn send(&self, data: &[u8]) -> impl std::future::Future<Output = Result<(), Error>> + Send;
    /// Receive a binary WebSocket message. Blocks until a message is available.
    fn recv(&self) -> impl std::future::Future<Output = Result<Vec<u8>, Error>> + Send;
}

// ---------------------------------------------------------------------------
// Connection state
// ---------------------------------------------------------------------------

/// BACnet/SC connection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScConnectionState {
    /// Not connected.
    Disconnected,
    /// Connect-Request sent, waiting for Connect-Accept.
    Connecting,
    /// Connected and operational.
    Connected,
    /// Disconnect requested.
    Disconnecting,
}

/// BACnet/SC hub connection manager.
pub struct ScConnection {
    pub state: ScConnectionState,
    pub local_vmac: Vmac,
    /// Device UUID (16 bytes, RFC 4122).
    pub device_uuid: [u8; 16],
    pub hub_vmac: Option<Vmac>,
    /// Maximum APDU length this node can accept (sent in ConnectRequest).
    pub max_apdu_length: u16,
    /// Maximum APDU length the hub can accept (learned from ConnectAccept).
    pub hub_max_apdu_length: u16,
    next_message_id: u16,
    /// Pending Disconnect-ACK to send after receiving a Disconnect-Request.
    pub disconnect_ack_to_send: Option<ScMessage>,
    /// Message ID of the last ConnectRequest sent (for response verification).
    pending_connect_message_id: Option<u16>,
    /// Device UUID of the connected hub.
    pub hub_device_uuid: Option<[u8; 16]>,
}

impl ScConnection {
    pub fn new(local_vmac: Vmac, device_uuid: [u8; 16]) -> Self {
        Self {
            state: ScConnectionState::Disconnected,
            local_vmac,
            device_uuid,
            hub_vmac: None,
            max_apdu_length: 1476,
            hub_max_apdu_length: 1476,
            next_message_id: 1,
            disconnect_ack_to_send: None,
            pending_connect_message_id: None,
            hub_device_uuid: None,
        }
    }

    /// Generate the next message ID.
    pub fn next_id(&mut self) -> u16 {
        let id = self.next_message_id;
        self.next_message_id = self.next_message_id.wrapping_add(1);
        id
    }

    /// Build a Connect-Request message (26-byte payload, no VMACs).
    pub fn build_connect_request(&mut self) -> ScMessage {
        self.state = ScConnectionState::Connecting;
        let mut payload_buf = Vec::with_capacity(26);
        payload_buf.extend_from_slice(&self.local_vmac);
        payload_buf.extend_from_slice(&self.device_uuid);
        payload_buf.extend_from_slice(&1476u16.to_be_bytes()); // Max-BVLC-Length
        payload_buf.extend_from_slice(&self.max_apdu_length.to_be_bytes()); // Max-NPDU-Length
        let msg_id = self.next_id();
        self.pending_connect_message_id = Some(msg_id);
        ScMessage {
            function: ScFunction::ConnectRequest,
            message_id: msg_id,
            originating_vmac: None,
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from(payload_buf),
        }
    }

    /// Handle a received Connect-Accept (26-byte payload).
    pub fn handle_connect_accept(&mut self, msg: &ScMessage) -> bool {
        if self.state != ScConnectionState::Connecting {
            return false;
        }
        if msg.function != ScFunction::ConnectAccept {
            return false;
        }
        // Verify message_id matches our ConnectRequest (spec AB.3.1.3)
        if let Some(expected_id) = self.pending_connect_message_id {
            if msg.message_id != expected_id {
                tracing::warn!(
                    "ConnectAccept message_id {:#x} does not match request {:#x}",
                    msg.message_id,
                    expected_id
                );
                return false;
            }
        }
        self.pending_connect_message_id = None;
        // Parse hub VMAC from first 6 bytes of payload
        if msg.payload.len() >= 26 {
            let mut hub_vmac = [0u8; 6];
            hub_vmac.copy_from_slice(&msg.payload[0..6]);
            self.hub_vmac = Some(hub_vmac);
            // Parse Device UUID (bytes 6..22)
            let mut uuid = [0u8; 16];
            uuid.copy_from_slice(&msg.payload[6..22]);
            self.hub_device_uuid = Some(uuid);
            // bytes [22..24] = Max-BVLC-Length
            // bytes [24..26] = Max-NPDU-Length
            self.hub_max_apdu_length = u16::from_be_bytes([msg.payload[24], msg.payload[25]]);
        } else if msg.payload.len() >= 6 {
            // Tolerate short payloads from non-2020 implementations
            let mut hub_vmac = [0u8; 6];
            hub_vmac.copy_from_slice(&msg.payload[0..6]);
            self.hub_vmac = Some(hub_vmac);
        }
        self.state = ScConnectionState::Connected;
        true
    }

    /// Build a Disconnect-Request message (no VMACs).
    ///
    /// Returns an error if not yet connected (no hub VMAC available).
    pub fn build_disconnect_request(&mut self) -> Result<ScMessage, Error> {
        if self.hub_vmac.is_none() {
            return Err(Error::Encoding(
                "cannot build DisconnectRequest: no hub VMAC (not connected)".into(),
            ));
        }
        self.state = ScConnectionState::Disconnecting;
        Ok(ScMessage {
            function: ScFunction::DisconnectRequest,
            message_id: self.next_id(),
            originating_vmac: None,
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::new(),
        })
    }

    /// Build a Heartbeat-Request message (no VMACs).
    pub fn build_heartbeat(&mut self) -> ScMessage {
        ScMessage {
            function: ScFunction::HeartbeatRequest,
            message_id: self.next_id(),
            originating_vmac: None,
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::new(),
        }
    }

    /// Build a Heartbeat-ACK message. Per spec AB.2.11, no VMACs.
    pub fn build_heartbeat_ack(&self, request_message_id: u16) -> ScMessage {
        ScMessage {
            function: ScFunction::HeartbeatAck,
            message_id: request_message_id,
            originating_vmac: None,
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::new(),
        }
    }

    /// Build an Encapsulated-NPDU message.
    pub fn build_encapsulated_npdu(&mut self, dest_vmac: Vmac, npdu: &[u8]) -> ScMessage {
        ScMessage {
            function: ScFunction::EncapsulatedNpdu,
            message_id: self.next_id(),
            originating_vmac: Some(self.local_vmac),
            destination_vmac: Some(dest_vmac),
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::copy_from_slice(npdu),
        }
    }

    /// Handle a received message. Returns NPDU data if it's an Encapsulated-NPDU for us.
    pub fn handle_received(&mut self, msg: &ScMessage) -> Option<(Bytes, Vmac)> {
        match msg.function {
            ScFunction::EncapsulatedNpdu => {
                if self.state != ScConnectionState::Connected {
                    debug!("Ignoring EncapsulatedNpdu in {:?} state", self.state);
                    return None;
                }
                // Check destination
                if let Some(dest) = msg.destination_vmac {
                    if dest != self.local_vmac && !is_broadcast_vmac(&dest) {
                        return None;
                    }
                }
                let source = msg.originating_vmac.unwrap_or([0; 6]);
                Some((msg.payload.clone(), source))
            }
            ScFunction::HeartbeatRequest => {
                // Will be handled by transport layer (send HeartbeatAck)
                None
            }
            ScFunction::DisconnectRequest => {
                self.state = ScConnectionState::Disconnected;
                self.disconnect_ack_to_send = Some(ScMessage {
                    function: ScFunction::DisconnectAck,
                    message_id: msg.message_id,
                    originating_vmac: None,
                    destination_vmac: None,
                    dest_options: Vec::new(),
                    data_options: Vec::new(),
                    payload: Bytes::new(),
                });
                None
            }
            ScFunction::DisconnectAck => {
                if self.state == ScConnectionState::Disconnecting {
                    self.state = ScConnectionState::Disconnected;
                }
                None
            }
            ScFunction::Result => {
                // Parse Result Code: byte 1 of payload (0x00=ACK, 0x01=NAK)
                let is_nak = msg.payload.len() >= 2 && msg.payload[1] == 0x01;
                if is_nak {
                    if msg.payload.len() >= 7 {
                        let result_for = msg.payload[0];
                        let error_class = u16::from_be_bytes([msg.payload[3], msg.payload[4]]);
                        let error_code = u16::from_be_bytes([msg.payload[5], msg.payload[6]]);
                        tracing::warn!(
                            "BACnet/SC BVLC-Result NAK: function={result_for:#x} \
                             error_class={error_class} error_code={error_code}"
                        );
                    }
                    self.state = ScConnectionState::Disconnected;
                }
                None
            }
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Reconnection configuration
// ---------------------------------------------------------------------------

/// Configuration for SC transport reconnection with exponential backoff.
#[derive(Debug, Clone)]
pub struct ScReconnectConfig {
    /// Initial delay before first reconnect attempt (ms).
    pub initial_delay_ms: u64,
    /// Maximum delay between reconnect attempts (ms).
    pub max_delay_ms: u64,
    /// Maximum number of reconnect attempts before giving up.
    pub max_retries: u32,
}

impl Default for ScReconnectConfig {
    fn default() -> Self {
        Self {
            initial_delay_ms: 10_000,
            max_delay_ms: 600_000,
            max_retries: 10,
        }
    }
}

// ---------------------------------------------------------------------------
// BACnet/SC Transport
// ---------------------------------------------------------------------------

/// BACnet/SC transport implementing [`TransportPort`].
pub struct ScTransport<W: WebSocketPort> {
    ws: Option<W>,
    ws_shared: Option<Arc<W>>, // kept after start() for send methods
    local_vmac: Vmac,
    /// Device UUID (16 bytes, RFC 4122).
    device_uuid: [u8; 16],
    connection: Option<Arc<Mutex<ScConnection>>>,
    recv_task: Option<JoinHandle<()>>,
    connect_timeout_ms: u64,
    heartbeat_interval_ms: u64,
    heartbeat_timeout_ms: u64,
    failover_ws: Option<W>,
    reconnect_config: Option<ScReconnectConfig>,
}

impl<W: WebSocketPort> ScTransport<W> {
    pub fn new(ws: W, local_vmac: Vmac) -> Self {
        Self {
            ws: Some(ws),
            ws_shared: None,
            local_vmac,
            device_uuid: [0u8; 16],
            connection: None,
            recv_task: None,
            connect_timeout_ms: 10_000,
            heartbeat_interval_ms: 30_000,
            heartbeat_timeout_ms: 60_000,
            failover_ws: None,
            reconnect_config: None,
        }
    }

    /// Set the device UUID (builder-style). Should be a persistent RFC 4122 UUID.
    pub fn with_device_uuid(mut self, uuid: [u8; 16]) -> Self {
        self.device_uuid = uuid;
        self
    }

    /// Set the connect handshake timeout in milliseconds (builder-style).
    pub fn with_connect_timeout_ms(mut self, ms: u64) -> Self {
        self.connect_timeout_ms = ms;
        self
    }

    /// Set the heartbeat send interval in milliseconds (builder-style).
    pub fn with_heartbeat_interval_ms(mut self, ms: u64) -> Self {
        self.heartbeat_interval_ms = ms;
        self
    }

    /// Set the heartbeat ack timeout in milliseconds (builder-style).
    pub fn with_heartbeat_timeout_ms(mut self, ms: u64) -> Self {
        self.heartbeat_timeout_ms = ms;
        self
    }

    /// Set a failover WebSocket to try if the primary connection fails (builder-style).
    pub fn with_failover(mut self, ws: W) -> Self {
        self.failover_ws = Some(ws);
        self
    }

    /// Enable reconnection with the given configuration.
    ///
    /// When the WebSocket connection drops, the transport will attempt to
    /// re-establish the connection using exponential backoff as configured.
    /// The local VMAC is preserved across reconnections.
    pub fn with_reconnect(mut self, config: ScReconnectConfig) -> Self {
        self.reconnect_config = Some(config);
        self
    }

    /// Get the connection state (for testing/inspection).
    pub fn connection(&self) -> Option<&Arc<Mutex<ScConnection>>> {
        self.connection.as_ref()
    }
}

/// Perform the Connect-Request / Connect-Accept handshake on a WebSocket.
///
/// Used for both the initial connection and reconnection attempts.
/// The local VMAC is preserved; connection state transitions to
/// [`ScConnectionState::Connected`] on success.
async fn perform_handshake<W: WebSocketPort>(
    ws: &W,
    conn: &Arc<Mutex<ScConnection>>,
    timeout_ms: u64,
) -> Result<(), Error> {
    // Send Connect-Request
    {
        let mut c = conn.lock().await;
        let msg = c.build_connect_request();
        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &msg);
        ws.send(&buf).await?;
    }

    // Wait for Connect-Accept with timeout
    let timeout_dur = Duration::from_millis(timeout_ms);
    let accept_result = tokio::time::timeout(timeout_dur, async {
        loop {
            let data = ws.recv().await?;
            let msg = decode_sc_message(&data)?;
            if msg.function == ScFunction::ConnectAccept {
                return Ok::<_, Error>(msg);
            }
        }
    })
    .await;

    match accept_result {
        Ok(Ok(msg)) => {
            let mut c = conn.lock().await;
            if c.handle_connect_accept(&msg) {
                debug!("BACnet/SC connected");
            }
            Ok(())
        }
        Ok(Err(e)) => Err(e),
        Err(_) => {
            let mut c = conn.lock().await;
            c.state = ScConnectionState::Disconnected;
            Err(Error::Encoding("BACnet/SC connect timeout".into()))
        }
    }
}

impl<W: WebSocketPort> ScTransport<W> {
    /// Attempt the Connect-Request / Connect-Accept handshake on the given
    /// WebSocket.  Returns the `Arc<W>` on success or the error on failure.
    async fn attempt_handshake(
        ws: Arc<W>,
        conn: &Arc<Mutex<ScConnection>>,
        timeout_ms: u64,
    ) -> Result<Arc<W>, Error> {
        perform_handshake(&*ws, conn, timeout_ms).await?;
        Ok(ws)
    }
}

impl<W: WebSocketPort> TransportPort for ScTransport<W> {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        /// NPDU receive channel capacity — smaller than BIP/Ethernet since SC is hub-relayed.
        const NPDU_CHANNEL_CAPACITY: usize = 64;

        let (npdu_tx, npdu_rx) = mpsc::channel(NPDU_CHANNEL_CAPACITY);

        let conn = Arc::new(Mutex::new(ScConnection::new(
            self.local_vmac,
            self.device_uuid,
        )));
        self.connection = Some(conn.clone());

        let primary_ws = self
            .ws
            .take()
            .ok_or_else(|| Error::Encoding("BACnet/SC transport already started".into()))?;

        let ws = Arc::new(primary_ws);

        // Attempt handshake on the primary WebSocket.
        let ws = match Self::attempt_handshake(ws, &conn, self.connect_timeout_ms).await {
            Ok(ws) => ws,
            Err(primary_err) => {
                // Primary failed — try failover if configured.
                if let Some(failover) = self.failover_ws.take() {
                    debug!("BACnet/SC primary connect failed, attempting failover");
                    // Reset connection state for the retry.
                    {
                        let mut c = conn.lock().await;
                        *c = ScConnection::new(self.local_vmac, self.device_uuid);
                    }
                    let failover_ws = Arc::new(failover);
                    Self::attempt_handshake(failover_ws, &conn, self.connect_timeout_ms)
                        .await
                        .map_err(|_| primary_err)?
                } else {
                    return Err(primary_err);
                }
            }
        };

        self.ws_shared = Some(ws.clone());

        // Receive loop (handshake already done — no ConnectAccept handling needed)
        let heartbeat_interval_ms = self.heartbeat_interval_ms;
        let heartbeat_timeout_ms = self.heartbeat_timeout_ms;
        let reconnect_config = self.reconnect_config.clone();
        let connect_timeout_ms = self.connect_timeout_ms;

        let ws_clone = ws.clone();
        let task = tokio::spawn(async move {
            'transport: loop {
                let mut hb_interval =
                    tokio::time::interval(Duration::from_millis(heartbeat_interval_ms));
                hb_interval.tick().await; // consume the first immediate tick
                let mut last_heartbeat_ack = Instant::now();

                loop {
                    tokio::select! {
                        data = ws_clone.recv() => {
                            match data {
                                Ok(data) => {
                                    let msg = match decode_sc_message(&data) {
                                        Ok(m) => m,
                                        Err(e) => {
                                            warn!("BACnet/SC decode error: {}", e);
                                            continue;
                                        }
                                    };

                                    // Handle HeartbeatAck — update last_heartbeat_ack timestamp
                                    if msg.function == ScFunction::HeartbeatAck {
                                        last_heartbeat_ack = Instant::now();
                                        continue;
                                    }

                                    // Handle Heartbeat-Request with Heartbeat-ACK
                                    if msg.function == ScFunction::HeartbeatRequest {
                                        let ack = {
                                            let c = conn.lock().await;
                                            c.build_heartbeat_ack(msg.message_id)
                                        };
                                        let mut buf = BytesMut::new();
                                        encode_sc_message(&mut buf, &ack);
                                        if let Err(e) = ws_clone.send(&buf).await {
                                            warn!("BACnet/SC heartbeat ack send error: {}", e);
                                        }
                                        continue;
                                    }

                                    // Handle NPDU — lock, extract results, drop before awaiting
                                    let (npdu_result, disconnect_ack) = {
                                        let mut c = conn.lock().await;
                                        let npdu = c.handle_received(&msg);
                                        let ack = c.disconnect_ack_to_send.take();
                                        (npdu, ack)
                                    };

                                    if let Some((npdu, source_vmac)) = npdu_result {
                                        if npdu_tx
                                            .try_send(ReceivedNpdu {
                                                npdu,
                                                source_mac: MacAddr::from_slice(&source_vmac),
                                                reply_tx: None,
                                            })
                                            .is_err()
                                        {
                                            warn!("SC transport: NPDU channel full, dropping incoming message");
                                        }
                                    }

                                    // After handle_received, check for pending DisconnectAck
                                    if let Some(ack) = disconnect_ack {
                                        let mut ack_buf = BytesMut::new();
                                        encode_sc_message(&mut ack_buf, &ack);
                                        if let Err(e) = ws_clone.send(&ack_buf).await {
                                            warn!("BACnet/SC disconnect ack send error: {}", e);
                                        }
                                    }
                                }
                                Err(e) => {
                                    warn!("BACnet/SC recv error: {}", e);
                                    break;
                                }
                            }
                        }
                        _ = hb_interval.tick() => {
                            // Send heartbeat request first
                            let mut c = conn.lock().await;
                            let hb_msg = c.build_heartbeat();
                            let mut buf = BytesMut::new();
                            encode_sc_message(&mut buf, &hb_msg);
                            drop(c);
                            if let Err(e) = ws_clone.send(&buf).await {
                                warn!("BACnet/SC heartbeat send error: {}", e);
                                break;
                            }
                            // Check heartbeat timeout: has the hub acked within
                            // the timeout window? Checking after send ensures the
                            // first heartbeat gets a full timeout period for a reply.
                            if last_heartbeat_ack.elapsed()
                                > Duration::from_millis(heartbeat_timeout_ms)
                            {
                                warn!("BACnet/SC heartbeat timeout — disconnecting");
                                let mut c = conn.lock().await;
                                c.state = ScConnectionState::Disconnected;
                                break;
                            }
                        }
                    }
                }

                // After recv loop exits (ws closed/error) — attempt reconnection
                let config = match &reconnect_config {
                    Some(cfg) => cfg,
                    None => break 'transport,
                };

                warn!("SC transport disconnected, attempting reconnection");
                let mut backoff = Duration::from_millis(config.initial_delay_ms);
                let max_backoff = Duration::from_millis(config.max_delay_ms);

                let mut reconnected = false;
                for attempt in 1..=config.max_retries {
                    tokio::time::sleep(backoff).await;

                    // Reset connection state, preserving VMAC and UUID
                    {
                        let mut c = conn.lock().await;
                        let vmac = c.local_vmac;
                        let uuid = c.device_uuid;
                        *c = ScConnection::new(vmac, uuid);
                    }

                    match perform_handshake(&*ws_clone, &conn, connect_timeout_ms).await {
                        Ok(()) => {
                            info!(attempt, "SC reconnected after backoff");
                            reconnected = true;
                            break;
                        }
                        Err(e) => {
                            warn!(%e, attempt, "SC reconnection failed, retrying in {:?}", backoff);
                            backoff = (backoff * 2).min(max_backoff);
                        }
                    }
                }

                if !reconnected {
                    warn!(
                        max_retries = config.max_retries,
                        "SC reconnection: max retries exhausted, giving up"
                    );
                    break 'transport;
                }
                // TODO: Hub failover — if primary hub fails N times, try failover hub URL
            }
        });

        self.recv_task = Some(task);
        Ok(npdu_rx)
    }

    async fn stop(&mut self) -> Result<(), Error> {
        // Attempt clean disconnect: send DisconnectRequest via the WebSocket
        if let (Some(ws), Some(conn)) = (&self.ws_shared, &self.connection) {
            let disconnect_msg = {
                let mut c = conn.lock().await;
                c.build_disconnect_request().ok()
            };
            if let Some(msg) = disconnect_msg {
                let mut buf = BytesMut::new();
                encode_sc_message(&mut buf, &msg);
                // Best-effort send — don't block indefinitely
                let _ =
                    tokio::time::timeout(std::time::Duration::from_secs(2), ws.send(&buf)).await;
            }
        }

        if let Some(task) = self.recv_task.take() {
            task.abort();
            let _ = task.await;
        }

        // Clear shared state to prevent stale sends
        if let Some(conn) = &self.connection {
            let mut c = conn.lock().await;
            c.state = ScConnectionState::Disconnected;
        }
        self.ws_shared = None;
        self.connection = None;
        Ok(())
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        if mac.len() != 6 {
            return Err(Error::Encoding(format!(
                "BACnet/SC VMAC must be 6 bytes, got {}",
                mac.len()
            )));
        }
        let ws = self.ws_shared.as_ref().ok_or_else(|| {
            Error::Transport(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "BACnet/SC transport not started",
            ))
        })?;
        let conn = self.connection.as_ref().ok_or_else(|| {
            Error::Transport(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "BACnet/SC transport not started",
            ))
        })?;

        let mut dest_vmac = [0u8; 6];
        dest_vmac.copy_from_slice(mac);

        let mut c = conn.lock().await;
        if c.state != ScConnectionState::Connected {
            return Err(Error::Encoding(
                "BACnet/SC transport not in Connected state".into(),
            ));
        }
        let msg = c.build_encapsulated_npdu(dest_vmac, npdu);
        drop(c);

        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &msg);
        ws.send(&buf).await
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.send_unicast(npdu, &BROADCAST_VMAC).await
    }

    fn local_mac(&self) -> &[u8] {
        // We need a reference with 'static-ish lifetime; store VMAC in struct
        // Since local_vmac is stored in the struct, we can reference it.
        // But local_mac returns &[u8] — we need the slice to outlive `self`.
        // Use a trick: reference the stored array.
        &self.local_vmac
    }
}

// ---------------------------------------------------------------------------
// Loopback WebSocket for testing
// ---------------------------------------------------------------------------

/// In-memory loopback WebSocket for unit testing.
pub struct LoopbackWebSocket {
    rx: Mutex<mpsc::Receiver<Vec<u8>>>,
    tx: mpsc::Sender<Vec<u8>>,
}

impl LoopbackWebSocket {
    /// Create a pair of connected loopback WebSockets.
    pub fn pair() -> (Self, Self) {
        let (tx_a, rx_b) = mpsc::channel(64);
        let (tx_b, rx_a) = mpsc::channel(64);
        (
            Self {
                rx: Mutex::new(rx_a),
                tx: tx_a,
            },
            Self {
                rx: Mutex::new(rx_b),
                tx: tx_b,
            },
        )
    }
}

impl WebSocketPort for LoopbackWebSocket {
    async fn send(&self, data: &[u8]) -> Result<(), Error> {
        self.tx
            .send(data.to_vec())
            .await
            .map_err(|_| Error::Encoding("loopback ws send failed".into()))
    }

    async fn recv(&self) -> Result<Vec<u8>, Error> {
        let mut rx = self.rx.lock().await;
        rx.recv()
            .await
            .ok_or_else(|| Error::Encoding("loopback ws channel closed".into()))
    }
}

#[cfg(test)]
mod tests;
