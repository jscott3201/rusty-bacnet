//! The B/IP server builder, which constructs its `BipTransport` from an
//! interface, a port and a broadcast address.

use super::*;

impl BACnetServer<BipTransport> {
    /// Create a BIP-specific builder with interface/port/broadcast fields.
    pub fn bip_builder() -> BipServerBuilder {
        BipServerBuilder {
            config: ServerConfig::default(),
            db: None,
            configured_device_bindings: Vec::new(),
            mode: BipMode::Normal,
            multicast: None,
            clock_zone: ClockConfig::default(),
        }
    }
}

/// BIP-specific builder that constructs `BipTransport` from interface/port/broadcast fields.
pub struct BipServerBuilder {
    pub(super) config: ServerConfig,
    pub(super) db: Option<ObjectDatabase>,
    pub(super) configured_device_bindings: Vec<DeviceBinding>,
    pub(super) mode: BipMode,
    pub(super) multicast: Option<BipMulticast>,
    pub(super) clock_zone: ClockConfig,
}

impl BipServerBuilder {
    /// Set the one Device identity. With no supplied database, startup creates
    /// a stock Device after resolving the owned transport's receive capacity.
    pub fn device(mut self, identity: bacnet_objects::DeviceIdentity) -> Self {
        self.config.device = identity;
        self
    }

    /// Set the object database (transfers ownership).
    pub fn database(mut self, db: ObjectDatabase) -> Self {
        self.db = Some(db);
        self
    }

    /// Register one explicit unicast route for a Device recipient.
    pub fn device_binding(mut self, binding: DeviceBinding) -> Result<Self, Error> {
        register_configured_binding(&mut self.configured_device_bindings, binding)?;
        Ok(self)
    }

    /// Select the only local Audit Log that receives authorized notifications.
    pub fn audit_notification_sink(mut self, sink: ObjectIdentifier) -> Self {
        self.config.audit_notification_sink = Some(sink);
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

    /// Build and start the server, constructing a BipTransport from the
    /// config. A refusal, a socket that won't bind among them, hands back
    /// the database ([`StartError`]).
    pub async fn build(mut self) -> Result<BACnetServer<BipTransport>, StartError> {
        let config = &self.config;
        let mut transport =
            BipTransport::new(config.interface, config.port, config.broadcast_address);
        transport.set_share_port_by_address(config.share_port_by_address);
        transport.set_share_wildcard_port(config.share_wildcard_port);
        transport.set_mode(self.mode);
        transport
            .set_multicast(self.multicast)
            .map_err(StartError::taking(&mut self.db))?;
        let db = crate::local_device::prepare_database(
            &mut self.config,
            self.db,
            transport.local_receive_apdu_capacity(),
        )?;
        BACnetServer::start_with_clock_mode_and_bindings(
            self.config,
            db,
            transport,
            Some(self.clock_zone),
            self.configured_device_bindings,
        )
        .await
    }
}
