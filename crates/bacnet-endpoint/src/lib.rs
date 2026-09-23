//! Proven single-owner BACnet endpoint API (RB-18).
//!
//! Compose client and server roles for one BACnet device under one transport
//! owner. One [`EndpointSession`] owns the transport, ingress, and shared
//! outbound coordinator. Construct sessions with the endpoint builders:
//! [`bip::BipEndpointBuilder`], [`sc::ScEndpointBuilder`],
//! [`mstp::MstpEndpointBuilder`].
//!
//! # Quick start (B/IP, one device, both roles)
//!
//! ```no_run
//! use std::net::Ipv4Addr;
//!
//! use bacnet_endpoint::bip::BipEndpointBuilder;
//! use bacnet_endpoint::identity::DeviceIdentity;
//! use bacnet_endpoint::session::SessionRole;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), bacnet_types::error::Error> {
//! let identity = DeviceIdentity::new(1001, 42)?
//!     .with_bip_port(1, 0, Ipv4Addr::LOCALHOST, 0)?;
//! let db = identity.build_database()?;
//! let mut session = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
//!     .role(SessionRole::Both)
//!     .database(db)
//!     .identity(identity)
//!     .build_session()?;
//! session.start().await?;
//! session.broadcast_i_am().await?;
//! session.stop().await?;
//! # Ok(())
//! # }
//! ```
//!
//! # What is proven
//!
//! - One socket / one serial owner per session (RB-16 B/IP + SC-hub proofs,
//!   RB-17 MS/TP simulator proof). No second hidden socket or serial owner is
//!   created by the builders or the session.
//! - I-Am identical to Device ReadProperty for the composed [`identity`](crate::identity)
//!   on real B/IP loopback UDP and on the constrained-TLS SC hub
//!   (I-Am readback matrix in `rb16_*_proof` tests).
//! - Loopback-only coverage for port/UUID/capability corners beyond that
//!   matrix: [`DeviceIdentity`] Network-Port population, SC UUID sync into
//!   `DEVICE_UUID`, and service-profile alignment are exercised on
//!   `LoopbackTransport` / `LoopbackWebSocket` for determinism, not as
//!   on-wire claims.
//! - Narrow server scope: the endpoint server role executes `ReadProperty`
//!   (+ `Reject`/`Abort` + segmentation-`Abort`). Explicit
//!   [`EndpointSession::with_device_writes`](session::EndpointSession::with_device_writes)
//!   enables authorized writes to the one local Device's Description and its
//!   installed source Audit recipient, with
//!   deterministic and real B/IP loopback tests. Full `bacnet-server`
//!   dispatch parity is out of scope.
//!
//! # Data-link support matrix (explicit, not silent)
//!
//! | Data link | Endpoint builder | Evidence |
//! |-----------|------------------|----------|
//! | B/IP IPv4 unicast + local broadcast | [`bip::BipEndpointBuilder`] | Real loopback UDP proof (RB-16) |
//! | B/IP BBMD / foreign-device mode | [`bip::BipEndpointBuilder::enable_bbmd`] et al. | **Experimental / unproven**: construction-only coverage, no wire BBMD proof; live BVLC queries stay on [`BipTransport`](bacnet_transport::bip::BipTransport) |
//! | BACnet/SC via hub | [`sc::ScEndpointBuilder`] | Real constrained-TLS hub proof (RB-16, `sc-tls`); loopback composition for unit validation |
//! | MS/TP | [`mstp::MstpEndpointBuilder`] | **Simulator evidence only** (`LoopbackSerial` ownership + frame sequencing); no bench or on-wire conformance; timing is RB-26 |
//! | BIPv6 | None | Explicitly unsupported: no endpoint builder; use `bacnet-transport` directly |
//! | Ethernet | None | Explicitly unsupported: no endpoint builder; use `bacnet-transport` directly |
//!
//! Do not expect identical administration across data links: BBMD, SC hub
//! dial, and MS/TP station/timing knobs stay transport-specific by design.
//!
//! # Ownership and lifecycle
//!
//! - [`session::EndpointSession`] is the sole lifecycle owner:
//!   start-once/completed-stop-once; canceled stop can resume joining. `Drop`
//!   aborts owned tasks and retains Audit membership through their quiescence. Role handles
//!   expose no lifecycle methods.
//! - Role handles ([`ClientRoleHandle`], [`ServerRoleHandle`]) hold only a
//!   [`Weak`](std::sync::Weak) session token plus role state. Dropping or
//!   stopping the session ends role work even when a handle was cloned out
//!   beforehand; cloned handles survive the drop as values but fail closed.
//! - `start(&mut self)` / `stop(&mut self)` take `&mut` so only the owner can
//!   drive lifecycle; `&self` borrows (`client`, `server`, counters,
//!   `broadcast_i_am`) stay usable while running. Cancellation is
//!   await-boundary only: aborting a pending `read_property*` future releases
//!   its exact coordinator lease via RAII for ordinary reads. Admitted audited
//!   reads are session-owned through their terminal outcome. `stop()` seals admission, cancels
//!   waiters, and joins dispatch exactly once.
//! - All fallible boundaries return typed [`bacnet_types::error::Error`];
//!   queue-capacity, BBMD-ordering, SC identity/heartbeat, and MS/TP
//!   addressing misconfiguration fail fast at build time.
//!
//! ```compile_fail,E0599
//! // Role handles expose no lifecycle: `stop` exists only on `EndpointSession`.
//! # use bacnet_endpoint::roles::ClientRoleHandle;
//! # fn forbidden(handle: ClientRoleHandle) {
//! handle.stop();
//! # }
//! ```
//!
//! ```compile_fail,E0599
//! // `close_for_owner` is owner-only (`pub(crate)`): external callers cannot
//! // drive role shutdown directly.
//! # use bacnet_endpoint::roles::ServerRoleHandle;
//! # fn forbidden(handle: ServerRoleHandle) {
//! handle.close_for_owner();
//! # }
//! ```
//!
//! # Trust: verified-origin getters (no new API)
//!
//! Provenance stays on [`bacnet_transport::port::TransportProvenance`];
//! the endpoint preserves it structurally. Configured authorizers decide whether
//! that provenance permits an operation. Read it with the existing getters:
//!
//! - `is_unverified()` — legacy origin, no assertion (B/IP, MS/TP, loopback).
//! - `is_direct_peer()` — authenticated immediate SC-TLS peer.
//! - `is_relayed_origin()` — hub-validated relayed origin (post
//!   source-admission; the hub peer is never substituted for the leaf).
//! - `is_verified()` — either verified variant.
//!
//! Only trusted SC validation code constructs verified values; every other
//! transport (including caller-supplied doubles) reports unverified.
//!
//! # Choosing standalone or shared client/server roles
//!
//! Standalone `BACnetClient` and `BACnetServer` remain public APIs with their own
//! service and data-link capabilities; they are not deprecated. Use the endpoint
//! builders when both roles need one transport and lifecycle owner, accounting
//! for the narrower endpoint responder scope above.
//!
//! | Standalone construction | Shared endpoint composition |
//! |---------------------|---------------------|
//! | `BACnetClient::bip_builder()...build().await` | `BipEndpointBuilder::new(iface, port, bcast).role(ClientOnly).build_session()?` then `start()` |
//! | `BACnetServer::bip_builder()...build().await` | `BipEndpointBuilder::new(iface, port, bcast).role(ServerOnly).database(db).identity(id).build_session()?` then `start()` |
//! | Client + server on one device | `BipEndpointBuilder::...role(Both).database(db).identity(id).build_session()?` (or the SC / MS/TP builder) |
//! | `BACnetClient::sc_builder()...build().await` | `ScEndpointBuilder::new(vmac, uuid)...build_hub_session(ws)?` (`sc-tls`; dial first, then compose) |
//! | `BACnetServer::sc_builder()...build().await` | Same `ScEndpointBuilder` with `ServerOnly` + `database` + `identity` |
//! | `generic_builder().transport(mstp)...` | `MstpEndpointBuilder::new(serial, station)...build_session()?` (one serial owner, simulator evidence) |
//!
//! Standalone BBMD helpers (`read_bdt` / `write_bdt` / `read_fdt` / foreign
//! registration) stay on [`BipTransport`](bacnet_transport::bip::BipTransport);
//! the endpoint BBMD setters only stage pre-start state and are experimental.
//! BIPv6/Ethernet have no endpoint builder: keep the standalone path there.
//! See `docs/rust-api.md` for the same table with service-scope notes.
//!
//! # Module map
//!
//! - [`session`] — [`EndpointSession`](session::EndpointSession),
//!   [`SessionRole`](session::SessionRole), [`SessionConfig`](session::SessionConfig),
//!   [`PolicyCountersSnapshot`](session::PolicyCountersSnapshot),
//!   [`SessionExit`](session::SessionExit).
//! - [`roles`] — [`ClientRoleHandle`](roles::ClientRoleHandle) (`read_property*`
//!   trio only) + [`ServerRoleHandle`](roles::ServerRoleHandle) (inbound /
//!   liveness / suspend / notification admit-complete).
//! - [`identity`] — [`DeviceIdentity`](identity::DeviceIdentity),
//!   [`NetworkPortEntry`](identity::NetworkPortEntry),
//!   [`build_database_with_extra`](identity::build_database_with_extra).
//! - [`bip`], [`sc`], [`mstp`] — the three endpoint builders.
//!
//! Hidden by design: `SessionToken`, single-admit/decode helpers,
//! `complete_pre_admitted`, `close_for_owner`, and all `__endpoint_*` hooks.

#![deny(unsafe_code)]

pub mod bip;
pub mod identity;
pub mod mstp;
pub mod roles;
pub mod sc;
pub mod session;
mod source_read;

pub use identity::{build_database_with_extra, DeviceIdentity, NetworkPortEntry};
pub use roles::{ClientRoleHandle, ServerRoleHandle};
pub use session::{
    EndpointSession, PolicyCountersSnapshot, SessionConfig, SessionExit, SessionRole,
};
