//! Concrete B/IP endpoint builder (no generic-trait erasure).
//!
//! It builds one concrete [`BipTransport`] (never `impl TransportPort`)
//! under the endpoint's roles, in the B/IP mode [`BipEndpointBuilder::mode`]
//! sets: a normal node, a foreign device or a BBMD (#938).
//!
//! # Evidence level
//!
//! Plain B/IP (unicast + local broadcast over one real UDP socket) is proven
//! on real loopback UDP (RB-16): one socket per session, bidirectional
//! confirmed traffic, I-Am identical to Device ReadProperty. BBMD and foreign
//! modes have local Network Number wire coverage: ServerOnly BBMD admission
//! and Original-Broadcast replies, ClientOnly foreign DBTN/retry and
//! alternate forwarders, and Both requester/responder progress with cleanup.
//! The modes themselves run on the same transport as the standalone client
//! and server. [`EndpointSession::foreign_registration`] reports a foreign
//! device's registration. Live BVLC queries (`read_bdt` / `write_bdt` /
//! `read_fdt` / …) stay on [`BipTransport`].
//!
//! ```no_run
//! use std::net::Ipv4Addr;
//!
//! use bacnet_endpoint::bip::BipEndpointBuilder;
//! use bacnet_endpoint::session::SessionRole;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), bacnet_types::error::Error> {
//! let mut session = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
//!     .role(SessionRole::Both)
//!     .queue_capacity(16)
//!     .client_timers(2_000, 0)
//!     .build_session()?;
//! session.start().await?;
//! session.stop().await?;
//! # Ok(())
//! # }
//! ```

use std::net::{Ipv4Addr, SocketAddrV4};

use bacnet_objects::database::ObjectDatabase;
use bacnet_transport::bip::{BipMode, BipMulticast, BipTransport};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use crate::session::{EndpointSession, SessionConfig, SessionRole};

/// Concrete B/IP endpoint builder.
///
/// [`mode`](Self::mode) sets the B/IP mode, as on the standalone client and
/// server builders; live BVLC queries (`read_bdt`/`write_bdt`/`read_fdt`/…)
/// stay on [`BipTransport`].
pub struct BipEndpointBuilder {
    interface: Ipv4Addr,
    port: u16,
    broadcast_address: Ipv4Addr,
    share_port_by_address: bool,
    role: SessionRole,
    session: SessionConfig,
    database: Option<ObjectDatabase>,
    registered_network_port: Option<ObjectIdentifier>,
    identity: Option<crate::identity::DeviceIdentity>,
    device_write_authorizer: Option<bacnet_server::mutation::DeviceWriteAuthorizer>,
    reinitialize: Option<bacnet_server::server::ReinitializeHandler>,
    reinit_password: Option<String>,
    source_audit_bindings: Vec<(ObjectIdentifier, SocketAddrV4)>,
    mode: BipMode,
    multicast: Option<BipMulticast>,
}

impl BipEndpointBuilder {
    /// Creates a B/IP endpoint builder with interface/port/broadcast.
    ///
    /// `port = 0` selects an ephemeral port (tests); production uses 47808.
    /// `interface` is the announced MAC IP; the socket binds `INADDR_ANY`
    /// unless [`share_port_by_address`](Self::share_port_by_address) asks for
    /// the interface address.
    pub fn new(interface: Ipv4Addr, port: u16, broadcast_address: Ipv4Addr) -> Self {
        Self {
            interface,
            port,
            broadcast_address,
            share_port_by_address: false,
            role: SessionRole::Both,
            session: SessionConfig::default(),
            database: None,
            registered_network_port: None,
            identity: None,
            device_write_authorizer: None,
            reinitialize: None,
            reinit_password: None,
            source_audit_bindings: Vec::new(),
            mode: BipMode::Normal,
            multicast: None,
        }
    }

    /// Binds the interface address itself, so endpoints on other addresses
    /// of this host can share the port, each getting only its own unicast
    /// (#1538). Off by default. Needs an explicit interface and a nonzero
    /// port, or starting fails. See
    /// [`BipTransport::set_share_port_by_address`](bacnet_transport::bip::BipTransport::set_share_port_by_address)
    /// for what changes.
    pub fn share_port_by_address(mut self, enabled: bool) -> Self {
        self.share_port_by_address = enabled;
        self
    }

