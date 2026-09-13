use super::*;
use crate::mutation::{MutationAuthorizationContext, MutationAuthorizer, MutationPolicy};

/// Server configuration.
#[derive(Clone)]
pub struct ServerConfig {
    /// Per-service GetAlarmSummary database scan and encoded response limits.
    pub get_alarm_summary_budget: GetAlarmSummaryBudget,
    /// Local complete-response limits for GetEnrollmentSummary.
    pub get_enrollment_summary_budget: GetEnrollmentSummaryBudget,
    /// Local AtomicReadFile raw-count and complete service-ACK limits.
    pub atomic_read_file_budget: AtomicReadFileBudget,
    /// Local AtomicWriteFile payload admission limits.
    pub atomic_write_file_budget: AtomicWriteFileBudget,
    /// ReadRange directional page item and logical service-byte limits.
    pub read_range_budget: ReadRangeBudget,
    /// GetEventInformation database admission and strict response-page limits.
    pub get_event_information_budget: GetEventInformationBudget,
    /// Per-service RPM work and encoded response limits (finite by default).
    pub read_property_multiple_budget: ReadPropertyMultipleBudget,
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
    /// Opt-in inbound time-sync restrictions; default allows all, with no step cap.
    pub time_sync_policy: TimeSyncPolicy,
    /// Optional fast, nonblocking observer invoked after the clock changes.
    /// A panic is caught (with unwind builds); the change stands and ingress
    /// continues. This callback cannot authorize or roll back synchronization.
    pub on_time_sync: Option<Arc<dyn Fn(TimeSyncData) + Send + Sync>>,
    /// Local mutation authorization mode (default: permissive). SC mTLS channel/peer
    /// authentication is not service authorization; addresses here are claimed,
    /// never certificate principals. See [`MutationPolicy`].
    pub mutation_policy: MutationPolicy,
    /// Opt-in mutation authorizer; `None` allows only in permissive mode.
    /// See [`MutationAuthorizer`].
    pub mutation_authorizer: Option<MutationAuthorizer>,
    /// Optional LifeSafetyOperation authorization policy.
    ///
    /// Absence is fail-closed: requests receive SERVICES /
    /// SERVICE_REQUEST_DENIED before object mutation.
    pub life_safety_operation_authorizer: Option<LifeSafetyOperationAuthorizer>,
    /// Exactly one explicitly configured Audit Log notification sink.
    ///
    /// Absence is fail-closed; the server never selects a sink by database
    /// iteration order.
    pub audit_notification_sink: Option<ObjectIdentifier>,
    /// Optional fast, nonblocking ConfirmedAuditNotification authorizer.
    ///
    /// Absence, `false`, or a panic denies the request before mutation.
    pub audit_notification_authorizer: Option<AuditNotificationAuthorizer>,
    /// Optional fast, nonblocking UnconfirmedAuditNotification authorizer.
    ///
    /// Absence, `false`, or a panic silently denies the request before mutation.
    pub unconfirmed_audit_notification_authorizer: Option<UnconfirmedAuditNotificationAuthorizer>,
    /// Optional password required for DeviceCommunicationControl.
    pub dcc_password: Option<String>,
    /// Local DCC authorization. Supplying a password alone does not enable DCC.
    pub dcc_policy: DccPolicy,
    /// Optional exact claimed-source restriction, valid only with RequirePassword.
    pub dcc_source_restriction: Option<DccSourceRestriction>,
    /// Optional global DISABLE_INITIATION budget; does not enable DCC authorization.
    pub dcc_disable_rate_limit: Option<DccDisableRateLimit>,
    /// Optional password required for ReinitializeDevice.
    pub reinit_password: Option<String>,
    /// Enable periodic fault detection / reliability evaluation.
    /// When true, the server invokes every object's opt-in, object-owned
    /// reliability evaluation hook every 10 seconds. Stock objects currently
    /// inherit the no-op default.
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
    /// Discovery rate-limiting and duplicate suppression policy.
    pub discovery_policy: DiscoveryPolicy,
    /// COV quota, rate accounting, and notification work budget policy.
    pub cov_policy: CovPolicy,
    /// Positive independent top-level request handler limits.
    pub request_admission_policy: RequestAdmissionPolicy,
}

