//! Initialize-Routing-Table and its Ack (Clauses 6.4.7, 6.4.8, 6.6.3.8,
//! 6.6.3.9).

use super::Storage;
use super::config::Options;
use super::control::Claimed;
use super::counters::bump;
use super::host::{ControlClass, Event, Host, Targets};
use super::ingress::{Arrival, Discard, Received};
use super::tasks::Task;
use crate::PortId;
use crate::message::RoutingEntries;
use crate::npdu::{GLOBAL_NETWORK, Npdu};

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
    /// Initialize-Routing-Table. A count of 0 is a query, answered with the
    /// whole table in Acks, ascending, each network with its port's wire
    /// Port ID; the table doesn't change. The host is asked only about a
    /// query that carries an SNET ([`ControlClass::InitQuery`], #2715),
    /// since its answer crosses networks toward a source the query only
    /// claims; refused, it draws no Ack. The
    /// Acks go back out of the arrival port to the link sender, toward the
    /// query's SNET and SADR when it has them, from the router's address
    /// the query named when that is on another port (named at the arrival
    /// port, the answer has no SNET, as for no DNET); each fits the arrival port's
    /// limit and 501 octets after a relay adds its SNET (at most 124
    /// entries). Any other count is an update, applied in order: Port ID
    /// 0 removes a learned route, any other installs one through that port
    /// and the sender, and the sender gets an Ack with no data. Reserved
    /// networks and unknown Port IDs are skipped, direct routes are never
    /// touched, and the route cap ends the update for new networks.
    ///
    /// An update that carries an SNET came from another network, and is
    /// dropped before the host is asked unless
    /// [`Options::remote_table_updates`] is on (#2717). Clause 6.6.3.8 has
    /// a router apply the update it receives and sets no limit on who may
    /// send one; this limit is the router's own.
    ///
    /// From an isolated port (one with an isolated peer), an entry is
    /// refused, counted and reported when its network's route goes through
    /// another isolated port, when it names another isolated port, or when
    /// it would bring to the arrival port a network the table reaches
    /// through any other port (#2684; decision of 2026-11-20, a departure
    /// from Clause 6.6.3.8's plain replacement). It may still remove a
    /// route through an ordinary port or point one at another ordinary
    /// port: isolation is no access control on table updates, which
    /// [`Host::authorize`] decides.
    pub(super) fn initialize_routing_table<'f>(
        &mut self,
        at: &Arrival<'f>,
        npdu: &Npdu<'f>,
        options: &Options,
        host: &mut impl Host,
    ) -> Received<'f> {
        let Ok(entries) = RoutingEntries::parse(npdu.payload) else {
            return self.bad_message();
        };
        if entries.count() == 0 {
            // Named at the arrival port, the router answers as a query
            // with no DNET is answered: a relay adds that SNET itself.
            let named = npdu
                .destination
                .and_then(|address| self.own_port(address))
                .filter(|port| *port != at.port);
            // The requester's SNET and SADR become the answer's DNET and
            // DADR: refuse any that no reply could carry.
            let source = npdu.destination.filter(|_| named.is_some());
            if super::ReplyPlan::new(at.port, at.source, source, npdu.source).is_err() {
                return self.bad_message();
            }
            // From another network, it is the host's to refuse (#2715).
            let class = ControlClass::InitQuery;
            let none = Targets::Entries(entries);
            if npdu.source.is_some() && !self.authorize(at, npdu, class, none, host) {
                return Received::Discarded(Discard::Denied);
            }
            self.out
                .push(Task::table_ack(at.port, at.source, npdu.source, named));
            return Received::Handled;
        }
        if let Some(source) = npdu.source
            && !options.remote_table_updates
        {
            bump(&mut self.counters.denied);
            host.observe(Event::RemoteUpdateRefused {
                port: at.port,
                snet: source.network,
            });
            return Received::Discarded(Discard::Denied);
        }
        if !self.authorize(
            at,
            npdu,
            ControlClass::InitMgmt,
            Targets::Entries(entries),
            host,
        ) {
            return Received::Discarded(Discard::Denied);
        }
        for entry in entries.iter() {
            let network = entry.network;
            if network == 0 || network == GLOBAL_NETWORK {
                continue;
            }
            if self.hidden_from(at.port, network) {
                self.isolation_refused(at.port, network, ControlClass::InitMgmt, host);
                continue;
            }
            if entry.port_id == 0 {
                if self.table.apply_management_removal(network) {
                    host.observe(Event::UpdateRemoved { network });
                }
                continue;
            }
            let Some(port) = PortId::from_wire_id(entry.port_id, self.port_count) else {
                host.observe(Event::UnknownPortId {
                    network,
                    port_id: entry.port_id,
                });
                continue;
            };
            let captures = port == at.port
                && self.isolated_arrival(port)
                && self
                    .table
                    .lookup(network)
                    .is_some_and(|route| route.port() != port);
            if captures || super::config::isolated_pair(&self.ports, at.port, port) {
                self.isolation_refused(at.port, network, ControlClass::InitMgmt, host);
                continue;
            }
            if self.table.len() >= options.cap(R) && self.table.lookup(network).is_none() {
                self.table.record_learning_cap();
                host.observe(Event::RouteCap {
                    class: ControlClass::InitMgmt,
                });
                break;
            }
            if self
                .table
                .apply_management_update(network, port, at.source_mac, at.now)
            {
                host.observe(Event::UpdateApplied { network, port });
            }
        }
        self.out.push(Task::update_ack(at.port, at.source_mac));
        Received::Handled
    }

    /// Initialize-Routing-Table-Ack: one with no data answers an update and
    /// says nothing. Otherwise its networks are learned through the arrival
    /// port and the sender, as I-Am-Router's are, at the route cap too,
    /// where the networks the table holds are still refreshed (#1698); its
    /// Port IDs are the sender's own numbering, so they are not used.
    /// Reserved networks are skipped.
    pub(super) fn routing_table_ack<'f>(
        &mut self,
        at: &Arrival<'f>,
        npdu: &Npdu<'f>,
        options: &Options,
        host: &mut impl Host,
    ) -> Received<'f> {
        if npdu.payload.is_empty() {
            return Received::Handled;
        }
        let Ok(entries) = RoutingEntries::parse(npdu.payload) else {
            return self.bad_message();
        };
        if !self.authorize(
            at,
            npdu,
            ControlClass::InitAck,
            Targets::Entries(entries),
            host,
        ) {
            return Received::Discarded(Discard::Denied);
        }
        let mut claimed = Claimed::new();
        for entry in entries.iter() {
            let network = entry.network;
            if network == 0 || network == GLOBAL_NETWORK {
                continue;
            }
            if !self.claim(
                at,
                network,
                &mut claimed,
                ControlClass::InitAck,
                options,
                host,
            ) {
                break;
            }
        }
        Received::Handled
    }
}
