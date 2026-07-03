//! BACnet/SC (Secure Connect) transport per ASHRAE 135-2020 Annex AB.
//!
//! Hub-and-spoke topology over WebSocket + TLS 1.3.
//! The actual WebSocket I/O is abstracted behind the [`WebSocketPort`] trait
//! so the connection state machine can be tested without a TLS stack.

use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

#[cfg(test)]
use bytes::Bytes;
use bytes::BytesMut;
use tokio::sync::{mpsc, watch, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::port::{DataAttribute, ReceivedNpdu, TransportPort};
#[cfg(test)]
use crate::sc_frame::{decode_sc_bvlc_result, ScMessage};
use crate::sc_frame::{decode_sc_message, encode_sc_message, ScFunction, Vmac, BROADCAST_VMAC};
use bacnet_types::error::Error;
use bacnet_types::MacAddr;

mod connect_result;
mod connection;
mod connector;
mod data_attributes;
mod errors;
mod failover;
mod handshake;
mod heartbeat;
mod loopback;
mod random48;
mod reconnect;
mod send;
pub use connection::{ScConnection, ScConnectionState};
use connector::{dial_failover_ws, dial_reconnect_ws, WebSocketConnector};
pub use errors::{ScConnectError, ScWebSocketErrorKind};
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
    state_tx: watch::Sender<ScConnectionState>,
    recv_task: Option<JoinHandle<()>>,
    connect_timeout_ms: u64,
    heartbeat_interval_ms: u64,
    heartbeat_timeout_ms: u64,
    failover_ws: Option<W>,
    primary_connector: Option<WebSocketConnector<W>>,
    failover_connector: Option<WebSocketConnector<W>>,
    reconnect_config: Option<ScReconnectConfig>,
    restore_disconnect_task: Arc<StdMutex<Option<JoinHandle<()>>>>,
    #[cfg(test)]
    allow_test_heartbeat_timing: bool,
}

