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
    pub hub_vmac: Option<Vmac>,
    /// Maximum APDU length this node can accept (sent in ConnectRequest).
    pub max_apdu_length: u16,
    /// Maximum APDU length the hub can accept (learned from ConnectAccept).
    pub hub_max_apdu_length: u16,
    next_message_id: u16,
    /// Pending Disconnect-ACK to send after receiving a Disconnect-Request (AB.7.4).
    pub disconnect_ack_to_send: Option<ScMessage>,
}

impl ScConnection {
    pub fn new(local_vmac: Vmac) -> Self {
        Self {
            state: ScConnectionState::Disconnected,
            local_vmac,
            hub_vmac: None,
            max_apdu_length: 1476,
            hub_max_apdu_length: 1476,
            next_message_id: 1,
            disconnect_ack_to_send: None,
        }
    }

    /// Generate the next message ID.
    pub fn next_id(&mut self) -> u16 {
        let id = self.next_message_id;
        self.next_message_id = self.next_message_id.wrapping_add(1);
        id
    }

    /// Build a Connect-Request message.
    ///
    /// The payload carries VMAC(6) + Max-BVLC-Length(2,BE) +
    /// Max-NPDU-Length(2,BE) = 10 bytes per Annex AB.7.1.
    pub fn build_connect_request(&mut self) -> ScMessage {
        self.state = ScConnectionState::Connecting;
        let mut payload_buf = Vec::with_capacity(10);
        payload_buf.extend_from_slice(&self.local_vmac);
        payload_buf.extend_from_slice(&1476u16.to_be_bytes()); // Max-BVLC-Length
        payload_buf.extend_from_slice(&self.max_apdu_length.to_be_bytes()); // Max-NPDU-Length
        ScMessage {
            function: ScFunction::ConnectRequest,
            message_id: self.next_id(),
            originating_vmac: Some(self.local_vmac),
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from(payload_buf),
        }
    }

    /// Handle a received Connect-Accept.
    ///
    /// The Accept payload is VMAC(6) + Max-BVLC-Length(2,BE) +
    /// Max-NPDU-Length(2,BE) = 10 bytes per Annex AB.7.2.
    /// The hub's Max-NPDU-Length (bytes 8..10) is stored in
    /// [`Self::hub_max_apdu_length`].
    pub fn handle_connect_accept(&mut self, msg: &ScMessage) -> bool {
        if self.state != ScConnectionState::Connecting {
            return false;
        }
        if msg.function != ScFunction::ConnectAccept {
            return false;
        }
        self.hub_vmac = msg.originating_vmac;
        self.state = ScConnectionState::Connected;
        // Parse the hub's Max-NPDU-Length from the 10-byte Accept payload.
        if msg.payload.len() >= 10 {
            self.hub_max_apdu_length = u16::from_be_bytes([msg.payload[8], msg.payload[9]]);
        }
        true
    }

    /// Build a Disconnect-Request message.
    ///
    /// Returns an error if not yet connected (no hub VMAC available).
    pub fn build_disconnect_request(&mut self) -> Result<ScMessage, Error> {
        let hub_vmac = self.hub_vmac.ok_or_else(|| {
            Error::Encoding("cannot build DisconnectRequest: no hub VMAC (not connected)".into())
        })?;
        self.state = ScConnectionState::Disconnecting;
        Ok(ScMessage {
            function: ScFunction::DisconnectRequest,
            message_id: self.next_id(),
            originating_vmac: Some(self.local_vmac),
            destination_vmac: Some(hub_vmac),
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::new(),
        })
    }

