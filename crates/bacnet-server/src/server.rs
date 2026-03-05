//! BACnetServer: builder, APDU dispatch, and lifecycle management.
//!
//! The server wraps a NetworkLayer behind Arc (shared with the dispatch task),
//! owns an ObjectDatabase via Arc<Mutex>, and spawns a dispatch task that
//! routes incoming APDUs to service handlers.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Instant;

use bytes::{Bytes, BytesMut};
use tokio::sync::{mpsc, Mutex, RwLock, Semaphore};
use tokio::task::JoinHandle;
use tokio::time::Duration;
use tracing::{debug, warn};

use bacnet_encoding::apdu::{
    self, encode_apdu, AbortPdu, Apdu, ComplexAck, ConfirmedRequest as ConfirmedRequestPdu,
    ErrorPdu, RejectPdu, SegmentAck as SegmentAckPdu, SimpleAck,
    UnconfirmedRequest as UnconfirmedRequestPdu,
};
use bacnet_encoding::primitives::encode_property_value;
use bacnet_encoding::segmentation::{
    max_segment_payload, split_payload, SegmentReceiver, SegmentedPduType,
};
use bacnet_network::layer::NetworkLayer;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::notification_class::get_notification_recipients;
use bacnet_services::alarm_event::EventNotificationRequest;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::cov::COVNotificationRequest;
use bacnet_services::who_is::{IAmRequest, WhoIsRequest};
use bacnet_transport::bip::BipTransport;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{
    AbortReason, ConfirmedServiceChoice, ErrorClass, ErrorCode, NetworkPriority, NotifyType,
    ObjectType, PropertyIdentifier, RejectReason, Segmentation, UnconfirmedServiceChoice,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier, PropertyValue, Time};
use bacnet_types::MacAddr;

use crate::cov::CovSubscriptionTable;
use crate::handlers;

/// Maximum number of concurrent segmented reassembly sessions.
const MAX_SEG_RECEIVERS: usize = 128;

/// Timeout for idle segmented reassembly sessions (Clause 9.1.6).
const SEG_RECEIVER_TIMEOUT: Duration = Duration::from_secs(4);

/// Maximum negative SegmentAck retries during segmented response send.
const MAX_NEG_SEGMENT_ACK_RETRIES: u8 = 3;

/// Default number of APDU retries for confirmed COV notifications (Clause 10.6.3).
const DEFAULT_APDU_RETRIES: u8 = 3;

// ---------------------------------------------------------------------------
// Server-side Transaction State Machine (TSM) for outgoing confirmed requests
// ---------------------------------------------------------------------------

/// Result of a confirmed COV notification from the subscriber's perspective.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CovAckResult {
    /// SimpleAck received — subscriber accepted the notification.
    Ack,
    /// Error or Reject/Abort received — subscriber rejected the notification.
    Error,
}

/// Lightweight TSM for tracking outgoing confirmed COV notifications.
///
/// The server allocates an invoke ID for each confirmed notification and the
/// dispatch loop writes the result into a shared map when a SimpleAck, Error,
/// Reject, or Abort is received.  The per-subscriber retry task polls the map
/// after each timeout to decide whether to resend.
pub struct ServerTsm {
    next_invoke_id: u8,
    /// Results written by the dispatch loop, read by retry tasks.
    pending: HashMap<u8, CovAckResult>,
}

impl ServerTsm {
    fn new() -> Self {
        Self {
            next_invoke_id: 0,
            pending: HashMap::new(),
        }
    }

    /// Allocate the next invoke ID.  The semaphore guarantees at most 255
    /// concurrent callers, so wrapping is safe.
    fn allocate(&mut self) -> u8 {
        let id = self.next_invoke_id;
        self.next_invoke_id = self.next_invoke_id.wrapping_add(1);
        id
    }

    /// Record a result from the dispatch loop (SimpleAck, Error, etc.).
    fn record_result(&mut self, invoke_id: u8, result: CovAckResult) {
        self.pending.insert(invoke_id, result);
    }

    /// Take the result for a given invoke ID (returns and removes it).
    fn take_result(&mut self, invoke_id: u8) -> Option<CovAckResult> {
        self.pending.remove(&invoke_id)
    }

    /// Remove a pending entry (cleanup on completion or exhaustion).
    fn remove(&mut self, invoke_id: u8) {
        self.pending.remove(&invoke_id);
    }
}

/// Data from a TimeSynchronization request.
#[derive(Debug, Clone)]
pub struct TimeSyncData {
    /// Raw service request bytes (caller can decode if needed).
    pub raw_service_data: Bytes,
    /// Whether this was a UTC time sync (vs. local).
    pub is_utc: bool,
}

/// Server configuration.
#[derive(Clone)]
pub struct ServerConfig {
    /// Local interface to bind.
    pub interface: Ipv4Addr,
    /// UDP port (default 0xBAC0 = 47808).
    pub port: u16,
    /// Directed broadcast address.
    pub broadcast_address: Ipv4Addr,
    /// Maximum APDU length accepted.
    pub max_apdu_length: u32,
    /// Segmentation support level.
    pub segmentation_supported: Segmentation,
    /// Vendor identifier.
    pub vendor_id: u16,
    /// Timeout in ms before retrying a failed confirmed COV notification send (default 3000ms).
    pub cov_retry_timeout_ms: u64,
    /// Optional callback invoked when a TimeSynchronization request is received.
    pub on_time_sync: Option<Arc<dyn Fn(TimeSyncData) + Send + Sync>>,
    /// Optional password required for DeviceCommunicationControl (Clause 16.4.1).
    pub dcc_password: Option<String>,
    /// Optional password required for ReinitializeDevice (Clause 16.4.2).
    pub reinit_password: Option<String>,
    /// Enable periodic fault detection / reliability evaluation (Clause 12).
    /// When true, the server evaluates analog objects every 10 s for
    /// OVER_RANGE / UNDER_RANGE faults.
    pub enable_fault_detection: bool,
}

impl std::fmt::Debug for ServerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerConfig")
            .field("interface", &self.interface)
            .field("port", &self.port)
            .field("broadcast_address", &self.broadcast_address)
            .field("max_apdu_length", &self.max_apdu_length)
            .field("segmentation_supported", &self.segmentation_supported)
            .field("vendor_id", &self.vendor_id)
            .field("cov_retry_timeout_ms", &self.cov_retry_timeout_ms)
            .field(
                "on_time_sync",
                &self.on_time_sync.as_ref().map(|_| "<callback>"),
            )
            .field("dcc_password", &self.dcc_password.as_ref().map(|_| "***"))
            .field(
                "reinit_password",
                &self.reinit_password.as_ref().map(|_| "***"),
            )
            .field("enable_fault_detection", &self.enable_fault_detection)
            .finish()
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            interface: Ipv4Addr::UNSPECIFIED,
            port: 0xBAC0,
            broadcast_address: Ipv4Addr::BROADCAST,
            max_apdu_length: 1476,
            segmentation_supported: Segmentation::NONE,
            vendor_id: 0,
            cov_retry_timeout_ms: 3000,
            on_time_sync: None,
            dcc_password: None,
            reinit_password: None,
            enable_fault_detection: false,
        }
    }
}

/// Generic builder for BACnetServer with a pre-built transport.
pub struct ServerBuilder<T: TransportPort> {
    config: ServerConfig,
    db: ObjectDatabase,
    transport: Option<T>,
}

impl<T: TransportPort + 'static> ServerBuilder<T> {
    /// Set the object database (transfers ownership).
    pub fn database(mut self, db: ObjectDatabase) -> Self {
        self.db = db;
        self
    }

    /// Set the pre-built transport.
    pub fn transport(mut self, transport: T) -> Self {
        self.transport = Some(transport);
        self
    }

    /// Set the password required for DeviceCommunicationControl requests.
    pub fn dcc_password(mut self, password: impl Into<String>) -> Self {
        self.config.dcc_password = Some(password.into());
        self
    }

    /// Set the password required for ReinitializeDevice requests.
    pub fn reinit_password(mut self, password: impl Into<String>) -> Self {
        self.config.reinit_password = Some(password.into());
        self
    }

    /// Enable periodic fault detection / reliability evaluation.
    pub fn enable_fault_detection(mut self, enabled: bool) -> Self {
        self.config.enable_fault_detection = enabled;
        self
    }

    /// Build and start the server.
    pub async fn build(self) -> Result<BACnetServer<T>, Error> {
        let transport = self
            .transport
            .ok_or_else(|| Error::Encoding("transport not set on ServerBuilder".into()))?;
        BACnetServer::start(self.config, self.db, transport).await
    }
}

/// BIP-specific builder that constructs `BipTransport` from interface/port/broadcast fields.
pub struct BipServerBuilder {
    config: ServerConfig,
    db: ObjectDatabase,
}

impl BipServerBuilder {
    /// Set the local interface IP.
    pub fn interface(mut self, ip: Ipv4Addr) -> Self {
        self.config.interface = ip;
        self
    }

    /// Set the UDP port.
    pub fn port(mut self, port: u16) -> Self {
        self.config.port = port;
        self
    }

    /// Set the directed broadcast address.
    pub fn broadcast_address(mut self, addr: Ipv4Addr) -> Self {
        self.config.broadcast_address = addr;
        self
    }

