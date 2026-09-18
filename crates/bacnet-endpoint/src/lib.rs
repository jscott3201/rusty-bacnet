//! Private single-owner BACnet endpoint session (RB-15).
//!
//! # Placement: why a new crate
//!
//! The dependency graph requires this narrow placement:
//!
//! - `bacnet-endpoint-core` cannot depend upward: it owns
//!   [`OutboundTransactionCoordinator`](bacnet_endpoint_core::coordinator::OutboundTransactionCoordinator)
//!   + [`EndpointIngress`](bacnet_endpoint_core::endpoint_ingress::EndpointIngress)/[`EndpointEgress`](bacnet_endpoint_core::endpoint_ingress::EndpointEgress)
//!   only, with no knowledge of client/server roles.
//! - A `server → client` production edge was rejected by the owner: the
//!   server must not depend on the client crate to compose.
//! - A `client → server` edge drags `bacnet-objects` (server depends on
//!   objects; client does not in production) into every client build.
//!
//! The only narrow placement is a small private crate above both sibling
//! roles that depends on `endpoint-core + client + server (+ network /
//! transport / objects as needed)` and owns composition: one
//! [`EndpointSession`] = one [`EndpointIngress`](bacnet_endpoint_core::endpoint_ingress::EndpointIngress)
//! + shared [`OutboundTransactionCoordinator`](bacnet_endpoint_core::coordinator::OutboundTransactionCoordinator)
//! + role registration + policy-outcome ownership + timers + egress +
//! termination.
//!
//! # No public facade
//!
//! Everything here is `#[doc(hidden)]`: no stability promise, no `Device`
//! identity design (deferred), no real-transport proofs (RB-16/17), no MS/TP,
//! no version/ledger/support-claim changes, no B/IP facade inside
//! `bacnet-server`. Standalone `BACnetClient`/`BACnetServer` stay as compat
//! surfaces. Issue #431 stays OPEN: RB-16/17 proofs + API stabilization
//! remain.
//!
//! # Ownership summary
//!
//! - [`EndpointSession`] is the sole lifecycle owner (start-once/stop-once;
//!   `Drop` aborts without orphaning).
//! - [`EndpointEgress`](bacnet_endpoint_core::endpoint_ingress::EndpointEgress)
//!   stays the ONLY send path (no second demultiplexer, no role-side
//!   Invoke-ID allocation; inbound server transactions reuse the wire invoke
//!   ID directly and never allocate from the outbound pool).
//! - Role handles expose no lifecycle methods; they keep an internal
//!   `close()` for the owner and hold session-bound tokens so no detached
//!   role outlives the session.
//! - Provenance/context (`source`/`dest`, raw `link_layer_group` + effective
//!   `is_group`, `data_attributes`, RB-07 `provenance`) is preserved
//!   structurally through every adapter (pass-through, no new decisions).
//! - §6.3 effective-group guard behavior is kept in the egress path.

#![deny(unsafe_code)]

#[doc(hidden)]
pub mod bip;
#[doc(hidden)]
pub mod identity;
#[doc(hidden)]
pub mod roles;
#[doc(hidden)]
pub mod sc;
#[doc(hidden)]
pub mod session;

#[doc(hidden)]
pub use roles::{ClientRoleHandle, ServerRoleHandle};
#[doc(hidden)]
pub use session::{EndpointSession, PolicyCountersSnapshot, SessionConfig, SessionRole};
