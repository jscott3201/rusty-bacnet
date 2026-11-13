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

use bacnet_device_core::report::ReportConfig;
use bacnet_objects::database::ObjectDatabase;
use std::sync::Arc;

use bacnet_transport::bbmd_lookup::BbmdResolver;
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
    share_wildcard_port: bool,
    role: SessionRole,
    session: SessionConfig,
    database: Option<ObjectDatabase>,
    registered_network_port: Option<ObjectIdentifier>,
    identity: Option<crate::identity::DeviceIdentity>,
    device_write_authorizer: Option<bacnet_server::mutation::DeviceWriteAuthorizer>,
    reinitialize: Option<bacnet_server::server::ReinitializeHandler>,
    reinit_password: Option<String>,
    cov: Option<ReportConfig>,
    source_audit_bindings: Vec<(ObjectIdentifier, SocketAddrV4)>,
    source_audit_routed: crate::session::RoutedBindings,
    mode: BipMode,
    multicast: Option<BipMulticast>,
    release_previous_bbmd: bool,
    bbmd_resolver: Option<Arc<dyn BbmdResolver>>,
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
            share_wildcard_port: false,
            role: SessionRole::Both,
            session: SessionConfig::default(),
            database: None,
            registered_network_port: None,
            identity: None,
            device_write_authorizer: None,
            reinitialize: None,
            reinit_password: None,
            cov: None,
            source_audit_bindings: Vec::new(),
            source_audit_routed: Vec::new(),
            mode: BipMode::Normal,
            multicast: None,
            release_previous_bbmd: false,
            bbmd_resolver: None,
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

    /// Shares an explicitly requested port with other sockets on this host
    /// that bind it the same way (#2366). Off by default: the transport
    /// claims the port, and starting fails when another socket holds it.
    /// With sharing on, unicast to the port reaches only one of the sockets
    /// on it. See
    /// [`BipTransport::set_share_wildcard_port`](bacnet_transport::bip::BipTransport::set_share_wildcard_port).
    pub fn share_wildcard_port(mut self, enabled: bool) -> Self {
        self.share_wildcard_port = enabled;
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

    /// Enables SubscribeCOV, SubscribeCOVProperty and
    /// SubscribeCOVPropertyMultiple for the objects of the database, with
    /// the COV notifications they call for, under `config`.
    /// Its bounds, startup requirements and refusals are those of
    /// [`EndpointSession::with_cov`]. Requires `build_session()`.
    /// The outbox holds at most
    /// [`COV_OUTBOX_MAX_ENTRIES`](crate::session::COV_OUTBOX_MAX_ENTRIES)
    /// entries and
    /// [`COV_OUTBOX_MAX_OCTETS`](crate::session::COV_OUTBOX_MAX_OCTETS)
    /// octets, and the confirmed timeout is 1 ms to
    /// [`COV_CONFIRMED_TIMEOUT_MAX_MS`](crate::session::COV_CONFIRMED_TIMEOUT_MAX_MS);
    /// past either, startup fails before the transport starts.
    pub fn cov(mut self, config: ReportConfig) -> Self {
        self.cov = Some(config);
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

    /// Bind a Device behind a router, on `network` at `mac` there, for
    /// source Audit delivery to that Device (#2371); see
    /// [`EndpointSession::with_source_audit_routed_device_binding`]. Every
    /// build refuses network 0 or 65535, an empty MAC or one past 18 octets,
    /// and a repeated or non-Device identifier. Route data only: provision
    /// the recipient on the Device itself.
    /// `build_transport` refuses these too.
    pub fn source_audit_routed_device_binding(
        mut self,
        device: ObjectIdentifier,
        network: u16,
        mac: bacnet_types::MacAddr,
    ) -> Self {
        self.source_audit_routed.push((device, network, mac));
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

    /// In [`BipMode::Foreign`], after a move to another BBMD (a Network
    /// Port activation, or the BBMD's host name resolving elsewhere), ask
    /// the BBMD left behind to delete this device's entry rather than let
    /// it lapse (#2307). Off by default; see
    /// [`BipTransport::set_release_previous_bbmd`] for why, and for the
    /// counts [`EndpointSession::foreign_registration`] then shows. Ignored,
    /// with a warning, in the other modes.
    pub fn release_previous_bbmd(mut self, release: bool) -> Self {
        self.release_previous_bbmd = release;
        self
    }

    /// Look up a BBMD named by host name with `resolver` instead of the
    /// system's resolver (#1852, #2307). See
    /// [`BipTransport::set_bbmd_resolver`].
    pub fn bbmd_resolver(mut self, resolver: Arc<dyn BbmdResolver>) -> Self {
        self.bbmd_resolver = Some(resolver);
        self
    }

    /// Builds the concrete B/IP transport with all pre-start controls applied.
    ///
    /// Returns a typed [`Error::Encoding`] when any session-owned
    /// configuration is set, since a bare transport would discard it: a
    /// Network Port registration, Device writes, a ReinitializeDevice handler
    /// or password (a password alone included), COV, or source Audit route
    /// data. Those require [`build_session`](Self::build_session).
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
        if self.cov.is_some() {
            return Err(Error::Encoding("COV requires build_session()".into()));
        }
        if !self.source_audit_bindings.is_empty() || !self.source_audit_routed.is_empty() {
            return Err(Error::Encoding(
                "source Audit route data requires build_session()".into(),
            ));
        }
        let mut transport = BipTransport::new(self.interface, self.port, self.broadcast_address);
        transport.set_share_port_by_address(self.share_port_by_address);
        transport.set_share_wildcard_port(self.share_wildcard_port);
        transport.set_mode(self.mode);
        if self.release_previous_bbmd {
            transport.set_release_previous_bbmd(true);
        }
        if let Some(resolver) = self.bbmd_resolver {
            transport.set_bbmd_resolver(resolver);
        }
        transport.set_multicast(self.multicast)?;
        Ok(transport)
    }

    /// Builds an unstarted session (caller drives `start()`/`stop()` once).
    ///
    /// One-socket proof: this builds exactly one [`BipTransport`] (one UDP
    /// socket after `start()`); no second hidden socket is created here or
    /// in [`EndpointSession`]. Bind-count proofs use a counting test double
    /// plus real-socket corroboration (single nonzero local MAC/port).
    pub fn build_session(self) -> Result<EndpointSession<BipTransport>, Error> {
        let mut bip_status = None;
        let mut endpoint = self.compose(|builder| {
            let transport = builder.build_transport()?;
            bip_status = Some(transport.bvlc_client_status());
            Ok(transport)
        })?;
        endpoint.bip_status = bip_status;
        Ok(endpoint)
    }

    /// Builds the session [`build_session`](Self::build_session) would, over
    /// `transport` instead of a B/IP socket. It still validates the B/IP
    /// socket controls (interface, port, mode, multicast) by building the
    /// B/IP transport, then drops that unstarted transport. Tests run it on
    /// an in-memory link under paused time (#2298).
    #[cfg(test)]
    pub(crate) fn build_session_over<T: bacnet_transport::port::TransportPort + 'static>(
        self,
        transport: T,
    ) -> Result<EndpointSession<T>, Error> {
        self.compose(|builder| {
            builder.build_transport()?;
            Ok(transport)
        })
    }

    /// Validates the session-owned configuration (source Audit routes
    /// included) and takes it out of the builder, then hands the rest of the
    /// builder to `transport` and composes the session over the transport it
    /// returns.
    fn compose<T: bacnet_transport::port::TransportPort + 'static>(
        mut self,
        transport: impl FnOnce(Self) -> Result<T, Error>,
    ) -> Result<EndpointSession<T>, Error> {
        let device_write_authorizer = self.device_write_authorizer.take();
        let reinitialize = self.reinitialize.take();
        let reinit_password = self.reinit_password.take();
        let cov = self.cov.take();
        let bindings = std::mem::take(&mut self.source_audit_bindings);
        let routed = std::mem::take(&mut self.source_audit_routed);
        crate::source_audit::recipient::SourceRoutes::new(
            &bindings,
            SocketAddrV4::new(self.broadcast_address, self.port),
        )?
        .bind_routed(&routed)?;
        let role = self.role;
        let session = self.session.clone();
        let database = self.database.take();
        let identity = self.identity.take();
        let registered_network_port = self.registered_network_port.take();
        let mut endpoint = EndpointSession::new(transport(self)?, role, session)?;
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
        if let Some(config) = cov {
            endpoint = endpoint.with_cov(config);
        }
        endpoint.reinitialize = reinitialize;
        endpoint.reinit_password = reinit_password;
        endpoint.source_audit_bindings = bindings;
        endpoint.source_audit_routed_bindings = routed;
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

    /// The builder hands `share_wildcard_port` to its transport (#2366): on
    /// a port another socket holds, a claiming transport's error names the
    /// opt-in, and a sharing one's says the port can't be shared. Windows
    /// decides a sharing bind beside a socket that set no options by the two
    /// sockets' accounts (Microsoft's SO_REUSEADDR table), so only the
    /// claiming case runs there.
    #[tokio::test]
    async fn the_builder_hands_its_wildcard_sharing_choice_to_the_transport() {
        let holder = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap();
        let port = holder.local_addr().unwrap().port();
        for (share, says) in [(false, "share_wildcard_port"), (true, "can't be shared")] {
            if share && cfg!(windows) {
                continue;
            }
            let mut endpoint =
                BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, port, Ipv4Addr::BROADCAST)
                    .role(SessionRole::ClientOnly)
                    .share_wildcard_port(share)
                    .build_session()
                    .unwrap();
            let Err(Error::Transport(refused)) = endpoint.start().await else {
                panic!("start must fail on a held port (share: {share})");
            };
            assert!(refused.to_string().contains(says), "{refused}");
        }
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
                bbmd: (Ipv4Addr::LOCALHOST, bbmd.local_addr().unwrap().port()).into(),
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

    /// A resolver that answers every name with loopback.
    struct Loopback;

    impl BbmdResolver for Loopback {
        fn resolve(&self, _host: &str) -> bacnet_transport::bbmd_lookup::BbmdLookup {
            Box::pin(async { Ok(vec![Ipv4Addr::LOCALHOST.into()]) })
        }
    }

    /// The builder hands its transport the BBMD resolver and the release of
    /// the BBMD a move leaves (#2307): a host name activated on the port
    /// moves the device to the BBMD the resolver names, and the BBMD it left
    /// is asked to delete its entry.
    #[tokio::test]
    async fn the_builder_hands_the_transport_its_resolver_and_release() {
        use bacnet_transport::port::TransportPort;
        use bacnet_types::bip_port::{BipPortMode, ForeignDeviceChanges};
        use bacnet_types::constructed::{BACnetHostAddress, BACnetHostNPort};
        let socket = || tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0));
        let (old, new) = (socket().await.unwrap(), socket().await.unwrap());
        let mut transport = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
            .mode(BipMode::Foreign(ForeignDeviceConfig {
                bbmd: (Ipv4Addr::LOCALHOST, old.local_addr().unwrap().port()).into(),
                renewal_interval: None,
                ttl: 60,
            }))
            .release_previous_bbmd(true)
            .bbmd_resolver(Arc::new(Loopback))
            .build_transport()
            .unwrap();
        let _rx = transport.start().await.unwrap();
        let mut packet = [0; 64];
        let wait = Duration::from_secs(5);
        let mut next = async |peer: &tokio::net::UdpSocket| {
            let (len, _) = tokio::time::timeout(wait, peer.recv_from(&mut packet))
                .await
                .expect("a datagram")
                .unwrap();
            packet[..len].to_vec()
        };
        assert_eq!(next(&old).await, [0x81, 5, 0, 6, 0, 60]);
        let Some(BipPortMode::Foreign { registration }) =
            transport.bip_port().map(|port| port.mode)
        else {
            panic!("a foreign device's port");
        };
        let mut changes = ForeignDeviceChanges::default();
        changes.bbmd = Some(BACnetHostNPort {
            host: BACnetHostAddress::Name("bbmd.test".into()),
            port: new.local_addr().unwrap().port(),
        });
        registration.activate(&changes).unwrap();
        assert_eq!(next(&new).await, [0x81, 5, 0, 6, 0, 60]);
        let mut release = vec![0x81, 0x08, 0, 10];
        release.extend_from_slice(transport.local_mac());
        assert_eq!(next(&old).await, release);
        transport.stop().await.unwrap();
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
