//! BACnetServer: builder, APDU dispatch, and lifecycle management.
//!
//! The server wraps a NetworkLayer behind Arc (shared with the dispatch task),
//! owns an ObjectDatabase via `Arc<RwLock>`, and spawns a dispatch task that
//! routes incoming APDUs to service handlers.

use std::collections::HashMap;
use std::net::Ipv4Addr;
#[cfg(test)]
pub(crate) use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use bytes::{Bytes, BytesMut};
use tokio::sync::{mpsc, oneshot, watch, Mutex, RwLock, Semaphore};
use tokio::task::JoinHandle;
use tokio::time::Duration;
use tracing::{debug, warn};

use event_delivery::EventDelivery;

use bacnet_encoding::apdu::{
    self, encode_apdu, AbortPdu, Apdu, ComplexAck, ConfirmedRequest as ConfirmedRequestPdu,
    ErrorPdu, RejectPdu, SegmentAck as SegmentAckPdu, SimpleAck,
    UnconfirmedRequest as UnconfirmedRequestPdu,
};
use bacnet_encoding::npdu::NpduAddress;
use bacnet_encoding::primitives::encode_property_value;
use bacnet_encoding::segmentation::{
    duplicate_in_window, max_segment_payload, split_payload, SegmentReceiver, SegmentedPduType,
};
use bacnet_network::layer::NetworkLayer;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::notification_class::{
    lookup_notification_recipients, resolve_transition_priority_ack,
};
use bacnet_services::alarm_event::EventNotificationRequest;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::cov::COVNotificationRequest;
use bacnet_services::cov_multiple::COVNotificationMultipleRequest;
use bacnet_services::who_is::{DeviceInstanceRange, IAmRequest, WhoIsRequest};
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

use crate::audit_notification::{
    AuditNotificationAuthorizationContext, AuditNotificationAuthorizer,
    UnconfirmedAuditNotificationAuthorizationContext, UnconfirmedAuditNotificationAuthorizer,
    MAX_AUDIT_NOTIFICATIONS, MAX_AUDIT_NOTIFICATION_BYTES,
};
pub use crate::cov::{CovCounters, CovPolicy};
use crate::cov::{
    CovNotificationKind, CovSubscription, CovSubscriptionSnapshot, CovSubscriptionTable,
};
use crate::life_safety::{LifeSafetyOperationAuthorizationContext, LifeSafetyOperationAuthorizer};
use crate::{handlers, runtime_clock};
use confirmed_request_tracker::{
    ConfirmedRequestAdmission, ConfirmedRequestTracker, PendingConfirmedRequest,
};
pub use device_bindings::DeviceBinding;
use device_bindings::{register_configured_binding, DeviceBindingTable};
use learned_router_cache::LearnedRouterCache;
use lso_replay::{LsoAdmission, PendingLsoReplay};
use notification_transactions::{
    canonical_direct_peer, canonical_routed_peer, run_notification_under_dcc,
    run_notification_worker, InitiationRestricted, NotificationTransactions,
    NotificationWorkerResult,
};
use request_services::{DispatchContext, RequestOrigin, RequestServices, UnconfirmedServices};
use requests::confirmed_response::ResponseTarget;
use requests::ConfirmedRequestOwnership;
use segmentation::{ComplexAckParams, SegmentedSendResources};

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

/// Maximum negative SegmentAck retries during segmented response send.
const MAX_NEG_SEGMENT_ACK_RETRIES: u8 = 3;

/// Default timeout while waiting for SegmentACK during segmented response send.
const DEFAULT_APDU_SEGMENT_TIMEOUT: Duration = Duration::from_secs(5);

/// Default retransmission budget for segmented response segments.
const DEFAULT_APDU_SEGMENT_RETRIES: u8 = MAX_NEG_SEGMENT_ACK_RETRIES;

/// Default number of APDU retries for confirmed notifications and the
/// WriteProperty requests Command actions send to other devices.
const DEFAULT_APDU_RETRIES: u8 = 3;

