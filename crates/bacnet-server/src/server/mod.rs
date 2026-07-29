//! BACnetServer: builder, APDU dispatch, and lifecycle management.
//!
//! The server wraps a NetworkLayer behind Arc (shared with the dispatch task),
//! owns an ObjectDatabase via Arc<Mutex>, and spawns a dispatch task that
//! routes incoming APDUs to service handlers.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Instant;

use bytes::{Bytes, BytesMut};
use tokio::sync::{mpsc, oneshot, watch, Mutex, RwLock, Semaphore};
use tokio::task::JoinHandle;
use tokio::time::Duration;
use tracing::{debug, warn};

use bacnet_encoding::apdu::{
    self, encode_apdu, validate_max_apdu_length, AbortPdu, Apdu, ComplexAck,
    ConfirmedRequest as ConfirmedRequestPdu, ErrorPdu, RejectPdu, SegmentAck as SegmentAckPdu,
    SimpleAck, UnconfirmedRequest as UnconfirmedRequestPdu,
};
use bacnet_encoding::npdu::NpduAddress;
use bacnet_encoding::primitives::{encode_ctx_unsigned, encode_property_value};
use bacnet_encoding::segmentation::{
    max_segment_payload, split_payload, SegmentReceiver, SegmentedPduType,
};
use bacnet_network::layer::NetworkLayer;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::event::EventStateChange;
use bacnet_objects::notification_class::{
    get_notification_recipients, local_day_and_time, resolve_transition_priority_ack,
};
use bacnet_services::alarm_event::EventNotificationRequest;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::cov::COVNotificationRequest;
use bacnet_services::cov_multiple::{
    COVNotificationItem, COVNotificationMultipleRequest, COVNotificationValue,
};
use bacnet_services::who_is::{IAmRequest, WhoIsRequest};
use bacnet_transport::bip::BipTransport;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{
    AbortReason, ConfirmedServiceChoice, ErrorClass, ErrorCode, NetworkPriority, NotifyType,
    ObjectType, PropertyIdentifier, RejectReason, Segmentation, UnconfirmedServiceChoice,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier, PropertyValue};
use bacnet_types::MacAddr;

use crate::cov::{CovNotificationKind, CovSubscription, CovSubscriptionTable};
use crate::handlers;

/// Maximum number of concurrent segmented reassembly sessions.
const MAX_SEG_RECEIVERS: usize = 128;

/// Maximum number of concurrent segmented response send sessions.
const MAX_SEG_SENDERS: usize = 128;

/// Timeout for idle segmented reassembly sessions.
const SEG_RECEIVER_TIMEOUT: Duration = Duration::from_secs(4);

/// Maximum negative SegmentAck retries during segmented response send.
const MAX_NEG_SEGMENT_ACK_RETRIES: u8 = 3;

/// Default timeout while waiting for SegmentACK during segmented response send.
const DEFAULT_APDU_SEGMENT_TIMEOUT: Duration = Duration::from_secs(5);

/// Default retransmission budget for segmented response segments.
const DEFAULT_APDU_SEGMENT_RETRIES: u8 = MAX_NEG_SEGMENT_ACK_RETRIES;

/// Default number of APDU retries for confirmed COV notifications.
const DEFAULT_APDU_RETRIES: u8 = 3;

