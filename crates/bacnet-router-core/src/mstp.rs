//! MS/TP ports: the glue between a [`Router`](crate::Router) and
//! `bacnet-mstp-core`'s master core (feature `mstp`).
//!
//! A router's MS/TP port takes extended frames: Clause 9.3 asks them of a
//! routing node, and they carry the 1,497-octet NPDUs that B/IP carries
//! (Table 6-1). [`MstpPort::new`] and [`port_config`] refuse a core
//! configuration without them. The port owns its [`MasterCore`], as
//! bip-core's `NodePort` owns its node, and translates between it and the
//! router.
//!
//! Wiring a port to the router:
//!
//! - **Actions:** poll [`MstpPort::poll_action`] after each call into the
//!   port or its core, until [`Action::Idle`]. A
//!   [`PortAction::Core`] is the core's own action, which the port's
//!   adapter carries out as for any core (switch the transceiver, send a
//!   frame).
//! - **Receive:** a [`PortAction::Arrived`] is an NPDU for the router: pass
//!   its frame to [`Router::receive`](crate::Router::receive). The frame
//!   borrows the core's delivery, so let go of the router's answer before
//!   the next step.
//! - **Release:** then hand the delivery's request, if it had one, to
//!   [`MstpPort::release`]. Unless the router's own application already
//!   answered it, the core answers Reply Postponed at once, as a router
//!   does for a request it passes on (Clauses 6.5.4, 9.5.6.9), and as the
//!   hosted router does. The port doesn't carry the far side's answer back
//!   within the reply window.
//! - **Send:** hand each [`Send`] for the port to [`MstpPort::send`], and
//!   an error to [`Router::refused`](crate::Router::refused). It queues
//!   the NPDU with [`MasterCore::enqueue_npdu`] at the send's network
//!   priority, so the core's queues serve the most urgent first.
//! - **Line and timers:** feed the line's events and deadlines to the core
//!   through [`MstpPort::core_mut`], as any adapter does.
//! - **Groups:** [`port_config`] installs the rule that station 255
//!   is the port's broadcast.
//!
//! # Clause 6, for a router's MS/TP port
//!
//! | Obligation, in our words | Clause | Test |
//! | --- | --- | --- |
//! | A router queues what it relays by network priority, first in first out within one | 6.5.4 | `tests/glue_mstp.rs`: `priority_reaches_the_queue` |
//! | A router lets a link answer a relayed request that expects a reply at once, which on MS/TP is Reply Postponed | 6.5.4, 9.5.6.9 | `tests/glue_mstp.rs`: `relayed_requests_are_postponed` |
//! | A router carries NPDUs of up to 1,497 octets between B/IP and MS/TP, which takes extended frames on MS/TP | 6.6, Table 6-1, 9.3 | `tests/glue_mstp.rs`: `extended_profile_admits_1497_bytes_and_emits_decodable_extended_frame`, `extended_profile_admits_the_first_extended_boundary` |
//!
//! Local rules: a refusal maps to one [`SendRefusal`], counted once
//! (`tests/glue_mstp.rs`: `refusals_map_to_the_routers`); a full queue ends
//! a table answer that still has parts to send
//! (`saturated_queue_backpressures_network_control`).

use core::fmt;

use bacnet_mstp_core::codec::{BROADCAST_MAC, MAX_EXTENDED_MPDU_DATA};
use bacnet_mstp_core::{
    Action, MasterCore, MstpCoreConfig, QueueRefusal, QueueStorage, ReplyDecision, RequestError,
    RequestId,
};

use crate::npdu::NpduLimits;
use crate::router::{Dest, Groups, PortConfig, Send, SendRefusal};
use crate::{Arrived, Instant, Mac, PortId};

/// A core configuration without extended frames, which a router's MS/TP
/// port needs (Clause 9.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotExtended;

impl fmt::Display for NotExtended {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a router's MS/TP port needs extended frames")
    }
}

impl core::error::Error for NotExtended {}

/// What [`MstpPort::poll_action`] returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "an action the adapter doesn't carry out is lost"]
pub enum PortAction<'a> {
    /// An NPDU the core delivered, for the router: pass
    /// [`Arrived::frame`] to [`Router::receive`](crate::Router::receive),
    /// then `request` to [`MstpPort::release`].
    Arrived {
        /// The router's view of the delivery.
        arrived: Arrived<'a>,
        /// The request the NPDU asked a reply to, if it did.
        request: Option<RequestId>,
    },
    /// Any other action of the core, for its adapter to carry out; never
    /// [`Action::DeliverNpdu`]. [`Action::Idle`] when nothing waits.
    Core(Action<'a>),
}

/// A router port on an MS/TP master core, which it owns.
///
/// `NPDU` is the core's: extended frames carry
/// [`MAX_EXTENDED_MPDU_DATA`] octets, so that is the default.
pub struct MstpPort<'rx, Q, const NPDU: usize = MAX_EXTENDED_MPDU_DATA> {
    port: PortId,
    core: MasterCore<'rx, Q, NPDU>,
}