    /// Set the object database (transfers ownership).
    pub fn database(mut self, db: ObjectDatabase) -> Self {
        self.db = db;
        self
    }

    /// Set the password required for DeviceCommunicationControl requests.
    pub fn dcc_password(mut self, password: impl Into<String>) -> Self {
        self.config.dcc_password = Some(password.into());
        self
    }

    /// Set the password required for ReinitializeDevice requests.
    pub fn reinit_password(mut self, password: impl Into<String>) -> Self {
        self.config.reinit_password = Some(password.into());
        self
    }

    /// Enable periodic fault detection / reliability evaluation.
    pub fn enable_fault_detection(mut self, enabled: bool) -> Self {
        self.config.enable_fault_detection = enabled;
        self
    }

    /// Build and start the server, constructing a BipTransport from the config.
    pub async fn build(self) -> Result<BACnetServer<BipTransport>, Error> {
        let transport = BipTransport::new(
            self.config.interface,
            self.config.port,
            self.config.broadcast_address,
        );
        BACnetServer::start(self.config, self.db, transport).await
    }
}

/// Key for tracking in-progress segmented sends: (source_mac, invoke_id).
type SegKey = (MacAddr, u8);

/// BACnet server with APDU dispatch and service handling.
pub struct BACnetServer<T: TransportPort> {
    #[allow(dead_code)]
    config: ServerConfig,
    /// Shared network layer (also held by dispatch task).
    #[allow(dead_code)]
    network: Arc<NetworkLayer<T>>,
    /// Shared object database.
    db: Arc<RwLock<ObjectDatabase>>,
    /// COV subscription table (held to keep Arc alive for dispatch task).
    #[allow(dead_code)]
    cov_table: Arc<RwLock<CovSubscriptionTable>>,
    /// Channels for routing SegmentAck PDUs to in-progress segmented sends.
    #[allow(dead_code)]
    seg_ack_senders: Arc<Mutex<HashMap<SegKey, mpsc::Sender<SegmentAckPdu>>>>,
    /// Semaphore that caps confirmed COV notifications at 255 in-flight
    /// to prevent invoke ID reuse (invoke IDs are u8 = 0..255).
    #[allow(dead_code)]
    cov_in_flight: Arc<Semaphore>,
    /// Server-side TSM for outgoing confirmed COV notifications (Clause 10.6.3).
    #[allow(dead_code)]
    server_tsm: Arc<Mutex<ServerTsm>>,
    /// Communication state per DeviceCommunicationControl (Clause 16.4.3).
    /// 0 = Enable, 1 = Disable, 2 = DisableInitiation.
    comm_state: Arc<AtomicU8>,
    /// Handle for the DCC auto-re-enable timer (Clause 16.4.3).
    /// When DCC DISABLE/DISABLE_INITIATION is received with a time_duration,
    /// a timer task is spawned to revert comm_state to ENABLE after the duration.
    /// Previous timers are aborted when a new DCC request arrives.
    #[allow(dead_code)]
    dcc_timer: Arc<Mutex<Option<JoinHandle<()>>>>,
    dispatch_task: Option<JoinHandle<()>>,
    cov_purge_task: Option<JoinHandle<()>>,
    fault_detection_task: Option<JoinHandle<()>>,
    event_enrollment_task: Option<JoinHandle<()>>,
    trend_log_task: Option<JoinHandle<()>>,
    schedule_tick_task: Option<JoinHandle<()>>,
    local_mac: MacAddr,
}

impl BACnetServer<BipTransport> {
    /// Create a BIP-specific builder with interface/port/broadcast fields.
    pub fn bip_builder() -> BipServerBuilder {
        BipServerBuilder {
            config: ServerConfig::default(),
            db: ObjectDatabase::new(),
        }
    }

    /// Create a BIP-specific builder (alias for backward compatibility).
    pub fn builder() -> BipServerBuilder {
        Self::bip_builder()
    }
}

#[cfg(feature = "sc-tls")]
impl BACnetServer<bacnet_transport::sc::ScTransport<bacnet_transport::sc_tls::TlsWebSocket>> {
    /// Create an SC-specific builder that connects to a BACnet/SC hub.
    pub fn sc_builder() -> ScServerBuilder {
        ScServerBuilder {
            config: ServerConfig::default(),
            db: ObjectDatabase::new(),
            hub_url: String::new(),
            tls_config: None,
            vmac: [0; 6],
            heartbeat_interval_ms: 30_000,
            heartbeat_timeout_ms: 60_000,
            reconnect: None,
        }
    }
}

/// SC-specific server builder.
///
/// Created by [`BACnetServer::sc_builder()`].  Requires the `sc-tls` feature.
#[cfg(feature = "sc-tls")]
pub struct ScServerBuilder {
    config: ServerConfig,
    db: ObjectDatabase,
    hub_url: String,
    tls_config: Option<std::sync::Arc<tokio_rustls::rustls::ClientConfig>>,
    vmac: bacnet_transport::sc_frame::Vmac,
    heartbeat_interval_ms: u64,
    heartbeat_timeout_ms: u64,
    reconnect: Option<bacnet_transport::sc::ScReconnectConfig>,
}

#[cfg(feature = "sc-tls")]
impl ScServerBuilder {
    /// Set the hub WebSocket URL (e.g. `wss://hub.example.com/bacnet`).
    pub fn hub_url(mut self, url: &str) -> Self {
        self.hub_url = url.to_string();
        self
    }

    /// Set the TLS client configuration.
    pub fn tls_config(
        mut self,
        config: std::sync::Arc<tokio_rustls::rustls::ClientConfig>,
    ) -> Self {
        self.tls_config = Some(config);
        self
    }

    /// Set the local VMAC address.
    pub fn vmac(mut self, vmac: [u8; 6]) -> Self {
        self.vmac = vmac;
        self
    }

    /// Set the object database (transfers ownership).
    pub fn database(mut self, db: ObjectDatabase) -> Self {
        self.db = db;
        self
    }

    /// Set the heartbeat interval in milliseconds (default 30 000).
    pub fn heartbeat_interval_ms(mut self, ms: u64) -> Self {
        self.heartbeat_interval_ms = ms;
        self
    }

    /// Set the heartbeat timeout in milliseconds (default 60 000).
    pub fn heartbeat_timeout_ms(mut self, ms: u64) -> Self {
        self.heartbeat_timeout_ms = ms;
        self
    }

    /// Enable automatic reconnection with the given configuration.
    pub fn reconnect(mut self, config: bacnet_transport::sc::ScReconnectConfig) -> Self {
        self.reconnect = Some(config);
        self
    }

    /// Set the password required for DeviceCommunicationControl requests.
    pub fn dcc_password(mut self, password: impl Into<String>) -> Self {
        self.config.dcc_password = Some(password.into());
        self
    }

    /// Set the password required for ReinitializeDevice requests.
    pub fn reinit_password(mut self, password: impl Into<String>) -> Self {
        self.config.reinit_password = Some(password.into());
        self
    }

    /// Enable periodic fault detection / reliability evaluation.
    pub fn enable_fault_detection(mut self, enabled: bool) -> Self {
        self.config.enable_fault_detection = enabled;
        self
    }

    /// Connect to the hub and start the server.
    pub async fn build(
        self,
    ) -> Result<
        BACnetServer<bacnet_transport::sc::ScTransport<bacnet_transport::sc_tls::TlsWebSocket>>,
        Error,
    > {
        let tls_config = self
            .tls_config
            .ok_or_else(|| Error::Encoding("SC server builder: tls_config is required".into()))?;

        let ws = bacnet_transport::sc_tls::TlsWebSocket::connect(&self.hub_url, tls_config).await?;

        let mut transport = bacnet_transport::sc::ScTransport::new(ws, self.vmac)
            .with_heartbeat_interval_ms(self.heartbeat_interval_ms)
            .with_heartbeat_timeout_ms(self.heartbeat_timeout_ms);
        if let Some(rc) = self.reconnect {
            #[allow(deprecated)]
            {
                transport = transport.with_reconnect(rc);
            }
        }

        BACnetServer::start(self.config, self.db, transport).await
    }
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Create a generic builder that accepts a pre-built transport.
    pub fn generic_builder() -> ServerBuilder<T> {
        ServerBuilder {
            config: ServerConfig::default(),
            db: ObjectDatabase::new(),
            transport: None,
        }
    }

