//! The rejects this router sends, and the ones it passes on.

use super::Group;
use super::Storage;
use super::counters::bump;
use super::host::{Event, Host};
use super::ingress::{Arrival, Discard, Received};
use super::tasks::Task;
use crate::npdu::RejectMessageReason;
use crate::npdu::{Address, GLOBAL_NETWORK, Npdu};
use crate::{Mac, PortId};

/// The owned return path for a routed send that can draw a length reject.
/// Copy it from [`Send::reject_origin`](super::Send::reject_origin) when
/// queueing that send, and return it to
/// [`Router::refused_later`](super::Router::refused_later) if refused.
///
/// Only the router constructs this record. It retains the original DNET
/// and the arrival's return path even when the queued NPDU no longer has
/// its destination. Forwarded rejects and fan-outs carry no such record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RejectOrigin {
    port: PortId,
    to: Mac,
    network: u16,
    destination_network: u16,
    destination: Mac,
}

impl RejectOrigin {
    // A valid routed DNET is never zero. Keep the empty value all zero
    // bits, as the firmware's static router storage requires.
    pub(super) const NONE: Self = Self {
        port: PortId(0),
        to: Mac::EMPTY,
        network: 0,
        destination_network: 0,
        destination: Mac::EMPTY,
    };

    pub(super) fn as_option(self) -> Option<Self> {
        (self.network != 0).then_some(self)
    }

    fn new(port: PortId, to: &[u8], destination: Option<Address<'_>>, network: u16) -> Self {
        // These addresses have already passed the ingress bounds.
        let mac = |bytes: &[u8]| Mac::from_slice(bytes).unwrap_or(Mac::EMPTY);
        Self {
            port,
            to: mac(to),
            network,
            destination_network: destination.map_or(0, |address| address.network),
            destination: destination.map_or(Mac::EMPTY, |address| mac(address.mac)),
        }
    }

    fn task(self, reason: RejectMessageReason) -> Task {
        let destination = (self.destination_network != 0).then_some(Address {
            network: self.destination_network,
            mac: self.destination.as_slice(),
        });
        Task::reject(
            self.port,
            self.to.as_slice(),
            destination,
            self.network,
            reason,
        )
    }
}

impl<
    const P: usize,
    const R: usize,
    const H: usize,
    const D: usize,
    const N: usize,
    G: super::GroupClassifier,
    const B: usize,
> Storage<P, R, H, D, N, G, B>
{
    /// Whether `address` is this router: a port's network with the
    /// router's MAC on that port.
    pub(super) fn is_own(&self, address: Address<'_>) -> bool {
        self.ports[..self.port_count]
            .iter()
            .any(|port| port.network == address.network && port.mac == *address.mac)
    }

    /// Queue the reject naming `network` with `reason` that answers the
    /// frame in hand. It is for whoever first sent the frame (Clause
    /// 6.4.4), whom `origin`, the frame's SNET and SADR, names:
    ///
    /// - no origin: the link sender, as a unicast on the arrival port;
    /// - one of the router's own networks: the SADR, as a unicast on that
    ///   network's port, with no DNET, since a node there drops an NPDU
    ///   that names a remote DNET;
    /// - the router itself, on one of its networks: nobody, for the frame
    ///   came back to the router that sent it;
    /// - any other network: the link sender, which knows the way back, with
    ///   the origin as the reject's DNET and DADR.
    ///
    /// The reject carries no SNET and starts with a hop count of 255.
    pub(super) fn reject(
        &mut self,
        at: &Arrival<'_>,
        origin: Option<Address<'_>>,
        network: u16,
        reason: RejectMessageReason,
        host: &mut impl Host,
    ) {
        if let Some(origin) = self.reject_origin(at, origin, network) {
            self.queue_reject(origin, reason);
        } else {
            host.observe(Event::OwnOrigin { network });
        }
    }

    /// Resolve the same return path for immediate and queued refusals.
    pub(super) fn reject_origin(
        &self,
        at: &Arrival<'_>,
        origin: Option<Address<'_>>,
        network: u16,
    ) -> Option<RejectOrigin> {
        Some(match origin {
            None => RejectOrigin::new(at.port, at.source, None, network),
            Some(origin) if self.is_own(origin) => return None,
            Some(origin) => {
                let ports = &self.ports[..self.port_count];
                match ports.iter().position(|port| port.network == origin.network) {
                    // At most 255 ports.
                    Some(index) => {
                        RejectOrigin::new(PortId(index as u8), origin.mac, None, network)
                    }
                    None => RejectOrigin::new(at.port, at.source, Some(origin), network),
                }
            }
        })
    }

    /// Append a rejection without replacing any already waiting output.
    pub(super) fn queue_reject(&mut self, origin: RejectOrigin, reason: RejectMessageReason) {
        bump(&mut self.counters.rejects);
        self.out.push(origin.task(reason));
    }

    /// Pass on a reject the table has taken (Clause 6.6.3.5). One with no
    /// DNET, or addressed to the router's own address, is for the router.
    /// Any other is routed toward the node its DNET and DADR name, without
    /// a reachability check, and is dropped when the router can't: its
    /// DNET is 0xFFFF or the arrival network, or the table has no route.
    /// One for a group DADR on its delivery port is dropped and counted as
    /// any network message is. Nothing answers a reject.
    pub(super) fn pass_on_reject<'f>(
        &mut self,
        at: &Arrival<'f>,
        npdu: &Npdu<'f>,
        host: &mut impl Host,
    ) -> Received<'f> {
        let destination = match npdu.destination {
            Some(destination) if !self.is_own(destination) => destination,
            _ => return Received::ToRouter(*npdu),
        };
        let route = match destination.network {
            GLOBAL_NETWORK => None,
            network if network == at.network => None,
            network => self.table.lookup(network),
        };
        match route {
            // Never as a broadcast (decisions L23 and L24 on #1677).
            Some(route) if self.group_of(&route, npdu) != Group::No => self.group_drop(),
            Some(route) => self.forward(at, npdu, &route, host),
            None => {
                bump(&mut self.counters.unroutable_rejects);
                Received::Discarded(Discard::Unroutable)
            }
        }
    }
}