/// The port's index and its core's state.
impl<Q: QueueStorage, const NPDU: usize> fmt::Debug for MstpPort<'_, Q, NPDU> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MstpPort")
            .field("port", &self.port)
            .field("core", &self.core)
            .finish()
    }
}

/// The [`PortConfig`] of an MS/TP port on `network` whose core runs with
/// `config`, for building the router before the core exists: the router's
/// MAC is the station, and it carries [`MstpCoreConfig::max_npdu`] octets,
/// 1,497, each way. Refused without extended frames. A port already built
/// gives the same with [`MstpPort::port_config`].
pub const fn port_config(config: &MstpCoreConfig, network: u16) -> Result<PortConfig, NotExtended> {
    if !config.extended_frames {
        return Err(NotExtended);
    }
    Ok(extended_port_config(config, network))
}

/// The [`PortConfig`] of a port whose core runs with `config`, which has
/// extended frames.
const fn extended_port_config(config: &MstpCoreConfig, network: u16) -> PortConfig {
    PortConfig::new(
        network,
        Mac::from_array([config.station]),
        // 1497 with extended frames on, so the cast is exact.
        NpduLimits::both(config.max_npdu() as u16),
        Groups::Mstp,
    )
}

impl<'rx, Q: QueueStorage, const NPDU: usize> MstpPort<'rx, Q, NPDU> {
    /// Port `port` over `core`, which must run with extended frames.
    pub fn new(port: PortId, core: MasterCore<'rx, Q, NPDU>) -> Result<Self, NotExtended> {
        if !core.config().extended_frames {
            return Err(NotExtended);
        }
        Ok(Self { port, core })
    }

    /// The port's index.
    #[must_use]
    pub const fn port(&self) -> PortId {
        self.port
    }

    /// The port's [`PortConfig`] on `network`, as the free
    /// [`port_config`](fn@port_config) builds it from the core's
    /// configuration.
    #[must_use]
    pub fn port_config(&self, network: u16) -> PortConfig {
        // `new` took only a core with extended frames.
        extended_port_config(self.core.config(), network)
    }

    /// The core.
    #[must_use]
    pub const fn core(&self) -> &MasterCore<'rx, Q, NPDU> {
        &self.core
    }

    /// The core, to feed it the line's events and deadlines and take its
    /// counters. Poll its actions through [`Self::poll_action`], so that
    /// each NPDU reaches the router as an [`Arrived`].
    pub fn core_mut(&mut self) -> &mut MasterCore<'rx, Q, NPDU> {
        &mut self.core
    }

    /// The core back, the port gone.
    #[must_use]
    pub fn into_core(self) -> MasterCore<'rx, Q, NPDU> {
        self.core
    }

    /// The core's next action, an NPDU delivery as the router's
    /// [`Arrived`]: from station `source`, a link broadcast when it went to
    /// station 255.
    pub fn poll_action(&mut self) -> PortAction<'_> {
        match self.core.poll_action() {
            Action::DeliverNpdu {
                source,
                destination,
                npdu,
                request_id,
            } => PortAction::Arrived {
                arrived: Arrived {
                    port: self.port,
                    source: Mac::from_array([source]),
                    link_group: destination == BROADCAST_MAC,
                    npdu,
                },
                request: request_id,
            },
            action => PortAction::Core(action),
        }
    }

    /// Release a delivered request once the router has taken its frame:
    /// unless the router's own application answered it already, the core
    /// answers Reply Postponed (Clauses 6.5.4, 9.5.6.9). `None`, a delivery
    /// with no request, does nothing; so does a request already answered.
    /// The core's other refusal, a release after the requester may have
    /// stopped waiting ([`RequestError::Expired`]), comes back as it is.
    pub fn release(
        &mut self,
        request: Option<RequestId>,
        now: Instant,
    ) -> Result<(), RequestError> {
        let Some(request) = request else {
            return Ok(());
        };
        match self
            .core
            .complete_request(request, ReplyDecision::Postponed, now)
        {
            Err(RequestError::NoPending | RequestError::Stale) => Ok(()),
            result => result,
        }
    }

    /// Carry out one of the router's sends on this port: queue the NPDU for
    /// its station, or for station 255 as a broadcast, at its network
    /// priority. A unicast must name one station, and never 255; a send
    /// for another port is refused.
    pub fn send(&mut self, send: &Send<'_>, now: Instant) -> Result<(), SendRefusal> {
        if send.port != self.port {
            return Err(SendRefusal::Other);
        }
        let destination = match send.to {
            Dest::Broadcast => BROADCAST_MAC,
            Dest::Unicast(&[station]) if station != BROADCAST_MAC => station,
            Dest::Unicast(_) => return Err(SendRefusal::Other),
        };
        self.core
            .enqueue_npdu(destination, send.npdu, send.priority, now)
            .map_err(|why| match why {
                QueueRefusal::Full => SendRefusal::Full,
                QueueRefusal::Oversize => SendRefusal::TooLong,
                _ => SendRefusal::Other,
            })
    }
}
