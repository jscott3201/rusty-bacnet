//! Network-layer messages: routed first when they name another network,
//! otherwise handled by type (Clause 6.6.3).
//!
//! Each handler reads its whole payload before it changes anything, so a
//! bad tail changes nothing and sends nothing. A protected message then
//! goes to [`Host::authorize`], and a refusal drops it silently.

use super::config::Options;
use super::counters::bump;
use super::host::{Claim, ControlClass, Event, Group, Host, Targets};
use super::ingress::{Arrival, Discard, Received};
use super::output::Kind;
use super::tasks::Task;
use super::Storage;
use crate::message::{
    DisconnectConnection, EstablishConnection, ICouldBeRouter, MessageType, NetworkNumberIs,
    Networks, Reject, RejectReason, WhoIsRouter,
};
use crate::npdu::{Address, Npdu, GLOBAL_NETWORK};
use crate::swap_apart;
use crate::table::Learned;

impl<const P: usize, const R: usize, const H: usize, const D: usize, const N: usize>
    Storage<P, R, H, D, N>
{
    /// A network message. What-Is, Number-Is and Reject are always handled
    /// here. Any other that names a DNET besides 0xFFFF and the arrival
    /// network is routed like an APDU, opaquely, and never taken locally;
    /// but one for a group DADR on its delivery port is dropped, never
    /// turned into a broadcast (decisions L23 and L24 on #1677).
    pub(super) fn network_message<'f>(
        &mut self,
        at: &Arrival<'f>,
        npdu: &Npdu<'f>,
        kind: MessageType,
        options: &Options,
        host: &mut impl Host,
    ) -> Received<'f> {
        let always_here = matches!(
            kind,
            MessageType::WHAT_IS_NETWORK_NUMBER
                | MessageType::NETWORK_NUMBER_IS
                | MessageType::REJECT_MESSAGE_TO_NETWORK
        );
        let elsewhere = npdu.destination.filter(|destination| {
            destination.network != GLOBAL_NETWORK && destination.network != at.network
        });
        if let Some(destination) = elsewhere.filter(|_| !always_here) {
            let Some(route) = self.find_route(at, npdu, destination.network, options, host) else {
                return Received::Handled;
            };
            // Never as a broadcast its sender didn't ask for (L23).
            if self.group_of(&route, npdu, host) != Group::No {
                return self.group_drop();
            }
            return self.forward(at, npdu, &route, host);
        }
        match kind {
            MessageType::WHO_IS_ROUTER_TO_NETWORK => self.who_is_router(at, npdu, host),
            MessageType::I_AM_ROUTER_TO_NETWORK => self.i_am_router(at, npdu, options, host),
            MessageType::I_COULD_BE_ROUTER_TO_NETWORK => {
                self.i_could_be_router(at, npdu, options, host)
            }
            MessageType::REJECT_MESSAGE_TO_NETWORK => self.received_reject(at, npdu, options, host),
            MessageType::ROUTER_BUSY_TO_NETWORK => self.flow_control(at, npdu, true, options, host),
            MessageType::ROUTER_AVAILABLE_TO_NETWORK => {
                self.flow_control(at, npdu, false, options, host)
            }
            MessageType::INITIALIZE_ROUTING_TABLE => {
                self.initialize_routing_table(at, npdu, options, host)
            }
            MessageType::INITIALIZE_ROUTING_TABLE_ACK => {
                self.routing_table_ack(at, npdu, options, host)
            }
            MessageType::ESTABLISH_CONNECTION_TO_NETWORK => {
                let Ok(request) = EstablishConnection::parse(npdu.payload) else {
                    return self.bad_message();
                };
                host.observe(Event::EstablishConnection {
                    network: request.network,
                    termination_time: request.termination_time,
                });
                Received::Handled
            }
            MessageType::DISCONNECT_CONNECTION_TO_NETWORK => {
                let Ok(request) = DisconnectConnection::parse(npdu.payload) else {
                    return self.bad_message();
                };
                host.observe(Event::DisconnectIgnored {
                    network: request.network,
                });
                self.table.record_disconnect_removal_ignored();
                Received::Handled
            }
            MessageType::WHAT_IS_NETWORK_NUMBER => {
                // Never routed, and with no data (Clause 6.4.19).
                if npdu.source.is_some() || npdu.destination.is_some() || !npdu.payload.is_empty() {
                    return self.bad_message();
                }
                self.out.push(Task::number_is(at.port));
                Received::Handled
            }
            MessageType::NETWORK_NUMBER_IS => self.network_number_is(at, npdu, host),
            _ if kind.is_removed_security() => {
                host.observe(Event::SecurityMessage { message_type: kind });
                Received::Handled
            }
            _ => {
                // A reserved type, or a proprietary one without a DNET to
                // route it by (Clause 6.6.3.5).
                host.observe(Event::UnknownMessage { message_type: kind });
                self.reject(at, npdu.source, 0, RejectReason::UNKNOWN_MESSAGE_TYPE, host);
                Received::Handled
            }
        }
    }

    /// Count a network message whose data or addressing its type doesn't
    /// allow.
    pub(super) fn bad_message<'f>(&mut self) -> Received<'f> {
        bump(&mut self.counters.bad_messages);
        Received::Discarded(Discard::BadMessage)
    }

    /// Ask the host whether to act on a protected message, counting a
    /// refusal.
    pub(super) fn authorize(
        &mut self,
        at: &Arrival<'_>,
        npdu: &Npdu<'_>,
        class: ControlClass,
        targets: Targets<'_>,
        host: &mut impl Host,
    ) -> bool {
        let claim = Claim {
            class,
            port: at.port,
            port_network: at.network,
            source: at.source,
            // Only a network message is protected, and it has a type.
            message_type: npdu.message_type.unwrap_or(MessageType::from_raw(u8::MAX)),
            vendor_id: npdu.vendor_id,
            snet: npdu.source.map(|source| source.network),
            dnet: npdu.destination.map(|destination| destination.network),
            targets,
            targets_omitted: class_omits(class) && targets.is_empty(),
        };
        let allowed = host.authorize(&claim);
        if !allowed {
            bump(&mut self.counters.denied);
        }
        allowed
    }

    /// Who-Is-Router (Clause 6.6.3.2). Unscoped, it is answered with every
    /// network the table reaches through another port, ascending, but those
    /// marked unreachable. Scoped to a network reached through another
    /// port, it is answered with that network, whatever its state. Scoped
    /// to one the table lacks, it is passed on to every other port with
    /// SNET and SADR, and no DNET. Scoped to one reached through the
    /// arrival port, it is left to the router there: the clause searches
    /// only for a network the table lacks. Network 0 is refused.
    fn who_is_router<'f>(
        &mut self,
        at: &Arrival<'f>,
        npdu: &Npdu<'f>,
        host: &mut impl Host,
    ) -> Received<'f> {
        let network = match WhoIsRouter::parse(npdu.payload) {
            Err(_) | Ok(WhoIsRouter::Network(0)) => return self.bad_message(),
            Ok(WhoIsRouter::All) => {
                self.out.push(Task::i_am_answer(at.port, None));
                return Received::Handled;
            }
            Ok(WhoIsRouter::Network(network)) => network,
        };
        match self.table.lookup(network) {
            Some(route) if route.port() != at.port => {
                self.out.push(Task::i_am_answer(at.port, Some(network)));
                return Received::Handled;
            }
            Some(_) => return Received::Handled,
            None => {}
        }
        let relay = Npdu {
            destination: None,
            source: npdu.source.or(Some(Address {
                network: at.network,
                mac: at.source,
            })),
            vendor_id: None,
            ..*npdu
        };
        let kind = Kind::Relay(MessageType::WHO_IS_ROUTER_TO_NETWORK);
        if self.write_fan_out(&relay, kind, at.port, host) {
            Received::Handled
        } else {
            Received::Discarded(Discard::Unwritable)
        }
    }

    /// Pass a network message's data on, verbatim, as a fresh message of
    /// its type to every port but the arrival one.
    fn rebroadcast<'f>(
        &mut self,
        at: &Arrival<'f>,
        npdu: &Npdu<'f>,
        kind: MessageType,
        host: &mut impl Host,
    ) -> Received<'f> {
        let relay = Npdu::message(kind, npdu.payload);
        if self.write_fan_out(&relay, Kind::Relay(kind), at.port, host) {
            Received::Handled
        } else {
            Received::Discarded(Discard::Unwritable)
        }
    }

    /// I-Am-Router (Clauses 6.6.3.2, 6.6.3.3): a non-empty list, each
    /// network learned through the arrival port and its sender in order,
    /// then the message passed on to every other port.
    fn i_am_router<'f>(
        &mut self,
        at: &Arrival<'f>,
        npdu: &Npdu<'f>,
        options: &Options,
        host: &mut impl Host,
    ) -> Received<'f> {
        let Some(list) = Networks::parse(npdu.payload)
            .ok()
            .filter(|list| !list.is_empty())
        else {
            return self.bad_message();
        };
        if !self.authorize(at, npdu, ControlClass::IAm, Targets::Networks(list), host) {
            return Received::Discarded(Discard::Denied);
        }
        let mut claimed = Claimed::new();
        for network in list.iter() {
            if !self.claim(at, network, &mut claimed, ControlClass::IAm, options, host) {
                break;
            }
        }
        self.rebroadcast(at, npdu, MessageType::I_AM_ROUTER_TO_NETWORK, host)
    }

    /// One learning claim of an I-Am-Router or an Init-ACK. `false` when the
    /// route cap ends the message's learning. At the cap, an I-Am-Router
    /// still refreshes networks the table holds; an Init-ACK learns nothing
    /// more, as today (control_messages.rs:247 against :762).
    ///
    /// A network listed again in one message is no second claim: once a
    /// claim in it was for a route on another port, a repeat that still
    /// finds the route there (hardened convergence held the first) is
    /// skipped, so one message never corroborates itself
    /// (control_messages.rs:253-261).
    pub(super) fn claim(
        &mut self,
        at: &Arrival<'_>,
        network: u16,
        claimed: &mut Claimed<R>,
        class: ControlClass,
        options: &Options,
        host: &mut impl Host,
    ) -> bool {
        let route = self.table.lookup(network);
        let refresh_at_cap = class == ControlClass::IAm && route.is_some();
        if self.table.len() >= usize::from(options.route_cap) && !refresh_at_cap {
            self.table.record_learning_cap();
            host.observe(Event::RouteCap { class });
            return false;
        }
        let moving = route.is_some_and(|route| !route.is_direct() && route.port() != at.port);
        if moving && !claimed.insert(network) {
            return true;
        }
        let learned =
            self.table
                .apply_claim(network, at.port, at.source_mac, at.now, &options.timing);
        if let Learned::Moved { flaps, .. } = learned {
            if learned.flapping() {
                host.observe(Event::Flapping { network, flaps });
            }
        }
        if learned.applied() {
            self.table.record_learned();
            host.observe(Event::Learned {
                network,
                port: at.port,
                class,
            });
        }
        true
    }

    /// I-Could-Be-Router (Clause 6.4.3): stored as a learned route only
    /// where the table has none, and only while the table is under the
    /// route cap. One the cap turns away is counted as learning's are
    /// (#1679: the hosted router once stored these past its cap).
    fn i_could_be_router<'f>(
        &mut self,
        at: &Arrival<'f>,
        npdu: &Npdu<'f>,
        options: &Options,
        host: &mut impl Host,
    ) -> Received<'f> {
        let Ok(offer) = ICouldBeRouter::parse(npdu.payload) else {
            return self.bad_message();
        };
        let named = Networks::parse(&npdu.payload[..2]).unwrap_or(Networks::EMPTY);
        if !self.authorize(
            at,
            npdu,
            ControlClass::ICouldBe,
            Targets::Networks(named),
            host,
        ) {
            return Received::Discarded(Discard::Denied);
        }
        let network = offer.network;
        let new = network != 0 && network != GLOBAL_NETWORK && self.table.lookup(network).is_none();
        let at_cap = self.table.len() >= usize::from(options.route_cap);
        if new && at_cap {
            self.table.record_learning_cap();
            host.observe(Event::RouteCap {
                class: ControlClass::ICouldBe,
            });
        }
        let stored = new
            && !at_cap
            && self
                .table
                .add_learned(network, at.port, at.source_mac, at.now);
        host.observe(Event::ICouldBe {
            network: offer.network,
            performance_index: offer.performance_index,
            stored,
        });
        Received::Handled
    }

    /// A received reject (Clause 6.6.3.5): applied to the table, then for
    /// the router itself or passed on, whatever its reason. A reason other
    /// than 1 or 2 changes no route.
    fn received_reject<'f>(
        &mut self,
        at: &Arrival<'f>,
        npdu: &Npdu<'f>,
        options: &Options,
        host: &mut impl Host,
    ) -> Received<'f> {
        let Ok(reject) = Reject::parse(npdu.payload) else {
            return self.bad_message();
        };
        let named = Networks::parse(&npdu.payload[1..]).unwrap_or(Networks::EMPTY);
        if !self.authorize(
            at,
            npdu,
            ControlClass::Reject,
            Targets::Networks(named),
            host,
        ) {
            return Received::Discarded(Discard::Denied);
        }
        let effect = self.table.apply_reject(
            reject.network,
            at.port,
            reject.reason,
            at.now,
            &options.timing,
        );
        host.observe(Event::RejectReceived {
            network: reject.network,
            reason: reject.reason,
            effect,
        });
        self.pass_on_reject(at, npdu, host)
    }

    /// Router-Busy or Router-Available (Clauses 6.6.3.6, 6.6.3.7). Its
    /// scope is the networks the sender serves: learned routes through the
    /// arrival port whose next hop is the sender. A list narrows that
    /// scope; no list means all of it. Busy marks last `busy_for` and never
    /// revive an Unreachable route; Available lifts both marks. Direct
    /// routes are never touched. The message is passed on either way.
    fn flow_control<'f>(
        &mut self,
        at: &Arrival<'f>,
        npdu: &Npdu<'f>,
        busy: bool,
        options: &Options,
        host: &mut impl Host,
    ) -> Received<'f> {
        let Ok(list) = Networks::parse(npdu.payload) else {
            return self.bad_message();
        };
        let class = if busy {
            ControlClass::Busy
        } else {
            ControlClass::Available
        };
        if !self.authorize(at, npdu, class, Targets::Networks(list), host) {
            return Received::Discarded(Discard::Denied);
        }
        let deadline = at.now + options.timing.busy_for;
        let mut mark = |table: &mut crate::table::Table<R, H>, network| {
            if busy {
                table.mark_busy(network, deadline);
            } else {
                table.mark_available(network);
            }
            host.observe(Event::Marked { network, busy });
        };
        if list.is_empty() {
            // Every network served via the sender, ascending. Marks change
            // neither the set nor its order, so each pass resumes after the
            // last network marked.
            let mut from = 0;
            loop {
                let next = self
                    .table
                    .routes_served_via_peer(at.port, at.source)
                    .find(|&network| u32::from(network) >= from);
                let Some(network) = next else {
                    break;
                };
                mark(&mut self.table, network);
                from = u32::from(network) + 1;
            }
        } else {
            for network in list.iter() {
                if self.table.is_served_via_peer(network, at.port, at.source) {
                    mark(&mut self.table, network);
                }
            }
        }
        let kind = if busy {
            MessageType::ROUTER_BUSY_TO_NETWORK
        } else {
            MessageType::ROUTER_AVAILABLE_TO_NETWORK
        };
        self.rebroadcast(at, npdu, kind, host)
    }

    /// Network-Number-Is (Clause 6.4.20): heard only as a link broadcast
    /// with no addresses; a number other than the port's is reported.
    fn network_number_is<'f>(
        &mut self,
        at: &Arrival<'f>,
        npdu: &Npdu<'f>,
        host: &mut impl Host,
    ) -> Received<'f> {
        if npdu.source.is_some() || npdu.destination.is_some() || !at.link_group {
            return self.bad_message();
        }
        let Ok(number) = NetworkNumberIs::parse(npdu.payload) else {
            return self.bad_message();
        };
        if number.network != at.network {
            host.observe(Event::NumberConflict {
                port: at.port,
                local: at.network,
                reported: number.network,
                configured: number.is_configured(),
            });
        }
        Received::Handled
    }
}

