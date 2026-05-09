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
use tokio::sync::{mpsc, oneshot, Mutex, RwLock, Semaphore};
use tokio::task::JoinHandle;
use tokio::time::Duration;
use tracing::{debug, warn};

use bacnet_encoding::apdu::{
    self, encode_apdu, validate_max_apdu_length, AbortPdu, Apdu, ComplexAck,
    ConfirmedRequest as ConfirmedRequestPdu, ErrorPdu, RejectPdu, SegmentAck as SegmentAckPdu,
    SimpleAck, UnconfirmedRequest as UnconfirmedRequestPdu,
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

/// Timeout for idle segmented reassembly sessions.
const SEG_RECEIVER_TIMEOUT: Duration = Duration::from_secs(4);

/// Maximum negative SegmentAck retries during segmented response send.
const MAX_NEG_SEGMENT_ACK_RETRIES: u8 = 3;

/// Default number of APDU retries for confirmed COV notifications.
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
    /// Oneshot senders keyed by peer MAC and invoke ID. When a result arrives
    /// from the dispatch loop, we send it directly — no polling needed.
    pending: HashMap<(MacAddr, u8), oneshot::Sender<CovAckResult>>,
}

impl ServerTsm {
    fn new() -> Self {
        Self {
            next_invoke_id: 0,
            pending: HashMap::new(),
        }
    }

    /// Allocate the next invoke ID and register a oneshot channel for the result.
    /// Returns (invoke_id, receiver).
    fn allocate(&mut self, peer: MacAddr) -> (u8, oneshot::Receiver<CovAckResult>) {
        let id = self.next_invoke_id;
        self.next_invoke_id = self.next_invoke_id.wrapping_add(1);
        let rx = self.register(peer, id);
        (id, rx)
    }

    /// Register or replace the pending receiver for a peer/invoke-id pair.
    fn register(&mut self, peer: MacAddr, invoke_id: u8) -> oneshot::Receiver<CovAckResult> {
        let (tx, rx) = oneshot::channel();
        self.pending.insert((peer, invoke_id), tx);
        rx
    }

    /// Record a result from the dispatch loop (SimpleAck, Error, etc.).
    /// Sends immediately through the oneshot channel.
    fn record_result(&mut self, peer: &MacAddr, invoke_id: u8, result: CovAckResult) -> bool {
        if let Some(tx) = self.pending.remove(&(peer.clone(), invoke_id)) {
            let _ = tx.send(result);
            true
        } else {
            false
        }
    }

    /// Remove a pending entry (cleanup on completion or exhaustion).
    fn remove(&mut self, peer: &MacAddr, invoke_id: u8) {
        self.pending.remove(&(peer.clone(), invoke_id));
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
    /// Optional password required for DeviceCommunicationControl.
    pub dcc_password: Option<String>,
    /// Optional password required for ReinitializeDevice.
    pub reinit_password: Option<String>,
    /// Enable periodic fault detection / reliability evaluation.
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

    /// Set the vendor identifier (used in IAm responses and protocol operations).
    pub fn vendor_id(mut self, id: u16) -> Self {
        self.config.vendor_id = id;
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

struct SegmentedRequestState {
    receiver: SegmentReceiver,
    first_req: bacnet_encoding::apdu::ConfirmedRequest,
    last_activity: Instant,
    expected_seq: u8,
    last_acked_seq: u8,
    window_pos: u8,
    actual_window_size: u8,
}

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
    /// Server-side TSM for outgoing confirmed COV notifications.
    #[allow(dead_code)]
    server_tsm: Arc<Mutex<ServerTsm>>,
    /// Communication state: 0 = Enable, 1 = Disable, 2 = DisableInitiation.
    comm_state: Arc<AtomicU8>,
    /// Handle for the DCC auto-re-enable timer. A new DCC request aborts
    /// any previous timer.
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

mod cov_notifications;
mod dispatch;
mod event_notifications;
mod lifecycle;
mod requests;
mod segmentation;

#[cfg(test)]
mod tests;

impl<T: TransportPort + 'static> BACnetServer<T> {
    pub fn generic_builder() -> ServerBuilder<T> {
        ServerBuilder {
            config: ServerConfig::default(),
            db: ObjectDatabase::new(),
            transport: None,
        }
    }
}
