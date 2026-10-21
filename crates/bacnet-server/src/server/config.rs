use super::*;
use crate::mutation::MutationPolicy;
use crate::ServerPolicy;

/// Server configuration.
#[derive(Clone)]
pub struct ServerConfig {
    /// Explicit concrete Network Port for one owned B/IP transport, in
    /// NORMAL, FOREIGN or BBMD mode (a BBMD that also registers as a foreign
    /// device is refused at start). None leaves configured objects unbound
    /// and receiving-port wildcard unavailable.
    pub registered_network_port: Option<ObjectIdentifier>,
    /// Optional target Audit Reporter profile. Its recipient is provisioned on
    /// the built-in Device and becomes writable while the runtime is installed.
    pub audit_reporters: Option<AuditReportersConfig>,
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
    /// B/IP only: bind the interface address itself, so devices on other
    /// addresses of this host can share the port (#1538). Off by default.
    /// Needs an explicit interface and a nonzero port; see
    /// `BipTransport::set_share_port_by_address` for what changes. Only the
    /// B/IP builder reads it; a transport passed to `start` has its own.
    pub share_port_by_address: bool,
    /// Device declarations shared by stock objects, discovery and active limits.
    /// Startup resolves its receive ceiling against the owned transport. An
    /// explicitly supplied selected Device must match; an empty database may
    /// later add this same OID, but never rebinds the runtime to another Device.
    pub device: bacnet_objects::DeviceIdentity,
    /// APDU timeout in ms of confirmed COV and event notifications and of the
    /// WriteProperty a Command action sends to another device (default
    /// 3000 ms): how long each attempt waits for the reply once its send has
    /// completed, before the next retry. The transport bounds the send itself.
    /// It is also the base of the COV hold-off after a failed confirmed report,
    /// one full retry cycle: this timeout times the attempts (#896).
    pub cov_retry_timeout_ms: u64,
    /// Opt-in inbound time-sync restrictions; default allows all, with no step cap.
    pub time_sync_policy: TimeSyncPolicy,
    /// Optional fast, nonblocking observer invoked after the clock changes.
    /// A panic is caught (with unwind builds); the change stands and ingress
    /// continues. This callback cannot authorize or roll back synchronization.
    pub on_time_sync: Option<Arc<dyn Fn(TimeSyncData) + Send + Sync>>,
    /// Optional ReinitializeDevice handler, called with the requester's
    /// [`ReinitializeContext`] once the request passes its password and state
    /// checks. Without it every such request is refused with
    /// SERVICES / SERVICE_REQUEST_DENIED.
    ///
    /// The rules are on [`ReinitializeHandler`]. In short: the SimpleACK goes
    /// out only after the handler returns, so register any restart with
    /// [`ReinitializeContext::after_reply`] rather than restarting inline; it
    /// runs once the reply has been handed to the transport (#1565), and
    /// never for a refusal. The handler runs synchronously with the
    /// object database write-locked, so keep it quick and hand slow work to a
    /// task; its database edits skip the COV, event and Audit work of
    /// [`BACnetServer::write_local`], so apply changes that need it there
    /// afterwards. Without [`reinit_password`](Self::reinit_password) any peer
    /// reaches it, and neither [`mutation_policy`](Self::mutation_policy) nor
    /// [`policy`](Self::policy) covers this service, so restrict sources
    /// through the context. Refuse
    /// with [`Error::Protocol`]: a panic or an [`Error::Reject`] is answered
    /// SERVICES / OTHER.
    pub on_reinitialize: Option<ReinitializeHandler>,
    /// Local mutation authorization mode (default: permissive). SC mTLS channel/peer
    /// authentication is not service authorization. See [`MutationPolicy`].
    /// `DenyAll` denies every covered mutation without asking
    /// [`policy`](Self::policy), each Channel write of an inbound WriteGroup
    /// included, counted in `BACnetServer::mutation_decision_counters` (#1319).
    /// Denials mutate nothing and write no audit log.
    pub mutation_policy: MutationPolicy,
    /// The application's authorization decisions (default: none). See
    /// [`ServerPolicy`] for what it decides, each method's default and the
    /// direct-SC principal its contexts carry. `None` behaves as those
    /// defaults: covered mutations are allowed (without being decoded for
    /// authorization first), and LifeSafetyOperation and both
    /// AuditNotification services are denied.
    pub policy: Option<Arc<dyn ServerPolicy>>,
    /// Exactly one explicitly configured Audit Log notification sink.
    ///
    /// Absence is fail-closed; the server never selects a sink by database
    /// iteration order. Storing a notification also needs
    /// [`policy`](Self::policy) to authorize it.
    /// A built-in AuditLogObject configured with `set_member_of` also forwards
    /// record-changing accepted batches once, after commit, to that parent using
    /// a configured DeviceBinding. No backlog, retries, or durable send progress.
    pub audit_notification_sink: Option<ObjectIdentifier>,
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
            .field("audit_reporters", &self.audit_reporters)
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
            .field("share_port_by_address", &self.share_port_by_address)
            .field("device", &self.device)
            .field("cov_retry_timeout_ms", &self.cov_retry_timeout_ms)
            .field("time_sync_policy", &self.time_sync_policy)
            .field("mutation_policy", &self.mutation_policy)
            .field(
                "on_time_sync",
                &self.on_time_sync.as_ref().map(|_| "<callback>"),
            )
            .field(
                "on_reinitialize",
                &self.on_reinitialize.as_ref().map(|_| "<callback>"),
            )
            .field("policy", &self.policy.as_ref().map(|_| "<policy>"))
            .field("audit_notification_sink", &self.audit_notification_sink)
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

impl ServerConfig {
    /// The installed policy, if it decides mutations
    /// ([`ServerPolicy::decides_mutations`]). `None` takes the no-policy
    /// path: allowed without decoding for authorization.
    pub(crate) fn mutation_decider(&self) -> Option<&dyn ServerPolicy> {
        self.policy
            .as_deref()
            .filter(|policy| policy.decides_mutations())
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            device: bacnet_objects::DeviceIdentity::default(),
            registered_network_port: None,
            audit_reporters: None,
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
            share_port_by_address: false,
            cov_retry_timeout_ms: 3000,
            time_sync_policy: TimeSyncPolicy::default(),
            on_time_sync: None,
            on_reinitialize: None,
            mutation_policy: MutationPolicy::default(),
            policy: None,
            audit_notification_sink: None,
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
    /// SC mTLS channel/peer authentication is not service authorization.
    pub fn mutation_policy(mut self, policy: MutationPolicy) -> Self {
        self.config.mutation_policy = policy;
        self
    }

    /// Install the server's authorization policy; see [`ServerPolicy`], or
    /// [`FnPolicy`](crate::FnPolicy) to build one from closures.
    /// It replaces any policy set before; put every decision in one policy
    /// (one `FnPolicy` chain) rather than calling this twice.
    pub fn policy(mut self, policy: impl ServerPolicy) -> Self {
        self.config.policy = Some(Arc::new(policy));
        self
    }
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

    /// Set the B/IP mode: a normal node (the default), a foreign device or
    /// a BBMD (#938). A foreign device's `build()` doesn't wait for its BBMD
    /// to accept the registration; poll
    /// [`BACnetServer::foreign_registration`] for how it went. A registered
    /// Network Port reports the mode in `BACnet_IP_Mode`.
    ///
    /// ```
    /// use bacnet_server::server::BACnetServer;
    /// use bacnet_transport::bbmd::ForeignDevicePolicy;
    /// use bacnet_transport::bip::{BbmdOptions, BipMode};
    /// let builder = BACnetServer::bip_builder().mode(BipMode::Bbmd(BbmdOptions {
    ///     foreign_device_policy: Some(ForeignDevicePolicy::default()),
    ///     ..BbmdOptions::default()
    /// }));
    /// ```
    pub fn mode(mut self, mode: bacnet_transport::bip::BipMode) -> Self {
        self.mode = mode;
        self
    }

    /// Make the server a B/IP-M node in `multicast`'s group (Annex J.8), or,
    /// with `None` (the default), not: its broadcasts go to the group, and
    /// it takes broadcasts only from the group. Needs an explicit
    /// [`interface`](Self::interface), and a nonzero [`port`](Self::port)
    /// unless the group has its own; `build()` fails on what
    /// [`BipTransport::set_multicast`](bacnet_transport::bip::BipTransport::set_multicast)
    /// refuses. With [`BipMode::Bbmd`]
    /// the server is the BBMD that serves the group (Annex J.8.3), and its
    /// own BDT row must be two-hop. A registered Network Port serves the
    /// group as `BACnet_IP_Multicast_Address`, writable when the server
    /// answers ReinitializeDevice and isn't a BBMD (#941).
    pub fn multicast(mut self, multicast: Option<BipMulticast>) -> Self {
        self.multicast = multicast;
        self
    }

    /// Bind the interface address itself, so devices on other addresses of
    /// this host can share the port, each getting only its own unicast
    /// (#1538). Off by default, which binds `0.0.0.0`. Needs an explicit
    /// [`interface`](Self::interface) and a nonzero port, or `build` fails.
    /// See [`BipTransport::set_share_port_by_address`] for what changes.
    pub fn share_port_by_address(mut self, enabled: bool) -> Self {
        self.config.share_port_by_address = enabled;
        self
    }

    /// Select local mutation authorization (default: [`MutationPolicy::Permissive`]).
    /// SC mTLS channel/peer authentication is not service authorization.
    ///
    /// ```
    /// use bacnet_server::{mutation::MutationPolicy, server::BACnetServer};
    /// let builder = BACnetServer::bip_builder().mutation_policy(MutationPolicy::DenyAll);
    /// ```
    pub fn mutation_policy(mut self, policy: MutationPolicy) -> Self {
        self.config.mutation_policy = policy;
        self
    }

    /// Install the server's authorization policy; see [`ServerPolicy`], or
    /// [`FnPolicy`](crate::FnPolicy) to build one from closures.
    /// It replaces any policy set before; put every decision in one policy
    /// (one `FnPolicy` chain) rather than calling this twice.
    ///
    /// ```
    /// use bacnet_server::server::BACnetServer;
    /// use bacnet_server::FnPolicy;
    /// // Refuse every remote write; LifeSafety and Audit keep denying.
    /// let builder = BACnetServer::bip_builder().policy(FnPolicy::new().mutation(|_| false));
    /// ```
    pub fn policy(mut self, policy: impl ServerPolicy) -> Self {
        self.config.policy = Some(Arc::new(policy));
        self
    }
}

#[cfg(feature = "sc-tls")]
impl ScServerBuilder {
    /// Select local mutation authorization (default: [`MutationPolicy::Permissive`]).
    /// SC mTLS channel/peer authentication is not service authorization.
    pub fn mutation_policy(mut self, policy: MutationPolicy) -> Self {
        self.config.mutation_policy = policy;
        self
    }

    /// Install the server's authorization policy; see [`ServerPolicy`], or
    /// [`FnPolicy`](crate::FnPolicy) to build one from closures.
    /// It replaces any policy set before; put every decision in one policy
    /// (one `FnPolicy` chain) rather than calling this twice.
    pub fn policy(mut self, policy: impl ServerPolicy) -> Self {
        self.config.policy = Some(Arc::new(policy));
        self
    }
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Start a server builder for a caller-supplied transport type, with default configuration and
    /// stock Device generation unless a database is supplied.
    pub fn generic_builder() -> ServerBuilder<T> {
        ServerBuilder {
            config: ServerConfig::default(),
            db: None,
            transport: None,
            configured_device_bindings: Vec::new(),
        }
    }
}
