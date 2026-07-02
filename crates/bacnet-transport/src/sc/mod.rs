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

use crate::port::{DataAttribute, ReceivedNpdu, TransportPort};
use crate::sc_frame::{
    decode_sc_bvlc_result, decode_sc_message, encode_sc_message, is_broadcast_vmac, ScBvlcResult,
    ScFunction, ScMessage, Vmac, BROADCAST_VMAC,
};
use bacnet_types::error::Error;
use bacnet_types::MacAddr;

mod connect_result;
mod data_attributes;
mod failover;
mod handshake;
mod heartbeat;
mod loopback;
mod random48;
mod reconnect;
mod send;
use failover::{attempt_primary_restore, ActiveHub};
use handshake::perform_handshake;
pub use loopback::LoopbackWebSocket;
pub(crate) use random48::generate_random48_vmac;
#[cfg(test)]
pub(crate) use random48::set_test_random48_vmac_generator;
pub use reconnect::ScReconnectConfig;

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
#[derive(Clone)]
pub struct ScConnection {
    pub state: ScConnectionState,
    pub local_vmac: Vmac,
    /// Device UUID (16 bytes, RFC 4122).
    pub device_uuid: [u8; 16],
    pub hub_vmac: Option<Vmac>,
    /// Maximum encoded BACnet/SC BVLC message length this node can accept.
    pub max_bvlc_length: u16,
    /// Maximum NPDU length this node can accept (sent in ConnectRequest).
    pub max_apdu_length: u16,
    /// Maximum encoded BACnet/SC BVLC message length the hub can accept.
    pub hub_max_bvlc_length: u16,
    /// Maximum NPDU length the hub can accept (learned from ConnectAccept).
    pub hub_max_apdu_length: u16,
    next_message_id: u16,
    /// Pending Disconnect-ACK to send after receiving a Disconnect-Request.
    pub disconnect_ack_to_send: Option<ScMessage>,
    /// Message ID of the last ConnectRequest sent (for response verification).
    pending_connect_message_id: Option<u16>,
    /// Device UUID of the connected hub.
    pub hub_device_uuid: Option<[u8; 16]>,
    /// Whether the last connect failure permits another connection attempt.
    connect_retry_allowed: bool,
}

