//! The loopback broadcast guard (#2691).
//!
//! A transport whose interface is a loopback address still binds its socket
//! to the wildcard address unless it shares its port by address (socket.rs):
//! the interface sets the node's own address, not the socket's. The host
//! picks the interface a datagram to 255.255.255.255 leaves through from its
//! routes, not from the loopback identity, so such a broadcast would go out
//! on a LAN, and its echo would come back from an address that is not the
//! node's own. The node refuses that local broadcast leg, counted in
//! [`SendCounters::loopback_limited_broadcast`](super::SendCounters::loopback_limited_broadcast).
//!
//! Only that local leg is refused, as a deliberate refusal of a
//! misconfigured send, not as an omission the standard offers. A BBMD's
//! Forwarded-NPDUs to its BDT peers and foreign devices are unicast and
//! still go. For a foreign device's Distribute-Broadcast-To-Network the
//! BBMD must also broadcast on its own subnet (Annex J.4.5); as that was
//! not done, it answers with the NAK J.4.5 provides for a forwarding it
//! could not complete.

use std::fmt;
use std::io;
use std::net::{Ipv4Addr, SocketAddrV4};

use bacnet_types::error::Error;
use tracing::warn;

use super::node::NodeHandle;
use super::socket::BindPlan;

/// The loopback subnet's broadcast address, which reaches the other B/IP
/// nodes on loopback without leaving the host.
const LOOPBACK_BROADCAST: Ipv4Addr = Ipv4Addr::new(127, 255, 255, 255);

/// Why a B/IP transport refused a broadcast: its interface is a loopback
/// address, its socket is bound to the wildcard address, and the broadcast
/// address is 255.255.255.255, which the host would route onto another
/// interface (#2691). Configure the loopback subnet's broadcast address,
/// 127.255.255.255, instead: the `broadcast_address` argument of
/// [`BipTransport::new`](super::BipTransport::new). It travels inside
/// [`Error::Transport`] with [`io::ErrorKind::InvalidInput`]; find it with
/// [`from_error`](Self::from_error), so a front end can name its own option.
///
/// On a BBMD the error comes back after its Forwarded-NPDUs to BDT peers and
/// foreign devices are queued, so a caller that retries the broadcast sends
/// those again. A foreign device whose Distribute-Broadcast-To-Network this
/// BBMD NAKs for the refused local leg may register again sooner: this
/// stack's own foreign device takes a NAK as a hint to re-register, at most
/// once per response timeout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct LoopbackBroadcastRefused;

impl LoopbackBroadcastRefused {
    /// The broadcast address to configure instead: 127.255.255.255.
    pub const LOOPBACK_BROADCAST: Ipv4Addr = LOOPBACK_BROADCAST;

    /// The refusal `error` carries, if it is one.
    pub fn from_error(error: &Error) -> Option<&Self> {
        match error {
            Error::Transport(io) => Self::from_io_error(io),
            _ => None,
        }
    }

    /// The refusal an I/O error carries, if it is one: directly, or inside
    /// an error a caller wrapped it in (bacnet-ops adds the address a
    /// broadcast went to) and kept as a source.
    pub fn from_io_error(error: &io::Error) -> Option<&Self> {
        let inner = error.get_ref()?;
        let mut next: Option<&(dyn std::error::Error + 'static)> = Some(inner);
        while let Some(error) = next {
            if let Some(found) = error.downcast_ref::<Self>() {
                return Some(found);
            }
            // An I/O error's own source skips the error it wraps.
            next = match error
                .downcast_ref::<io::Error>()
                .and_then(io::Error::get_ref)
            {
                Some(wrapped) => Some(wrapped as &(dyn std::error::Error + 'static)),
                None => error.source(),
            };
        }
        None
    }

    fn error() -> Error {
        Error::Transport(io::Error::new(io::ErrorKind::InvalidInput, Self::default()))
    }
}

impl fmt::Display for LoopbackBroadcastRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "a B/IP transport on a loopback interface with a wildcard-bound socket can't \
             broadcast to 255.255.255.255: the host would route it onto another interface. \
             Set the broadcast address to the loopback subnet's, {LOOPBACK_BROADCAST}"
        )
    }
}

impl std::error::Error for LoopbackBroadcastRefused {}

impl BindPlan {
    /// Whether the node this plan starts guards its local broadcasts: an
    /// explicit loopback interface on the wildcard socket. Per-address
    /// binding puts the socket on the loopback address itself, so its
    /// broadcasts stay on loopback.
    pub(super) fn loopback_on_wildcard(&self) -> bool {
        self.interface.is_loopback() && !self.per_address()
    }
}

impl NodeHandle {
    /// Whether a local broadcast to `to` is one this node refuses.
    pub(super) fn refuses_local_broadcast(&self, to: SocketAddrV4) -> bool {
        self.loopback_on_wildcard && to.ip().is_broadcast()
    }

    /// The error for this node's own local broadcast, refused, counted.
    pub(super) fn refuse_local_broadcast(&self) -> Error {
        self.refusals.count_loopback_limited_broadcast();
        LoopbackBroadcastRefused::error()
    }

    /// Count a BBMD's refused local rebroadcast of a frame it received,
    /// warning through a throttle, as nothing returns it to a caller.
    pub(super) fn refuse_local_rebroadcast(&self, to: SocketAddrV4) -> Option<u64> {
        self.refusals.count_loopback_limited_broadcast();
        crate::warn_throttled!(
            self.refusals.rebroadcast_warning,
            %to,
            "B/IP BBMD on a loopback interface: not rebroadcasting a forwarded broadcast to \
             255.255.255.255, which would leave through another interface; its peers and \
             foreign devices still get it. Use 127.255.255.255 as the broadcast address"
        )
    }
}

/// Warn, once at start, and return `true`, when a normal node or a BBMD is
/// set to broadcast to 255.255.255.255 from an explicit loopback interface
/// on the wildcard socket: each local broadcast it tries will be refused.
/// A foreign device sends its broadcasts to its BBMD, and a B/IP-M member
/// to its group, so neither is warned about.
pub(super) fn warn_at_start(plan: &BindPlan, multicast: bool, foreign: bool) -> bool {
    let refused =
        plan.loopback_on_wildcard() && plan.broadcast.is_broadcast() && !multicast && !foreign;
    if refused {
        warn!(
            interface = %plan.interface,
            "B/IP transport on a loopback interface has broadcast address 255.255.255.255: \
             nothing is wrong while it only sends unicast, but each local broadcast (a \
             Who-Is to the local network, for one) will be refused, as the host would send \
             it onto another interface. Use the loopback subnet's broadcast address, \
             127.255.255.255"
        );
    }
    refused
}

#[cfg(test)]
#[path = "loopback_guard_tests.rs"]
mod tests;
