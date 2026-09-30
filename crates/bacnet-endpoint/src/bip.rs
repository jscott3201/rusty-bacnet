//! Concrete B/IP endpoint builder (no generic-trait erasure).
//!
//! Closes the server-BIP-builder gap: `BipServerBuilder` lacks BDT/FDT
//! controls while [`BipTransport`] + [`bbmd`](bacnet_transport::bbmd) own
//! them. This builder exposes them first-class on the endpoint composition
//! above the sibling roles (concrete `BipTransport`, never `impl
//! TransportPort`).
//!
//! # Evidence level
//!
//! Plain B/IP (unicast + local broadcast over one real UDP socket) is proven
//! on real loopback UDP (RB-16): one socket per session, bidirectional
//! confirmed traffic, I-Am identical to Device ReadProperty. BBMD and foreign
//! modes have bounded local Network Number wire coverage: ServerOnly BBMD
//! admission and Original-Broadcast replies, ClientOnly foreign DBTN/retry and
//! alternate forwarders, and Both requester/responder progress with cleanup.
//! Broader BBMD/foreign behavior remains experimental. Management ACL,
//! persistence and fanout controls retain construction-only endpoint coverage.
//! Live BVLC queries (`read_bdt` / `write_bdt` / `read_fdt` / …) stay on
//! [`BipTransport`].
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
use bacnet_transport::bbmd::BdtEntry;
use bacnet_transport::bbmd::ForeignDevicePolicy;
use bacnet_transport::bip::{BipTransport, FanoutPolicy, ForeignDeviceConfig};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use crate::session::{EndpointSession, SessionConfig, SessionRole};

