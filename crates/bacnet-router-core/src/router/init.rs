//! Initialize-Routing-Table and its Ack (Clauses 6.4.7, 6.4.8, 6.6.3.8,
//! 6.6.3.9).

use super::config::Options;
use super::control::Claimed;
use super::host::{ControlClass, Event, Host, Targets};
use super::ingress::{Arrival, Discard, Received};
use super::tasks::Task;
use super::Storage;
use crate::message::RoutingEntries;
use crate::npdu::{Npdu, GLOBAL_NETWORK};
use crate::PortId;

impl<const P: usize, const R: usize, const H: usize, const D: usize, const N: usize>
    Storage<P, R, H, D, N>
{
    /// Initialize-Routing-Table. A count of 0 is a query, answered with the
    /// whole table in Acks of at most 124 entries, ascending, each network
    /// with its port's wire Port ID; the table doesn't change, and the host
    /// isn't asked. Any other count is an update, applied in order: Port ID
    /// 0 removes a learned route, any other installs one through that port
    /// and the sender, and the sender gets an Ack with no data. Reserved
    /// networks and unknown Port IDs are skipped, direct routes are never
    /// touched, and the route cap ends the update for new networks.
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
            self.out.push(Task::table_ack(at.port, at.source_mac));
            return Received::Handled;
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
            if self.table.len() >= usize::from(options.route_cap)
                && self.table.lookup(network).is_none()
            {
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
    /// port and the sender, as I-Am-Router's are; its Port IDs are the
    /// sender's own numbering, so they are not used. Reserved networks are
    /// skipped.
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