impl ScConnection {
    pub fn new(local_vmac: Vmac, device_uuid: [u8; 16]) -> Self {
        Self {
            state: ScConnectionState::Disconnected,
            local_vmac,
            device_uuid,
            hub_vmac: None,
            max_bvlc_length: 1476,
            max_apdu_length: 1476,
            hub_max_bvlc_length: 1476,
            hub_max_apdu_length: 1476,
            next_message_id: 1,
            disconnect_ack_to_send: None,
            pending_connect_message_id: None,
            hub_device_uuid: None,
            connect_retry_allowed: true,
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
        payload_buf.extend_from_slice(&self.max_bvlc_length.to_be_bytes());
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
        if msg.payload.len() != 26 {
            tracing::warn!(
                "ConnectAccept payload has {} bytes, expected 26",
                msg.payload.len()
            );
            return false;
        }
        self.pending_connect_message_id = None;
        let mut hub_vmac = [0u8; 6];
        hub_vmac.copy_from_slice(&msg.payload[0..6]);
        self.hub_vmac = Some(hub_vmac);
        let mut uuid = [0u8; 16];
        uuid.copy_from_slice(&msg.payload[6..22]);
        self.hub_device_uuid = Some(uuid);
        self.hub_max_bvlc_length = u16::from_be_bytes([msg.payload[22], msg.payload[23]]);
        self.hub_max_apdu_length = u16::from_be_bytes([msg.payload[24], msg.payload[25]]);
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

    /// Build a Heartbeat-ACK message. Per Annex AB.2.15, no VMACs.
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
        self.build_encapsulated_npdu_with_data_attributes(dest_vmac, npdu, &[])
            .expect("empty data attributes are valid")
    }

    /// Build an Encapsulated-NPDU message with BACnet/SC Data Options.
    pub fn build_encapsulated_npdu_with_data_attributes(
        &mut self,
        dest_vmac: Vmac,
        npdu: &[u8],
        data_attributes: &[DataAttribute],
    ) -> Result<ScMessage, Error> {
        Ok(ScMessage {
            function: ScFunction::EncapsulatedNpdu,
            message_id: self.next_id(),
            originating_vmac: None,
            destination_vmac: Some(dest_vmac),
            dest_options: Vec::new(),
            data_options: data_attributes::to_data_options(data_attributes)?,
            payload: Bytes::copy_from_slice(npdu),
        })
    }

    /// Handle a received message. Returns NPDU data if it's an Encapsulated-NPDU for us.
    pub fn handle_received(&mut self, msg: &ScMessage) -> Option<(Bytes, Vmac)> {
        match msg.function {
            ScFunction::EncapsulatedNpdu => {
                if self.state != ScConnectionState::Connected {
                    debug!("Ignoring EncapsulatedNpdu in {:?} state", self.state);
                    return None;
                }
                // Hub-relayed unicast messages do not carry a Destination
                // Virtual Address; broadcast relay keeps the broadcast VMAC.
                if let Some(dest) = msg.destination_vmac {
                    if !is_broadcast_vmac(&dest) {
                        return None;
                    }
                }
                if msg.payload.len() > self.max_apdu_length as usize {
                    warn!(
                        "BACnet/SC NPDU ({} bytes) exceeds local Max-NPDU-Length ({}), dropping",
                        msg.payload.len(),
                        self.max_apdu_length
                    );
                    return None;
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
                match decode_sc_bvlc_result(msg) {
                    Ok(ScBvlcResult::Ack { .. }) => {}
                    Ok(ScBvlcResult::Nak {
                        result_for,
                        error_class,
                        error_code,
                        ..
                    }) => {
                        warn!(
                            "BACnet/SC BVLC-Result NAK: function={:#x} \
                             error_class={} error_code={}",
                            result_for.to_raw(),
                            error_class,
                            error_code
                        );
                        self.state = ScConnectionState::Disconnected;
                    }
                    Err(e) => {
                        warn!("Malformed BACnet/SC BVLC-Result: {e}");
                        self.state = ScConnectionState::Disconnected;
                    }
                }
                None
            }
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// BACnet/SC Transport
// ---------------------------------------------------------------------------

/// BACnet/SC transport implementing [`TransportPort`].
pub struct ScTransport<W: WebSocketPort> {
    ws: Option<W>,
    ws_shared: Option<Arc<Mutex<Arc<W>>>>, // current active WebSocket for send methods
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
    #[cfg(test)]
    allow_test_heartbeat_timing: bool,
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
            heartbeat_interval_ms: heartbeat::DEFAULT_HEARTBEAT_INTERVAL_MS,
            heartbeat_timeout_ms: heartbeat::DEFAULT_HEARTBEAT_TIMEOUT_MS,
            failover_ws: None,
            reconnect_config: None,
            #[cfg(test)]
            allow_test_heartbeat_timing: false,
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
    ///
    /// Production transports validate this at [`TransportPort::start`] against
    /// the Annex AB.6.3 configurable heartbeat timeout range of 3..300 seconds.
    pub fn with_heartbeat_interval_ms(mut self, ms: u64) -> Self {
        self.heartbeat_interval_ms = ms;
        self
    }

    /// Set the heartbeat ack timeout in milliseconds (builder-style).
    ///
    /// This disconnect timeout must be greater than the heartbeat interval so
    /// the peer has a chance to send a Heartbeat-ACK.
    pub fn with_heartbeat_timeout_ms(mut self, ms: u64) -> Self {
        self.heartbeat_timeout_ms = ms;
        self
    }

    #[cfg(test)]
    fn with_test_heartbeat_timing_ms(mut self, interval_ms: u64, timeout_ms: u64) -> Self {
        self.heartbeat_interval_ms = interval_ms;
        self.heartbeat_timeout_ms = timeout_ms;
        self.allow_test_heartbeat_timing = true;
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

impl<W: WebSocketPort> TransportPort for ScTransport<W> {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        /// NPDU receive channel capacity — smaller than BIP/Ethernet since SC is hub-relayed.
        const NPDU_CHANNEL_CAPACITY: usize = 64;

        #[cfg(test)]
        let validate_heartbeat_timing = !self.allow_test_heartbeat_timing;
        #[cfg(not(test))]
        let validate_heartbeat_timing = true;
        if validate_heartbeat_timing {
            heartbeat::validate_heartbeat_timing_ms(
                self.heartbeat_interval_ms,
                self.heartbeat_timeout_ms,
            )?;
        }

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

        let primary_ws = Arc::new(primary_ws);
        let mut failover_ws = self.failover_ws.take().map(Arc::new);

        // Attempt handshake on the primary WebSocket.
        let (ws, active_hub) =
            match perform_handshake(&*primary_ws, &conn, self.connect_timeout_ms).await {
                Ok(()) => (primary_ws.clone(), ActiveHub::Primary),
                Err(primary_err) => {
                    // Primary failed — try failover if configured.
                    if !conn.lock().await.connect_retry_allowed {
                        self.local_vmac = conn.lock().await.local_vmac;
                        return Err(primary_err);
                    } else if let Some(failover) = failover_ws.take() {
                        debug!("BACnet/SC primary connect failed, attempting failover");
                        // Reset connection state for the retry.
                        {
                            let mut c = conn.lock().await;
                            c.reset_for_connect_retry();
                        }
                        perform_handshake(&*failover, &conn, self.connect_timeout_ms)
                            .await
                            .map(|()| (failover, ActiveHub::Failover))
                            .map_err(|_| primary_err)?
                    } else {
                        self.local_vmac = conn.lock().await.local_vmac;
                        return Err(primary_err);
                    }
                }
            };

        self.local_vmac = conn.lock().await.local_vmac;

        let active_ws = Arc::new(Mutex::new(ws.clone()));
        self.ws_shared = Some(active_ws.clone());

        // Receive loop (handshake already done — no ConnectAccept handling needed)
        let heartbeat_interval_ms = self.heartbeat_interval_ms;
        let heartbeat_timeout_ms = self.heartbeat_timeout_ms;
        let reconnect_config = self.reconnect_config.clone();
        let connect_timeout_ms = self.connect_timeout_ms;
        let restore_enabled = reconnect_config.is_some();
        let restore_interval_ms = reconnect_config
            .as_ref()
            .map(|cfg| cfg.initial_delay_ms.max(1))
            .unwrap_or(heartbeat_interval_ms.max(1));

        let primary_ws = primary_ws.clone();
        let mut ws_clone = ws.clone();
        let mut active_hub = active_hub;
        let task = tokio::spawn(async move {
            let mut primary_restore_interval =
                tokio::time::interval(Duration::from_millis(restore_interval_ms));
            primary_restore_interval.tick().await;

            'transport: loop {
                let mut hb_interval =
                    tokio::time::interval(Duration::from_millis(heartbeat_interval_ms));
                hb_interval.tick().await; // consume the first immediate tick
                let mut last_bvlc_received = Instant::now();
                let mut pending_heartbeat_id = None;

                loop {
                    let recv_ws = ws_clone.clone();
                    tokio::select! {
                        data = recv_ws.recv() => {
                            match data {
                                Ok(data) => {
                                    if data.len() > conn.lock().await.max_bvlc_length as usize {
                                        warn!("BACnet/SC frame exceeds local Max-BVLC-Length, dropping");
                                        continue;
                                    }
                                    let msg = match decode_sc_message(&data) {
                                        Ok(m) => m,
                                        Err(e) if heartbeat::is_bvlc_result_wire(&data) => {
                                            warn!("Malformed wire-level BACnet/SC BVLC-Result: {e}");
                                            let mut c = conn.lock().await;
                                            c.state = ScConnectionState::Disconnected;
                                            break;
                                        }
                                        Err(e) => {
                                            warn!("BACnet/SC decode error: {}", e);
                                            continue;
                                        }
                                    };

                                    if msg.function == ScFunction::HeartbeatAck {
                                        if heartbeat::ack_matches_outstanding(&msg, pending_heartbeat_id) {
                                            last_bvlc_received = Instant::now();
                                            pending_heartbeat_id = None;
                                        } else {
                                            warn!("BACnet/SC ignored unexpected Heartbeat-ACK");
                                        }
                                        continue;
                                    }

                                    last_bvlc_received = Instant::now();
                                    pending_heartbeat_id = None;

                                    if data_attributes::reject_unsupported_must_understand_data_option(
                                        &msg,
                                        &*ws_clone,
                                    )
                                    .await
                                    {
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
                                    let fatal_result = {
                                        let c = conn.lock().await;
                                        msg.function == ScFunction::Result
                                            && c.state == ScConnectionState::Disconnected
                                    };

                                    if let Some((npdu, source_vmac)) = npdu_result {
                                        if npdu_tx
                                            .try_send(ReceivedNpdu {
                                                npdu,
                                                source_mac: MacAddr::from_slice(&source_vmac),
                                                data_attributes: data_attributes::from_data_options(&msg),
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

                                    if fatal_result {
                                        warn!("BACnet/SC fatal BVLC-Result received; closing transport loop");
                                        break;
                                    }
                                }
                                Err(e) => {
                                    warn!("BACnet/SC recv error: {}", e);
                                    let mut c = conn.lock().await;
                                    c.state = ScConnectionState::Disconnected;
                                    break;
                                }
                            }
                        }
                        _ = primary_restore_interval.tick(), if restore_enabled && active_hub == ActiveHub::Failover => {
                            if !conn.lock().await.connect_retry_allowed {
                                debug!("SC primary restore skipped without retry eligibility");
                                continue;
                            }
                            match attempt_primary_restore(
                                &primary_ws,
                                &ws_clone,
                                &active_ws,
                                &conn,
                                connect_timeout_ms,
                            )
                            .await
                            {
                                Ok(()) => {
                                    ws_clone = primary_ws.clone();
                                    active_hub = ActiveHub::Primary;
                                    last_bvlc_received = Instant::now();
                                    pending_heartbeat_id = None;
                                    info!("SC restored primary hub while failover was active");
                                }
                                Err(e) => {
                                    debug!(%e, "SC primary restore attempt failed while failover active");
                                }
                            }
                        }
                        _ = hb_interval.tick() => {
                            let idle_for = last_bvlc_received.elapsed();
                            if idle_for >= Duration::from_millis(heartbeat_interval_ms)
                                && pending_heartbeat_id.is_none()
                            {
                                let mut c = conn.lock().await;
                                let hb_msg = c.build_heartbeat();
                                let mut buf = BytesMut::new();
                                encode_sc_message(&mut buf, &hb_msg);
                                let heartbeat_message_id = hb_msg.message_id;
                                drop(c);
                                if let Err(e) = ws_clone.send(&buf).await {
                                    warn!("BACnet/SC heartbeat send error: {}", e);
                                    let mut c = conn.lock().await;
                                    c.state = ScConnectionState::Disconnected;
                                    break;
                                }
                                pending_heartbeat_id = Some(heartbeat_message_id);
                            }

                            if idle_for > Duration::from_millis(heartbeat_timeout_ms) {
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

                    if !conn.lock().await.connect_retry_allowed {
                        warn!(attempt, "SC reconnection skipped without retry eligibility");
                        break;
                    }

                    // Reset connection state, preserving VMAC and UUID
                    {
                        let mut c = conn.lock().await;
                        c.reset_for_connect_retry();
                    }

                    match perform_handshake(&*ws_clone, &conn, connect_timeout_ms).await {
                        Ok(()) => {
                            info!(attempt, "SC reconnected after backoff");
                            reconnected = true;
                            break;
                        }
                        Err(e) => {
                            if !conn.lock().await.connect_retry_allowed {
                                warn!(
                                    %e,
                                    attempt,
                                    "SC reconnection failed without retry eligibility"
                                );
                                break;
                            }
                            warn!(%e, attempt, "SC reconnection failed, retrying in {:?}", backoff);
                            backoff = (backoff * 2).min(max_backoff);
                        }
                    }
                }

                if !reconnected && conn.lock().await.connect_retry_allowed {
                    if let Some(failover) = failover_ws.take() {
                        warn!("SC primary reconnection exhausted, attempting failover hub");

                        {
                            let mut c = conn.lock().await;
                            c.reset_for_connect_retry();
                        }

                        match perform_handshake(&*failover, &conn, connect_timeout_ms).await {
                            Ok(()) => {
                                {
                                    let mut current = active_ws.lock().await;
                                    *current = failover.clone();
                                }
                                ws_clone = failover;
                                active_hub = ActiveHub::Failover;
                                info!("SC connected to failover hub after primary reconnect exhaustion");
                                reconnected = true;
                            }
                            Err(e) => {
                                warn!(%e, "SC failover connection failed");
                            }
                        }
                    }
                }

                if !reconnected {
                    warn!(
                        max_retries = config.max_retries,
                        "SC reconnection: max retries exhausted, giving up"
                    );
                    let mut c = conn.lock().await;
                    c.state = ScConnectionState::Disconnected;
                    break 'transport;
                }
            }
        });

        self.recv_task = Some(task);
        Ok(npdu_rx)
    }

    async fn stop(&mut self) -> Result<(), Error> {
        // Attempt clean disconnect: send DisconnectRequest via the WebSocket
        if let (Some(ws), Some(conn)) = (&self.ws_shared, &self.connection) {
            let ws = ws.lock().await.clone();
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
        self.send_unicast_inner(npdu, mac, &[]).await
    }

    async fn send_unicast_with_data_attributes(
        &self,
        npdu: &[u8],
        mac: &[u8],
        data_attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        self.send_unicast_inner(npdu, mac, data_attributes).await
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.send_unicast(npdu, &BROADCAST_VMAC).await
    }

    async fn send_broadcast_with_data_attributes(
        &self,
        npdu: &[u8],
        data_attributes: &[DataAttribute],
    ) -> Result<(), Error> {
        self.send_unicast_inner(npdu, &BROADCAST_VMAC, data_attributes)
            .await
    }

    fn local_mac(&self) -> &[u8] {
        // We need a reference with 'static-ish lifetime; store VMAC in struct
        // Since local_vmac is stored in the struct, we can reference it.
        // But local_mac returns &[u8] — we need the slice to outlive `self`.
        // Use a trick: reference the stored array.
        &self.local_vmac
    }
}

#[cfg(test)]
mod data_attribute_tests;

#[cfg(test)]
mod receive_state_tests;

#[cfg(test)]
mod result_tests;

#[cfg(test)]
mod primary_restore_tests;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod transport_lifecycle_tests;
