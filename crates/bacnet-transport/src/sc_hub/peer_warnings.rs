//! Hub warnings a peer could otherwise set off once per connection or frame
//! (#1950), each through a hub-wide throttle that warns once per
//! [`WARNING_INTERVAL`](crate::warn_throttle::WARNING_INTERVAL) with the
//! count since the last warning and logs the rest at debug.
//!
//! - A refused Connect-Request ends its connection, so a peer that
//!   reconnects over and over could set one off per connection. These go
//!   through
//!   [`AdmissionRuntime::connection_warning`](super::admission::AdmissionRuntime::connection_warning).
//! - A failed broadcast relay to a client that stops reading would set one
//!   off per frame relayed to it. These go through
//!   [`ClientRegistry::relay_warning`](super::client::ClientRegistry::relay_warning),
//!   as the unicast, opaque and Result relay failures do.

use std::net::SocketAddr;

use super::client::ClientRegistry;
use crate::sc_frame::Vmac;
use crate::warn_throttle::WarnThrottle;

/// Log a failed or timed-out broadcast relay to `target`.
pub(super) fn relay_failed(clients: &ClientRegistry, target: Vmac) -> Option<u64> {
    crate::warn_throttled!(
        clients.relay_warning,
        "Hub: broadcast relay failed to {target:02x?}"
    )
}

/// Why the hub refused a Connect-Request.
#[derive(Clone, Copy, Debug)]
pub(super) enum ConnectRefusal {
    /// The client already registered under this VMAC sent another.
    AlreadyConnected(Vmac),
    /// It claims a reserved VMAC.
    ReservedVmac(Vmac),
    /// Its VMAC is the hub's own, or another client's.
    VmacCollision(Vmac),
    /// The certificate bindings or the admission policy denied it.
    AdminDenied,
    /// The hub already serves as many clients as it may.
    MaxClients,
}

/// Log `refusal` of a Connect-Request from `peer` through `warning`.
/// Returns the occurrences reported, when it warns.
pub(super) fn connect_refused(
    warning: &WarnThrottle,
    peer: SocketAddr,
    refusal: ConnectRefusal,
) -> Option<u64> {
    match refusal {
        ConnectRefusal::AlreadyConnected(vmac) => crate::warn_throttled!(
            warning,
            "Hub: ConnectRequest from already connected client {peer} (vmac={vmac:02x?}), closing"
        ),
        ConnectRefusal::ReservedVmac(vmac) => crate::warn_throttled!(
            warning,
            "Hub: rejecting reserved VMAC {vmac:02x?} from {peer}"
        ),
        ConnectRefusal::VmacCollision(vmac) => {
            crate::warn_throttled!(warning, "Hub: VMAC collision for {vmac:02x?} from {peer}")
        }
        ConnectRefusal::AdminDenied => crate::warn_throttled!(
            warning,
            "Hub: admin admission denied ConnectRequest from {peer}"
        ),
        ConnectRefusal::MaxClients => crate::warn_throttled!(
            warning,
            "SC Hub: max clients reached, rejecting connection from {peer}"
        ),
    }
}
