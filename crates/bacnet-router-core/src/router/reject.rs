//! The rejects this router sends, and the ones it passes on.

use super::counters::bump;
use super::host::{Event, Group, Host};
use super::ingress::{Arrival, Discard, Received};
use super::tasks::Task;
use super::Storage;
use crate::message::RejectReason;
use crate::npdu::{Address, Npdu, GLOBAL_NETWORK};
use crate::PortId;

impl<const P: usize, const R: usize, const H: usize, const D: usize, const N: usize>
    Storage<P, R, H, D, N>
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
        reason: RejectReason,
        host: &mut impl Host,
    ) {
        let task = match origin {
            None => Task::reject(at.port, at.source, None, network, reason),
            Some(origin) if self.is_own(origin) => {
                host.observe(Event::OwnOrigin { network });
                return;
            }
            Some(origin) => {
                let ports = &self.ports[..self.port_count];
                match ports.iter().position(|port| port.network == origin.network) {
                    // At most 255 ports.
                    Some(index) => {
                        Task::reject(PortId(index as u8), origin.mac, None, network, reason)
                    }
                    None => Task::reject(at.port, at.source, Some(origin), network, reason),
                }
            }
        };
        bump(&mut self.counters.rejects);
        self.out.push(task);
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
            Some(route) if self.group_of(&route, npdu, host) != Group::No => self.group_drop(),
            Some(route) => self.forward(at, npdu, &route, host),
            None => {
                bump(&mut self.counters.unroutable_rejects);
                Received::Discarded(Discard::Unroutable)
            }
        }
    }
}
