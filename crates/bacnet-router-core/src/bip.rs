//! B/IP ports: the router's adapter over `bacnet-bip-core`'s port driver
//! (feature `bip`).
//!
//! The port itself is bip-core's ([`bacnet_bip_core::port`]),
//! one type per mode, since a link is never both a BBMD and a foreign
//! device: a [`NodePort`] for a node in normal or foreign-device mode, and a
//! [`BbmdPort`] for a BBMD, which forwards broadcasts and answers the
//! management requests from its tables. It sends what its node asks (the
//! NAKs, a foreign device's registrations, the broadcasts it holds until
//! the first registration resolves) through the caller's [`Datagrams`].
//!
//! To the router a B/IP port is data, the [`PortConfig`] that
//! [`BipPort::config`] reads from the node's [`Link`]. [`BipPort`] is a
//! thin translator that keeps no state: the caller owns the bip-core port,
//! which owns its node, and lends it to each call.
//!
//! Wiring a port to the router:
//!
//! - **Receive:** hand each datagram to the bip-core port's `receive`. For
//!   [`Ingress::Npdu`], pass [`BipPort::arrived`]'s frame to
//!   [`Router::receive`](crate::Router::receive). Its source is the B/IP
//!   address the node delivered from, which for a Forwarded-NPDU is the
//!   origin (Annex J.2.5), so the SADR the router adds names the origin
//!   (#1678). A datagram the node drops never reaches the router, and
//!   neither does a management request a BBMD port answered.
//! - **Send:** hand each [`Send`] for the port to [`BipPort::send`], and an
//!   error to [`Router::refused`](crate::Router::refused). A unicast goes
//!   to the B/IP address its MAC spells, and a group address is refused
//!   (#1677); a broadcast is held while a foreign device's first
//!   registration is outstanding, and also forwarded by a BBMD.
//! - **Start:** call [`NodePort::start`] once, before anything else: a
//!   foreign device's first registration is due at once, and its response
//!   timeout is the port's first deadline. A [`BbmdPort`] registers
//!   nowhere, and needs no start.
//! - **Timers:** sleep until the earliest of the router's and the ports'
//!   `next_deadline`, and call each one's `on_deadline`.
//! - **Groups:** [`BipPort::config`] installs the link's rules in the
//!   router, including the UDP port its local broadcast reaches.
//!
//! [`Ingress::Npdu`]: bacnet_bip_core::port::Ingress::Npdu
//!
//! # Annex J and Clause 6, for a router's B/IP port
//!
//! | Obligation, in our words | Clause | Test |
//! | --- | --- | --- |
//! | A Forwarded-NPDU's originating address is the source of its NPDU, so a router names it in the SADR it adds, and the network priority travels with the message | J.2.5, 6.2.2, 6.5.4 | `tests/glue_bip_mstp.rs`: `forwarded_origin_and_priority_survive_bip_to_mstp_composition` |
//! | A port that is a foreign-device registration registers before the router announces its networks there | J.7.2, J.7.4 | `tests/glue_bip_foreign.rs`: `foreign_port_announces_after_registration`, `foreign_port_announces_after_the_response_timeout`, `held_broadcasts_leave_before_a_later_one` |
//! | A BBMD port hands the router the broadcasts its subnet, its peers and its foreign devices send, and forwards them, and forwards the router's own broadcasts too | J.4.5, J.7.1 | `tests/glue_bip.rs`: `bbmd_port_delivers_and_forwards` |
//! | A BBMD port answers the management requests itself, so none reaches the router | J.4.4.1, J.5.2 | `tests/glue_bip.rs`: `bbmd_port_delivers_and_forwards` |
//! | A directed message goes to one node's own address, never to a group address | J.2.11, J.3 | `tests/glue_bip.rs`: `unicast_group_destinations_are_rejected_before_encoding_or_send` |
//!
//! Local rules: a datagram the node drops, a malformed one included,
//! changes no router or queue state (`tests/glue_bip_mstp.rs`:
//! `strict_bip_failure_precedes_router_or_queue_mutation`); each refusal of
//! a router send maps to one [`SendRefusal`] and is counted once
//! (`real_bip_success_and_admission_failures_are_observed_once`,
//! `tests/glue_bip.rs`: `refusals_map_to_the_routers`). The port's own
//! rules are bip-core's ([`bacnet_bip_core::port`]).

use core::net::SocketAddrV4;

use bacnet_bip_core::codec::{address_from_slice, address_to_octets};
use bacnet_bip_core::node::{self, Cast, Delivery, Link};
use bacnet_bip_core::port::{BbmdPort, Datagrams, NodePort};

use crate::npdu::NpduLimits;
use crate::router::{Dest, Groups, PortConfig, Send, SendRefusal};
use crate::{Arrived, Instant, Mac, PortId, MAX_NPDU};

/// A router port on a B/IP link, by its index: the translator between the
/// router and a bip-core [`NodePort`] or [`BbmdPort`], which the caller owns
/// and lends to each call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BipPort {
    port: PortId,
}

impl BipPort {
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