impl std::fmt::Debug for ServerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerConfig")
            .field("get_alarm_summary_budget", &self.get_alarm_summary_budget)
            .field(
                "get_enrollment_summary_budget",
                &self.get_enrollment_summary_budget,
            )
            .field("atomic_read_file_budget", &self.atomic_read_file_budget)
            .field("atomic_write_file_budget", &self.atomic_write_file_budget)
            .field("read_range_budget", &self.read_range_budget)
            .field(
                "get_event_information_budget",
                &self.get_event_information_budget,
            )
            .field(
                "read_property_multiple_budget",
                &self.read_property_multiple_budget,
            )
            .field("interface", &self.interface)
            .field("port", &self.port)
            .field("broadcast_address", &self.broadcast_address)
            .field("max_apdu_length", &self.max_apdu_length)
            .field("segmentation_supported", &self.segmentation_supported)
            .field("vendor_id", &self.vendor_id)
            .field("cov_retry_timeout_ms", &self.cov_retry_timeout_ms)
            .field("time_sync_policy", &self.time_sync_policy)
            .field("mutation_policy", &self.mutation_policy)
            .field(
                "on_time_sync",
                &self.on_time_sync.as_ref().map(|_| "<callback>"),
            )
            .field(
                "mutation_authorizer",
                &self.mutation_authorizer.as_ref().map(|_| "<callback>"),
            )
            .field(
                "life_safety_operation_authorizer",
                &self
                    .life_safety_operation_authorizer
                    .as_ref()
                    .map(|_| "<callback>"),
            )
            .field("audit_notification_sink", &self.audit_notification_sink)
            .field(
                "audit_notification_authorizer",
                &self
                    .audit_notification_authorizer
                    .as_ref()
                    .map(|_| "<callback>"),
            )
            .field(
                "unconfirmed_audit_notification_authorizer",
                &self
                    .unconfirmed_audit_notification_authorizer
                    .as_ref()
                    .map(|_| "<callback>"),
            )
            .field("dcc_password", &self.dcc_password.as_ref().map(|_| "***"))
            .field("dcc_policy", &self.dcc_policy)
            .field("dcc_source_restriction", &self.dcc_source_restriction)
            .field("dcc_disable_rate_limit", &self.dcc_disable_rate_limit)
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
            .field("discovery_policy", &self.discovery_policy)
            .field("cov_policy", &self.cov_policy)
            .field("request_admission_policy", &self.request_admission_policy)
            .finish()
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            interface: Ipv4Addr::UNSPECIFIED,
            read_property_multiple_budget: ReadPropertyMultipleBudget::default(),
            get_alarm_summary_budget: GetAlarmSummaryBudget::default(),
            get_enrollment_summary_budget: GetEnrollmentSummaryBudget::default(),
            atomic_read_file_budget: AtomicReadFileBudget::default(),
            atomic_write_file_budget: AtomicWriteFileBudget::default(),
            read_range_budget: ReadRangeBudget::default(),
            get_event_information_budget: GetEventInformationBudget::default(),
            port: 0xBAC0,
            broadcast_address: Ipv4Addr::BROADCAST,
            max_apdu_length: 1476,
            segmentation_supported: Segmentation::NONE,
            vendor_id: 0,
            cov_retry_timeout_ms: 3000,
            time_sync_policy: TimeSyncPolicy::default(),
            on_time_sync: None,
            mutation_policy: MutationPolicy::default(),
            mutation_authorizer: None,
            life_safety_operation_authorizer: None,
            audit_notification_sink: None,
            audit_notification_authorizer: None,
            unconfirmed_audit_notification_authorizer: None,
            dcc_password: None,
            dcc_policy: DccPolicy::default(),
            dcc_source_restriction: None,
            dcc_disable_rate_limit: None,
            reinit_password: None,
            enable_fault_detection: false,
            enable_event_enrollment: true,
            event_enrollment_interval_secs: 10,
            discovery_policy: DiscoveryPolicy::default(),
            cov_policy: CovPolicy::default(),
            request_admission_policy: RequestAdmissionPolicy::default(),
        }
    }
}

impl<T: TransportPort + 'static> ServerBuilder<T> {
    /// Select local mutation authorization (default: [`MutationPolicy::Permissive`]).
    /// SC mTLS channel/peer authentication is not service authorization; identities
    /// here are claimed link/routed addresses, never certificate principals.
    pub fn mutation_policy(mut self, policy: MutationPolicy) -> Self {
        self.config.mutation_policy = policy;
        self
    }

    /// Set opt-in mutation policy. See [`MutationAuthorizer`] for callback rules.
    pub fn mutation_authorizer<F>(mut self, authorizer: F) -> Self
    where
        F: Fn(&MutationAuthorizationContext) -> bool + Send + Sync + 'static,
    {
        self.config.mutation_authorizer = Some(Arc::new(authorizer));
        self
    }
}

impl BipServerBuilder {
    /// Select local mutation authorization (default: [`MutationPolicy::Permissive`]).
    /// SC mTLS channel/peer authentication is not service authorization; identities
    /// here are claimed link/routed addresses, never certificate principals.
    ///
    /// ```
    /// use bacnet_server::{mutation::MutationPolicy, server::BACnetServer};
    /// let builder = BACnetServer::bip_builder().mutation_policy(MutationPolicy::DenyAll);
    /// ```
    pub fn mutation_policy(mut self, policy: MutationPolicy) -> Self {
        self.config.mutation_policy = policy;
        self
    }

    /// Set opt-in mutation policy. See [`MutationAuthorizer`] for callback rules.
    ///
    /// ```
    /// use bacnet_server::server::BACnetServer;
    /// let builder = BACnetServer::bip_builder().mutation_authorizer(|_| false);
    /// ```
    pub fn mutation_authorizer<F>(mut self, authorizer: F) -> Self
    where
        F: Fn(&MutationAuthorizationContext) -> bool + Send + Sync + 'static,
    {
        self.config.mutation_authorizer = Some(Arc::new(authorizer));
        self
    }
}

#[cfg(feature = "sc-tls")]
impl ScServerBuilder {
    /// Select local mutation authorization (default: [`MutationPolicy::Permissive`]).
    /// SC mTLS channel/peer authentication is not service authorization; identities
    /// here are claimed link/routed addresses, never certificate principals.
    pub fn mutation_policy(mut self, policy: MutationPolicy) -> Self {
        self.config.mutation_policy = policy;
        self
    }

    /// Set opt-in mutation policy. See [`MutationAuthorizer`] for callback rules.
    pub fn mutation_authorizer<F>(mut self, authorizer: F) -> Self
    where
        F: Fn(&MutationAuthorizationContext) -> bool + Send + Sync + 'static,
    {
        self.config.mutation_authorizer = Some(Arc::new(authorizer));
        self
    }
}