impl<W: WebSocketPort> ScTransport<W> {
    pub fn new(ws: W, local_vmac: Vmac) -> Self {
        let (state_tx, _) = watch::channel(ScConnectionState::Disconnected);
        Self {
            ws: Some(ws),
            ws_shared: None,
            local_vmac,
            device_uuid: [0u8; 16],
            connection: None,
            state_tx,
            recv_task: None,
            connect_timeout_ms: 10_000,
            heartbeat_interval_ms: heartbeat::DEFAULT_HEARTBEAT_INTERVAL_MS,
            heartbeat_timeout_ms: heartbeat::DEFAULT_HEARTBEAT_TIMEOUT_MS,
            failover_ws: None,
            primary_connector: None,
            failover_connector: None,
            reconnect_config: None,
            restore_disconnect_task: Arc::new(StdMutex::new(None)),
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
    /// When the BACnet/SC connection drops, the transport will attempt to reconnect
    /// using exponential backoff as configured. Configure [`Self::with_connector`]
    /// for true transport-level recovery from a dead WebSocket/TCP/TLS connection;
    /// otherwise reconnect attempts can only reuse the current WebSocket object.
    /// The local VMAC is preserved across reconnections.
    pub fn with_reconnect(mut self, config: ScReconnectConfig) -> Self {
        self.reconnect_config = Some(config);
        self
    }

    /// Get the connection state (for testing/inspection).
    pub fn connection(&self) -> Option<&Arc<Mutex<ScConnection>>> {
        self.connection.as_ref()
    }

    /// Subscribe to BACnet/SC connection state changes.
    ///
    /// The returned watch receiver yields the latest known state immediately and
    /// change notifications for subsequent state updates observed by the
    /// transport. Rapid updates can coalesce under Tokio watch semantics, so use
    /// this as a current-state signal rather than a durable transition log.
    pub fn connection_state_changes(&self) -> watch::Receiver<ScConnectionState> {
        self.state_tx.subscribe()
    }

    fn abort_background_task_and_drop_sockets(
        &mut self,
    ) -> (Option<JoinHandle<()>>, Option<JoinHandle<()>>) {
        let task = self.recv_task.take();
        if let Some(task) = &task {
            task.abort();
        }
        let restore_task = self
            .restore_disconnect_task
            .lock()
            .ok()
            .and_then(|mut task| task.take());
        if let Some(task) = &restore_task {
            task.abort();
        }
        if let Some(conn) = &self.connection {
            if let Ok(mut c) = conn.try_lock() {
                c.state = ScConnectionState::Disconnected;
            }
        }
        self.state_tx.send_replace(ScConnectionState::Disconnected);
        self.ws_shared = None;
        self.connection = None;
        self.ws = None;
        self.failover_ws = None;
        (task, restore_task)
    }
}

async fn connect_probe_from(conn: &Arc<Mutex<ScConnection>>) -> Arc<Mutex<ScConnection>> {
    Arc::new(Mutex::new(conn.lock().await.connect_probe()))
}

async fn absorb_failed_connect_probe(
    conn: &Arc<Mutex<ScConnection>>,
    probe_conn: &Arc<Mutex<ScConnection>>,
) {
    let probe = probe_conn.lock().await;
    let mut c = conn.lock().await;
    c.absorb_failed_probe(&probe);
}

async fn publish_connected_ws<W: WebSocketPort>(
    conn: &Arc<Mutex<ScConnection>>,
    active_ws: &Arc<Mutex<Arc<W>>>,
    ws: &Arc<W>,
    probe_conn: &Arc<Mutex<ScConnection>>,
    state_tx: &watch::Sender<ScConnectionState>,
) {
    let restored = probe_conn.lock().await.clone();
    let mut current = active_ws.lock().await;
    let mut c = conn.lock().await;
    *c = restored;
    *current = ws.clone();
    state_tx.send_replace(c.state);
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
        let primary_connector = self.primary_connector.clone();
        let failover_connector = self.failover_connector.clone();
        let state_tx = self.state_tx.clone();

        // Attempt handshake on the primary WebSocket.
        let (ws, active_hub) = match perform_handshake(
            &*primary_ws,
            &conn,
            Some(&state_tx),
            self.connect_timeout_ms,
        )
        .await
        {
            Ok(()) => (primary_ws.clone(), ActiveHub::Primary),
            Err(primary_err) => {
                // Primary failed — try failover if configured.
                if !conn.lock().await.connect_retry_allowed {
                    self.local_vmac = conn.lock().await.local_vmac;
                    return Err(primary_err);
                } else if let Some(failover) = dial_failover_ws(
                    &failover_connector,
                    &mut failover_ws,
                    self.connect_timeout_ms,
                )
                .await
                {
                    debug!("BACnet/SC primary connect failed, attempting failover");
                    // Reset connection state for the retry.
                    {
                        let mut c = conn.lock().await;
                        c.reset_for_connect_retry();
                        state_tx.send_replace(c.state);
                    }
                    perform_handshake(&*failover, &conn, Some(&state_tx), self.connect_timeout_ms)
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
        let restore_disconnect_task = self.restore_disconnect_task.clone();
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
                                            state_tx.send_replace(c.state);
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
                                    let (npdu_result, disconnect_ack, fatal_result, state_change) = {
                                        let mut c = conn.lock().await;
                                        let before_state = c.state;
                                        let npdu = c.handle_received(&msg);
                                        let ack = c.disconnect_ack_to_send.take();
                                        let after_state = c.state;
                                        (
                                            npdu,
                                            ack,
                                            msg.function == ScFunction::Result
                                                && after_state == ScConnectionState::Disconnected,
                                            (after_state != before_state).then_some(after_state),
                                        )
                                    };
                                    if let Some(state) = state_change {
                                        state_tx.send_replace(state);
                                    }

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
                                    state_tx.send_replace(c.state);
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
                                primary_connector.as_ref(),
                                &ws_clone,
                                &active_ws,
                                &conn,
                                &restore_disconnect_task,
                                &state_tx,
                                connect_timeout_ms,
                            )
                            .await
                            {
                                Ok(restored_ws) => {
                                    ws_clone = restored_ws;
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
                                    state_tx.send_replace(c.state);
                                    break;
                                }
                                pending_heartbeat_id = Some(heartbeat_message_id);
                            }

                            if idle_for > Duration::from_millis(heartbeat_timeout_ms) {
                                warn!("BACnet/SC heartbeat timeout — disconnecting");
                                let mut c = conn.lock().await;
                                c.state = ScConnectionState::Disconnected;
                                state_tx.send_replace(c.state);
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
                        state_tx.send_replace(c.state);
                    }

                    let reconnect_ws = match dial_reconnect_ws(
                        active_hub,
                        &primary_connector,
                        &failover_connector,
                        connect_timeout_ms,
                    )
                    .await
                    {
                        Ok(Some(ws)) => ws,
                        Ok(None) => ws_clone.clone(),
                        Err(e) => {
                            warn!(%e, attempt, "SC reconnection redial failed");
                            backoff = (backoff * 2).min(max_backoff);
                            continue;
                        }
                    };

                    let probe_conn = connect_probe_from(&conn).await;
                    match perform_handshake(&*reconnect_ws, &probe_conn, None, connect_timeout_ms)
                        .await
                    {
                        Ok(()) => {
                            publish_connected_ws(
                                &conn,
                                &active_ws,
                                &reconnect_ws,
                                &probe_conn,
                                &state_tx,
                            )
                            .await;
                            ws_clone = reconnect_ws;
                            info!(attempt, "SC reconnected after backoff");
                            reconnected = true;
                            break;
                        }
                        Err(e) => {
                            absorb_failed_connect_probe(&conn, &probe_conn).await;
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

                if !reconnected
                    && active_hub == ActiveHub::Primary
                    && conn.lock().await.connect_retry_allowed
                {
                    if let Some(failover) =
                        dial_failover_ws(&failover_connector, &mut failover_ws, connect_timeout_ms)
                            .await
                    {
                        warn!("SC primary reconnection exhausted, attempting failover hub");

                        {
                            let mut c = conn.lock().await;
                            c.reset_for_connect_retry();
                            state_tx.send_replace(c.state);
                        }

                        let probe_conn = connect_probe_from(&conn).await;
                        match perform_handshake(&*failover, &probe_conn, None, connect_timeout_ms)
                            .await
                        {
                            Ok(()) => {
                                publish_connected_ws(
                                    &conn,
                                    &active_ws,
                                    &failover,
                                    &probe_conn,
                                    &state_tx,
                                )
                                .await;
                                ws_clone = failover;
                                active_hub = ActiveHub::Failover;
                                info!("SC connected to failover hub after primary reconnect exhaustion");
                                reconnected = true;
                            }
                            Err(e) => {
                                absorb_failed_connect_probe(&conn, &probe_conn).await;
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
                    state_tx.send_replace(c.state);
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
            let (ws, disconnect_msg) = {
                let ws = ws.lock().await;
                let mut c = conn.lock().await;
                let disconnect_msg = c.build_disconnect_request().ok();
                if disconnect_msg.is_some() {
                    self.state_tx.send_replace(c.state);
                }
                (ws.clone(), disconnect_msg)
            };
            if let Some(msg) = disconnect_msg {
                let mut buf = BytesMut::new();
                encode_sc_message(&mut buf, &msg);
                // Best-effort send — don't block indefinitely
                let _ =
                    tokio::time::timeout(std::time::Duration::from_secs(2), ws.send(&buf)).await;
            }
        }

        let conn_for_state = self.connection.clone();
        let (recv_task, restore_task) = self.abort_background_task_and_drop_sockets();
        if let Some(task) = recv_task {
            let _ = task.await;
        }
        if let Some(task) = restore_task {
            let _ = task.await;
        }

        if let Some(conn) = conn_for_state {
            let mut c = conn.lock().await;
            c.state = ScConnectionState::Disconnected;
            self.state_tx.send_replace(c.state);
        }
        Ok(())
    }

    fn abort(&mut self) {
        let _ = self.abort_background_task_and_drop_sockets();
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

impl<W: WebSocketPort> Drop for ScTransport<W> {
    fn drop(&mut self) {
        self.abort();
    }
}

#[cfg(test)]
mod data_attribute_tests;

#[cfg(test)]
mod drop_tests;

#[cfg(test)]
mod receive_state_tests;

#[cfg(test)]
mod result_tests;

#[cfg(test)]
mod state_watch_tests;

#[cfg(test)]
mod primary_restore_tests;

#[cfg(test)]
mod redial_tests;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod transport_lifecycle_tests;
