//! Concrete BACnet/SC controls at endpoint level (no generic-trait erasure).
//!
//! Exposes [`ScTransport`] builders + hub handles first-class on the endpoint
//! composition. Loopback composition stays for unit validation; the real-hub
//! dial path ([`ScEndpointBuilder::build_hub_session`], `sc-tls` only) dials a
//! local constrained-TLS [`ScHub`](bacnet_transport::sc_hub::ScHub) via
//! [`TlsWebSocket`](bacnet_transport::sc_tls::TlsWebSocket) for RB-16 proofs.
//! MS/TP/Ethernet/IPv6 surfaces are out of scope.

use bacnet_objects::database::ObjectDatabase;
use bacnet_transport::sc::{ScReconnectConfig, ScTransport};
use bacnet_transport::sc_frame::Vmac;
use bacnet_types::error::Error;

use crate::session::{EndpointSession, SessionConfig, SessionRole};

/// Concrete SC endpoint builder (Loopback WebSocket for unit composition).
///
/// Real hub dialing previously lived only in `bacnet-client` /
/// `bacnet-server` SC builders; RB-16 adds the hub-dial path here
/// ([`build_hub_session`](Self::build_hub_session), `sc-tls` only) so the
/// endpoint composition owns the same concrete validation (VMAC reservation,
/// UUID presence, heartbeat timing, reconnect) without hiding behind
/// `impl TransportPort`.
#[doc(hidden)]
pub struct ScEndpointBuilder {
    role: SessionRole,
    session: SessionConfig,
    database: Option<ObjectDatabase>,
    identity: Option<crate::identity::DeviceIdentity>,
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
            identity: None,
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

    /// Composes the single Device identity (overrides SessionConfig 480).
    ///
    /// The identity VMAC + UUID must equal this builder's `vmac` +
    /// `device_uuid` (checked at hub-dial build time); the database should
    /// already be built from the same identity so I-Am vs ReadProperty vs
    /// role limits agree. The builder neither generates nor stores the UUID
    /// beyond the caller-supplied bytes (mirror `sc_builder` docs).
    #[doc(hidden)]
    pub fn identity(mut self, identity: crate::identity::DeviceIdentity) -> Self {
        self.identity = Some(identity);
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
    /// unit proofs. RB-15 tests prefer [`LoopbackTransport`](bacnet_transport::loopback::LoopbackTransport)
    /// for full session I/O and use this builder for config validation only.
    #[doc(hidden)]
    pub fn build_loopback_session(
        mut self,
        ws: bacnet_transport::sc::LoopbackWebSocket,
    ) -> Result<EndpointSession<ScTransport<bacnet_transport::sc::LoopbackWebSocket>>, Error> {
        self.validate()?;
        let vmac = self.vmac;
        let uuid = self.device_uuid;
        let hb_interval = self.heartbeat_interval_ms;
        let hb_timeout = self.heartbeat_timeout_ms;
        let reconnect = self.reconnect.take();
        let role = self.role;
        let session = self.session.clone();
        let transport = ScTransport::new(ws, vmac)
            .with_device_uuid(uuid)
            .with_heartbeat_interval_ms(hb_interval)
            .with_heartbeat_timeout_ms(hb_timeout);
        let transport = match reconnect {
            Some(config) => transport.with_reconnect(config),
            None => transport,
        };
        let mut endpoint = EndpointSession::new(transport, role, session)?;
        if let Some(db) = self.database.take() {
            endpoint = endpoint.with_database(db);
        }
        if let Some(id) = self.identity.take() {
            endpoint = endpoint.with_identity(id);
        }
        Ok(endpoint)
    }

    /// Builds an unstarted real-hub SC session over constrained TLS (RB-16).
    ///
    /// NEW in RB-16: `ScEndpointBuilder` previously offered loopback only.
    /// The caller dials first (`TlsWebSocket::connect(url, node_tls).await`)
    /// against a local constrained-TLS [`ScHub`](bacnet_transport::sc_hub::ScHub)
    /// (rcgen-CA in proofs); this constructor validates VMAC/UUID/heartbeat/
    /// reconnect identically to the loopback path, then composes the session
    /// above the connected socket. Identity VMAC+UUID must match the builder
    /// when an identity is composed; the device UUID stays durable-caller-owned
    /// (neither generated nor stored here). One SC node connection per session;
    /// kill/reconnect preserves VMAC+UUID via the caller-owned reconnect
    /// config (vs transient connections that renegotiate).
    #[cfg(feature = "sc-tls")]
    #[doc(hidden)]
    pub fn build_hub_session(
        mut self,
        ws: bacnet_transport::sc_tls::TlsWebSocket,
    ) -> Result<EndpointSession<ScTransport<bacnet_transport::sc_tls::TlsWebSocket>>, Error> {
        self.validate()?;
        if let Some(identity) = self.identity.as_ref() {
            // Identity VMAC is the SC port MAC when an SC port is present;
            // otherwise the identity carries no VMAC and the builder VMAC wins.
            // UUID must always agree when both are present (durable-caller-owned).
            if identity.device_uuid() != [0; 16] && identity.device_uuid() != self.device_uuid {
                return Err(Error::Encoding(
                    "SC endpoint: identity device UUID must equal builder device_uuid".into(),
                ));
            }
            let sc_macs: Vec<_> = identity
                .network_ports()
                .iter()
                .filter(|p| {
                    p.network_type == bacnet_types::enums::NetworkType::VIRTUAL.to_raw()
                        && p.mac.as_slice().len() == 6
                })
                .map(|p| p.mac.as_slice().to_vec())
                .collect();
            if !sc_macs.is_empty() && !sc_macs.iter().any(|m| m.as_slice() == self.vmac) {
                return Err(Error::Encoding(
                    "SC endpoint: identity SC port VMAC must equal builder vmac".into(),
                ));
            }
        }
        let vmac = self.vmac;
        let uuid = self.device_uuid;
        let hb_interval = self.heartbeat_interval_ms;
        let hb_timeout = self.heartbeat_timeout_ms;
        let reconnect = self.reconnect.take();
        let role = self.role;
        let session = self.session.clone();
        let transport = ScTransport::new(ws, vmac)
            .with_device_uuid(uuid)
            .with_heartbeat_interval_ms(hb_interval)
            .with_heartbeat_timeout_ms(hb_timeout);
        let transport = match reconnect {
            Some(config) => transport.with_reconnect(config),
            None => transport,
        };
        let mut endpoint = EndpointSession::new(transport, role, session)?;
        if let Some(db) = self.database.take() {
            endpoint = endpoint.with_database(db);
        }
        if let Some(id) = self.identity.take() {
            endpoint = endpoint.with_identity(id);
        }
        Ok(endpoint)
    }
}