    /// Selects the composed roles (default [`Both`](SessionRole::Both)).
    pub fn role(mut self, role: SessionRole) -> Self {
        self.role = role;
        self
    }

    /// Sets the bounded queue capacity for every ingress/egress queue.
    ///
    /// Must be greater than zero; [`build_session`](Self::build_session)
    /// fails otherwise via [`EndpointSession::new`].
    pub fn queue_capacity(mut self, capacity: usize) -> Self {
        self.session.queue_capacity = capacity;
        self
    }

    /// Sets the server role's read work limit (default 256): the result rows
    /// one ReadProperty may expand, a Group's member rows included. See
    /// [`SessionConfig::read_work_limit`]; zero fails
    /// [`build_session`](Self::build_session) via [`EndpointSession::new`].
    pub fn read_work_limit(mut self, limit: usize) -> Self {
        self.session.read_work_limit = limit;
        self
    }

    /// Sets client APDU timeout/retries (session-owned timers).
    pub fn client_timers(mut self, timeout_ms: u64, retries: u8) -> Self {
        self.session.apdu_timeout_ms = timeout_ms;
        self.session.apdu_retries = retries;
        self
    }

    /// Least time between the client role's confirmed requests to one
    /// destination (default 0: no pacing). See
    /// [`SessionConfig::min_request_interval_ms`]; more than an hour fails
    /// [`build_session`](Self::build_session).
    pub fn min_request_interval_ms(mut self, ms: u64) -> Self {
        self.session.min_request_interval_ms = ms;
        self
    }

    /// Attaches the object database for the server responder.
    ///
    /// Build it from the same identity passed to
    /// [`device`](Self::device) so Device readback agrees with I-Am.
    pub fn database(mut self, db: ObjectDatabase) -> Self {
        self.database = Some(db);
        self
    }

    /// Explicitly associate the selected concrete built-in port with this B/IP
    /// transport, in whichever single mode it runs (normal, foreign device or
    /// BBMD). Declaring an identity entry alone does not register it.
    pub fn registered_network_port(mut self, oid: ObjectIdentifier) -> Self {
        self.registered_network_port = Some(oid);
        self
    }

    /// Composes the single Device identity (overrides SessionConfig 480).
    ///
    /// At start, the requested ceiling resolves against the transport's
    /// stable local receive capacity. An omitted database generates a Device
    /// from that resolved identity. A supplied database is preserved and its
    /// selected Device must match. No additional socket is created.
    pub fn device(mut self, identity: crate::identity::DeviceIdentity) -> Self {
        self.identity = Some(identity);
        self
    }

    /// Enables authorized writes to the local Device's Description and installed Audit recipient.
    ///
    /// The mandatory callback and atomic startup requirements are those of
    /// [`EndpointSession::with_device_writes`]. Other objects/properties and
    /// WritePropertyMultiple remain unsupported. Requires `build_session()`;
    /// a bare transport cannot retain this session-owned authorization.
    pub fn device_writes(
        mut self,
        authorizer: bacnet_server::mutation::DeviceWriteAuthorizer,
    ) -> Self {
        self.device_write_authorizer = Some(authorizer);
        self
    }

    /// Enables ReinitializeDevice. The handler must reply before restarting
    /// and stay quick, or it stalls the session; see
    /// [`EndpointSession::with_reinitialize`]. Requires `build_session()`.
    pub fn reinitialize<F>(mut self, handler: F) -> Self
    where
        F: Fn(
                &bacnet_server::server::ReinitializeContext,
                &mut ObjectDatabase,
            ) -> Result<(), Error>
            + Send
            + Sync
            + 'static,
    {
        self.reinitialize = Some(std::sync::Arc::new(handler));
        self
    }

