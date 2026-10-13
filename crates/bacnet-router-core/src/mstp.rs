//! MS/TP ports: the glue between a [`Router`](crate::Router) and
//! `bacnet-mstp-core`'s master core (feature `mstp`).
//!
//! A router's MS/TP port takes extended frames: Clause 9.3 asks them of a
//! routing node, and they carry the 1,497-octet NPDUs that B/IP carries
//! (Table 6-1). [`MstpPort::config`] refuses a core configuration without
//! them. The port is a thin translator that keeps no state: the caller owns
//! the [`MasterCore`] and lends it to each call.
//!
//! Wiring a port to the router:
//!
//! - **Receive:** for each [`Action::DeliverNpdu`](bacnet_mstp_core::Action::DeliverNpdu)
//!   the core polls, pass [`MstpPort::arrived`]'s frame to
//!   [`Router::receive`](crate::Router::receive). The frame borrows the
//!   core's delivery, so let go of the router's answer before the next
//!   step.
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
//! - **Groups:** answer [`Host::group`](crate::router::Host::group) for
//!   the port with [`group`]: station 255 is the port's broadcast.
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

use bacnet_mstp_core::codec::BROADCAST_MAC;
use bacnet_mstp_core::{
    CoreError, MasterCore, MstpCoreConfig, NetworkPriority, QueueRefusal, QueueStorage,
    ReplyDecision, RequestId,
};

use crate::router::{Dest, Group, Limits, PortConfig, Send, SendRefusal};
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

/// What `mac` is on an MS/TP port, for [`Host::group`](crate::router::Host::group):
/// station 255 is its broadcast.
#[must_use]
pub fn group(mac: &[u8]) -> Group {
    if mac == [BROADCAST_MAC] {
        Group::Broadcast
    } else {
        Group::No
    }
}

/// A router port on an MS/TP master core.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MstpPort {
    port: PortId,
}

impl MstpPort {
    /// Port `port`.
    #[must_use]
    pub const fn new(port: PortId) -> Self {
        Self { port }
    }

    /// The port's index.
    #[must_use]
    pub const fn port(&self) -> PortId {
        self.port
    }

    /// The [`PortConfig`] of an MS/TP port on `network` whose core runs
    /// with `config`: the router's MAC is the station, and it carries
    /// [`MstpCoreConfig::max_npdu`] octets, 1,497, each way.
    /// Refused without extended frames.
    pub const fn config(config: &MstpCoreConfig, network: u16) -> Result<PortConfig, NotExtended> {
        if !config.extended_frames {
            return Err(NotExtended);
        }
        Ok(PortConfig {
            network,
            mac: Mac::from_array([config.station]),
            // 1497 with extended frames on, so the cast is exact.
            max_npdu: Limits::both(config.max_npdu() as u16),
        })
    }

    /// The router's view of a delivery the core polled
    /// ([`Action::DeliverNpdu`](bacnet_mstp_core::Action::DeliverNpdu)):
    /// from station `source`, a link broadcast when `destination` is 255.
    #[must_use]
    pub fn arrived<'d>(&self, source: u8, destination: u8, npdu: &'d [u8]) -> Arrived<'d> {
        Arrived {
            port: self.port,
            source: Mac::from_array([source]),
            link_group: destination == BROADCAST_MAC,
            npdu,
        }
    }

    /// Release a delivered request once the router has taken its frame:
    /// unless the router's own application answered it already, the core
    /// answers Reply Postponed (Clauses 6.5.4, 9.5.6.9). `None`, a delivery
    /// with no request, does nothing; so does a request already answered.
    /// The core's other refusals, such as a release after the requester may
    /// have stopped waiting ([`CoreError::RequestExpired`]), come back as
    /// they are.
    pub fn release<Q: QueueStorage, const NPDU: usize>(
        core: &mut MasterCore<'_, Q, NPDU>,
        request: Option<RequestId>,
        now: Instant,
    ) -> Result<(), CoreError> {
        let Some(request) = request else {
            return Ok(());
        };
        match core.complete_request(request, ReplyDecision::Postponed, now) {
            Err(CoreError::NoPendingRequest | CoreError::StaleRequest) => Ok(()),
            result => result,
        }
    }

    /// Carry out one of the router's sends on this port: queue the NPDU for
    /// its station, or for station 255 as a broadcast, at its network
    /// priority. A unicast must name one station, and never 255; a send
    /// for another port is refused.
    pub fn send<Q: QueueStorage, const NPDU: usize>(
        &self,
        core: &mut MasterCore<'_, Q, NPDU>,
        send: &Send<'_>,
        now: Instant,
    ) -> Result<(), SendRefusal> {
        if send.port != self.port {
            return Err(SendRefusal::Other);
        }
        let destination = match send.to {
            Dest::Broadcast => BROADCAST_MAC,
            Dest::Unicast(&[station]) if station != BROADCAST_MAC => station,
            Dest::Unicast(_) => return Err(SendRefusal::Other),
        };
        let priority = NetworkPriority::try_from(send.priority).map_err(|_| SendRefusal::Other)?;
        core.enqueue_npdu(destination, send.npdu, priority, now)
            .map_err(|why| match why {
                QueueRefusal::Full => SendRefusal::Full,
                QueueRefusal::Oversize => SendRefusal::TooLong,
                _ => SendRefusal::Other,
            })
    }
}
