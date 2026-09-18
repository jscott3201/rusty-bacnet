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
//! confirmed traffic, I-Am identical to Device ReadProperty. **BBMD /
//! foreign-device mode is experimental and unproven**: `enable_bbmd`,
//! `foreign_device_policy`, `bbmd_management_acl`, `bdt_persist_path`,
//! `fanout_policy`, and `register_as_foreign_device` have construction-only
//! coverage (transport builds pre-start); there is no wire BBMD proof in this
//! crate. Live BVLC queries (`read_bdt` / `write_bdt` / `read_fdt` / …) stay
//! on [`BipTransport`](bacnet_transport::bip::BipTransport). Do not present
//! BBMD mode as proven.
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

use std::net::Ipv4Addr;

use bacnet_objects::database::ObjectDatabase;
use bacnet_transport::bbmd::BdtEntry;
use bacnet_transport::bbmd::ForeignDevicePolicy;
use bacnet_transport::bip::{BipTransport, FanoutPolicy, ForeignDeviceConfig};
use bacnet_types::error::Error;

use crate::session::{EndpointSession, SessionConfig, SessionRole};

/// Concrete B/IP endpoint builder.
///
/// Pre-start BBMD/FDT controls mirror `transport/bip` + `bbmd.rs`; live
/// BVLC queries (`read_bdt`/`write_bdt`/`read_fdt`/…) stay on
/// [`BipTransport`] for real-transport proofs (RB-16/17).
///
/// Plain mode is proven on real loopback UDP; BBMD/foreign-device setters are
/// experimental (construction-only, no wire proof) — see the module docs.
pub struct BipEndpointBuilder {
    interface: Ipv4Addr,
    port: u16,
    broadcast_address: Ipv4Addr,
    role: SessionRole,
    session: SessionConfig,
    database: Option<ObjectDatabase>,
    identity: Option<crate::identity::DeviceIdentity>,
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
    /// `interface` is the announced MAC IP; the socket binds `INADDR_ANY` so
    /// subnet/limited broadcast reaches it.
    pub fn new(interface: Ipv4Addr, port: u16, broadcast_address: Ipv4Addr) -> Self {
        Self {
            interface,
            port,
            broadcast_address,
            role: SessionRole::Both,
            session: SessionConfig::default(),
            database: None,
            identity: None,
            bbmd_bdt: None,
            foreign_policy: None,
            management_acl: None,
            bdt_persist_path: None,
            fanout_policy: None,
            foreign_device: None,
        }
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

    /// Composes the single Device identity (overrides SessionConfig 480).
    ///
    /// Truth direction: the database should already be built from the same
    /// identity (`DeviceIdentity::build_database`); this only wires I-Am +
    /// role limits. No generation, no extra socket.
    pub fn identity(mut self, identity: crate::identity::DeviceIdentity) -> Self {
        self.identity = Some(identity);
        self
    }

    /// Enables BBMD mode with the initial BDT (before start).
    ///
    /// **Experimental / unproven**: staged pre-start only, construction
    /// coverage, no wire BBMD proof. BBMD controls require this first:
    /// [`foreign_device_policy`](Self::foreign_device_policy) /
    /// [`bbmd_management_acl`](Self::bbmd_management_acl) without it fail
    /// [`build_transport`](Self::build_transport) with a typed
    /// [`Error::Encoding`](bacnet_types::error::Error::Encoding).
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
    /// **Experimental / unproven**: staged pre-start only, no wire proof.
    pub fn register_as_foreign_device(mut self, config: ForeignDeviceConfig) -> Self {
        self.foreign_device = Some(config);
        self
    }

    /// Builds the concrete B/IP transport with all pre-start controls applied.
    ///
    /// Returns a typed [`Error::Encoding`](bacnet_types::error::Error::Encoding)
    /// when BBMD-dependent controls are set without
    /// [`enable_bbmd`](Self::enable_bbmd).
    pub fn build_transport(self) -> Result<BipTransport, Error> {
        let Self {
            interface,
            port,
            broadcast_address,
            bbmd_bdt,
            foreign_policy,
            management_acl,
            bdt_persist_path,
            fanout_policy,
            foreign_device,
            ..
        } = self;
        let mut transport = BipTransport::new(interface, port, broadcast_address);
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
        let role = self.role;
        let session = self.session.clone();
        let database = self.database.take();
        let identity = self.identity.take();
        let transport = self.build_transport()?;
        let mut endpoint = EndpointSession::new(transport, role, session)?;
        if let Some(db) = database {
            endpoint = endpoint.with_database(db);
        }
        if let Some(id) = identity {
            endpoint = endpoint.with_identity(id);
        }
        Ok(endpoint)
    }
}