/// Concrete B/IP endpoint builder.
///
/// Pre-start BBMD/FDT controls mirror `transport/bip` + `bbmd.rs`; live
/// BVLC queries (`read_bdt`/`write_bdt`/`read_fdt`/…) stay on
/// [`BipTransport`] for real-transport proofs (RB-16/17).
///
/// Plain mode is proven on real loopback UDP; BBMD/foreign-device setters are
/// experimental beyond the bounded Number wire cases described above.
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
    device_write_authorizer: Option<bacnet_server::mutation::MutationAuthorizer>,
    reinitialize: Option<bacnet_server::server::ReinitializeHandler>,
    reinit_password: Option<String>,
    source_audit_bindings: Vec<(ObjectIdentifier, SocketAddrV4)>,
    bbmd_bdt: Option<Vec<BdtEntry>>,
    foreign_policy: Option<ForeignDevicePolicy>,
    management_acl: Option<Vec<[u8; 4]>>,
    bdt_persist_path: Option<std::path::PathBuf>,
    fanout_policy: Option<FanoutPolicy>,
    foreign_device: Option<ForeignDeviceConfig>,
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
            bbmd_bdt: None,
            foreign_policy: None,
            management_acl: None,
            bdt_persist_path: None,
            fanout_policy: None,
            foreign_device: None,
        }
    }

    /// Binds the interface address itself, so endpoints on other addresses
    /// of this host can share the port, each getting only its own unicast
    /// (#1538). Off by default. Needs an explicit interface and a nonzero
    /// port, or starting fails. Broadcasts and unicast are then received in
    /// no fixed order; see
    /// [`BipTransport::set_share_port_by_address`](bacnet_transport::bip::BipTransport::set_share_port_by_address).
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

    /// Attaches the object database for the server responder.
    ///
    /// Build it from the same identity passed to
    /// [`identity`](Self::identity) so Device readback agrees with I-Am.
    pub fn database(mut self, db: ObjectDatabase) -> Self {
        self.database = Some(db);
        self
    }

    /// Explicitly associate the selected concrete built-in port with this NORMAL
    /// B/IP transport. Declaring an identity entry alone does not register it.
    pub fn registered_network_port(mut self, oid: ObjectIdentifier) -> Self {
        self.registered_network_port = Some(oid);
        self
    }

    /// Composes the single Device identity (overrides SessionConfig 480).
    ///
    /// Truth direction: the database should already be built from the same
    /// identity (`DeviceIdentity::build_database`); this only wires I-Am +
    /// role limits. No generation, no extra socket.
    pub fn identity(mut self, identity: crate::identity::DeviceIdentity) -> Self {
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
        authorizer: bacnet_server::mutation::MutationAuthorizer,
    ) -> Self {
        self.device_write_authorizer = Some(authorizer);
        self
    }

    /// Enables ReinitializeDevice. See [`EndpointSession::with_reinitialize`].
    /// Requires `build_session()`.
    pub fn reinitialize<F>(mut self, handler: F) -> Self
    where
        F: Fn(bacnet_types::enums::ReinitializedState, &mut ObjectDatabase) -> Result<(), Error>
            + Send
            + Sync
            + 'static,
    {
        self.reinitialize = Some(std::sync::Arc::new(handler));
        self
    }

    /// Sets the password a ReinitializeDevice request must carry.
    /// See [`EndpointSession::with_reinit_password`].
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

    /// Enables BBMD mode with the initial BDT (before start).
    ///
    /// With a `0.0.0.0` interface, the BBMD's own B/IP address comes from the
    /// BDT row at one of the host's addresses and the bound port, as described
    /// at [`BipTransport::enable_bbmd`]; bind an explicit interface address to
    /// avoid that choice.
    ///
    /// Local Number controls have BBMD wire coverage; broader BBMD behavior
    /// remains experimental. BBMD controls require this first:
    /// [`foreign_device_policy`](Self::foreign_device_policy) /
    /// [`bbmd_management_acl`](Self::bbmd_management_acl) without it fail
    /// [`build_transport`](Self::build_transport) with a typed
    /// [`Error::Encoding`].
    pub fn enable_bbmd(mut self, bdt: Vec<BdtEntry>) -> Self {
        self.bbmd_bdt = Some(bdt);
        self
    }

    /// Enables foreign-device registration policy (after `enable_bbmd`).
    ///
    /// **Experimental / unproven**: see [`enable_bbmd`](Self::enable_bbmd).
    pub fn foreign_device_policy(mut self, policy: ForeignDevicePolicy) -> Self {
        self.foreign_policy = Some(policy);
        self
    }

    /// Sets the BBMD Delete-FDT-Entry management ACL (fail-closed when empty).
    ///
    /// **Experimental / unproven**: see [`enable_bbmd`](Self::enable_bbmd).
    pub fn bbmd_management_acl(mut self, acl: Vec<[u8; 4]>) -> Self {
        self.management_acl = Some(acl);
        self
    }

    /// Sets the externally provisioned persisted-BDT path (wire format).
    ///
    /// **Experimental / unproven**: see [`enable_bbmd`](Self::enable_bbmd).
    pub fn bdt_persist_path(mut self, path: std::path::PathBuf) -> Self {
        self.bdt_persist_path = Some(path);
        self
    }

    /// Sets the broadcast fanout policy.
    ///
    /// **Experimental / unproven**: staged pre-start only, no wire proof.
    pub fn fanout_policy(mut self, policy: FanoutPolicy) -> Self {
        self.fanout_policy = Some(policy);
        self
    }

    /// Registers this endpoint as a foreign device (before start).
    ///
    /// Local Number controls have foreign DBTN/retry wire coverage; broader
    /// foreign-device behavior remains experimental.
    pub fn register_as_foreign_device(mut self, config: ForeignDeviceConfig) -> Self {
        self.foreign_device = Some(config);
        self
    }

    /// Builds the concrete B/IP transport with all pre-start controls applied.
    ///
    /// Returns a typed [`Error::Encoding`]
    /// when BBMD-dependent controls are set without
    /// [`enable_bbmd`](Self::enable_bbmd), or a source Audit route data
    /// is set (it requires [`build_session`](Self::build_session), not a bare
    /// transport that would discard the endpoint-owned configuration).
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
        if self.reinitialize.is_some() {
            return Err(Error::Encoding(
                "ReinitializeDevice requires build_session()".into(),
            ));
        }
        if !self.source_audit_bindings.is_empty() {
            return Err(Error::Encoding(
                "source Audit route data requires build_session()".into(),
            ));
        }
        let Self {
            interface,
            port,
            broadcast_address,
            share_port_by_address,
            bbmd_bdt,
            foreign_policy,
            management_acl,
            bdt_persist_path,
            fanout_policy,
            foreign_device,
            ..
        } = self;
        let mut transport = BipTransport::new(interface, port, broadcast_address);
        transport.set_share_port_by_address(share_port_by_address);
        if let Some(bdt) = bbmd_bdt {
            transport.enable_bbmd(bdt);
            if let Some(policy) = foreign_policy {
                transport.enable_foreign_device_registration(policy);
            }
            if let Some(acl) = management_acl {
                transport.set_bbmd_management_acl(acl);
            }
        } else if foreign_policy.is_some() || management_acl.is_some() {
            return Err(Error::Encoding(
                "B/IP endpoint: BBMD controls require enable_bbmd() first".into(),
            ));
        }
        if let Some(path) = bdt_persist_path {
            transport.set_bdt_persist_path(path);
        }
        if let Some(policy) = fanout_policy {
            transport.set_fanout_policy(policy);
        }
        if let Some(config) = foreign_device {
            transport.register_as_foreign_device(config);
        }
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
        let mut endpoint = EndpointSession::new(transport, role, session)?;
        if let Some(db) = database {
            endpoint = endpoint.with_database(db);
        }
        if let Some(id) = identity {
            endpoint = endpoint.with_identity(id);
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