type TsmPeer = (MacAddr, Option<NpduAddress>);
type TsmKey = (MacAddr, Option<NpduAddress>, u8);

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
    pending: HashMap<TsmKey, oneshot::Sender<CovAckResult>>,
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
    fn allocate(&mut self, peer: TsmPeer) -> Option<(u8, oneshot::Receiver<CovAckResult>)> {
        for offset in 0..=u8::MAX {
            let id = self.next_invoke_id.wrapping_add(offset);
            if !self
                .pending
                .contains_key(&(peer.0.clone(), peer.1.clone(), id))
            {
                self.next_invoke_id = id.wrapping_add(1);
                let rx = self.register(peer, id);
                return Some((id, rx));
            }
        }
        None
    }

    /// Register or replace the pending receiver for a peer/invoke-id pair.
    fn register(&mut self, peer: TsmPeer, invoke_id: u8) -> oneshot::Receiver<CovAckResult> {
        let (tx, rx) = oneshot::channel();
        self.pending.insert((peer.0, peer.1, invoke_id), tx);
        rx
    }

    /// Record a result from the dispatch loop (SimpleAck, Error, etc.).
    /// Sends immediately through the oneshot channel.
    fn record_result(
        &mut self,
        peer: &MacAddr,
        network: Option<&NpduAddress>,
        invoke_id: u8,
        result: CovAckResult,
    ) -> bool {
        if let Some(tx) = self
            .pending
            .remove(&(peer.clone(), network.cloned(), invoke_id))
        {
            let _ = tx.send(result);
            true
        } else {
            false
        }
    }

    /// Remove a pending entry (cleanup on completion or exhaustion).
    fn remove(&mut self, peer: &TsmPeer, invoke_id: u8) {
        self.pending
            .remove(&(peer.0.clone(), peer.1.clone(), invoke_id));
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
    ///
    /// This governs reliability evaluation only. Event Enrollment evaluation
    /// is configured separately via [`enable_event_enrollment`](Self::enable_event_enrollment).
    pub enable_fault_detection: bool,
    /// Enable periodic Event Enrollment evaluation (default `true`).
    ///
    /// When true, the server re-reads the property each Event Enrollment object
    /// names in its `Object_Property_Reference` and applies the configured event
    /// algorithm. Startup: the task is spawned by [`start`](BACnetServer::start)
    /// and its first pass runs immediately, then once per interval. Shutdown:
    /// [`stop`](BACnetServer::stop) aborts it and awaits the abort.
    ///
    /// This switch governs the evaluation task; it is not the per-object
    /// `Event_Detection_Enable` property of ASHRAE 135-2020 Clause 13.2.2.1.
    /// Setting it false stops evaluation without performing the reset that
    /// clause requires of a disabled detector (`Event_State` to NORMAL, with the
    /// corresponding timestamp and acknowledgment state), so a device carrying
    /// active enrollments will hold whatever state it last detected.
    ///
    /// Evaluation is a no-op on databases holding no Event Enrollment objects,
    /// so the default is on.
    ///
    /// What a detected transition currently does is limited: it updates
    /// `Event_State` and is logged. Routing it into the notification pipeline is
    /// not implemented yet (see issue #127), nor are the `Acked_Transitions` and
    /// `Event_Time_Stamps` updates a transition is supposed to carry (#123). A
    /// device holding active enrollments will therefore change `Event_State`
    /// without emitting an EventNotification, so a client learns of the alarm
    /// only by polling. Enabling this by default makes that the standing
    /// behavior rather than an opt-in one.
    pub enable_event_enrollment: bool,
    /// Interval in seconds between Event Enrollment evaluation passes (default 10).
    ///
    /// This is a sampling cadence with no basis in ASHRAE 135-2020, which
    /// prescribes no evaluation frequency and leaves acquisition of a monitored
    /// value a local matter (Clause 12.12). It is not the `Time_Delay` of an
    /// event algorithm, which is how long a condition must persist before a
    /// transition is indicated (Clause 13.3) — a coarse interval delays
    /// detection and can miss a condition that both appears and clears between
    /// two passes.
    ///
    /// A value of `0` is clamped to one second. Ignored when
    /// [`enable_event_enrollment`](Self::enable_event_enrollment) is false.
    pub event_enrollment_interval_secs: u64,
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
            .field("enable_event_enrollment", &self.enable_event_enrollment)
            .field(
                "event_enrollment_interval_secs",
                &self.event_enrollment_interval_secs,
            )
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
            enable_event_enrollment: true,
            event_enrollment_interval_secs: 10,
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
    ///
    /// Reliability evaluation only; Event Enrollment evaluation is configured
    /// by [`enable_event_enrollment`](Self::enable_event_enrollment).
    pub fn enable_fault_detection(mut self, enabled: bool) -> Self {
        self.config.enable_fault_detection = enabled;
        self
    }

    /// Enable periodic Event Enrollment evaluation (default `true`).
    pub fn enable_event_enrollment(mut self, enabled: bool) -> Self {
        self.config.enable_event_enrollment = enabled;
        self
    }

    /// Set the interval in seconds between Event Enrollment evaluation passes
    /// (default 10).
    pub fn event_enrollment_interval_secs(mut self, secs: u64) -> Self {
        self.config.event_enrollment_interval_secs = secs;
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
    ///
    /// Reliability evaluation only; Event Enrollment evaluation is configured
    /// by [`enable_event_enrollment`](Self::enable_event_enrollment).
    pub fn enable_fault_detection(mut self, enabled: bool) -> Self {
        self.config.enable_fault_detection = enabled;
        self
    }

    /// Enable periodic Event Enrollment evaluation (default `true`).
    pub fn enable_event_enrollment(mut self, enabled: bool) -> Self {
        self.config.enable_event_enrollment = enabled;
        self
    }

    /// Set the interval in seconds between Event Enrollment evaluation passes
    /// (default 10).
    pub fn event_enrollment_interval_secs(mut self, secs: u64) -> Self {
        self.config.event_enrollment_interval_secs = secs;
        self
    }

    /// Set the vendor identifier advertised in I-Am responses.
    pub fn vendor_id(mut self, id: u16) -> Self {
        self.config.vendor_id = id;
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

/// Key for tracking in-progress segmented sends:
/// (source MAC/router MAC, optional routed source, invoke_id).
type SegKey = (MacAddr, Option<NpduAddress>, u8);

#[derive(Debug)]
enum SegmentedSendEvent {
    SegmentAck(SegmentAckPdu),
    Abort(AbortPdu),
}

#[derive(Debug, Clone)]
enum SegmentedSendControlEvent {
    Abort(AbortPdu),
    Cancel,
}

struct SegmentedSendHandle {
    segment_ack_tx: mpsc::Sender<SegmentAckPdu>,
    control_tx: watch::Sender<Option<SegmentedSendControlEvent>>,
    closed: AtomicBool,
    current_sequence: AtomicU16,
    total_segments: usize,
}

impl SegmentedSendHandle {
    fn new(
        segment_ack_tx: mpsc::Sender<SegmentAckPdu>,
        control_tx: watch::Sender<Option<SegmentedSendControlEvent>>,
        total_segments: usize,
    ) -> Self {
        Self {
            segment_ack_tx,
            control_tx,
            closed: AtomicBool::new(false),
            current_sequence: AtomicU16::new(u16::MAX),
            total_segments,
        }
    }

    fn accepts_segment_ack(&self, ack: &SegmentAckPdu) -> bool {
        if ack.sent_by_server || self.closed.load(Ordering::Acquire) {
            return false;
        }

        let current = self.current_sequence.load(Ordering::Acquire) as usize;
        if current >= self.total_segments {
            return false;
        }

        if ack.negative_ack {
            let ack_seq = ack.sequence_number as usize;
            let requested = if ack_seq == 0 && current == 0 {
                0
            } else {
                ack_seq.saturating_add(1)
            };
            requested < self.total_segments && requested == current
        } else {
            ack.sequence_number as usize == current
        }
    }

    fn send_control(&self, event: SegmentedSendControlEvent) {
        self.closed.store(true, Ordering::Release);
        self.control_tx.send_replace(Some(event));
    }

    fn same_channel(&self, sender: &mpsc::Sender<SegmentAckPdu>) -> bool {
        self.segment_ack_tx.same_channel(sender)
    }
}

#[derive(Debug, Clone, Copy)]
struct SegmentedSendOptions {
    segment_timeout: Duration,
    max_retries: u8,
}

impl Default for SegmentedSendOptions {
    fn default() -> Self {
        Self {
            segment_timeout: DEFAULT_APDU_SEGMENT_TIMEOUT,
            max_retries: DEFAULT_APDU_SEGMENT_RETRIES,
        }
    }
}

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
    config: ServerConfig,
    /// Shared network layer (also held by dispatch task; read by
    /// [`write_local`](Self::write_local) for post-write COV/event sends).
    network: Arc<NetworkLayer<T>>,
    /// Shared object database.
    db: Arc<RwLock<ObjectDatabase>>,
    /// COV subscription table (also held by dispatch task; read by
    /// [`write_local`](Self::write_local) to fire post-write notifications).
    cov_table: Arc<RwLock<CovSubscriptionTable>>,
    /// Channels for routing segmented-send events to in-progress segmented sends.
    #[allow(dead_code)]
    seg_ack_senders: Arc<Mutex<HashMap<SegKey, Arc<SegmentedSendHandle>>>>,
    /// Permits that cap live segmented response sender tasks, including
    /// cancelled senders that have not yet exited a transport send.
    #[allow(dead_code)]
    seg_send_permits: Arc<Semaphore>,
    /// Semaphore that caps confirmed COV notifications at 255 in-flight
    /// to prevent invoke ID reuse (invoke IDs are u8 = 0..255). Read by
    /// [`write_local`](Self::write_local).
    cov_in_flight: Arc<Semaphore>,
    /// Server-side TSM for outgoing confirmed COV notifications. Read by
    /// [`write_local`](Self::write_local).
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
    /// One-second `Time_Delay` confirmation task for intrinsic reporting.
    intrinsic_reporting_task: Option<JoinHandle<()>>,
    local_mac: MacAddr,
}

/// Cloneable handle for sending unsolicited I-Am announcements.
pub struct IAmBroadcaster<T: TransportPort> {
    config: ServerConfig,
    network: Arc<NetworkLayer<T>>,
    db: Arc<RwLock<ObjectDatabase>>,
}

impl<T: TransportPort> Clone for IAmBroadcaster<T> {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            network: Arc::clone(&self.network),
            db: Arc::clone(&self.db),
        }
    }
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
    ///
    /// Reliability evaluation only; Event Enrollment evaluation is configured
    /// by [`enable_event_enrollment`](Self::enable_event_enrollment).
    pub fn enable_fault_detection(mut self, enabled: bool) -> Self {
        self.config.enable_fault_detection = enabled;
        self
    }

    /// Enable periodic Event Enrollment evaluation (default `true`).
    pub fn enable_event_enrollment(mut self, enabled: bool) -> Self {
        self.config.enable_event_enrollment = enabled;
        self
    }

    /// Set the interval in seconds between Event Enrollment evaluation passes
    /// (default 10).
    pub fn event_enrollment_interval_secs(mut self, secs: u64) -> Self {
        self.config.event_enrollment_interval_secs = secs;
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

        let ws = bacnet_transport::sc_tls::TlsWebSocket::connect(&self.hub_url, tls_config.clone())
            .await?;

        let mut transport = bacnet_transport::sc::ScTransport::new(ws, self.vmac)
            .with_heartbeat_interval_ms(self.heartbeat_interval_ms)
            .with_heartbeat_timeout_ms(self.heartbeat_timeout_ms);
        if let Some(rc) = self.reconnect {
            let hub_url = self.hub_url.clone();
            let tls_config = tls_config.clone();
            #[allow(deprecated)]
            {
                transport = transport
                    .with_connector(move || {
                        let hub_url = hub_url.clone();
                        let tls_config = tls_config.clone();
                        async move {
                            bacnet_transport::sc_tls::TlsWebSocket::connect(&hub_url, tls_config)
                                .await
                        }
                    })
                    .with_reconnect(rc);
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
pub(crate) use requests::{EXECUTED_CONFIRMED, EXECUTED_UNCONFIRMED};
mod responses;
mod segmentation;

#[cfg(test)]
mod cov_notifications_tests;
#[cfg(test)]
mod event_notifications_tests;
#[cfg(test)]
mod segmentation_tests;
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