    /// Start the server with a pre-built transport.
    pub async fn start(
        mut config: ServerConfig,
        db: ObjectDatabase,
        transport: T,
    ) -> Result<Self, Error> {
        // Clamp max_apdu_length to the transport's physical limit.
        let transport_max = transport.max_apdu_length() as u32;
        config.max_apdu_length = config.max_apdu_length.min(transport_max);

        if config.vendor_id == 0 {
            warn!("vendor_id is 0 (ASHRAE reserved); set a valid vendor ID for production use");
        }

        let mut network = NetworkLayer::new(transport);
        let apdu_rx = network.start().await?;
        let local_mac = MacAddr::from_slice(network.local_mac());

        let network = Arc::new(network);
        let db = Arc::new(RwLock::new(db));
        let cov_table = Arc::new(RwLock::new(CovSubscriptionTable::new()));
        let seg_ack_senders: Arc<Mutex<HashMap<SegKey, mpsc::Sender<SegmentAckPdu>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let cov_in_flight = Arc::new(Semaphore::new(255));
        let server_tsm = Arc::new(Mutex::new(ServerTsm::new()));
        let comm_state = Arc::new(AtomicU8::new(0)); // 0 = Enable (default)
        let dcc_timer: Arc<Mutex<Option<JoinHandle<()>>>> = Arc::new(Mutex::new(None));

        let network_dispatch = Arc::clone(&network);
        let db_dispatch = Arc::clone(&db);
        let cov_dispatch = Arc::clone(&cov_table);
        let seg_ack_dispatch = Arc::clone(&seg_ack_senders);
        let cov_in_flight_dispatch = Arc::clone(&cov_in_flight);
        let server_tsm_dispatch = Arc::clone(&server_tsm);
        let comm_state_dispatch = Arc::clone(&comm_state);
        let dcc_timer_dispatch = Arc::clone(&dcc_timer);
        let config_dispatch = config.clone();

        let dispatch_task = tokio::spawn(async move {
            let mut apdu_rx = apdu_rx;
            // State for reassembling segmented ConfirmedRequests from clients.
            // Key: (source_mac, invoke_id).
            // Value: (receiver, first segment's request with all metadata).
            let mut seg_receivers: HashMap<
                SegKey,
                (
                    SegmentReceiver,
                    bacnet_encoding::apdu::ConfirmedRequest,
                    Instant,
                ),
            > = HashMap::new();

            while let Some(received) = apdu_rx.recv().await {
                // Reap timed-out segmented reassembly sessions (Clause 9.1.6).
                let now = Instant::now();
                seg_receivers.retain(|_key, (_rx, _req, last_activity)| {
                    now.duration_since(*last_activity) < SEG_RECEIVER_TIMEOUT
                });

                match apdu::decode_apdu(received.apdu.clone()) {
                    Ok(decoded) => {
                        let source_mac = received.source_mac.clone();
                        let mut received = Some(received);
                        // Intercept segmented ConfirmedRequests for reassembly.
                        let handled = if let Apdu::ConfirmedRequest(ref req) = decoded {
                            if req.segmented {
                                let seq = req.sequence_number.unwrap_or(0);
                                let key: SegKey = (source_mac.clone(), req.invoke_id);

                                if seq == 0 {
                                    // Reject if too many concurrent segmented sessions
                                    if seg_receivers.len() >= MAX_SEG_RECEIVERS {
                                        warn!("Too many concurrent segmented sessions ({}), rejecting", seg_receivers.len());
                                        let abort_pdu = Apdu::Abort(AbortPdu {
                                            sent_by_server: true,
                                            invoke_id: req.invoke_id,
                                            abort_reason: AbortReason::BUFFER_OVERFLOW,
                                        });
                                        let mut abort_buf = BytesMut::new();
                                        encode_apdu(&mut abort_buf, &abort_pdu);
                                        let _ = network_dispatch
                                            .send_apdu(
                                                &abort_buf,
                                                &source_mac,
                                                false,
                                                NetworkPriority::NORMAL,
                                            )
                                            .await;
                                        continue;
                                    }
                                    // First segment: create a new receiver and
                                    // store the request metadata.
                                    let mut receiver = SegmentReceiver::new();
                                    if let Err(e) =
                                        receiver.receive(seq, req.service_request.clone())
                                    {
                                        warn!(error = %e, "Rejecting oversized segment");
                                        continue;
                                    }
                                    seg_receivers.insert(
                                        key.clone(),
                                        (receiver, req.clone(), Instant::now()),
                                    );
                                } else if let Some((receiver, _, last_activity)) =
                                    seg_receivers.get_mut(&key)
                                {
                                    if let Err(e) =
                                        receiver.receive(seq, req.service_request.clone())
                                    {
                                        warn!(error = %e, "Rejecting oversized segment");
                                        continue;
                                    }
                                    *last_activity = Instant::now();
                                } else {
                                    // Non-initial segment without prior segment 0:
                                    // send Abort PDU per Clause 9.20.1.7.
                                    warn!(
                                        invoke_id = req.invoke_id,
                                        seq = seq,
                                        "Received non-initial segment without \
                                         prior segment 0, aborting"
                                    );
                                    let abort_pdu = Apdu::Abort(AbortPdu {
                                        sent_by_server: true,
                                        invoke_id: req.invoke_id,
                                        abort_reason: AbortReason::INVALID_APDU_IN_THIS_STATE,
                                    });
                                    let mut abort_buf = BytesMut::new();
                                    encode_apdu(&mut abort_buf, &abort_pdu);
                                    let _ = network_dispatch
                                        .send_apdu(
                                            &abort_buf,
                                            &source_mac,
                                            false,
                                            NetworkPriority::NORMAL,
                                        )
                                        .await;
                                    continue;
                                }

                                // Send SegmentAck back to the client.
                                let seg_ack = Apdu::SegmentAck(SegmentAckPdu {
                                    negative_ack: false,
                                    sent_by_server: true,
                                    invoke_id: req.invoke_id,
                                    sequence_number: seq,
                                    actual_window_size: 1,
                                });
                                let mut ack_buf = BytesMut::new();
                                encode_apdu(&mut ack_buf, &seg_ack);
                                if let Err(e) = network_dispatch
                                    .send_apdu(
                                        &ack_buf,
                                        &source_mac,
                                        false,
                                        NetworkPriority::NORMAL,
                                    )
                                    .await
                                {
                                    warn!(
                                        error = %e,
                                        "Failed to send SegmentAck for \
                                         segmented request"
                                    );
                                }

                                // Last segment: reassemble and dispatch.
                                if !req.more_follows {
                                    if let Some((receiver, first_req, _)) =
                                        seg_receivers.remove(&key)
                                    {
                                        let total = receiver.received_count();
                                        match receiver.reassemble(total) {
                                            Ok(full_data) => {
                                                let reassembled =
                                                    bacnet_encoding::apdu::ConfirmedRequest {
                                                        segmented: false,
                                                        more_follows: false,
                                                        sequence_number: None,
                                                        proposed_window_size: None,
                                                        service_request: Bytes::from(full_data),
                                                        invoke_id: first_req.invoke_id,
                                                        service_choice: first_req.service_choice,
                                                        max_apdu_length: first_req.max_apdu_length,
                                                        segmented_response_accepted: first_req
                                                            .segmented_response_accepted,
                                                        max_segments: first_req.max_segments,
                                                    };
                                                debug!(
                                                    invoke_id = reassembled.invoke_id,
                                                    segments = total,
                                                    payload_len = reassembled.service_request.len(),
                                                    "Reassembled segmented ConfirmedRequest"
                                                );
                                                Self::dispatch(
                                                    &db_dispatch,
                                                    &network_dispatch,
                                                    &cov_dispatch,
                                                    &seg_ack_dispatch,
                                                    &cov_in_flight_dispatch,
                                                    &server_tsm_dispatch,
                                                    &comm_state_dispatch,
                                                    &dcc_timer_dispatch,
                                                    &config_dispatch,
                                                    &source_mac,
                                                    Apdu::ConfirmedRequest(reassembled),
                                                    received
                                                        .take()
                                                        .expect("received consumed twice"),
                                                )
                                                .await;
                                            }
                                            Err(e) => {
                                                warn!(
                                                    error = %e,
                                                    "Failed to reassemble \
                                                     segmented request"
                                                );
                                            }
                                        }
                                    }
                                }

                                true // Already handled
                            } else {
                                false // Not segmented, dispatch normally
                            }
                        } else {
                            false // Not a ConfirmedRequest, dispatch normally
                        };

                        if !handled {
                            Self::dispatch(
                                &db_dispatch,
                                &network_dispatch,
                                &cov_dispatch,
                                &seg_ack_dispatch,
                                &cov_in_flight_dispatch,
                                &server_tsm_dispatch,
                                &comm_state_dispatch,
                                &dcc_timer_dispatch,
                                &config_dispatch,
                                &source_mac,
                                decoded,
                                received.take().expect("received consumed twice"),
                            )
                            .await;
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "Server failed to decode received APDU");
                    }
                }
            }
        });

        let cov_table_for_purge = Arc::clone(&cov_table);
        let cov_purge_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                let mut table = cov_table_for_purge.write().await;
                let purged = table.purge_expired();
                if purged > 0 {
                    debug!(purged, "Purged expired COV subscriptions");
                }
            }
        });

        let fault_detection_task = if config.enable_fault_detection {
            let db_fault = Arc::clone(&db);
            Some(tokio::spawn(async move {
                let detector = crate::fault_detection::FaultDetector::default();
                let mut interval = tokio::time::interval(Duration::from_secs(10));
                loop {
                    interval.tick().await;
                    let mut db_guard = db_fault.write().await;
                    let changes = detector.evaluate(&mut db_guard);
                    for change in &changes {
                        debug!(
                            object = %change.object_id,
                            old = change.old_reliability,
                            new = change.new_reliability,
                            "Fault detection: reliability changed"
                        );
                    }
                }
            }))
        } else {
            None
        };

        // Event Enrollment evaluation task (Clause 13.4): evaluates EventEnrollment
        // objects every 10 seconds alongside fault detection.
        let event_enrollment_task = if config.enable_fault_detection {
            let db_ee = Arc::clone(&db);
            Some(tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(10));
                loop {
                    interval.tick().await;
                    let mut db_guard = db_ee.write().await;
                    let transitions =
                        crate::event_enrollment::evaluate_event_enrollments(&mut db_guard);
                    for t in &transitions {
                        debug!(
                            enrollment = %t.enrollment_oid,
                            monitored = %t.monitored_oid,
                            from = ?t.change.from,
                            to = ?t.change.to,
                            "Event enrollment: state changed"
                        );
                    }
                }
            }))
        } else {
            None
        };

        // Trend log polling task: polls TrendLog objects whose log_interval > 0.
        let db_trend = Arc::clone(&db);
        let trend_log_task = Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                interval.tick().await;
                crate::trend_log::poll_trend_logs(&db_trend).await;
            }
        }));

        // Schedule execution task (Clause 12.24): evaluates Schedule objects
        // every 60 seconds and writes to controlled properties.
        let db_schedule = Arc::clone(&db);
        let schedule_tick_task = Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                crate::schedule::tick_schedules(&db_schedule).await;
            }
        }));

        Ok(Self {
            config,
            network,
            db,
            cov_table,
            seg_ack_senders,
            cov_in_flight,
            server_tsm,
            comm_state,
            dcc_timer,
            dispatch_task: Some(dispatch_task),
            cov_purge_task: Some(cov_purge_task),
            fault_detection_task,
            event_enrollment_task,
            trend_log_task,
            schedule_tick_task,
            local_mac,
        })
    }

    /// Get the server's local MAC address.
    pub fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }

    /// Get a reference to the shared object database.
    pub fn database(&self) -> &Arc<RwLock<ObjectDatabase>> {
        &self.db
    }

    /// Get the communication state per DeviceCommunicationControl.
    ///
    /// Returns 0 (Enable), 1 (Disable), or 2 (DisableInitiation).
    pub fn comm_state(&self) -> u8 {
        self.comm_state.load(Ordering::Acquire)
    }

    /// Generate a PICS document from the current object database and server configuration.
    ///
    /// The caller must supply a [`PicsConfig`] for fields not available from the server
    /// (vendor name, model, firmware revision, etc.).
    pub async fn generate_pics(&self, pics_config: &crate::pics::PicsConfig) -> crate::pics::Pics {
        let db = self.db.read().await;
        crate::pics::PicsGenerator::new(&db, &self.config, pics_config).generate()
    }

    /// Stop the server.
    pub async fn stop(&mut self) -> Result<(), Error> {
        if let Some(task) = self.fault_detection_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.event_enrollment_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.trend_log_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.schedule_tick_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.cov_purge_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = self.dispatch_task.take() {
            task.abort();
            let _ = task.await;
        }
        // Network cleanup happens when Arc is dropped (socket close on drop).
        Ok(())
    }

    /// Dispatch a received APDU.
    #[allow(clippy::too_many_arguments)]
    async fn dispatch(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        seg_ack_senders: &Arc<Mutex<HashMap<SegKey, mpsc::Sender<SegmentAckPdu>>>>,
        cov_in_flight: &Arc<Semaphore>,
        server_tsm: &Arc<Mutex<ServerTsm>>,
        comm_state: &Arc<AtomicU8>,
        dcc_timer: &Arc<Mutex<Option<JoinHandle<()>>>>,
        config: &ServerConfig,
        source_mac: &[u8],
        apdu: Apdu,
        mut received: bacnet_network::layer::ReceivedApdu,
    ) {
        match apdu {
            Apdu::ConfirmedRequest(req) => {
                let reply_tx = received.reply_tx.take();
                Self::handle_confirmed_request(
                    db,
                    network,
                    cov_table,
                    seg_ack_senders,
                    cov_in_flight,
                    server_tsm,
                    comm_state,
                    dcc_timer,
                    config,
                    source_mac,
                    req,
                    reply_tx,
                )
                .await;
            }
            Apdu::UnconfirmedRequest(req) => {
                Self::handle_unconfirmed_request(db, network, config, comm_state, req, &received)
                    .await;
            }
            Apdu::SimpleAck(sa) => {
                // Correlate with pending confirmed COV notification.
                let mut tsm = server_tsm.lock().await;
                tsm.record_result(sa.invoke_id, CovAckResult::Ack);
                debug!(
                    invoke_id = sa.invoke_id,
                    "SimpleAck received for outgoing confirmed notification"
                );
            }
            Apdu::Error(err) => {
                // Correlate with pending confirmed COV notification.
                let mut tsm = server_tsm.lock().await;
                tsm.record_result(err.invoke_id, CovAckResult::Error);
                debug!(
                    invoke_id = err.invoke_id,
                    error_class = err.error_class.to_raw(),
                    error_code = err.error_code.to_raw(),
                    "Error received for outgoing confirmed notification"
                );
            }
            Apdu::Reject(rej) => {
                // Treat Reject like Error for COV TSM purposes.
                let mut tsm = server_tsm.lock().await;
                tsm.record_result(rej.invoke_id, CovAckResult::Error);
                debug!(
                    invoke_id = rej.invoke_id,
                    "Reject received for outgoing confirmed notification"
                );
            }
            Apdu::Abort(abort) if !abort.sent_by_server => {
                // Client-sent Abort for our outgoing request.
                let mut tsm = server_tsm.lock().await;
                tsm.record_result(abort.invoke_id, CovAckResult::Error);
                debug!(
                    invoke_id = abort.invoke_id,
                    "Abort received for outgoing confirmed notification"
                );
            }
            Apdu::SegmentAck(sa) => {
                let key = (MacAddr::from_slice(source_mac), sa.invoke_id);
                let senders = seg_ack_senders.lock().await;
                if let Some(tx) = senders.get(&key) {
                    let _ = tx.try_send(sa);
                } else {
                    debug!(
                        invoke_id = sa.invoke_id,
                        "Server ignoring SegmentAck for unknown transaction"
                    );
                }
            }
            _ => {
                debug!("Server ignoring unhandled APDU type");
            }
        }
    }

    /// Handle a confirmed request.
    #[allow(clippy::too_many_arguments)]
    async fn handle_confirmed_request(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        seg_ack_senders: &Arc<Mutex<HashMap<SegKey, mpsc::Sender<SegmentAckPdu>>>>,
        cov_in_flight: &Arc<Semaphore>,
        server_tsm: &Arc<Mutex<ServerTsm>>,
        comm_state: &Arc<AtomicU8>,
        dcc_timer: &Arc<Mutex<Option<JoinHandle<()>>>>,
        config: &ServerConfig,
        source_mac: &[u8],
        req: bacnet_encoding::apdu::ConfirmedRequest,
        reply_tx: Option<tokio::sync::oneshot::Sender<Bytes>>,
    ) {
        let invoke_id = req.invoke_id;
        let service_choice = req.service_choice;
        let client_max_apdu = req.max_apdu_length;
        let client_accepts_segmented = req.segmented_response_accepted;
        let mut written_oids: Vec<ObjectIdentifier> = Vec::new();

        // DCC DISABLE enforcement (Clause 16.4.3):
        // When state == 1 (DISABLE), only DeviceCommunicationControl and
        // ReinitializeDevice are permitted; all other requests are silently dropped.
        let state = comm_state.load(Ordering::Acquire);
        if state == 1
            && service_choice != ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL
            && service_choice != ConfirmedServiceChoice::REINITIALIZE_DEVICE
        {
            debug!(
                service = service_choice.to_raw(),
                "DCC DISABLE: dropping confirmed request"
            );
            return;
        }

        // Helper closures for common response patterns
        let complex_ack = |ack_buf: BytesMut| -> Apdu {
            Apdu::ComplexAck(ComplexAck {
                segmented: false,
                more_follows: false,
                invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice,
                service_ack: ack_buf.freeze(),
            })
        };
        let simple_ack = || -> Apdu {
            Apdu::SimpleAck(SimpleAck {
                invoke_id,
                service_choice,
            })
        };

        let mut ack_buf = BytesMut::with_capacity(512);
        let response = match service_choice {
            s if s == ConfirmedServiceChoice::READ_PROPERTY => {
                let db = db.read().await;
                match handlers::handle_read_property(&db, &req.service_request, &mut ack_buf) {
                    Ok(()) => complex_ack(ack_buf),
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::WRITE_PROPERTY => {
                let result = {
                    let mut db = db.write().await;
                    handlers::handle_write_property(&mut db, &req.service_request)
                };
                match result {
                    Ok(oid) => {
                        written_oids.push(oid);
                        simple_ack()
                    }
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE => {
                let db = db.read().await;
                match handlers::handle_read_property_multiple(
                    &db,
                    &req.service_request,
                    &mut ack_buf,
                ) {
                    Ok(()) => complex_ack(ack_buf),
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE => {
                let result = {
                    let mut db = db.write().await;
                    handlers::handle_write_property_multiple(&mut db, &req.service_request)
                };
                match result {
                    Ok(oids) => {
                        written_oids = oids;
                        simple_ack()
                    }
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::SUBSCRIBE_COV => {
                let db = db.read().await;
                let mut table = cov_table.write().await;
                match handlers::handle_subscribe_cov(
                    &mut table,
                    &db,
                    source_mac,
                    &req.service_request,
                ) {
                    Ok(()) => simple_ack(),
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY => {
                let db = db.read().await;
                let mut table = cov_table.write().await;
                match handlers::handle_subscribe_cov_property(
                    &mut table,
                    &db,
                    source_mac,
                    &req.service_request,
                ) {
                    Ok(()) => simple_ack(),
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::CREATE_OBJECT => {
                let result = {
                    let mut db = db.write().await;
                    handlers::handle_create_object(&mut db, &req.service_request, &mut ack_buf)
                };
                match result {
                    Ok(()) => complex_ack(ack_buf),
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::DELETE_OBJECT => {
                let result = {
                    let mut db = db.write().await;
                    handlers::handle_delete_object(&mut db, &req.service_request)
                };
                match result {
                    Ok(()) => simple_ack(),
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL => {
                match handlers::handle_device_communication_control(
                    &req.service_request,
                    comm_state,
                    &config.dcc_password,
                ) {
                    Ok((_state, duration)) => {
                        // Abort any previous DCC timer.
                        if let Some(prev) = dcc_timer.lock().await.take() {
                            prev.abort();
                        }
                        // If duration is specified for DISABLE/DISABLE_INITIATION,
                        // spawn a timer to auto-revert to ENABLE (Clause 16.4.3).
                        if let Some(minutes) = duration {
                            let comm = Arc::clone(comm_state);
                            let handle = tokio::spawn(async move {
                                tokio::time::sleep(std::time::Duration::from_secs(
                                    minutes as u64 * 60,
                                ))
                                .await;
                                comm.store(0, Ordering::Release);
                                tracing::debug!(
                                    "DCC timer expired after {} min, state reverted to ENABLE",
                                    minutes
                                );
                            });
                            *dcc_timer.lock().await = Some(handle);
                        }
                        simple_ack()
                    }
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::REINITIALIZE_DEVICE => {
                match handlers::handle_reinitialize_device(
                    &req.service_request,
                    &config.reinit_password,
                ) {
                    Ok(()) => simple_ack(),
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::GET_EVENT_INFORMATION => {
                let db = db.read().await;
                match handlers::handle_get_event_information(
                    &db,
                    &req.service_request,
                    &mut ack_buf,
                ) {
                    Ok(()) => complex_ack(ack_buf),
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::ACKNOWLEDGE_ALARM => {
                let mut db = db.write().await;
                match handlers::handle_acknowledge_alarm(&mut db, &req.service_request) {
                    Ok(()) => simple_ack(),
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::READ_RANGE => {
                let db = db.read().await;
                match handlers::handle_read_range(&db, &req.service_request, &mut ack_buf) {
                    Ok(()) => complex_ack(ack_buf),
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::ATOMIC_READ_FILE => {
                let db = db.read().await;
                match handlers::handle_atomic_read_file(&db, &req.service_request, &mut ack_buf) {
                    Ok(()) => complex_ack(ack_buf),
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::ATOMIC_WRITE_FILE => {
                let result = {
                    let mut db = db.write().await;
                    handlers::handle_atomic_write_file(&mut db, &req.service_request, &mut ack_buf)
                };
                match result {
                    Ok(()) => complex_ack(ack_buf),
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::ADD_LIST_ELEMENT => {
                let mut db = db.write().await;
                match handlers::handle_add_list_element(&mut db, &req.service_request) {
                    Ok(()) => simple_ack(),
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            s if s == ConfirmedServiceChoice::REMOVE_LIST_ELEMENT => {
                let mut db = db.write().await;
                match handlers::handle_remove_list_element(&mut db, &req.service_request) {
                    Ok(()) => simple_ack(),
                    Err(e) => Self::error_apdu_from_error(invoke_id, service_choice, &e),
                }
            }
            _ => {
                debug!(
                    service = service_choice.to_raw(),
                    "Unsupported confirmed service"
                );
                Apdu::Reject(RejectPdu {
                    invoke_id,
                    reject_reason: RejectReason::UNRECOGNIZED_SERVICE,
                })
            }
        };

        // Check if segmentation is needed for ComplexAck responses.
        if let Apdu::ComplexAck(ref ack) = response {
            // Encode the full unsegmented response to check its size.
            let mut full_buf = BytesMut::new();
            encode_apdu(&mut full_buf, &response);

            if full_buf.len() > client_max_apdu as usize {
                // Response exceeds the client's max APDU length — segmentation required.
                if !client_accepts_segmented {
                    // Client does not accept segmented responses — send Abort.
                    let abort = Apdu::Abort(AbortPdu {
                        sent_by_server: true,
                        invoke_id,
                        abort_reason: AbortReason::SEGMENTATION_NOT_SUPPORTED,
                    });
                    let mut buf = BytesMut::new();
                    encode_apdu(&mut buf, &abort);
                    if let Err(e) = network
                        .send_apdu(&buf, source_mac, false, NetworkPriority::NORMAL)
                        .await
                    {
                        warn!(error = %e, "Failed to send Abort for segmentation-not-supported");
                    }
                } else {
                    // Spawn the segmented send as a background task so the
                    // dispatch loop can continue processing incoming SegmentAck
                    // PDUs from the client.
                    let network = Arc::clone(network);
                    let seg_ack_senders = Arc::clone(seg_ack_senders);
                    let source_mac = MacAddr::from_slice(source_mac);
                    let service_ack_data = ack.service_ack.clone();
                    tokio::spawn(async move {
                        Self::send_segmented_complex_ack(
                            &network,
                            &seg_ack_senders,
                            &source_mac,
                            invoke_id,
                            service_choice,
                            &service_ack_data,
                            client_max_apdu,
                        )
                        .await;
                    });
                }

                // Fire post-write notifications even for segmented responses.
                for oid in &written_oids {
                    Self::fire_event_notifications(db, network, comm_state, oid).await;
                }
                for oid in &written_oids {
                    Self::fire_cov_notifications(
                        db,
                        network,
                        cov_table,
                        cov_in_flight,
                        server_tsm,
                        comm_state,
                        config,
                        oid,
                    )
                    .await;
                }
                return;
            }
        }

        // Non-segmented path: send the response.
        // If a reply_tx channel is available (MS/TP DataExpectingReply), encode as
        // NPDU and send through the channel for a fast in-window reply. Otherwise
        // fall back to the normal network send_apdu path.
        let mut buf = BytesMut::new();
        encode_apdu(&mut buf, &response);

        if let Some(tx) = reply_tx {
            use bacnet_encoding::npdu::{encode_npdu, Npdu};
            let apdu_bytes = buf.freeze();
            let npdu = Npdu {
                is_network_message: false,
                expecting_reply: false,
                priority: NetworkPriority::NORMAL,
                destination: None,
                source: None,
                payload: apdu_bytes.clone(),
                ..Npdu::default()
            };
            let mut npdu_buf = BytesMut::with_capacity(2 + apdu_bytes.len());
            match encode_npdu(&mut npdu_buf, &npdu) {
                Ok(()) => {
                    let _ = tx.send(npdu_buf.freeze());
                }
                Err(e) => {
                    warn!(error = %e, "Failed to encode NPDU for MS/TP reply");
                    // Fallback: try normal path (reply_tx consumed, so this will go
                    // through the transport as a separate frame)
                    if let Err(e) = network
                        .send_apdu(&apdu_bytes, source_mac, false, NetworkPriority::NORMAL)
                        .await
                    {
                        warn!(error = %e, "Failed to send response");
                    }
                }
            }
        } else if let Err(e) = network
            .send_apdu(&buf, source_mac, false, NetworkPriority::NORMAL)
            .await
        {
            warn!(error = %e, "Failed to send response");
        }

        // Evaluate intrinsic reporting and fire event notifications
        for oid in &written_oids {
            Self::fire_event_notifications(db, network, comm_state, oid).await;
        }

        // Fire COV notifications for any written objects
        for oid in &written_oids {
            Self::fire_cov_notifications(
                db,
                network,
                cov_table,
                cov_in_flight,
                server_tsm,
                comm_state,
                config,
                oid,
            )
            .await;
        }
    }

    /// Send a ComplexAck response using segmented transfer.
    ///
    /// Splits the service ack data into segments that fit within the client's
    /// max APDU length, sends each segment, and waits for SegmentAck from
    /// the client before sending the next (window size 1).
    async fn send_segmented_complex_ack(
        network: &Arc<NetworkLayer<T>>,
        seg_ack_senders: &Arc<Mutex<HashMap<SegKey, mpsc::Sender<SegmentAckPdu>>>>,
        source_mac: &[u8],
        invoke_id: u8,
        service_choice: ConfirmedServiceChoice,
        service_ack_data: &[u8],
        client_max_apdu: u16,
    ) {
        let max_seg_size = max_segment_payload(client_max_apdu, SegmentedPduType::ComplexAck);
        let segments = split_payload(service_ack_data, max_seg_size);
        let total_segments = segments.len();

        if total_segments > 255 {
            warn!(
                total_segments,
                "Response requires too many segments, aborting"
            );
            let abort = Apdu::Abort(AbortPdu {
                sent_by_server: true,
                invoke_id,
                abort_reason: AbortReason::BUFFER_OVERFLOW,
            });
            let mut buf = BytesMut::new();
            encode_apdu(&mut buf, &abort);
            let _ = network
                .send_apdu(&buf, source_mac, false, NetworkPriority::NORMAL)
                .await;
            return;
        }

        debug!(
            total_segments,
            max_seg_size,
            payload_len = service_ack_data.len(),
            "Starting segmented ComplexAck send"
        );

        // Register a channel for receiving SegmentAck PDUs during the send.
        let (seg_ack_tx, mut seg_ack_rx) = mpsc::channel(16);
        let key = (MacAddr::from_slice(source_mac), invoke_id);
        {
            seg_ack_senders.lock().await.insert(key.clone(), seg_ack_tx);
        }

        let seg_timeout = Duration::from_secs(5);
        let mut seg_idx: usize = 0;
        let mut neg_ack_retries: u8 = 0;

        while seg_idx < total_segments {
            let is_last = seg_idx == total_segments - 1;

            let pdu = Apdu::ComplexAck(ComplexAck {
                segmented: true,
                more_follows: !is_last,
                invoke_id,
                sequence_number: Some(seg_idx as u8),
                proposed_window_size: Some(1),
                service_choice,
                service_ack: segments[seg_idx].clone(),
            });

            let mut buf = BytesMut::with_capacity(client_max_apdu as usize);
            encode_apdu(&mut buf, &pdu);

            if let Err(e) = network
                .send_apdu(&buf, source_mac, false, NetworkPriority::NORMAL)
                .await
            {
                warn!(error = %e, seq = seg_idx, "Failed to send segment");
                break;
            }

            debug!(seq = seg_idx, is_last, "Sent ComplexAck segment");

            // Wait for SegmentAck from the client before sending the next segment.
            if !is_last {
                match tokio::time::timeout(seg_timeout, seg_ack_rx.recv()).await {
                    Ok(Some(ack)) => {
                        debug!(
                            seq = ack.sequence_number,
                            negative = ack.negative_ack,
                            "Received SegmentAck for ComplexAck"
                        );
                        if ack.negative_ack {
                            neg_ack_retries += 1;
                            if neg_ack_retries > MAX_NEG_SEGMENT_ACK_RETRIES {
                                warn!(
                                    invoke_id,
                                    retries = neg_ack_retries,
                                    "Too many negative SegmentAck retries, aborting segmented send"
                                );
                                let abort = Apdu::Abort(AbortPdu {
                                    sent_by_server: true,
                                    invoke_id,
                                    abort_reason: AbortReason::TSM_TIMEOUT,
                                });
                                let mut abort_buf = BytesMut::new();
                                encode_apdu(&mut abort_buf, &abort);
                                let _ = network
                                    .send_apdu(
                                        &abort_buf,
                                        source_mac,
                                        false,
                                        NetworkPriority::NORMAL,
                                    )
                                    .await;
                                break;
                            }
                            // Retransmit from the requested sequence number
                            let requested = ack.sequence_number as usize;
                            if requested >= total_segments {
                                tracing::warn!(
                                    seq = requested,
                                    total = total_segments,
                                    "negative SegmentAck requests out-of-range sequence, aborting"
                                );
                                break;
                            }
                            debug!(
                                seq = ack.sequence_number,
                                "Negative SegmentAck — retransmitting from requested sequence"
                            );
                            seg_idx = requested;
                            continue;
                        }
                        // Positive ack — proceed to next segment
                    }
                    Ok(None) => {
                        warn!("SegmentAck channel closed during segmented send");
                        break;
                    }
                    Err(_) => {
                        warn!(
                            seq = seg_idx,
                            "Timeout waiting for SegmentAck, aborting segmented send"
                        );
                        let abort = Apdu::Abort(AbortPdu {
                            sent_by_server: true,
                            invoke_id,
                            abort_reason: AbortReason::TSM_TIMEOUT,
                        });
                        let mut abort_buf = BytesMut::new();
                        encode_apdu(&mut abort_buf, &abort);
                        let _ = network
                            .send_apdu(&abort_buf, source_mac, false, NetworkPriority::NORMAL)
                            .await;
                        break;
                    }
                }
            }

            seg_idx += 1;
        }

        // Wait for final SegmentAck (best-effort, per Clause 9.22)
        match tokio::time::timeout(seg_timeout, seg_ack_rx.recv()).await {
            Ok(Some(_ack)) => {
                debug!("Received final SegmentAck for ComplexAck");
            }
            _ => {
                warn!("No final SegmentAck received for ComplexAck");
            }
        }

        // Clean up the seg_ack channel.
        seg_ack_senders.lock().await.remove(&key);
    }

    /// Handle an unconfirmed request (e.g., WhoIs).
    async fn handle_unconfirmed_request(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        config: &ServerConfig,
        comm_state: &Arc<AtomicU8>,
        req: UnconfirmedRequestPdu,
        received: &bacnet_network::layer::ReceivedApdu,
    ) {
        let comm = comm_state.load(Ordering::Acquire);
        if comm == 1 {
            tracing::debug!("Dropping unconfirmed service: DCC is DISABLE");
            return;
        }

        if req.service_choice == UnconfirmedServiceChoice::WHO_IS {
            let who_is = match WhoIsRequest::decode(&req.service_request) {
                Ok(r) => r,
                Err(e) => {
                    warn!(error = %e, "Failed to decode WhoIs");
                    return;
                }
            };

            let db = db.read().await;
            let device_oid = db
                .list_objects()
                .into_iter()
                .find(|oid| oid.object_type() == ObjectType::DEVICE);

            if let Some(device_oid) = device_oid {
                let instance = device_oid.instance_number();

                let in_range = match (who_is.low_limit, who_is.high_limit) {
                    (Some(low), Some(high)) => instance >= low && instance <= high,
                    _ => true,
                };

                if in_range {
                    let i_am = IAmRequest {
                        object_identifier: device_oid,
                        max_apdu_length: config.max_apdu_length,
                        segmentation_supported: config.segmentation_supported,
                        vendor_id: config.vendor_id,
                    };

                    let mut service_buf = BytesMut::new();
                    i_am.encode(&mut service_buf);

                    let pdu = Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
                        service_choice: UnconfirmedServiceChoice::I_AM,
                        service_request: Bytes::from(service_buf.to_vec()),
                    });

                    let mut buf = BytesMut::new();
                    encode_apdu(&mut buf, &pdu);

                    // Per Clause 16.10: if the WhoIs came from a remote network
                    // (NPDU has SNET/SADR), route the IAm back through the local
                    // router. Otherwise broadcast locally.
                    if let Some(ref source_net) = received.source_network {
                        if let Err(e) = network
                            .send_apdu_routed(
                                &buf,
                                source_net.network,
                                &source_net.mac_address,
                                &received.source_mac,
                                false,
                                NetworkPriority::NORMAL,
                            )
                            .await
                        {
                            warn!(error = %e, "Failed to route IAm back to remote requester");
                        }
                    } else if let Err(e) = network
                        .broadcast_apdu(&buf, false, NetworkPriority::NORMAL)
                        .await
                    {
                        warn!(error = %e, "Failed to send IAm broadcast");
                    }
                }
            }
        } else if req.service_choice == UnconfirmedServiceChoice::WHO_HAS {
            let db = db.read().await;
            let device_oid = db
                .list_objects()
                .into_iter()
                .find(|oid| oid.object_type() == ObjectType::DEVICE);

            if let Some(device_oid) = device_oid {
                match handlers::handle_who_has(&db, &req.service_request, device_oid) {
                    Ok(Some(i_have)) => {
                        let mut service_buf = BytesMut::new();
                        if let Err(e) = i_have.encode(&mut service_buf) {
                            warn!(error = %e, "Failed to encode IHave");
                        } else {
                            let pdu = Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
                                service_choice: UnconfirmedServiceChoice::I_HAVE,
                                service_request: Bytes::from(service_buf.to_vec()),
                            });

                            let mut buf = BytesMut::new();
                            encode_apdu(&mut buf, &pdu);

                            if let Err(e) = network
                                .broadcast_apdu(&buf, false, NetworkPriority::NORMAL)
                                .await
                            {
                                warn!(error = %e, "Failed to send IHave broadcast");
                            }
                        }
                    }
                    Ok(None) => { /* Object not found or not in range — no response */ }
                    Err(e) => {
                        warn!(error = %e, "Failed to decode WhoHas");
                    }
                }
            }
        } else if req.service_choice == UnconfirmedServiceChoice::TIME_SYNCHRONIZATION
            || req.service_choice == UnconfirmedServiceChoice::UTC_TIME_SYNCHRONIZATION
        {
            debug!("Received time synchronization request");
            if let Some(ref callback) = config.on_time_sync {
                let data = TimeSyncData {
                    raw_service_data: req.service_request.clone(),
                    is_utc: req.service_choice
                        == UnconfirmedServiceChoice::UTC_TIME_SYNCHRONIZATION,
                };
                callback(data);
            }
        } else {
            debug!(
                service = req.service_choice.to_raw(),
                "Ignoring unsupported unconfirmed service"
            );
        }
    }

    /// Evaluate intrinsic reporting on an object and send event notifications
    /// to NotificationClass recipients (or broadcast if none configured).
    ///
    /// Called after a successful write to present_value (or any property
    /// that might affect event evaluation).
    ///
    /// Skipped when `comm_state >= 1` (DISABLE or DISABLE_INITIATION) per
    /// BACnet Clause 16.4.3 — the server must not initiate notifications.
    async fn fire_event_notifications(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        comm_state: &Arc<AtomicU8>,
        oid: &ObjectIdentifier,
    ) {
        if comm_state.load(Ordering::Acquire) >= 1 {
            return;
        }

        // Compute current day-of-week bit and time (UTC) for recipient filtering.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let total_secs = now.as_secs();
        // Jan 1, 1970 = Thursday; 0=Sunday, 1=Monday, ..., 6=Saturday
        let dow = ((total_secs / 86400 + 4) % 7) as u8;
        let today_bit = 1u8 << dow;
        let day_secs = (total_secs % 86400) as u32;
        let current_time = Time {
            hour: (day_secs / 3600) as u8,
            minute: ((day_secs % 3600) / 60) as u8,
            second: (day_secs % 60) as u8,
            hundredths: (now.subsec_millis() / 10) as u8,
        };

        let (notification, recipients) = {
            let mut db = db.write().await;

            let device_oid = db
                .list_objects()
                .into_iter()
                .find(|o| o.object_type() == ObjectType::DEVICE)
                .unwrap_or_else(|| ObjectIdentifier::new(ObjectType::DEVICE, 0).unwrap());

            let object = match db.get_mut(oid) {
                Some(o) => o,
                None => return,
            };

            let change = match object.evaluate_intrinsic_reporting() {
                Some(c) => c,
                None => return,
            };

            // Read notification parameters from the object
            let notification_class = object
                .read_property(PropertyIdentifier::NOTIFICATION_CLASS, None)
                .ok()
                .and_then(|v| match v {
                    PropertyValue::Unsigned(n) => Some(n as u32),
                    _ => None,
                })
                .unwrap_or(0);

            let notify_type = object
                .read_property(PropertyIdentifier::NOTIFY_TYPE, None)
                .ok()
                .and_then(|v| match v {
                    PropertyValue::Enumerated(n) => Some(n),
                    _ => None,
                })
                .unwrap_or(NotifyType::ALARM.to_raw());

            // Priority: HIGH_LIMIT/LOW_LIMIT -> high priority (100), NORMAL -> low (200)
            let priority = if change.to == bacnet_types::enums::EventState::NORMAL {
                200u8
            } else {
                100u8
            };

            let transition = change.transition();

            let base_notification = EventNotificationRequest {
                process_identifier: 0,
                initiating_device_identifier: device_oid,
                event_object_identifier: *oid,
                timestamp: BACnetTimeStamp::SequenceNumber(total_secs),
                notification_class,
                priority,
                event_type: change.event_type().to_raw(),
                notify_type,
                ack_required: notify_type != NotifyType::ACK_NOTIFICATION.to_raw(),
                from_state: change.from.to_raw(),
                to_state: change.to.to_raw(),
                event_values: None,
            };

            // Look up recipients from the NotificationClass object
            let recipients = get_notification_recipients(
                &db,
                notification_class,
                transition,
                today_bit,
                &current_time,
            );

            (base_notification, recipients)
        };

        if recipients.is_empty() {
            // No matching recipients — fall back to broadcast (backward compatible)
            let mut service_buf = BytesMut::new();
            if let Err(e) = notification.encode(&mut service_buf) {
                warn!(error = %e, "Failed to encode EventNotification");
                return;
            }

            let pdu = Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
                service_choice: UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION,
                service_request: Bytes::from(service_buf.to_vec()),
            });

            let mut buf = BytesMut::new();
            encode_apdu(&mut buf, &pdu);

            if let Err(e) = network
                .broadcast_apdu(&buf, false, NetworkPriority::NORMAL)
                .await
            {
                warn!(error = %e, "Failed to broadcast EventNotification");
            }
        } else {
            // Send to each matching recipient
            for (recipient, process_id, confirmed) in &recipients {
                let mut targeted = notification.clone();
                targeted.process_identifier = *process_id;

                let mut service_buf = BytesMut::new();
                if let Err(e) = targeted.encode(&mut service_buf) {
                    warn!(error = %e, "Failed to encode EventNotification");
                    continue;
                }

                let service_bytes = Bytes::from(service_buf.to_vec());

                if *confirmed {
                    let pdu = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
                        segmented: false,
                        more_follows: false,
                        segmented_response_accepted: false,
                        max_segments: None,
                        max_apdu_length: 1476,
                        invoke_id: 0, // simplified: no TSM for event notifications yet
                        sequence_number: None,
                        proposed_window_size: None,
                        service_choice: ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
                        service_request: service_bytes,
                    });

                    let mut buf = BytesMut::new();
                    encode_apdu(&mut buf, &pdu);

                    match recipient {
                        bacnet_types::constructed::BACnetRecipient::Address(addr) => {
                            if let Err(e) = network
                                .send_apdu(&buf, &addr.mac_address, true, NetworkPriority::NORMAL)
                                .await
                            {
                                warn!(error = %e, "Failed to send confirmed EventNotification");
                            }
                        }
                        bacnet_types::constructed::BACnetRecipient::Device(_) => {
                            // Cannot resolve Device OID to MAC; broadcast as fallback
                            if let Err(e) = network
                                .broadcast_apdu(&buf, true, NetworkPriority::NORMAL)
                                .await
                            {
                                warn!(error = %e, "Failed to broadcast confirmed EventNotification");
                            }
                        }
                    }
                } else {
                    let pdu = Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
                        service_choice: UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION,
                        service_request: service_bytes,
                    });

                    let mut buf = BytesMut::new();
                    encode_apdu(&mut buf, &pdu);

                    match recipient {
                        bacnet_types::constructed::BACnetRecipient::Address(addr) => {
                            if let Err(e) = network
                                .send_apdu(&buf, &addr.mac_address, false, NetworkPriority::NORMAL)
                                .await
                            {
                                warn!(
                                    error = %e,
                                    "Failed to send unconfirmed EventNotification"
                                );
                            }
                        }
                        bacnet_types::constructed::BACnetRecipient::Device(_) => {
                            // Cannot resolve Device OID to MAC; broadcast as fallback
                            if let Err(e) = network
                                .broadcast_apdu(&buf, false, NetworkPriority::NORMAL)
                                .await
                            {
                                warn!(
                                    error = %e,
                                    "Failed to broadcast unconfirmed EventNotification"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// Fire COV notifications for all active subscriptions on the given object.
    ///
    /// Called after a successful write. Reads the object's Present_Value and
    /// Status_Flags, checks COV_Increment filtering, and sends notifications
    /// to subscribers whose change threshold is met.
    ///
    /// Skipped when `comm_state >= 1` (DISABLE or DISABLE_INITIATION) per
    /// BACnet Clause 16.4.3 — the server must not initiate notifications.
    #[allow(clippy::too_many_arguments)]
    async fn fire_cov_notifications(
        db: &Arc<RwLock<ObjectDatabase>>,
        network: &Arc<NetworkLayer<T>>,
        cov_table: &Arc<RwLock<CovSubscriptionTable>>,
        cov_in_flight: &Arc<Semaphore>,
        server_tsm: &Arc<Mutex<ServerTsm>>,
        comm_state: &Arc<AtomicU8>,
        config: &ServerConfig,
        oid: &ObjectIdentifier,
    ) {
        if comm_state.load(Ordering::Acquire) >= 1 {
            return;
        }
        // 1. Get active subscriptions for this object
        let subs: Vec<crate::cov::CovSubscription> = {
            let mut table = cov_table.write().await;
            table.subscriptions_for(oid).into_iter().cloned().collect()
        };

        if subs.is_empty() {
            return;
        }

        // 2. Read COV-relevant properties and COV_Increment from the object
        let (device_oid, values, current_pv, cov_increment) = {
            let db = db.read().await;
            let object = match db.get(oid) {
                Some(o) => o,
                None => return,
            };

            let cov_increment = object.cov_increment();

            let mut current_pv: Option<f32> = None;
            let mut values = Vec::new();
            if let Ok(pv) = object.read_property(PropertyIdentifier::PRESENT_VALUE, None) {
                if let PropertyValue::Real(v) = &pv {
                    current_pv = Some(*v);
                }
                let mut buf = BytesMut::new();
                if encode_property_value(&mut buf, &pv).is_ok() {
                    values.push(BACnetPropertyValue {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                        value: buf.to_vec(),
                        priority: None,
                    });
                }
            }
            if let Ok(sf) = object.read_property(PropertyIdentifier::STATUS_FLAGS, None) {
                let mut buf = BytesMut::new();
                if encode_property_value(&mut buf, &sf).is_ok() {
                    values.push(BACnetPropertyValue {
                        property_identifier: PropertyIdentifier::STATUS_FLAGS,
                        property_array_index: None,
                        value: buf.to_vec(),
                        priority: None,
                    });
                }
            }

            let device_oid = db
                .list_objects()
                .into_iter()
                .find(|o| o.object_type() == ObjectType::DEVICE)
                .unwrap_or_else(|| ObjectIdentifier::new(ObjectType::DEVICE, 0).unwrap());

            (device_oid, values, current_pv, cov_increment)
        };

        if values.is_empty() {
            return;
        }

        // 3. Send a notification to each subscriber (if change exceeds COV_Increment)
        for sub in &subs {
            if !CovSubscriptionTable::should_notify(sub, current_pv, cov_increment) {
                continue;
            }
            let time_remaining = sub.expires_at.map_or(0, |exp| {
                exp.saturating_duration_since(Instant::now()).as_secs() as u32
            });

            let notification = COVNotificationRequest {
                subscriber_process_identifier: sub.subscriber_process_identifier,
                initiating_device_identifier: device_oid,
                monitored_object_identifier: *oid,
                time_remaining,
                list_of_values: values.clone(),
            };

            let mut service_buf = BytesMut::new();
            notification.encode(&mut service_buf);

            if sub.issue_confirmed_notifications {
                // Acquire a permit from the semaphore to cap in-flight confirmed
                // COV notifications at 255, preventing invoke ID reuse.
                let permit = match cov_in_flight.clone().try_acquire_owned() {
                    Ok(permit) => permit,
                    Err(_) => {
                        warn!(
                            object = ?oid,
                            "255 confirmed COV notifications in-flight, skipping notification"
                        );
                        continue;
                    }
                };

                // Allocate invoke ID from the server TSM.
                let id = {
                    let mut tsm = server_tsm.lock().await;
                    tsm.allocate()
                };

                let pdu = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
                    segmented: false,
                    more_follows: false,
                    segmented_response_accepted: false,
                    max_segments: None,
                    max_apdu_length: config.max_apdu_length as u16,
                    invoke_id: id,
                    sequence_number: None,
                    proposed_window_size: None,
                    service_choice: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
                    service_request: Bytes::from(service_buf.to_vec()),
                });

                let mut buf = BytesMut::new();
                encode_apdu(&mut buf, &pdu);

                // Update last-notified value optimistically for confirmed sends
                if let Some(pv) = current_pv {
                    let mut table = cov_table.write().await;
                    table.set_last_notified_value(
                        &sub.subscriber_mac,
                        sub.subscriber_process_identifier,
                        sub.monitored_object_identifier,
                        pv,
                    );
                }

                let network = Arc::clone(network);
                let mac = sub.subscriber_mac.clone();
                let apdu_timeout = Duration::from_millis(config.cov_retry_timeout_ms);
                let tsm = Arc::clone(server_tsm);
                let apdu_retries = DEFAULT_APDU_RETRIES;
                // The permit is moved into the spawned task; it is automatically
                // released when the task completes (ACK, error, or max retries).
                tokio::spawn(async move {
                    let _permit = permit; // hold until task completes

                    for attempt in 0..=apdu_retries {
                        // Send (or resend) the ConfirmedCOVNotification.
                        if let Err(e) = network
                            .send_apdu(&buf, &mac, true, NetworkPriority::NORMAL)
                            .await
                        {
                            warn!(error = %e, attempt, "COV notification send failed");
                            // Network-level failure: still retry after timeout
                        } else {
                            debug!(invoke_id = id, attempt, "Confirmed COV notification sent");
                        }

                        // Wait up to apdu_timeout for the dispatch loop to
                        // record a result, checking at short intervals.
                        let poll_interval = Duration::from_millis(50);
                        let result = tokio::time::timeout(apdu_timeout, async {
                            loop {
                                tokio::time::sleep(poll_interval).await;
                                let mut tsm = tsm.lock().await;
                                if let Some(r) = tsm.take_result(id) {
                                    return r;
                                }
                            }
                        })
                        .await;

                        match result {
                            Ok(CovAckResult::Ack) => {
                                debug!(invoke_id = id, "COV notification acknowledged");
                                return;
                            }
                            Ok(CovAckResult::Error) => {
                                warn!(invoke_id = id, "COV notification rejected by subscriber");
                                return;
                            }
                            Err(_) => {
                                // Timeout — no response yet.
                                if attempt < apdu_retries {
                                    debug!(
                                        invoke_id = id,
                                        attempt, "COV notification timeout, retrying"
                                    );
                                } else {
                                    warn!(
                                        invoke_id = id,
                                        "COV notification failed after {} retries", apdu_retries
                                    );
                                }
                            }
                        }
                    }

                    // Final cleanup: ensure no stale entry in the TSM map.
                    let mut tsm = tsm.lock().await;
                    tsm.remove(id);
                });
            } else {
                let pdu = Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
                    service_choice: UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION,
                    service_request: Bytes::from(service_buf.to_vec()),
                });

                let mut buf = BytesMut::new();
                encode_apdu(&mut buf, &pdu);

                if let Err(e) = network
                    .send_apdu(&buf, &sub.subscriber_mac, false, NetworkPriority::NORMAL)
                    .await
                {
                    warn!(error = %e, "Failed to send COV notification");
                } else if let Some(pv) = current_pv {
                    // Update last-notified value on successful send
                    let mut table = cov_table.write().await;
                    table.set_last_notified_value(
                        &sub.subscriber_mac,
                        sub.subscriber_process_identifier,
                        sub.monitored_object_identifier,
                        pv,
                    );
                }
            }
        }
    }

    /// Convert an Error into an Error APDU.
    fn error_apdu_from_error(
        invoke_id: u8,
        service_choice: ConfirmedServiceChoice,
        error: &Error,
    ) -> Apdu {
        let (class, code) = match error {
            Error::Protocol { class, code } => (*class, *code),
            _ => (
                ErrorClass::SERVICES.to_raw() as u32,
                ErrorCode::OTHER.to_raw() as u32,
            ),
        };
        Apdu::Error(ErrorPdu {
            invoke_id,
            service_choice,
            error_class: ErrorClass::from_raw(class as u16),
            error_code: ErrorCode::from_raw(code as u16),
            error_data: Bytes::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_config_cov_retry_timeout_default() {
        let config = ServerConfig::default();
        assert_eq!(config.cov_retry_timeout_ms, 3000);
    }

    #[test]
    fn server_config_time_sync_callback_default_is_none() {
        let config = ServerConfig::default();
        assert!(config.on_time_sync.is_none());
    }

    // -----------------------------------------------------------------------
    // ServerTsm unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn server_tsm_allocate_increments() {
        let mut tsm = ServerTsm::new();
        assert_eq!(tsm.allocate(), 0);
        assert_eq!(tsm.allocate(), 1);
        assert_eq!(tsm.allocate(), 2);
    }

    #[test]
    fn server_tsm_allocate_wraps_at_255() {
        let mut tsm = ServerTsm::new();
        tsm.next_invoke_id = 255;
        assert_eq!(tsm.allocate(), 255);
        assert_eq!(tsm.allocate(), 0); // wraps
    }

    #[test]
    fn server_tsm_record_and_take_ack() {
        let mut tsm = ServerTsm::new();
        tsm.record_result(42, CovAckResult::Ack);
        assert_eq!(tsm.take_result(42), Some(CovAckResult::Ack));
        // Second take returns None (already consumed)
        assert_eq!(tsm.take_result(42), None);
    }

    #[test]
    fn server_tsm_record_and_take_error() {
        let mut tsm = ServerTsm::new();
        tsm.record_result(7, CovAckResult::Error);
        assert_eq!(tsm.take_result(7), Some(CovAckResult::Error));
    }

    #[test]
    fn server_tsm_take_nonexistent_returns_none() {
        let mut tsm = ServerTsm::new();
        assert_eq!(tsm.take_result(99), None);
    }

    #[test]
    fn server_tsm_remove_cleans_up() {
        let mut tsm = ServerTsm::new();
        tsm.record_result(10, CovAckResult::Ack);
        tsm.remove(10);
        assert_eq!(tsm.take_result(10), None);
    }

    #[test]
    fn server_tsm_multiple_pending() {
        let mut tsm = ServerTsm::new();
        tsm.record_result(1, CovAckResult::Ack);
        tsm.record_result(2, CovAckResult::Error);
        tsm.record_result(3, CovAckResult::Ack);

        assert_eq!(tsm.take_result(2), Some(CovAckResult::Error));
        assert_eq!(tsm.take_result(1), Some(CovAckResult::Ack));
        assert_eq!(tsm.take_result(3), Some(CovAckResult::Ack));
    }

    #[test]
    fn server_tsm_overwrite_result() {
        let mut tsm = ServerTsm::new();
        tsm.record_result(5, CovAckResult::Ack);
        // Overwrite with Error (e.g., duplicate response)
        tsm.record_result(5, CovAckResult::Error);
        assert_eq!(tsm.take_result(5), Some(CovAckResult::Error));
    }

    #[test]
    fn cov_ack_result_debug_and_eq() {
        // Ensure derived traits work.
        assert_eq!(CovAckResult::Ack, CovAckResult::Ack);
        assert_ne!(CovAckResult::Ack, CovAckResult::Error);
        let _debug = format!("{:?}", CovAckResult::Ack);
    }

    #[test]
    fn default_apdu_retries_constant() {
        assert_eq!(DEFAULT_APDU_RETRIES, 3);
    }

    #[test]
    fn seg_receiver_timeout_is_4s() {
        assert_eq!(SEG_RECEIVER_TIMEOUT, Duration::from_secs(4));
    }

    #[test]
    fn max_neg_segment_ack_retries_constant() {
        assert_eq!(MAX_NEG_SEGMENT_ACK_RETRIES, 3);
    }
}
