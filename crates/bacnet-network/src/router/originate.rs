//! The router's own application requests, sent through its engine (#1699).

use std::fmt;
use std::sync::Weak;

use bacnet_router_core::router::{OriginateError, Request};

use super::BACnetRouter;
use super::actions::drain;
use super::dispatch::{Outputs, Shared};

/// Why [`BACnetRouter::originate`] failed: it queued nothing, but for a
/// global broadcast's [`QueueFull`](Self::QueueFull).
///
/// It is `#[non_exhaustive]`, so a new reason isn't a breaking change: a
/// `match` needs a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OriginateFailure {
    /// The engine refused the request, for this reason.
    Refused(OriginateError),
    /// The egress port's queue was full, or closing; the refusal is
    /// counted in [`Counters::refused`](super::Counters::refused).
    ///
    /// For a global broadcast, which goes on every port's queue, this
    /// means at least one port refused its copy: the other ports' copies
    /// were queued and are sent. Retrying the whole request sends those
    /// ports a second copy, so don't retry a global broadcast blindly.
    QueueFull,
    /// The router has stopped.
    Stopped,
}

impl fmt::Display for OriginateFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(error) => error.fmt(f),
            Self::QueueFull => f.write_str("the egress port's queue refused the request"),
            Self::Stopped => f.write_str("the router has stopped"),
        }
    }
}

impl std::error::Error for OriginateFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Refused(error) => Some(error),
            _ => None,
        }
    }
}

impl From<OriginateError> for OriginateFailure {
    fn from(error: OriginateError) -> Self {
        Self::Refused(error)
    }
}

impl BACnetRouter {
    /// Send a request from the router's own application to another
    /// network, routed by its table like any other traffic (#1699).
    ///
    /// The engine picks the route and writes the NPDU: toward the next
    /// router with the DNET, DADR and a full hop count, or onto a directly
    /// attached network to the DADR, with the source address
    /// [`Request::source`] selects. By default that is the router's
    /// address on the egress port, so a reply comes back to that port and
    /// reaches the router's local APDU receiver as any APDU for the router
    /// does. The NPDU goes on the egress port's queue at the request's
    /// network priority, as any routed send does, with no data attributes.
    /// `Ok` means it was queued, not sent.
    ///
    /// Synchronous: it takes the engine's lock for the call and its drain,
    /// like a port's dispatch of one frame, and never waits. A network the
    /// table lacks is refused with [`OriginateError::NoRoute`], and a
    /// Who-Is-Router for it goes out on every port, once per cooldown;
    /// retry once it is learned. After [`Self::stop`] every call fails with
    /// [`OriginateFailure::Stopped`].
    ///
    /// A global broadcast (DNET 0xFFFF with an empty DADR, such as a
    /// Who-Is for every network; #1809) goes on every port's queue as that
    /// link's broadcast, still global with a full hop count so the routers
    /// there pass it on. With [`SourceSelection::EgressPort`] each copy is
    /// from its own port's address; with [`SourceSelection::Port`] the
    /// other ports' copies name that port's address as SNET and SADR. A
    /// queue that refuses its copy fails the call with
    /// [`OriginateFailure::QueueFull`], but the copies the other ports'
    /// queues took are still sent: the call is not all or nothing, and a
    /// retry repeats the broadcast on those ports.
    ///
    /// [`SourceSelection::EgressPort`]: bacnet_router_core::router::SourceSelection::EgressPort
    /// [`SourceSelection::Port`]: bacnet_router_core::router::SourceSelection::Port
    pub fn originate(&self, request: &Request<'_>) -> Result<(), OriginateFailure> {
        originate(&self.outputs, &self.shared, request)
    }
}

/// [`BACnetRouter::originate`] on the router's queues and engine: the
/// router's own call, and its [application port's](super::RouterApplicationPort),
/// which holds the same two handles.
pub(super) fn originate(
    outputs: &Weak<Outputs>,
    shared: &Shared,
    request: &Request<'_>,
) -> Result<(), OriginateFailure> {
    // Only the dispatch tasks own the queues: once `stop` has ended them,
    // nothing is left to send on.
    let outputs = outputs.upgrade().ok_or(OriginateFailure::Stopped)?;
    let now = shared.engine.now();
    let mut router = shared.engine.lock();
    let result = router.originate(request, now);
    // A search for a missing route is polled too. Only the request's own
    // refusal fails it: a Router-Busy or search refused in the same drain
    // doesn't.
    let drained = drain(&mut router, &[], &outputs.send_txs, now);
    drop(router);
    result?;
    if drained.originate_refused {
        Err(OriginateFailure::QueueFull)
    } else {
        Ok(())
    }
}