    /// Sets the password a ReinitializeDevice request must carry. Requires
    /// [`reinitialize`](Self::reinitialize) and `build_session()`; see
    /// [`EndpointSession::with_reinit_password`].
    pub fn reinit_password(mut self, password: impl Into<String>) -> Self {
        self.reinit_password = Some(password.into());
        self
    }

    /// Register one trusted direct IPv4 B/IP route for source Audit delivery.
    /// This is route data only; provision the recipient on the actual Device.
    /// Multiple entries may resolve old/new Device choices. Duplicate keys,
    /// non-Device/wildcard identifiers and invalid unicast endpoints fail
    /// `build_session()`. A configured broadcast endpoint is ineligible; the
    /// actual bound port is rechecked after startup. Address choices need no
    /// binding. Without source selection these facts are inert. `build_transport`
    /// rejects them because a bare transport cannot retain this route authority.
    pub fn source_audit_device_binding(
        mut self,
        device: ObjectIdentifier,
        address: SocketAddrV4,
    ) -> Self {
        self.source_audit_bindings.push((device, address));
        self
    }

    /// Sets the B/IP mode: a normal node (the default), a foreign device or
    /// a BBMD (#938). See [`BipMode`]. With a `0.0.0.0` interface, a BBMD's
    /// own B/IP address comes from its BDT, as described at
    /// [`BipTransport::enable_bbmd`]. A registered Network Port reports the
    /// mode in `BACnet_IP_Mode`.
    pub fn mode(mut self, mode: BipMode) -> Self {
        self.mode = mode;
        self
    }

    /// Makes the endpoint a B/IP-M node in `multicast`'s group (Annex J.8),
    /// or, with `None` (the default), not. Needs an explicit interface; see
    /// [`BipTransport::set_multicast`], whose refusals fail the build. In
    /// BBMD mode the endpoint serves the group (Annex J.8.3). A registered
    /// Network Port serves the group as `BACnet_IP_Multicast_Address`
    /// (#941).
    pub fn multicast(mut self, multicast: Option<BipMulticast>) -> Self {
        self.multicast = multicast;
        self
    }

    /// Builds the concrete B/IP transport with all pre-start controls applied.
    ///
    /// Returns a typed [`Error::Encoding`] when any session-owned
    /// configuration is set, since a bare transport would discard it: a
    /// Network Port registration, Device writes, a ReinitializeDevice handler
    /// or password (a password alone included), or source Audit route data.
    /// Those require [`build_session`](Self::build_session).
    pub fn build_transport(self) -> Result<BipTransport, Error> {
        if self.registered_network_port.is_some() {
            return Err(Error::Encoding(
                "Network Port registration requires build_session()".into(),
            ));
        }
        if self.device_write_authorizer.is_some() {
            return Err(Error::Encoding(
                "Device writes require build_session()".into(),
            ));
        }
        if self.reinitialize.is_some() || self.reinit_password.is_some() {
            return Err(Error::Encoding(
                "ReinitializeDevice and its password require build_session()".into(),
            ));
        }
        if !self.source_audit_bindings.is_empty() {
            return Err(Error::Encoding(
                "source Audit route data requires build_session()".into(),
            ));
        }
        let mut transport = BipTransport::new(self.interface, self.port, self.broadcast_address);
        transport.set_share_port_by_address(self.share_port_by_address);
        transport.set_mode(self.mode);
        transport.set_multicast(self.multicast)?;
        Ok(transport)
    }

