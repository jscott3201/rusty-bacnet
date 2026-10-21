//! The router's own application requests, sent through its engine (#1699).

use std::fmt;

use bacnet_router_core::router::{OriginateError, Request};

use super::actions::drain;
use super::BACnetRouter;

/// Why [`BACnetRouter::originate`] queued nothing.
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
    /// A global broadcast (DNET 0xFFFF) has no path here yet and is refused
    /// with [`OriginateError::ReservedNetwork`]: send a remote broadcast
    /// (an empty DADR) to each network the router reaches instead, its own
    /// ports' networks included, where each leaves as that port's local
    /// broadcast.
    pub fn originate(&self, request: &Request<'_>) -> Result<(), OriginateFailure> {
        // Only the dispatch tasks own the queues: once `stop` has ended
        // them, nothing is left to send on.
        let outputs = self.outputs.upgrade().ok_or(OriginateFailure::Stopped)?;
        let shared = &*self.shared;
        let now = shared.engine.now();
        let mut router = shared.engine.lock();
        let result = router.originate(request, now);
        // A search for a missing route is polled too. Only the request's
        // own refusal fails it: a Router-Busy or search refused in the
        // same drain doesn't.
        let drained = drain(&mut router, &[], &outputs.send_txs, now);
        drop(router);
        result?;
        if drained.originate_refused {
            Err(OriginateFailure::QueueFull)
        } else {
            Ok(())
        }
    }
}