/// The networks one message has claimed for a route on another port, which
/// the hosted router keeps in a set: a repeat of one is skipped. Only a
/// network the table holds gets in, so it never holds more than `R`, and
/// it stays sorted, so a list of thousands costs a binary search per entry
/// and at most `R` insertions. It lives on the stack for one message: 2 ×
/// `R` octets.
pub(super) struct Claimed<const R: usize> {
    networks: [u16; R],
    len: usize,
}

impl<const R: usize> Claimed<R> {
    /// None yet.
    pub(super) const fn new() -> Self {
        Self {
            networks: [0; R],
            len: 0,
        }
    }

    /// Record `network`, and say whether it is new to this message.
    fn insert(&mut self, network: u16) -> bool {
        match self.networks[..self.len].binary_search(&network) {
            Ok(_) => false,
            Err(at) => {
                // Full can't happen: the table holds at most R networks.
                if self.len < R {
                    // Swapped back into place from the end, with no
                    // memmove (`swap_apart`).
                    self.networks[self.len] = network;
                    for index in (at..self.len).rev() {
                        swap_apart(&mut self.networks, index, index + 1);
                    }
                    self.len += 1;
                }
                true
            }
        }
    }
}

/// Whether a message of `class` with no networks covers every network its
/// sender serves.
fn class_omits(class: ControlClass) -> bool {
    matches!(class, ControlClass::Busy | ControlClass::Available)
}