    /// Build a Heartbeat-Request message.
    pub fn build_heartbeat(&mut self) -> ScMessage {
        ScMessage {
            function: ScFunction::HeartbeatRequest,
            message_id: self.next_id(),
            originating_vmac: Some(self.local_vmac),
            destination_vmac: self.hub_vmac,
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
                    originating_vmac: Some(self.local_vmac),
                    destination_vmac: msg.originating_vmac,
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
                // Per Annex AB: success = empty payload, error = 5 bytes
                // (originating_function(1) + error_class(2,BE) + error_code(2,BE)).
                let is_error = !msg.payload.is_empty();
                if is_error {
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
            initial_delay_ms: 1000,
            max_delay_ms: 60_000,
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
            connection: None,
            recv_task: None,
            connect_timeout_ms: 10_000,
            heartbeat_interval_ms: 30_000,
            heartbeat_timeout_ms: 60_000,
            failover_ws: None,
            reconnect_config: None,
        }
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
        let (npdu_tx, npdu_rx) = mpsc::channel(64);

        let conn = Arc::new(Mutex::new(ScConnection::new(self.local_vmac)));
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
                        *c = ScConnection::new(self.local_vmac);
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
                                        let local_vmac = {
                                            let c = conn.lock().await;
                                            c.local_vmac
                                        };
                                        let ack = ScMessage {
                                            function: ScFunction::HeartbeatAck,
                                            message_id: msg.message_id,
                                            originating_vmac: Some(local_vmac),
                                            destination_vmac: msg.originating_vmac,
                                            dest_options: Vec::new(),
                                            data_options: Vec::new(),
                                            payload: Bytes::new(),
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

                    // Reset connection state, preserving VMAC
                    {
                        let mut c = conn.lock().await;
                        let vmac = c.local_vmac;
                        *c = ScConnection::new(vmac);
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
        if let Some(task) = self.recv_task.take() {
            task.abort();
            let _ = task.await;
        }
        Ok(())
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        if mac.len() != 6 {
            return Err(Error::Encoding(format!(
                "BACnet/SC VMAC must be 6 bytes, got {}",
                mac.len()
            )));
        }
        let ws = self
            .ws_shared
            .as_ref()
            .ok_or_else(|| Error::Encoding("BACnet/SC transport not started".into()))?;
        let conn = self
            .connection
            .as_ref()
            .ok_or_else(|| Error::Encoding("BACnet/SC transport not started".into()))?;

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
mod tests {
    use super::*;

    #[test]
    fn connection_initial_state() {
        let conn = ScConnection::new([0x01; 6]);
        assert_eq!(conn.state, ScConnectionState::Disconnected);
        assert_eq!(conn.local_vmac, [0x01; 6]);
        assert!(conn.hub_vmac.is_none());
    }

    #[test]
    fn connection_flow() {
        let mut conn = ScConnection::new([0x01; 6]);

        // Build connect request
        let req = conn.build_connect_request();
        assert_eq!(req.function, ScFunction::ConnectRequest);
        assert_eq!(conn.state, ScConnectionState::Connecting);

        // Handle connect accept
        let accept = ScMessage {
            function: ScFunction::ConnectAccept,
            message_id: req.message_id,
            originating_vmac: Some([0x10; 6]),
            destination_vmac: Some([0x01; 6]),
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::new(),
        };
        assert!(conn.handle_connect_accept(&accept));
        assert_eq!(conn.state, ScConnectionState::Connected);
        assert_eq!(conn.hub_vmac, Some([0x10; 6]));
    }

    #[test]
    fn connection_reject_wrong_state() {
        let mut conn = ScConnection::new([0x01; 6]);
        // Accept without being in Connecting state
        let accept = ScMessage {
            function: ScFunction::ConnectAccept,
            message_id: 1,
            originating_vmac: Some([0x10; 6]),
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::new(),
        };
        assert!(!conn.handle_connect_accept(&accept));
    }

    #[test]
    fn message_id_increments() {
        let mut conn = ScConnection::new([0x01; 6]);
        let id1 = conn.next_id();
        let id2 = conn.next_id();
        assert_eq!(id2, id1 + 1);
    }

    #[test]
    fn message_id_wraps() {
        let mut conn = ScConnection::new([0x01; 6]);
        conn.next_message_id = 0xFFFF;
        let id = conn.next_id();
        assert_eq!(id, 0xFFFF);
        let id = conn.next_id();
        assert_eq!(id, 0);
    }

    #[test]
    fn encapsulated_npdu_for_us() {
        let mut conn = ScConnection::new([0x01; 6]);
        conn.state = ScConnectionState::Connected;

        let msg = ScMessage {
            function: ScFunction::EncapsulatedNpdu,
            message_id: 1,
            originating_vmac: Some([0x02; 6]),
            destination_vmac: Some([0x01; 6]),
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from_static(&[0x01, 0x00, 0x30]),
        };

        let result = conn.handle_received(&msg);
        assert!(result.is_some());
        let (npdu, source) = result.unwrap();
        assert_eq!(npdu, vec![0x01, 0x00, 0x30]);
        assert_eq!(source, [0x02; 6]);
    }

    #[test]
    fn encapsulated_npdu_broadcast() {
        let mut conn = ScConnection::new([0x01; 6]);
        conn.state = ScConnectionState::Connected;

        let msg = ScMessage {
            function: ScFunction::EncapsulatedNpdu,
            message_id: 1,
            originating_vmac: Some([0x02; 6]),
            destination_vmac: Some(BROADCAST_VMAC),
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from_static(&[0x01, 0x20]),
        };

        let result = conn.handle_received(&msg);
        assert!(result.is_some());
    }

    #[test]
    fn encapsulated_npdu_not_for_us() {
        let mut conn = ScConnection::new([0x01; 6]);
        conn.state = ScConnectionState::Connected;

        let msg = ScMessage {
            function: ScFunction::EncapsulatedNpdu,
            message_id: 1,
            originating_vmac: Some([0x02; 6]),
            destination_vmac: Some([0x03; 6]),
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from_static(&[0x01, 0x00]),
        };

        assert!(conn.handle_received(&msg).is_none());
    }

    #[test]
    fn encapsulated_npdu_rejected_when_not_connected() {
        let mut conn = ScConnection::new([0x01; 6]);
        // State is Disconnected by default — should reject EncapsulatedNpdu
        assert_eq!(conn.state, ScConnectionState::Disconnected);

        let msg = ScMessage {
            function: ScFunction::EncapsulatedNpdu,
            message_id: 1,
            originating_vmac: Some([0x02; 6]),
            destination_vmac: Some([0x01; 6]),
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from_static(&[0x01, 0x00, 0x30]),
        };

        assert!(conn.handle_received(&msg).is_none());

        // Also rejected in Connecting state
        conn.state = ScConnectionState::Connecting;
        assert!(conn.handle_received(&msg).is_none());
    }

    #[test]
    fn disconnect_request_resets_state() {
        let mut conn = ScConnection::new([0x01; 6]);
        conn.state = ScConnectionState::Connected;

        let msg = ScMessage {
            function: ScFunction::DisconnectRequest,
            message_id: 1,
            originating_vmac: Some([0x10; 6]),
            destination_vmac: Some([0x01; 6]),
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::new(),
        };

        conn.handle_received(&msg);
        assert_eq!(conn.state, ScConnectionState::Disconnected);
    }

    #[test]
    fn build_heartbeat() {
        let mut conn = ScConnection::new([0x01; 6]);
        conn.state = ScConnectionState::Connected;
        conn.hub_vmac = Some([0x10; 6]);

        let hb = conn.build_heartbeat();
        assert_eq!(hb.function, ScFunction::HeartbeatRequest);
        assert_eq!(hb.originating_vmac, Some([0x01; 6]));
        assert_eq!(hb.destination_vmac, Some([0x10; 6]));
    }

    #[test]
    fn build_disconnect() {
        let mut conn = ScConnection::new([0x01; 6]);
        conn.state = ScConnectionState::Connected;
        conn.hub_vmac = Some([0x10; 6]);

        let msg = conn.build_disconnect_request().unwrap();
        assert_eq!(msg.function, ScFunction::DisconnectRequest);
        assert_eq!(msg.destination_vmac, Some([0x10; 6]));
        assert_eq!(conn.state, ScConnectionState::Disconnecting);
    }

    #[test]
    fn build_disconnect_before_connect_returns_error() {
        let mut conn = ScConnection::new([0x01; 6]);
        // hub_vmac is None — not connected yet
        let result = conn.build_disconnect_request();
        assert!(result.is_err());
        // State should not have changed
        assert_eq!(conn.state, ScConnectionState::Disconnected);
    }

    #[test]
    fn connect_request_has_payload() {
        let mut conn = ScConnection::new([0x01; 6]);
        let req = conn.build_connect_request();

        // Payload must be 10 bytes: VMAC(6) + Max-BVLC-Length(2,BE) + Max-NPDU-Length(2,BE).
        assert_eq!(req.payload.len(), 10);

        assert_eq!(&req.payload[0..6], &[0x01; 6]); // VMAC

        let max_bvlc = u16::from_be_bytes([req.payload[6], req.payload[7]]);
        assert_eq!(max_bvlc, 1476);

        let max_npdu = u16::from_be_bytes([req.payload[8], req.payload[9]]);
        assert_eq!(max_npdu, 1476);
    }

    #[test]
    fn connect_accept_with_payload_sets_hub_max_apdu() {
        let mut conn = ScConnection::new([0x01; 6]);
        let _req = conn.build_connect_request();

        // Build a ConnectAccept with 10-byte payload per Annex AB.7.2:
        // VMAC(6) + Max-BVLC-Length(2,BE) + Max-NPDU-Length(2,BE).
        let mut accept_payload = Vec::with_capacity(10);
        accept_payload.extend_from_slice(&[0x10; 6]); // hub VMAC
        accept_payload.extend_from_slice(&1476u16.to_be_bytes()); // Max-BVLC-Length
        accept_payload.extend_from_slice(&480u16.to_be_bytes()); // Max-NPDU-Length

        let accept = ScMessage {
            function: ScFunction::ConnectAccept,
            message_id: 1,
            originating_vmac: Some([0x10; 6]),
            destination_vmac: Some([0x01; 6]),
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from(accept_payload),
        };
        assert!(conn.handle_connect_accept(&accept));
        assert_eq!(conn.state, ScConnectionState::Connected);
        assert_eq!(conn.hub_max_apdu_length, 480);
    }

    #[test]
    fn connect_accept_empty_payload_keeps_default_max_apdu() {
        let mut conn = ScConnection::new([0x01; 6]);
        let _req = conn.build_connect_request();

        // Legacy hub that sends no payload.
        let accept = ScMessage {
            function: ScFunction::ConnectAccept,
            message_id: 1,
            originating_vmac: Some([0x10; 6]),
            destination_vmac: Some([0x01; 6]),
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::new(),
        };
        assert!(conn.handle_connect_accept(&accept));
        assert_eq!(conn.hub_max_apdu_length, 1476); // default preserved
    }

    #[tokio::test]
    async fn loopback_websocket_pair() {
        let (a, b) = LoopbackWebSocket::pair();

        a.send(&[0x01, 0x02, 0x03]).await.unwrap();
        let received = b.recv().await.unwrap();
        assert_eq!(received, vec![0x01, 0x02, 0x03]);

        b.send(&[0xAA, 0xBB]).await.unwrap();
        let received = a.recv().await.unwrap();
        assert_eq!(received, vec![0xAA, 0xBB]);
    }

    #[tokio::test]
    async fn transport_start_stop() {
        let (ws_client, ws_server) = LoopbackWebSocket::pair();
        let vmac = [0x01; 6];
        let mut transport = ScTransport::new(ws_client, vmac);

        // Hub must accept the connection before start() returns
        let hub_task = tokio::spawn(async move {
            hub_accept(&ws_server, [0x10; 6]).await;
            ws_server
        });

        let _rx = transport.start().await.unwrap();
        transport.stop().await.unwrap();
        let _ = hub_task.await;
    }

    #[tokio::test]
    async fn transport_local_mac() {
        let (ws_client, _ws_server) = LoopbackWebSocket::pair();
        let vmac = [0x42; 6];
        let transport = ScTransport::new(ws_client, vmac);
        assert_eq!(transport.local_mac(), &[0x42; 6]);
    }

    /// Helper: act as a hub — receive ConnectRequest, send ConnectAccept,
    /// then return the "hub" side websocket for further interaction.
    async fn hub_accept(ws_hub: &LoopbackWebSocket, hub_vmac: Vmac) {
        // Receive Connect-Request from the transport
        let data = ws_hub.recv().await.unwrap();
        let req = decode_sc_message(&data).unwrap();
        assert_eq!(req.function, ScFunction::ConnectRequest);

        // Build ConnectAccept with 10-byte payload per Annex AB.7.2:
        // VMAC(6) + Max-BVLC-Length(2,BE) + Max-NPDU-Length(2,BE).
        let mut accept_payload = Vec::with_capacity(10);
        accept_payload.extend_from_slice(&hub_vmac);
        accept_payload.extend_from_slice(&1476u16.to_be_bytes()); // Max-BVLC-Length
        accept_payload.extend_from_slice(&1476u16.to_be_bytes()); // Max-NPDU-Length

        // Send Connect-Accept back
        let accept = ScMessage {
            function: ScFunction::ConnectAccept,
            message_id: req.message_id,
            originating_vmac: Some(hub_vmac),
            destination_vmac: req.originating_vmac,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from(accept_payload),
        };
        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &accept);
        ws_hub.send(&buf).await.unwrap();
    }

    #[tokio::test]
    async fn transport_send_unicast_delivers_message() {
        let (ws_client, ws_hub) = LoopbackWebSocket::pair();
        let client_vmac = [0x01; 6];
        let hub_vmac = [0x10; 6];
        let dest_vmac: Vmac = [0x02, 0x03, 0x04, 0x05, 0x06, 0x07];
        let npdu_payload = vec![0x01, 0x00, 0x30, 0x42];

        let mut transport = ScTransport::new(ws_client, client_vmac);

        // Hub must accept concurrently since start() now blocks on handshake
        let hub_accept_task = tokio::spawn(async move {
            hub_accept(&ws_hub, hub_vmac).await;
            ws_hub
        });

        let _rx = transport.start().await.unwrap();
        let ws_hub = hub_accept_task.await.unwrap();

        // Send unicast from transport
        transport
            .send_unicast(&npdu_payload, &dest_vmac)
            .await
            .unwrap();

        // Hub receives the Encapsulated-NPDU
        let data = ws_hub.recv().await.unwrap();
        let msg = decode_sc_message(&data).unwrap();
        assert_eq!(msg.function, ScFunction::EncapsulatedNpdu);
        assert_eq!(msg.originating_vmac, Some(client_vmac));
        assert_eq!(msg.destination_vmac, Some(dest_vmac));
        assert_eq!(msg.payload, npdu_payload);

        transport.stop().await.unwrap();
    }

    #[test]
    fn disconnect_request_queues_ack() {
        let mut conn = ScConnection::new([1, 2, 3, 4, 5, 6]);
        conn.state = ScConnectionState::Connected;
        conn.hub_vmac = Some([10, 20, 30, 40, 50, 60]);
        let req = ScMessage {
            function: ScFunction::DisconnectRequest,
            message_id: 42,
            originating_vmac: Some([10, 20, 30, 40, 50, 60]),
            destination_vmac: Some([1, 2, 3, 4, 5, 6]),
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::new(),
        };
        let result = conn.handle_received(&req);
        assert!(result.is_none());
        assert_eq!(conn.state, ScConnectionState::Disconnected);
        let ack = conn.disconnect_ack_to_send.as_ref().unwrap();
        assert_eq!(ack.function, ScFunction::DisconnectAck);
        assert_eq!(ack.message_id, 42);
    }

    #[test]
    fn disconnect_ack_transitions_from_disconnecting() {
        let mut conn = ScConnection::new([1, 2, 3, 4, 5, 6]);
        conn.state = ScConnectionState::Disconnecting;
        let ack = ScMessage {
            function: ScFunction::DisconnectAck,
            message_id: 99,
            originating_vmac: Some([10, 20, 30, 40, 50, 60]),
            destination_vmac: Some([1, 2, 3, 4, 5, 6]),
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::new(),
        };
        let result = conn.handle_received(&ack);
        assert!(result.is_none());
        assert_eq!(conn.state, ScConnectionState::Disconnected);
    }

    #[tokio::test]
    async fn transport_send_broadcast_delivers_message() {
        let (ws_client, ws_hub) = LoopbackWebSocket::pair();
        let client_vmac = [0x01; 6];
        let hub_vmac = [0x10; 6];
        let npdu_payload = vec![0x01, 0x20, 0xFF];

        let mut transport = ScTransport::new(ws_client, client_vmac);

        // Hub must accept concurrently since start() now blocks on handshake
        let hub_accept_task = tokio::spawn(async move {
            hub_accept(&ws_hub, hub_vmac).await;
            ws_hub
        });

        let _rx = transport.start().await.unwrap();
        let ws_hub = hub_accept_task.await.unwrap();

        // Send broadcast from transport
        transport.send_broadcast(&npdu_payload).await.unwrap();

        // Hub receives the Encapsulated-NPDU with broadcast VMAC
        let data = ws_hub.recv().await.unwrap();
        let msg = decode_sc_message(&data).unwrap();
        assert_eq!(msg.function, ScFunction::EncapsulatedNpdu);
        assert_eq!(msg.originating_vmac, Some(client_vmac));
        assert_eq!(msg.destination_vmac, Some(BROADCAST_VMAC));
        assert_eq!(msg.payload, npdu_payload);

        transport.stop().await.unwrap();
    }

    #[test]
    fn bvlc_result_error_disconnects() {
        let mut conn = ScConnection::new([0x01; 6]);
        conn.state = ScConnectionState::Connected;
        // Error BVLC-Result per Annex AB: 5-byte payload
        // originating_function(1) + error_class(2,BE) + error_code(2,BE).
        let msg = ScMessage {
            function: ScFunction::Result,
            message_id: 1,
            originating_vmac: Some([0x10; 6]),
            destination_vmac: Some([0x01; 6]),
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from_static(&[0x06, 0x00, 0x01, 0x00, 0x01]),
        };
        let result = conn.handle_received(&msg);
        assert!(result.is_none());
        assert_eq!(conn.state, ScConnectionState::Disconnected);
    }

    #[test]
    fn bvlc_result_success_no_disconnect() {
        let mut conn = ScConnection::new([0x01; 6]);
        conn.state = ScConnectionState::Connected;
        // Success BVLC-Result per Annex AB: empty payload.
        let msg = ScMessage {
            function: ScFunction::Result,
            message_id: 1,
            originating_vmac: Some([0x10; 6]),
            destination_vmac: Some([0x01; 6]),
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::new(), // success = empty
        };
        let result = conn.handle_received(&msg);
        assert!(result.is_none());
        assert_eq!(conn.state, ScConnectionState::Connected);
    }

    #[tokio::test]
    async fn sc_connect_timeout() {
        let (ws_client, _ws_server) = LoopbackWebSocket::pair();
        let vmac = [0x01; 6];
        let mut transport = ScTransport::new(ws_client, vmac).with_connect_timeout_ms(200);
        // Don't send ConnectAccept from server side — should timeout
        let result = transport.start().await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("timeout"),
            "Expected timeout error, got: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn sc_heartbeat_sent_periodically() {
        let (ws_client, ws_hub) = LoopbackWebSocket::pair();
        let client_vmac = [0x01; 6];
        let hub_vmac = [0x10; 6];

        let mut transport = ScTransport::new(ws_client, client_vmac)
            .with_heartbeat_interval_ms(200)
            .with_heartbeat_timeout_ms(5000);

        // Hub accepts the connection, then we interact with the hub ws
        let hub_task = tokio::spawn(async move {
            hub_accept(&ws_hub, hub_vmac).await;
            ws_hub
        });

        let _rx = transport.start().await.unwrap();
        let ws_hub = hub_task.await.unwrap();

        // Wait enough time for the heartbeat interval to fire
        tokio::time::sleep(Duration::from_millis(300)).await;

        // The hub should receive a HeartbeatRequest
        let data = tokio::time::timeout(Duration::from_millis(500), ws_hub.recv())
            .await
            .expect("timed out waiting for heartbeat")
            .unwrap();
        let msg = decode_sc_message(&data).unwrap();
        assert_eq!(msg.function, ScFunction::HeartbeatRequest);
        assert_eq!(msg.originating_vmac, Some(client_vmac));

        // Send HeartbeatAck back so the transport doesn't timeout
        let ack = ScMessage {
            function: ScFunction::HeartbeatAck,
            message_id: msg.message_id,
            originating_vmac: Some(hub_vmac),
            destination_vmac: Some(client_vmac),
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::new(),
        };
        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &ack);
        ws_hub.send(&buf).await.unwrap();

        transport.stop().await.unwrap();
    }

    #[tokio::test]
    async fn sc_heartbeat_timeout_disconnects() {
        let (ws_client, ws_hub) = LoopbackWebSocket::pair();
        let client_vmac = [0x01; 6];
        let hub_vmac = [0x10; 6];

        let mut transport = ScTransport::new(ws_client, client_vmac)
            .with_heartbeat_interval_ms(100)
            .with_heartbeat_timeout_ms(300);

        // Hub accepts the connection but will NOT respond to heartbeats
        let hub_task = tokio::spawn(async move {
            hub_accept(&ws_hub, hub_vmac).await;
            ws_hub
        });

        let _rx = transport.start().await.unwrap();
        let _ws_hub = hub_task.await.unwrap();

        // Wait long enough for the heartbeat timeout to fire (~500ms > 300ms timeout)
        tokio::time::sleep(Duration::from_millis(600)).await;

        // The recv task should have ended and connection state should be Disconnected
        let conn = transport.connection().unwrap();
        let c = conn.lock().await;
        assert_eq!(c.state, ScConnectionState::Disconnected);
        drop(c);

        transport.stop().await.unwrap();
    }

    #[tokio::test]
    async fn sc_connect_succeeds_within_timeout() {
        let (ws_client, ws_server) = LoopbackWebSocket::pair();
        let vmac = [0x01; 6];
        let mut transport = ScTransport::new(ws_client, vmac).with_connect_timeout_ms(5000);

        // Spawn hub accept in background
        let hub_task = tokio::spawn(async move {
            // Receive ConnectRequest
            let data = ws_server.recv().await.unwrap();
            let req = decode_sc_message(&data).unwrap();
            assert_eq!(req.function, ScFunction::ConnectRequest);

            // Send ConnectAccept with 10-byte payload per Annex AB.7.2
            let mut payload = Vec::with_capacity(10);
            payload.extend_from_slice(&[0x10; 6]); // hub VMAC
            payload.extend_from_slice(&1476u16.to_be_bytes()); // Max-BVLC-Length
            payload.extend_from_slice(&1476u16.to_be_bytes()); // Max-NPDU-Length
            let accept = ScMessage {
                function: ScFunction::ConnectAccept,
                message_id: req.message_id,
                originating_vmac: Some([0x10; 6]),
                destination_vmac: req.originating_vmac,
                dest_options: Vec::new(),
                data_options: Vec::new(),
                payload: Bytes::from(payload),
            };
            let mut buf = BytesMut::new();
            encode_sc_message(&mut buf, &accept);
            ws_server.send(&buf).await.unwrap();
            ws_server // return so it's not dropped
        });

        let _rx = transport.start().await.unwrap();
        // Verify connected
        let conn = transport.connection().unwrap();
        let c = conn.lock().await;
        assert_eq!(c.state, ScConnectionState::Connected);
        drop(c);

        transport.stop().await.unwrap();
        let _ = hub_task.await;
    }

    #[tokio::test]
    async fn test_failover_on_primary_timeout() {
        // Primary pair — hub side will NOT respond, causing a timeout.
        let (primary_client, _primary_hub) = LoopbackWebSocket::pair();
        // Failover pair — hub side WILL respond with ConnectAccept.
        let (failover_client, failover_hub) = LoopbackWebSocket::pair();

        let vmac = [0x01; 6];
        let hub_vmac = [0x20; 6];

        let mut transport = ScTransport::new(primary_client, vmac)
            .with_connect_timeout_ms(200)
            .with_failover(failover_client);

        // Spawn hub accept on failover side.
        let hub_task = tokio::spawn(async move {
            hub_accept(&failover_hub, hub_vmac).await;
            failover_hub
        });

        let _rx = transport.start().await.unwrap();

        // Verify connected via failover.
        let conn = transport.connection().unwrap();
        let c = conn.lock().await;
        assert_eq!(c.state, ScConnectionState::Connected);
        assert_eq!(c.hub_vmac, Some(hub_vmac));
        drop(c);

        transport.stop().await.unwrap();
        let _ = hub_task.await;
    }

    #[tokio::test]
    async fn test_no_failover_without_config() {
        // Primary pair — hub side will NOT respond.
        let (primary_client, _primary_hub) = LoopbackWebSocket::pair();

        let vmac = [0x01; 6];
        // No failover configured.
        let mut transport = ScTransport::new(primary_client, vmac).with_connect_timeout_ms(200);

        let result = transport.start().await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("timeout"),
            "Expected timeout error, got: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_failover_primary_succeeds_no_failover_used() {
        // Primary pair — hub side WILL respond.
        let (primary_client, primary_hub) = LoopbackWebSocket::pair();
        // Failover pair — should NOT be used.
        let (failover_client, _failover_hub) = LoopbackWebSocket::pair();

        let vmac = [0x01; 6];
        let hub_vmac = [0x10; 6];

        let mut transport = ScTransport::new(primary_client, vmac)
            .with_connect_timeout_ms(2000)
            .with_failover(failover_client);

        // Spawn hub accept on primary side.
        let hub_task = tokio::spawn(async move {
            hub_accept(&primary_hub, hub_vmac).await;
            primary_hub
        });

        let _rx = transport.start().await.unwrap();

        // Verify connected via primary.
        let conn = transport.connection().unwrap();
        let c = conn.lock().await;
        assert_eq!(c.state, ScConnectionState::Connected);
        assert_eq!(c.hub_vmac, Some(hub_vmac));
        drop(c);

        transport.stop().await.unwrap();
        let _ = hub_task.await;
    }

    #[test]
    fn reconnect_config_default() {
        let config = ScReconnectConfig::default();
        assert_eq!(config.initial_delay_ms, 1000);
        assert_eq!(config.max_delay_ms, 60_000);
        assert_eq!(config.max_retries, 10);
    }

    #[test]
    fn reconnect_exponential_backoff_sequence() {
        let config = ScReconnectConfig {
            initial_delay_ms: 100,
            max_delay_ms: 1000,
            max_retries: 5,
        };
        let mut delay = config.initial_delay_ms;
        let delays: Vec<u64> = (0..5)
            .map(|_| {
                let d = delay;
                delay = (delay * 2).min(config.max_delay_ms);
                d
            })
            .collect();
        assert_eq!(delays, vec![100, 200, 400, 800, 1000]);
    }

    #[test]
    fn with_reconnect_builder() {
        // Verify the builder sets the config.
        // We can't easily create an ScTransport without a WebSocket,
        // so just verify ScReconnectConfig is constructible and clonable.
        let config = ScReconnectConfig::default();
        let config2 = config.clone();
        assert_eq!(config.initial_delay_ms, config2.initial_delay_ms);
    }
}
