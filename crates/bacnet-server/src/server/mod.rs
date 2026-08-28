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
use bacnet_encoding::primitives::encode_property_value;
use bacnet_encoding::segmentation::{
    duplicate_in_window, max_segment_payload, split_payload, SegmentReceiver, SegmentedPduType,
};
use bacnet_network::layer::NetworkLayer;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::event::EventStateChange;
use bacnet_objects::notification_class::{
    lookup_notification_recipients, resolve_transition_priority_ack,
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
    AbortReason, ConfirmedServiceChoice, ErrorClass, ErrorCode, LifeSafetyOperation,
    NetworkPriority, NotifyType, ObjectType, PropertyIdentifier, RejectReason, Segmentation,
    UnconfirmedServiceChoice,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bacnet_types::MacAddr;

use crate::cov::{CovNotificationKind, CovSubscription, CovSubscriptionTable};
use crate::handlers;
use crate::life_safety::{LifeSafetyOperationAuthorizationContext, LifeSafetyOperationAuthorizer};
pub use device_bindings::DeviceBinding;
use device_bindings::{register_configured_binding, DeviceBindingTable};
use notification_transactions::{
    canonical_direct_peer, canonical_routed_peer, run_notification_worker,
    NotificationTransactions, NotificationWorkerResult,
};

/// Maximum number of concurrent segmented reassembly sessions.
const MAX_SEG_RECEIVERS: usize = 128;

/// Hard per-request reassembly ceiling: the sequence-number space (#364).
///
/// A local storage bound, not a protocol one. Clause 20.1.2.7 makes the
/// request sequence number modulo 256, so a longer request is entirely
/// representable on the wire — this server simply keys its segment store by
/// that `u8` and cannot tell segment 256 from segment 0. A Device object
/// configured to receive segments does publish a tighter advertisement — its
/// `Max_Segments_Accepted` (Clause 12.11) defaults to `Unsigned(65)` — but
/// enforcing it here is deliberately not done: accepting more segments than
/// advertised is permissive, not a violation, while the sequence space is the
/// line past which acceptance silently corrupts. Exactly 256 segments
/// reassemble correctly and must keep working; 257 is the first that would
/// corrupt the payload.
const MAX_REQUEST_SEGMENTS: usize = 256;

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

/// Legacy server transaction state and learned-router cache.
///
/// The allocation and pending-result methods remain available to existing
/// server internals and tests. Standalone confirmed notification paths use the
/// private endpoint-core adapter instead.
pub struct ServerTsm {
    #[allow(dead_code)]
    next_invoke_id: u8,
    /// Oneshot senders keyed by peer MAC and invoke ID. When a result arrives
    /// from the dispatch loop, we send it directly — no polling needed.
    #[allow(dead_code)]
    pending: HashMap<TsmKey, oneshot::Sender<CovAckResult>>,
    /// Router MACs learned per remote network, Clause 6.5.3 method 4: "using
    /// the local broadcast MAC address in the initial transmission to a device
    /// on a remote DNET and noting the SA associated with any subsequent
    /// responses from the remote device" (#375). Consulted so later confirmed
    /// sends to that DNET can unicast to the router instead of broadcasting.
    routers: HashMap<u16, MacAddr>,
}

/// Cap on learned router entries; a full cache just means later networks keep
/// using the (always-correct) broadcast form of Clause 6.5.3.
const MAX_LEARNED_ROUTERS: usize = 64;

impl ServerTsm {
    fn new() -> Self {
        Self {
            next_invoke_id: 0,
            pending: HashMap::new(),
            routers: HashMap::new(),
        }
    }

    /// Allocate the next invoke ID and register a oneshot channel for the result.
    /// Returns (invoke_id, receiver).
    #[allow(dead_code)]
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
    #[allow(dead_code)]
    fn register(&mut self, peer: TsmPeer, invoke_id: u8) -> oneshot::Receiver<CovAckResult> {
        let (tx, rx) = oneshot::channel();
        self.pending.insert((peer.0, peer.1, invoke_id), tx);
        rx
    }

    /// Record a result from the dispatch loop (SimpleAck, Error, etc.).
    /// Sends immediately through the oneshot channel.
    #[allow(dead_code)]
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
    #[allow(dead_code)]
    fn remove(&mut self, peer: &TsmPeer, invoke_id: u8) {
        self.pending
            .remove(&(peer.0.clone(), peer.1.clone(), invoke_id));
    }

    /// Correlate an inbound response with the transaction awaiting it (#375).
    ///
    /// Three key shapes are tried, most specific first:
    /// 1. exactly as the sender registered it — the immediate MAC plus any
    ///    routed identity;
    /// 2. the router-unknown form — an empty local half with the routed
    ///    identity, used when the request went out via the Clause 6.5.3
    ///    broadcast DA and the delivering router's MAC was unknowable at
    ///    registration;
    /// 3. the legacy wildcard `(empty, None)`, which nothing registers today
    ///    but which older callers may still expect.
    ///
    /// A hit that carries a routed identity also teaches the router cache:
    /// the response's immediate MAC is "the SA associated with [a] subsequent
    /// response from the remote device" (Clause 6.5.3 method 4).
    #[allow(dead_code)]
    fn record_result_correlated(
        &mut self,
        source_mac: &MacAddr,
        source_network: Option<&NpduAddress>,
        invoke_id: u8,
        result: CovAckResult,
    ) -> bool {
        let hit = self.record_result(source_mac, source_network, invoke_id, result)
            || (source_network.is_some()
                && self.record_result(&MacAddr::new(), source_network, invoke_id, result))
            || self.record_result(&MacAddr::new(), None, invoke_id, result);
        if hit {
            if let Some(address) = source_network {
                self.learn_router(address.network, source_mac);
            }
        }
        hit
    }

    /// Cache `router` as the way to reach `network`, bounded by
    /// [`MAX_LEARNED_ROUTERS`].
    fn learn_router(&mut self, network: u16, router: &MacAddr) {
        if router.is_empty() {
            return;
        }
        if self.routers.len() >= MAX_LEARNED_ROUTERS && !self.routers.contains_key(&network) {
            return;
        }
        self.routers.insert(network, router.clone());
    }

    /// The learned router MAC for `network`, if any.
    fn cached_router(&self, network: u16) -> Option<MacAddr> {
        self.routers.get(&network).cloned()
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
    ///
    /// Enforced, not just advertised: the dispatch loop reassembles inbound
    /// segmented requests only under `BOTH`/`RECEIVE` and transmits
    /// segmented responses only under `BOTH`/`TRANSMIT` (Clauses 5.4.5.1 and
    /// 5.4.5.3); anything else draws a SEGMENTATION_NOT_SUPPORTED Abort. The
    /// default is `NONE`, so a default-configured server refuses segmented
    /// traffic in both directions — set this to what the device should
    /// actually honor.
    pub segmentation_supported: Segmentation,
    /// Vendor identifier.
    pub vendor_id: u16,
    /// Timeout in ms before retrying a failed confirmed COV notification send (default 3000ms).
    pub cov_retry_timeout_ms: u64,
    /// Optional observer invoked after a time-synchronization request is accepted.
    pub on_time_sync: Option<Arc<dyn Fn(TimeSyncData) + Send + Sync>>,
    /// Optional LifeSafetyOperation authorization policy.
    ///
    /// Absence is fail-closed: requests receive SERVICES /
    /// SERVICE_REQUEST_DENIED before object mutation.
    pub life_safety_operation_authorizer: Option<LifeSafetyOperationAuthorizer>,
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
    /// Successful enabled transitions commit `Event_State`,
    /// `Acked_Transitions`, and `Event_Time_Stamps` atomically and are then
    /// routed through the shared EventNotification sender. Event Enrollment
    /// message text remains intentionally absent, and exact event-specific
    /// notification values are deferred to the payload projection work.
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
            .field(
                "life_safety_operation_authorizer",
                &self
                    .life_safety_operation_authorizer
                    .as_ref()
                    .map(|_| "<callback>"),
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
            life_safety_operation_authorizer: None,
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
    configured_device_bindings: Vec<DeviceBinding>,
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

    /// Register one explicit unicast route for a Device recipient.
    pub fn device_binding(mut self, binding: DeviceBinding) -> Result<Self, Error> {
        register_configured_binding(&mut self.configured_device_bindings, binding)?;
        Ok(self)
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

    /// Set the policy that authorizes inbound LifeSafetyOperation requests.
    pub fn life_safety_operation_authorizer<F>(mut self, authorizer: F) -> Self
    where
        F: Fn(&LifeSafetyOperationAuthorizationContext) -> bool + Send + Sync + 'static,
    {
        self.config.life_safety_operation_authorizer = Some(Arc::new(authorizer));
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

    /// Set the segmentation support this device advertises and enforces.
    ///
    /// The dispatch loop honors the advertisement (Clause 5.4.5.1): inbound
    /// segmented requests are reassembled only under `BOTH` or `RECEIVE`, and
    /// draw a SEGMENTATION_NOT_SUPPORTED Abort otherwise. The default is
    /// `NONE`.
    pub fn segmentation_supported(mut self, segmentation: Segmentation) -> Self {
        self.config.segmentation_supported = segmentation;
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
        BACnetServer::start_with_clock_mode_and_bindings(
            self.config,
            self.db,
            transport,
            Some(ClockConfig::default()),
            self.configured_device_bindings,
        )
        .await
    }
}

/// BIP-specific builder that constructs `BipTransport` from interface/port/broadcast fields.
pub struct BipServerBuilder {
    config: ServerConfig,
    db: ObjectDatabase,
    configured_device_bindings: Vec<DeviceBinding>,
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

    /// Register one explicit unicast route for a Device recipient.
    pub fn device_binding(mut self, binding: DeviceBinding) -> Result<Self, Error> {
        register_configured_binding(&mut self.configured_device_bindings, binding)?;
        Ok(self)
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

    /// Set the policy that authorizes inbound LifeSafetyOperation requests.
    pub fn life_safety_operation_authorizer<F>(mut self, authorizer: F) -> Self
    where
        F: Fn(&LifeSafetyOperationAuthorizationContext) -> bool + Send + Sync + 'static,
    {
        self.config.life_safety_operation_authorizer = Some(Arc::new(authorizer));
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

    /// Set the segmentation support this device advertises and enforces.
    ///
    /// The dispatch loop honors the advertisement (Clause 5.4.5.1): inbound
    /// segmented requests are reassembled only under `BOTH` or `RECEIVE`, and
    /// draw a SEGMENTATION_NOT_SUPPORTED Abort otherwise. The default is
    /// `NONE`.
    pub fn segmentation_supported(mut self, segmentation: Segmentation) -> Self {
        self.config.segmentation_supported = segmentation;
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
        BACnetServer::start_with_clock_mode_and_bindings(
            self.config,
            self.db,
            transport,
            Some(ClockConfig::default()),
            self.configured_device_bindings,
        )
        .await
    }
}

/// Key for tracking segmented transactions by peer and invoke ID.
type SegKey = (MacAddr, Option<NpduAddress>, u8);

fn segmented_transaction_key(
    source_mac: &[u8],
    source_network: Option<&NpduAddress>,
    invoke_id: u8,
) -> SegKey {
    match source_network {
        Some(address)
            if (1..=0xFFFE).contains(&address.network) && !address.mac_address.is_empty() =>
        {
            (MacAddr::new(), Some(address.clone()), invoke_id)
        }
        _ => (
            MacAddr::from_slice(source_mac),
            source_network.cloned(),
            invoke_id,
        ),
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SegmentAckDisposition {
    Advance,
    Retransmit,
}

fn segment_ack_disposition(
    ack: &SegmentAckPdu,
    current: usize,
    total_segments: usize,
) -> Option<SegmentAckDisposition> {
    if current >= total_segments {
        return None;
    }

    let ack_seq = ack.sequence_number as usize;
    if ack_seq >= total_segments {
        return None;
    }

    // Clause 5.4.4.2 treats either ACK flavor's sequence number as the last
    // segment accepted. A NAK for the preceding segment asks for `current`
    // again; a NAK for `current` confirms it and advances the send window.
    if ack_seq == current {
        Some(SegmentAckDisposition::Advance)
    } else if ack.negative_ack && current.checked_sub(1) == Some(ack_seq) {
        Some(SegmentAckDisposition::Retransmit)
    } else {
        None
    }
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

        segment_ack_disposition(ack, current, self.total_segments).is_some()
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
    /// Last sequence number in the previously completed receive window.
    initial_sequence_number: u8,
    /// Duplicates silently discarded in the current receive window.
    duplicate_count: u8,
    /// Last segment accepted in order (Clause 5.4.2 LastSequenceNumber).
    last_acked_seq: u8,
    window_pos: u8,
    actual_window_size: u8,
    /// Monotonic count of segments accepted in order (#364).
    ///
    /// The reassembly total. `expected_seq` cannot serve: Clause 20.1.2.7
    /// makes the sequence number modulo 256, so a 260-segment request ends at
    /// sequence 3 and `seq + 1` names a four-segment total. This counter also
    /// carries the overrun cap — acceptance is strictly in order, so it
    /// reaches [`MAX_REQUEST_SEGMENTS`] exactly when the sequence number is
    /// about to wrap onto stored segment 0.
    accepted_segments: usize,
}

/// BACnet server with APDU dispatch and service handling.
pub struct BACnetServer<T: TransportPort> {
    config: ServerConfig,
    /// Server-owned clock controller; absent in explicit clockless mode.
    _clock: Option<Arc<ServerClock>>,
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
    /// Operational cap of 255 concurrent confirmed COV notification workers.
    /// Invoke-ID ownership is handled by `notification_transactions`.
    cov_in_flight: Arc<Semaphore>,
    /// Legacy public TSM state and the learned DNET-to-router cache.
    server_tsm: Arc<Mutex<ServerTsm>>,
    /// Invoke-ID ownership and terminal admission for confirmed notifications.
    notification_transactions: Arc<NotificationTransactions>,
    /// Shared configured and passively observed Device recipient authority.
    device_bindings: Arc<RwLock<DeviceBindingTable>>,
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
            configured_device_bindings: Vec::new(),
        }
    }

    /// Create a BIP-specific builder (alias for backward compatibility).
    pub fn builder() -> BipServerBuilder {
        Self::bip_builder()
    }
}

mod clock;
#[cfg(test)]
pub(crate) use clock::clocked_test_database;
pub use clock::ClockConfig;
use clock::ServerClock;
mod cov_clock;
mod cov_notifications;
mod device_bindings;
mod dispatch;
mod event_enrollment_lifecycle;
mod event_message_policy;
mod event_notifications;
mod event_recipient_route;
pub(crate) mod event_timestamp;
mod lifecycle;
mod notification_transactions;
mod requests;
#[cfg(feature = "sc-tls")]
mod sc_builder;
#[cfg(test)]
pub(crate) use requests::{EXECUTED_CONFIRMED, EXECUTED_UNCONFIRMED};
#[cfg(feature = "sc-tls")]
pub use sc_builder::ScServerBuilder;
mod responses;
mod segmentation;
mod segmented_receive;
mod shutdown;

#[cfg(test)]
mod cov_notifications_tests;
#[cfg(test)]
mod dcc_event_detection_tests;
#[cfg(test)]
mod device_bindings_tests;
#[cfg(test)]
mod device_recipient_routing_tests;
#[cfg(test)]
mod event_confirmed_routing_tests;
#[cfg(test)]
mod event_enable_distribution_tests;
#[cfg(test)]
mod event_enrollment_task_tests;
#[cfg(test)]
mod event_network_priority_tests;
#[cfg(test)]
mod event_notifications_tests;
#[cfg(test)]
mod event_recipient_routing_tests;
#[cfg(test)]
mod life_safety_operation_tests;
#[cfg(test)]
mod notification_transactions_tests;
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
            configured_device_bindings: Vec::new(),
        }
    }
}
