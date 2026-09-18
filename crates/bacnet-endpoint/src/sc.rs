//! Concrete BACnet/SC controls at endpoint level (no generic-trait erasure).
//!
//! Exposes [`ScTransport`] builders + hub handles first-class on the endpoint
//! composition. Real-transport proofs (handshake, hub relay, TLS) are RB-16/17
//! and explicitly out of scope here: this module validates concrete SC
//! identity/timing/reconnect configuration without starting real hub I/O.
//! MS/TP/Ethernet/IPv6 surfaces are out of scope.

use bacnet_objects::database::ObjectDatabase;
use bacnet_transport::sc::{ScReconnectConfig, ScTransport};
use bacnet_transport::sc_frame::Vmac;
use bacnet_types::error::Error;

use crate::session::{EndpointSession, SessionConfig, SessionRole};

/// Concrete SC endpoint builder (Loopback WebSocket for unit composition).
///
/// Real hub dialing (`TlsWebSocket::connect`) stays in `bacnet-client` /
/// `bacnet-server` SC builders; this endpoint builder owns the same concrete
/// validation (VMAC reservation, UUID presence, heartbeat timing, reconnect)
/// so SC controls are not hidden behind `impl TransportPort`.
#[doc(hidden)]
pub struct ScEndpointBuilder {
    role: SessionRole,
    session: SessionConfig,
    database: Option<ObjectDatabase>,
    vmac: Vmac,
    device_uuid: [u8; 16],
    heartbeat_interval_ms: u64,
    heartbeat_timeout_ms: u64,
    reconnect: Option<ScReconnectConfig>,
}

impl ScEndpointBuilder {
    /// Creates an SC endpoint builder with identity + timing.
    #[doc(hidden)]
    pub fn new(vmac: Vmac, device_uuid: [u8; 16]) -> Self {
        Self {
            role: SessionRole::Both,
            session: SessionConfig::default(),
            database: None,
            vmac,
            device_uuid,
            heartbeat_interval_ms: 30_000,
            heartbeat_timeout_ms: 60_000,
            reconnect: None,
        }
    }

    /// Selects the composed roles.
    #[doc(hidden)]
    pub fn role(mut self, role: SessionRole) -> Self {
        self.role = role;
        self
    }

    /// Sets the bounded queue capacity.
    #[doc(hidden)]
    pub fn queue_capacity(mut self, capacity: usize) -> Self {
        self.session.queue_capacity = capacity;
        self
    }

    /// Attaches the object database for the server responder.
    #[doc(hidden)]
    pub fn database(mut self, db: ObjectDatabase) -> Self {
        self.database = Some(db);
        self
    }

    /// Sets heartbeat interval/timeout (ms).
    #[doc(hidden)]
    pub fn heartbeat(mut self, interval_ms: u64, timeout_ms: u64) -> Self {
        self.heartbeat_interval_ms = interval_ms;
        self.heartbeat_timeout_ms = timeout_ms;
        self
    }

    /// Enables reconnect with the given concrete config.
    #[doc(hidden)]
    pub fn reconnect(mut self, config: ScReconnectConfig) -> Self {
        self.reconnect = Some(config);
        self
    }

    fn validate(&self) -> Result<(), Error> {
        if self.vmac == [0; 6] || self.vmac == bacnet_transport::sc_frame::BROADCAST_VMAC {
            return Err(Error::Encoding(
                "SC endpoint builder: vmac must not be zero or broadcast".into(),
            ));
        }
        if self.device_uuid == [0; 16] {
            return Err(Error::Encoding(
                "SC endpoint builder: device_uuid is required and must not be all zero".into(),
            ));
        }
        if let Some(config) = &self.reconnect {
            config.validate()?;
        }
        // Heartbeat timing mirrors `ScTransport` startup validation range
        // (Annex AB.6.3, 3..300s interval, timeout > interval): validated
        // here without hub I/O so misconfiguration fails fast at composition
        // time. Duplicated (not imported) to keep the transport seam narrow.
        if !(3_000..=300_000).contains(&self.heartbeat_interval_ms) {
            return Err(Error::Encoding(format!(
                "SC endpoint builder: heartbeat interval must be 3000..=300000 ms, got {} ms",
                self.heartbeat_interval_ms
            )));
        }
        if self.heartbeat_timeout_ms <= self.heartbeat_interval_ms {
            return Err(Error::Encoding(format!(
                "SC endpoint builder: heartbeat timeout must exceed interval, got interval={} ms timeout={} ms",
                self.heartbeat_interval_ms, self.heartbeat_timeout_ms
            )));
        }
        Ok(())
    }

    /// Validates concrete SC configuration without hub I/O.
    #[doc(hidden)]
    pub fn validate_only(&self) -> Result<(), Error> {
        self.validate()
    }

    /// Builds an unstarted Loopback-SC session for unit composition.
    ///
    /// Uses [`LoopbackWebSocket`](bacnet_transport::sc::LoopbackWebSocket):
    /// no TLS, no hub dial; the caller drives the hub side for handshake in
    /// RB-16/17 proofs. RB-15 tests prefer [`LoopbackTransport`](bacnet_transport::loopback::LoopbackTransport)
    /// for full session I/O and use this builder for config validation only.
    #[doc(hidden)]
    pub fn build_loopback_session(
        self,
        ws: bacnet_transport::sc::LoopbackWebSocket,
    ) -> Result<EndpointSession<ScTransport<bacnet_transport::sc::LoopbackWebSocket>>, Error> {
        self.validate()?;
        let transport = ScTransport::new(ws, self.vmac)
            .with_device_uuid(self.device_uuid)
            .with_heartbeat_interval_ms(self.heartbeat_interval_ms)
            .with_heartbeat_timeout_ms(self.heartbeat_timeout_ms);
        let transport = match self.reconnect {
            Some(config) => transport.with_reconnect(config),
            None => transport,
        };
        let mut endpoint = EndpointSession::new(transport, self.role, self.session)?;
        if let Some(db) = self.database {
            endpoint = endpoint.with_database(db);
        }
        Ok(endpoint)
    }
}