    /// The [`PortConfig`] of a B/IP port on `network` over `link`: the
    /// router's MAC is the link's own B/IP address, and its limits are
    /// [`Link::max_npdu`] for each cast, at most [`MAX_NPDU`] (Table 6-1).
    #[must_use]
    pub fn config(link: &Link, network: u16) -> PortConfig {
        // At most MAX_NPDU, so the casts are exact.
        let limit = |cast| link.max_npdu(cast).min(MAX_NPDU) as u16;
        PortConfig::new(
            network,
            Mac::from_array(address_to_octets(link.address)),
            NpduLimits {
                unicast: limit(Cast::Unicast),
                broadcast: limit(Cast::Broadcast),
            },
            Groups::Bip {
                destinations: link.groups(),
                port: link.address.port(),
            },
        )
    }

    /// The router's view of an NPDU the port delivered
    /// ([`Ingress::Npdu`](bacnet_bip_core::port::Ingress::Npdu)): from its
    /// source B/IP address, a link broadcast when it reached more than this
    /// node.
    #[must_use]
    pub fn arrived<'d>(&self, delivery: &Delivery<'d>) -> Arrived<'d> {
        Arrived {
            port: self.port,
            source: Mac::from_array(address_to_octets(delivery.source)),
            link_group: delivery.group,
            npdu: delivery.npdu,
        }
    }

    /// Carry out one of the router's sends on `link`, the bip-core port of
    /// this router port: a unicast to the B/IP address its MAC spells,
    /// which must be six octets and no group address, or a broadcast. A
    /// send for another port is refused.
    ///
    /// Each refusal maps to the router's: an NPDU too long for the link, or
    /// for a held broadcast's slot, is [`SendRefusal::TooLong`]; a full
    /// hold or socket is [`SendRefusal::Full`]; anything else, a group
    /// destination included, [`SendRefusal::Other`].
    pub fn send(
        &self,
        link: &mut impl Port,
        send: &Send<'_>,
        now: Instant,
        out: &mut impl Datagrams,
    ) -> Result<(), SendRefusal> {
        if send.port != self.port {
            return Err(SendRefusal::Other);
        }
        match send.to {
            Dest::Unicast(mac) => {
                let to = address_from_slice(mac).map_err(|_| SendRefusal::Other)?;
                link.unicast_to(to, send.npdu, now, out)
            }
            Dest::Broadcast => link.broadcast_all(send.npdu, now, out),
        }
        .map_err(refusal)
    }
}

/// A bip-core port the router sends through: a [`NodePort`] or a
/// [`BbmdPort`]. It is sealed: only those two implement it.
pub trait Port: sealed::Sealed {}

impl<const X: usize, const N: usize> Port for NodePort<'_, X, N> {}

impl<const B: usize, const F: usize> Port for BbmdPort<'_, B, F> {}

mod sealed {
    use super::{BbmdPort, Datagrams, Instant, NodePort, SocketAddrV4};
    use bacnet_bip_core::node::SendRefusal;

    /// What [`BipPort::send`](super::BipPort::send) asks of a port.
    pub trait Sealed {
        /// The port's unicast.
        fn unicast_to(
            &mut self,
            to: SocketAddrV4,
            npdu: &[u8],
            now: Instant,
            out: &mut impl Datagrams,
        ) -> Result<(), SendRefusal>;

        /// The port's broadcast, held or sent.
        fn broadcast_all(
            &mut self,
            npdu: &[u8],
            now: Instant,
            out: &mut impl Datagrams,
        ) -> Result<(), SendRefusal>;
    }

    impl<const X: usize, const N: usize> Sealed for NodePort<'_, X, N> {
        fn unicast_to(
            &mut self,
            to: SocketAddrV4,
            npdu: &[u8],
            now: Instant,
            out: &mut impl Datagrams,
        ) -> Result<(), SendRefusal> {
            self.unicast(to, npdu, now, out)
        }

        fn broadcast_all(
            &mut self,
            npdu: &[u8],
            now: Instant,
            out: &mut impl Datagrams,
        ) -> Result<(), SendRefusal> {
            self.broadcast(npdu, now, out).map(|_| ())
        }
    }

    impl<const B: usize, const F: usize> Sealed for BbmdPort<'_, B, F> {
        fn unicast_to(
            &mut self,
            to: SocketAddrV4,
            npdu: &[u8],
            now: Instant,
            out: &mut impl Datagrams,
        ) -> Result<(), SendRefusal> {
            self.unicast(to, npdu, now, out)
        }

        fn broadcast_all(
            &mut self,
            npdu: &[u8],
            now: Instant,
            out: &mut impl Datagrams,
        ) -> Result<(), SendRefusal> {
            self.broadcast(npdu, now, out)
        }
    }
}

/// The router's refusal for the port's.
fn refusal(why: node::SendRefusal) -> SendRefusal {
    match why {
        node::SendRefusal::TooLong { .. } => SendRefusal::TooLong,
        node::SendRefusal::HoldFull | node::SendRefusal::SocketFull => SendRefusal::Full,
        _ => SendRefusal::Other,
    }
}