    /// Builds an unstarted session (caller drives `start()`/`stop()` once).
    ///
    /// One-socket proof: this builds exactly one [`BipTransport`] (one UDP
    /// socket after `start()`); no second hidden socket is created here or
    /// in [`EndpointSession`]. Bind-count proofs use a counting test double
    /// plus real-socket corroboration (single nonzero local MAC/port).
    pub fn build_session(mut self) -> Result<EndpointSession<BipTransport>, Error> {
        let device_write_authorizer = self.device_write_authorizer.take();
        let reinitialize = self.reinitialize.take();
        let reinit_password = self.reinit_password.take();
        let bindings = std::mem::take(&mut self.source_audit_bindings);
        crate::source_audit::recipient::SourceRoutes::new(
            &bindings,
            SocketAddrV4::new(self.broadcast_address, self.port),
        )?;
        let role = self.role;
        let session = self.session.clone();
        let database = self.database.take();
        let identity = self.identity.take();
        let registered_network_port = self.registered_network_port.take();
        let transport = self.build_transport()?;
        let bip_status = transport.bvlc_client_status();
        let mut endpoint = EndpointSession::new(transport, role, session)?;
        endpoint.bip_status = Some(bip_status);
        if let Some(db) = database {
            endpoint = endpoint.with_database(db);
        }
        if let Some(id) = identity {
            endpoint = endpoint.device(id);
        }
        if let Some(authorizer) = device_write_authorizer {
            endpoint = endpoint.with_device_writes(authorizer);
        }
        if let Some(oid) = registered_network_port {
            endpoint = endpoint.with_registered_network_port(oid);
        }
        endpoint.reinitialize = reinitialize;
        endpoint.reinit_password = reinit_password;
        endpoint.source_audit_bindings = bindings;
        Ok(endpoint)
    }
}

#[cfg(test)]
mod share_port_tests {
    use super::*;

    /// The builder hands `share_port_by_address` to its transport, which
    /// refuses it without an explicit interface (#1538).
    #[tokio::test]
    async fn the_builder_shares_the_port_by_address_only_with_an_address() {
        let mut endpoint =
            BipEndpointBuilder::new(Ipv4Addr::UNSPECIFIED, 0xBAC0, Ipv4Addr::BROADCAST)
                .role(SessionRole::ClientOnly)
                .share_port_by_address(true)
                .build_session()
                .unwrap();
        let Err(Error::Transport(refused)) = endpoint.start().await else {
            panic!("start must refuse");
        };
        assert_eq!(refused.kind(), std::io::ErrorKind::InvalidInput);
    }
}

#[cfg(test)]
mod mode_tests {
    use super::*;
    use crate::session::SessionConfig;
    use bacnet_transport::bip::{ForeignDeviceConfig, ForeignRegistrationOutcome};
    use bacnet_transport::bvll::BvlcResultCode;
    use std::time::Duration;

    /// The session reports its transport's registration (#938) while it
    /// runs, a BBMD's refusal included, and the totals after stop.
    #[tokio::test]
    async fn the_session_reports_a_refused_foreign_registration() {
        let bbmd = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let mut endpoint = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
            .role(SessionRole::ClientOnly)
            .mode(BipMode::Foreign(ForeignDeviceConfig {
                bbmd_ip: Ipv4Addr::LOCALHOST,
                bbmd_port: bbmd.local_addr().unwrap().port(),
                renewal_interval: None,
                ttl: 60,
            }))
            .build_session()
            .unwrap();
        assert_eq!(endpoint.foreign_registration().unwrap().attempts, 0);
        endpoint.start().await.unwrap();
        let mut packet = [0; 64];
        let (len, from) = bbmd.recv_from(&mut packet).await.unwrap();
        assert_eq!(&packet[..len], &[0x81, 5, 0, 6, 0, 60]);
        bbmd.send_to(&[0x81, 0, 0, 6, 0, 0x30], from).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while endpoint.foreign_registration().unwrap().refused == 0 {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("the refusal reached the session's status");
        let registration = endpoint.foreign_registration().unwrap();
        assert!(!registration.registered);
        assert_eq!(
            registration.last_result,
            Some(BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK)
        );
        assert_eq!(
            registration.last_outcome,
            Some(ForeignRegistrationOutcome::Refused)
        );
        endpoint.stop().await.unwrap();
        let registration = endpoint.foreign_registration().unwrap();
        assert_eq!((registration.refused, registration.last_outcome), (1, None));
    }

    /// Only a session the B/IP builder built has a registration to report.
    #[test]
    fn a_session_built_without_the_builder_reports_none() {
        let (transport, _peer) =
            bacnet_transport::loopback::LoopbackTransport::pair(vec![1], vec![2]);
        let session =
            EndpointSession::new(transport, SessionRole::Both, SessionConfig::default()).unwrap();
        assert!(session.foreign_registration().is_none());
    }
}