/// Data from a TimeSynchronization request.
#[derive(Debug, Clone)]
pub struct TimeSyncData {
    /// Raw service request bytes (caller can decode if needed).
    pub raw_service_data: Bytes,
    /// Whether this was a UTC time sync (vs. local).
    pub is_utc: bool,
    /// Transport-native source MAC; on SC this is the source VMAC, not a
    /// certificate principal. This metadata is a claimed identity only.
    pub source_mac: MacAddr,
    /// Claimed routed NPDU source, if present; takes precedence for policy matching.
    pub source_network: Option<NpduAddress>,
    /// Honest transport + origin provenance, threaded for diagnostics only
    /// (RB-07 compat mode: decisions unchanged; RB-09 consumes it later).
    pub provenance: bacnet_transport::port::TransportProvenance,
}

mod config;
pub use config::ServerConfig;
mod audit_batch_queue;
mod audit_batch_runtime;
mod audit_context_preparation;
mod audit_forwarder;
#[cfg(test)]
mod audit_forwarder_tests;
mod audit_recipient;
mod audit_recipient_routes;
mod audit_reporter;
mod audit_reporter_changes;
mod audit_send_now;
mod heap_futures;
pub use audit_reporter::{valid_bip_audit_address, AuditReportersConfig};
#[cfg(test)]
mod audit_reporter_tests;

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

    /// Set the policy that authorizes inbound LifeSafetyOperation requests.
    pub fn life_safety_operation_authorizer<F>(mut self, authorizer: F) -> Self
    where
        F: Fn(&LifeSafetyOperationAuthorizationContext) -> bool + Send + Sync + 'static,
    {
        self.config.life_safety_operation_authorizer = Some(Arc::new(authorizer));
        self
    }

    /// Select the only local Audit Log that receives authorized notifications.
    pub fn audit_notification_sink(mut self, sink: ObjectIdentifier) -> Self {
        self.config.audit_notification_sink = Some(sink);
        self
    }

    /// Set the fail-closed ConfirmedAuditNotification authorization policy.
    pub fn audit_notification_authorizer<F>(mut self, authorizer: F) -> Self
    where
        F: Fn(&AuditNotificationAuthorizationContext) -> bool + Send + Sync + 'static,
    {
        self.config.audit_notification_authorizer = Some(Arc::new(authorizer));
        self
    }

    /// Set the fail-closed UnconfirmedAuditNotification authorization policy.
    pub fn unconfirmed_audit_notification_authorizer<F>(mut self, authorizer: F) -> Self
    where
        F: Fn(&UnconfirmedAuditNotificationAuthorizationContext) -> bool + Send + Sync + 'static,
    {
        self.config.unconfirmed_audit_notification_authorizer = Some(Arc::new(authorizer));
        self
    }

    /// Enable periodic fault detection / reliability evaluation.
    ///
    /// When enabled, every object's opt-in reliability hook runs every 10
    /// seconds; the default hook is a no-op.
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

    /// Set the discovery rate-limiting and duplicate suppression policy.
    pub fn discovery_policy(mut self, policy: DiscoveryPolicy) -> Self {
        self.config.discovery_policy = policy;
        self
    }

    /// Set the COV quota and notification work budget policy, checked by
    /// [`CovPolicy::validate`] before transport startup.
    pub fn cov_policy(mut self, policy: CovPolicy) -> Self {
        self.config.cov_policy = policy;
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

    /// Set the policy that authorizes inbound LifeSafetyOperation requests.
    pub fn life_safety_operation_authorizer<F>(mut self, authorizer: F) -> Self
    where
        F: Fn(&LifeSafetyOperationAuthorizationContext) -> bool + Send + Sync + 'static,
    {
        self.config.life_safety_operation_authorizer = Some(Arc::new(authorizer));
        self
    }

    /// Select the only local Audit Log that receives authorized notifications.
    pub fn audit_notification_sink(mut self, sink: ObjectIdentifier) -> Self {
        self.config.audit_notification_sink = Some(sink);
        self
    }

    /// Set the fail-closed ConfirmedAuditNotification authorization policy.
    pub fn audit_notification_authorizer<F>(mut self, authorizer: F) -> Self
    where
        F: Fn(&AuditNotificationAuthorizationContext) -> bool + Send + Sync + 'static,
    {
        self.config.audit_notification_authorizer = Some(Arc::new(authorizer));
        self
    }

    /// Set the fail-closed UnconfirmedAuditNotification authorization policy.
    pub fn unconfirmed_audit_notification_authorizer<F>(mut self, authorizer: F) -> Self
    where
        F: Fn(&UnconfirmedAuditNotificationAuthorizationContext) -> bool + Send + Sync + 'static,
    {
        self.config.unconfirmed_audit_notification_authorizer = Some(Arc::new(authorizer));
        self
    }

    /// Enable periodic fault detection / reliability evaluation.
    ///
    /// When enabled, every object's opt-in reliability hook runs every 10
    /// seconds; the default hook is a no-op.
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

    /// Set the discovery rate-limiting and duplicate suppression policy.
    pub fn discovery_policy(mut self, policy: DiscoveryPolicy) -> Self {
        self.config.discovery_policy = policy;
        self
    }

    /// Set the COV quota and notification work budget policy, checked by
    /// [`CovPolicy::validate`] before transport startup.
    pub fn cov_policy(mut self, policy: CovPolicy) -> Self {
        self.config.cov_policy = policy;
        self
    }

    /// Build and start the server, constructing a BipTransport from the config.
    pub async fn build(self) -> Result<BACnetServer<BipTransport>, Error> {
        let config = &self.config;
        let mut transport =
            BipTransport::new(config.interface, config.port, config.broadcast_address);
        transport.set_share_port_by_address(config.share_port_by_address);
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

type TransportCleanup<T> = JoinHandle<(NetworkLayer<T>, Result<(), Error>)>;

/// BACnet server with APDU dispatch and service handling.
pub struct BACnetServer<T: TransportPort> {
    target_audit: Option<Arc<audit_recipient::TargetAudit<T>>>,
    /// Shared with dispatch and every task the server starts (#1521).
    config: Arc<ServerConfig>,
    discovery_limiter: Arc<DiscoveryLimiter>,
    #[allow(dead_code)] // Retained with the server, including direct dispatch tests.
    time_sync_limiter: Arc<TimeSyncLimiter>,
    /// Server-owned clock controller; absent in explicit clockless mode.
    _clock: Option<Arc<ServerClock>>,
    /// Shared network layer (also held by dispatch task; read by
    /// [`write_local`](Self::write_local) for post-write COV/event sends).
    network: Option<Arc<NetworkLayer<T>>>,
    broadcaster: Arc<broadcaster::BroadcasterState<T>>,
    transport_cleanup: Option<TransportCleanup<T>>,
    transport_cleanup_error: Option<String>,
    network_number_task: Option<JoinHandle<()>>,
    /// Shared object database.
    db: Arc<RwLock<ObjectDatabase>>,
    /// COV subscription table (also held by dispatch task; read by
    /// [`write_local`](Self::write_local) to fire post-write notifications).
    cov_table: Arc<RwLock<CovSubscriptionTable>>,
    cov_counters: Arc<crate::cov::AtomicCovCounters>,
    /// Channels for routing segmented-send events to in-progress segmented sends.
    #[allow(dead_code)]
    seg_ack_senders: Arc<segmented_send::SegmentedSendRegistry>,
    /// Permits that cap live segmented response sender tasks, including
    /// cancelled senders that have not yet exited a transport send.
    #[allow(dead_code)]
    seg_send_permits: Arc<Semaphore>,
    /// Operational cap of 255 concurrent confirmed COV notification workers.
    /// Invoke-ID ownership is handled by `notification_transactions`.
    cov_in_flight: Arc<Semaphore>,
    /// Bounded DNET-to-router cache learned from admitted notification terminals.
    learned_routers: Arc<Mutex<LearnedRouterCache>>,
    /// Invoke-ID ownership and terminal admission for confirmed notifications.
    notification_transactions: Arc<NotificationTransactions>,
    /// Server-lifetime exact inbound ConfirmedRequest duplicate state.
    confirmed_request_tracker: Arc<ConfirmedRequestTracker>,
    /// Shared configured and passively observed Device recipient authority.
    device_bindings: Arc<RwLock<DeviceBindingTable>>,
    /// DeviceCommunicationControl state; only `dcc_timer` changes it.
    comm_state: Arc<CommState>,
    /// DCC timer owner and replacement/expiry serialization boundary.
    /// Valid replacement and explicit stop abort and join before clearing it.
    dcc_timer: Arc<Mutex<crate::server::dcc_timer::TimerSlot>>,
    dcc_outcomes: Arc<dcc_outcomes::DccOutcomes>,
    /// Lifetime totals of undelivered event notifications (#1142).
    event_suppressions: Arc<event_suppression::EventSuppressions>,
    mutation_decisions: Arc<crate::mutation::MutationDecisions>,
    dispatch_task: Option<JoinHandle<()>>,
    request_tasks: Arc<request_tasks::RequestTasks>,
    cov_purge_task: Option<JoinHandle<()>>,
    fault_detection_task: Option<JoinHandle<()>>,
    event_enrollment_task: Option<JoinHandle<()>>,
    trend_log_task: Option<JoinHandle<()>>,
    schedule_tick_task: Option<JoinHandle<()>>,
    /// One-second `Time_Delay` confirmation task for intrinsic reporting.
    intrinsic_reporting_task: Option<JoinHandle<()>>,
    /// Monotonic object-operation task: Binary Lighting Output
    /// WARN_OFF/WARN_RELINQUISH egress and Access Door pulse relock (#1073).
    binary_lighting_operation_task: Option<JoinHandle<()>>,
    /// Follow-up fanout after acknowledged confirmed COV reports (#896).
    cov_revisit_task: Option<JoinHandle<()>>,
    local_mac: MacAddr,
}

/// Cloneable handle for unsolicited I-Am announcements while its server runs.
///
/// At most 32 local broadcasts may be in flight. Admission fails immediately
/// at capacity or after shutdown begins. Admitted sends belong to the server:
/// cancelling their caller only drops its waiter; stop cancels and joins them.
/// Retaining this handle does not retain the transport after shutdown.
pub struct IAmBroadcaster<T: TransportPort> {
    state: std::sync::Weak<broadcaster::BroadcasterState<T>>,
}

impl<T: TransportPort> Clone for IAmBroadcaster<T> {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
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
}

mod clock;
mod command_runs;
mod reinitialize;
mod time_sync_policy;
pub use reinitialize::{ReinitializeContext, ReinitializeHandler};
mod write_group;
#[cfg(test)]
pub(crate) use clock::clocked_test_database;
pub use clock::ClockConfig;
use clock::ServerClock;
use time_sync_policy::{request_limiters, TimeSyncLimiter};
pub use time_sync_policy::{
    TimeSyncPolicy, TimeSyncRateLimit, TimeSyncSource, TimeSyncSourceRestriction,
};
mod binary_lighting_lifecycle;
#[cfg(test)]
mod confirmed_issuance_tests;
mod confirmed_request_tracker;
mod cov_clock;
mod cov_encoding;
mod cov_fanout;
mod cov_notifications;
mod cov_notify_context;
mod cov_snapshot;
mod dcc_disable_rate;
pub(crate) mod dcc_outcomes;
mod dcc_policy;
mod lso_replay;
pub use dcc_disable_rate::DccDisableRateLimit;
mod dcc_timer;
pub use dcc_outcomes::DccOutcomeCounters;
pub use dcc_policy::{DccPolicy, DccSource, DccSourceRestriction};
use dcc_timer::CommState;
pub use dcc_timer::DccState;
mod binding_probes;
mod device_bindings;
mod discovery;
#[doc(hidden)]
pub use discovery::iam_request_for as discovery_iam_for_test;
pub use discovery::{DiscoveryCounters, DiscoveryPolicy};
pub(crate) use discovery::{DiscoveryLimiter, PreCheckDecision, WhoHasTarget};
mod audit_log_purge;
mod dispatch;
mod durable_writes;
#[doc(hidden)]
pub use durable_writes::settle_endpoint as __endpoint_settle_durable_writes;
mod event_delivery;
mod event_enrollment_lifecycle;
mod event_forwarding;
pub use event_forwarding::MAX_FORWARDED_DESTINATIONS;
mod event_forwarding_repeats;
mod event_message_policy;
pub(crate) mod event_notification_payload;
mod event_notifications;
mod event_recipient_route;
mod event_send;
mod event_suppression;
pub use event_suppression::EventNotificationCounters;
mod received_event_log;
pub use received_event_log::{RECEIVED_EVENT_LOG_RATE, RECEIVED_EVENT_LOG_SOURCES};
mod confirmed_answer;
pub(crate) mod event_timestamp;
mod handles;
mod learned_router_cache;
mod lifecycle;
mod local_writes;
mod network_port;
#[cfg(test)]
mod network_port_tests;
pub use confirmed_answer::{CovAckResult, Refusal};
mod remote_writes;
pub(crate) use remote_writes::RemoteRequestError;
mod notification_transactions;
#[doc(hidden)]
pub use notification_transactions::{
    canonical_direct_peer as __endpoint_canonical_direct_peer,
    canonical_routed_peer as __endpoint_canonical_routed_peer,
    run_notification_worker as __endpoint_run_notification_worker,
    AuditFailureContext as __endpoint_AuditFailureContext,
    AuditFailureQueue as __endpoint_AuditFailureQueue,
    AuditFailureTicket as __endpoint_AuditFailureTicket,
    NotificationOperation as __endpoint_NotificationOperation,
    NotificationReserveError as __endpoint_NotificationReserveError,
    NotificationTransactions as __endpoint_NotificationTransactions,
    NotificationWorkerResult as __endpoint_NotificationWorkerResult,
};
mod requests;
#[doc(hidden)]
pub use requests::endpoint_responder::EndpointResponder as __endpoint_EndpointResponder;
#[cfg(feature = "sc-tls")]
mod sc_builder;
#[cfg(test)]
pub(crate) use requests::{EXECUTED_CONFIRMED, EXECUTED_UNCONFIRMED};
#[cfg(test)]
mod audit_notification_tests;
#[cfg(test)]
mod unconfirmed_audit_notification_tests;
#[cfg(feature = "sc-tls")]
pub use sc_builder::ScServerBuilder;
mod responses;
mod segmentation;
mod segmented_receive;
mod segmented_send;
pub(crate) use segmented_send::*;
mod request_admission;
mod rpm_budget;
pub use rpm_budget::ReadPropertyMultipleBudget;
mod alarm_summary_budget;
pub use alarm_summary_budget::GetAlarmSummaryBudget;
mod atomic_read_file_budget;
mod atomic_write_file_budget;
mod read_range_budget;
pub use read_range_budget::ReadRangeBudget;
mod event_information_budget;
pub use event_information_budget::GetEventInformationBudget;
mod enrollment_summary_budget;
pub use atomic_read_file_budget::AtomicReadFileBudget;
pub use atomic_write_file_budget::AtomicWriteFileBudget;
pub use enrollment_summary_budget::GetEnrollmentSummaryBudget;
#[cfg(test)]
mod atomic_read_file_tests;
#[cfg(test)]
mod atomic_write_file_tests;
#[cfg(test)]
mod enrollment_summary_tests;
mod request_peer;
mod request_services;
mod request_tasks;
pub use request_admission::{RequestAdmissionCounters, RequestAdmissionPolicy};
mod shutdown;
pub use shutdown::drop_database_off_runtime;

#[cfg(test)]
mod access_door_event_tests;
#[cfg(test)]
mod access_door_pulse_task_tests;
#[cfg(test)]
mod access_zone_event_tests;
#[cfg(test)]
mod acknowledge_alarm_tests;
#[cfg(test)]
mod active_cov_subscriptions_tests;
#[cfg(test)]
mod audit_log_query_tests;
#[cfg(test)]
mod averaging_reference_write_tests;
#[cfg(test)]
mod averaging_sample_tests;
#[cfg(test)]
mod binary_lighting_task_tests;
#[cfg(test)]
mod channel_member_concurrency_tests;
#[cfg(test)]
mod channel_reliability_tests;
#[cfg(test)]
mod channel_remote_datatype_tests;
#[cfg(test)]
mod channel_remote_write_tests;
#[cfg(test)]
mod channel_run_tests;
#[cfg(test)]
mod channel_wire_tests;
#[cfg(test)]
mod color_channel_member_tests;
#[cfg(test)]
mod color_engine_task_tests;
#[cfg(test)]
mod command_action_run_tests;
#[cfg(test)]
mod command_action_wire_tests;
#[cfg(test)]
mod command_remote_write_tests;
#[cfg(test)]
mod command_run_cancel_tests;
#[cfg(test)]
mod command_run_stop_tests;
#[cfg(test)]
mod confirmed_broadcast_tests;
#[cfg(test)]
mod cov_background_tests;
#[cfg(test)]
mod cov_budget_tests;
#[cfg(test)]
mod cov_confirmed_baseline_tests;
#[cfg(test)]
mod cov_confirmed_context_tests;
#[cfg(test)]
mod cov_multiple_admission_tests;
#[cfg(test)]
mod cov_notifications_tests;
#[cfg(test)]
mod cov_quota_tests;
#[cfg(test)]
mod cov_timed_capture_order_tests;
#[cfg(test)]
mod cov_timed_chunk_tests;
#[cfg(test)]
mod cov_timed_deadline_tests;
#[cfg(test)]
mod cov_timed_multiple_tests;
#[cfg(test)]
mod cov_timed_producer_tests;
#[cfg(test)]
mod cov_timed_value_split_tests;
#[cfg(test)]
mod cov_untimed_split_tests;
#[cfg(test)]
mod cov_wire_test_support;
#[cfg(test)]
mod dcc_event_detection_tests;
#[cfg(test)]
mod device_bindings_tests;
#[cfg(test)]
mod device_recipient_routing_tests;
#[cfg(all(test, feature = "sc-tls"))]
mod direct_principal_tests;
#[cfg(test)]
mod discovery_dcc_tests;
#[cfg(test)]
mod discovery_tests;
#[cfg(test)]
mod event_confirmed_routing_tests;
#[cfg(test)]
mod event_enable_distribution_tests;
#[cfg(test)]
mod event_enrollment_task_tests;
#[cfg(test)]
mod event_forwarding_bounds_tests;
#[cfg(test)]
mod event_forwarding_cap_tests;
#[cfg(test)]
mod event_forwarding_origin_tests;
#[cfg(test)]
mod event_forwarding_rule_tests;
#[cfg(test)]
mod event_forwarding_tests;
#[cfg(test)]
mod event_log_notification_tests;
#[cfg(test)]
mod event_network_priority_tests;
#[cfg(test)]
mod event_notifications_tests;
#[cfg(test)]
mod event_recipient_routing_tests;
#[cfg(test)]
mod life_safety_application_tests;
#[cfg(test)]
mod life_safety_cov_tests;
#[cfg(test)]
mod life_safety_operation_replay_tests;
#[cfg(test)]
mod life_safety_operation_tests;
#[cfg(test)]
mod lighting_command_member_tests;
#[cfg(test)]
mod lighting_engine_task_tests;
#[cfg(test)]
mod lighting_present_value_tests;
#[cfg(test)]
mod list_element_event_tests;
#[cfg(test)]
mod local_network_number_tests;
#[cfg(test)]
mod local_network_outbound_tests;
#[cfg(test)]
mod log_reference_resize_tests;
#[cfg(test)]
mod loop_controlled_variable_tests;
#[cfg(test)]
mod loop_cov_tests;
#[cfg(test)]
mod mistagged_request_wire_tests;
#[cfg(test)]
mod noncommandable_null_cov_tests;
#[cfg(test)]
mod notification_dcc_tests;
#[cfg(test)]
mod notification_transactions_tests;
#[cfg(test)]
mod pulse_converter_cov_tests;
#[cfg(test)]
mod pulse_converter_reference_tests;
#[cfg(test)]
mod rb07_provenance_tests;
#[cfg(test)]
mod reference_write_wire_tests;
#[cfg(test)]
mod remote_write_answer_tests;
#[cfg(test)]
mod remote_write_discovery_tests;
#[cfg(test)]
mod run_cycle_tests;
#[cfg(test)]
mod schedule_create_retry_tests;
#[cfg(test)]
mod schedule_reference_list_tests;
#[cfg(test)]
mod schedule_reference_write_tests;
#[cfg(test)]
mod schedule_write_tests;
#[cfg(test)]
mod segmentation_tests;
#[cfg(test)]
mod staging_cov_tests;
#[cfg(test)]
mod staging_reference_write_tests;
#[cfg(test)]
mod subscribed_recipients_expiry_tests;
#[cfg(test)]
mod table_13_1_cov_tests;
#[cfg(test)]
pub(crate) mod test_forwarder;
#[cfg(test)]
pub(crate) mod test_transport;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod trailing_octet_wire_tests;
#[cfg(test)]
mod truncated_request_wire_tests;
#[cfg(test)]
mod value_cov_increment_tests;
#[cfg(test)]
mod value_source_local_tests;

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Start a server builder for a caller-supplied transport type, with default configuration and
    /// an empty object database.
    pub fn generic_builder() -> ServerBuilder<T> {
        ServerBuilder {
            config: ServerConfig::default(),
            db: ObjectDatabase::new(),
            transport: None,
            configured_device_bindings: Vec::new(),
        }
    }

    /// Get a snapshot of discovery rate-limiting counters.
    pub fn discovery_counters(&self) -> DiscoveryCounters {
        self.discovery_limiter.counters()
    }

    /// Get a snapshot of COV operational and telemetry counters.
    pub fn cov_counters(&self) -> CovCounters {
        self.cov_counters.snapshot()
    }

    /// Sample lifetime mutation authorization decisions, including after `stop()`.
    /// Counts decisions, not completed handlers or response delivery; see
    /// [`MutationDecisionCounters`](crate::mutation::MutationDecisionCounters).
    pub fn mutation_decision_counters(&self) -> crate::mutation::MutationDecisionCounters {
        self.mutation_decisions.snapshot()
    }

    /// Remove COV subscriptions using this current immediate MAC and routed source.
    /// Cleanup of an obsolete router leaves subscriptions renewed onto another route.
    /// Only removed entries release their contribution to the shared quota group.
    pub async fn remove_peer_subscriptions(
        &self,
        mac: &[u8],
        network: Option<&NpduAddress>,
    ) -> usize {
        let mut table = self.cov_table.write().await;
        table.remove_peer_subscriptions(mac, network)
    }
}

#[cfg(test)]
mod cov_identity_completion_tests;

mod broadcaster;
