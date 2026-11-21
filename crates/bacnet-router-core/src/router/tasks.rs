//! Outputs that wait in the queue, and how each is written when polled.

use super::Storage;
use super::config::{PortConfig, isolated_pair};
use super::counters::bump;
use super::output::{Cause, Current, Kind, Target};
use crate::discovery::{Gap, Omitted};
use crate::message::{NetworkNumberIs, Networks, RoutingEntries};
use crate::npdu::{Address, NetworkPriority, Npdu};
use crate::npdu::{NetworkMessageType, RejectMessageReason, RejectMessageToNetwork};
use crate::table::{Reachability, Table};
use crate::{Mac, PortId};

/// What a queued task writes. Zero is an empty slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TaskKind {
    Empty,
    /// A Who-Is-Router for `network` on the eligible ports still needing a search.
    Solicit,
    /// A reject with `reason` naming `network`, unicast to `to` on `port`,
    /// with `origin` as its DNET and DADR when `origin_network` isn't 0.
    Reject,
    /// An I-Am-Router on `port`: for `network`, or for 0 every network the
    /// table reaches through another port but those marked unreachable.
    IAmAnswer,
    /// An Initialize-Routing-Table-Ack with no data, to `to` on `port`.
    UpdateAck,
    /// The next Initialize-Routing-Table-Ack of a table answer, to `to` on
    /// `port`, from network `cursor` on, with `origin` as its DNET and DADR
    /// when `origin_network` isn't 0, and the router's address on port
    /// `named - 1` as its SNET and SADR when `named` isn't 0.
    TableAck,
    /// A Network-Number-Is answering on `port`.
    NumberIs,
    /// The startup announcements, from `cursor` = port index × 2 + step.
    Announce,
}

/// A queued output: all zero bits when empty.
///
/// It stays as small as it was before routed table answers (#2699): a
/// routed answer's source is always one of the router's own port
/// addresses, so the task keeps that port's index and the writer rebuilds
/// the address from the port's configuration. In every router's storage,
/// firmware included, that saves an 18-octet MAC per queued task.
#[derive(Debug, Clone, Copy)]
pub(super) struct Task {
    pub(super) kind: TaskKind,
    port: PortId,
    network: u16,
    reason: RejectMessageReason,
    to: Mac,
    origin_network: u16,
    origin: Mac,
    /// The port, plus one, whose address a routed query named; 0 for none.
    named: u8,
    search_gap: Omitted,
    pub(super) cursor: u32,
}

/// `octets` as a [`Mac`]: a parsed or configured address is never over the
/// bound.
fn mac(octets: &[u8]) -> Mac {
    Mac::from_slice(octets).unwrap_or(Mac::EMPTY)
}

impl Task {
    pub(super) const EMPTY: Self = Self {
        kind: TaskKind::Empty,
        port: PortId(0),
        network: 0,
        reason: RejectMessageReason::OTHER,
        to: Mac::EMPTY,
        origin_network: 0,
        origin: Mac::EMPTY,
        named: 0,
        search_gap: Omitted::NONE,
        cursor: 0,
    };

    const fn of(kind: TaskKind, port: PortId) -> Self {
        let mut task = Self::EMPTY;
        task.kind = kind;
        task.port = port;
        task
    }

    /// `origin` as the task's destination fields: network 0 for none.
    fn with_origin(mut self, origin: Option<Address<'_>>) -> Self {
        if let Some(origin) = origin {
            self.origin_network = origin.network;
            self.origin = mac(origin.mac);
        }
        self
    }

    /// The destination a reject or table answer carries, if any.
    fn origin(&self) -> Option<Address<'_>> {
        (self.origin_network != 0).then(|| Address {
            network: self.origin_network,
            mac: self.origin.as_slice(),
        })
    }

    /// Search from `except`, restricted to a previous search's gap when it has one.
    pub(super) fn solicit(except: PortId, network: u16, search_gap: Omitted) -> Self {
        Self {
            network,
            search_gap,
            ..Self::of(TaskKind::Solicit, except)
        }
    }

    /// A reject with `reason` naming `network`, to `to` on `port`, carrying
    /// `origin` as its destination.
    pub(super) fn reject(
        port: PortId,
        to: &[u8],
        origin: Option<Address<'_>>,
        network: u16,
        reason: RejectMessageReason,
    ) -> Self {
        Self {
            network,
            reason,
            to: mac(to),
            ..Self::of(TaskKind::Reject, port)
        }
        .with_origin(origin)
    }

    /// An I-Am-Router on `port` for `network`, or for every network beyond
    /// another port.
    pub(super) fn i_am_answer(port: PortId, network: Option<u16>) -> Self {
        Self {
            network: network.unwrap_or(0),
            ..Self::of(TaskKind::IAmAnswer, port)
        }
    }

    /// The data-less Ack to an update, to `to` on `port`.
    pub(super) fn update_ack(port: PortId, to: Mac) -> Self {
        Self {
            to,
            ..Self::of(TaskKind::UpdateAck, port)
        }
    }

    /// The whole table, in Acks to `to` on `port`: to `origin` beyond it
    /// when the query came from another network, and from the router's
    /// address on port `named` when the query named that address on a port
    /// other than `port`.
    pub(super) fn table_ack(
        port: PortId,
        to: &[u8],
        origin: Option<Address<'_>>,
        named: Option<PortId>,
    ) -> Self {
        Self {
            to: mac(to),
            // At most 255 ports, so index 254 plus one fits.
            named: named.map_or(0, |port| {
                debug_assert!(port.0 < u8::MAX, "Router::new caps ports at 255");
                port.0.saturating_add(1)
            }),
            ..Self::of(TaskKind::TableAck, port)
        }
        .with_origin(origin)
    }

    /// A Network-Number-Is on `port`.
    pub(super) fn number_is(port: PortId) -> Self {
        Self::of(TaskKind::NumberIs, port)
    }

    /// The startup announcements.
    pub(super) fn announce() -> Self {
        Self::of(TaskKind::Announce, PortId(0))
    }
}

/// Write a network message of type `kind` with no addresses into `scratch`:
/// the three-octet header, then the data `data` writes behind it.
fn write_message(
    scratch: &mut [u8],
    kind: NetworkMessageType,
    data: impl FnOnce(&mut [u8]) -> Option<usize>,
) -> Option<usize> {
    write_with_header(scratch, Npdu::message(kind, &[]), data)
}

fn write_with_header(
    scratch: &mut [u8],
    header: Npdu<'_>,
    data: impl FnOnce(&mut [u8]) -> Option<usize>,
) -> Option<usize> {
    let at = header.encode_into(scratch).ok()?;
    Some(at + data(&mut scratch[at..])?)
}

/// The routes from network `from` on that have a wire Port ID, ascending,
/// as (network, Port ID).
fn wire_entries<'a, G, const R: usize, const H: usize>(
    table: &'a Table<R, H>,
    ports: &'a [PortConfig<G>],
    requester: PortId,
    from: u32,
) -> impl Iterator<Item = (u16, u8)> + Clone + 'a {
    table
        .routes()
        .filter(move |(network, _)| u32::from(*network) >= from)
        .filter(move |(_, route)| !isolated_pair(ports, requester, route.port()))
        .filter_map(|(network, route)| Some((network, route.port().wire_id()?)))
}

/// Every other port's network, in port order.
fn other_networks<G>(
    ports: &[PortConfig<G>],
    port: PortId,
) -> impl Iterator<Item = u16> + Clone + '_ {
    ports
        .iter()
        .enumerate()
        .filter(move |(index, _)| {
            *index != usize::from(port.0) && !isolated_pair(ports, port, PortId(*index as u8))
        })
        .map(|(_, config)| config.network)
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
    /// Write the front task's next output into the scratch as the current
    /// one. `false` when no task is left.
    pub(super) fn write_task(&mut self) -> bool {
        let Some(task) = self.out.front() else {
            return self.write_announcement();
        };
        let mut next = None;
        // The output's length (`None`: the writer refused it), kind, cause
        // and target, or `None` when the task has nothing to send.
        let output = match task.kind {
            TaskKind::Empty => None,
            TaskKind::Solicit => {
                let data = task.network.to_be_bytes();
                let npdu = Npdu::message(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK, &data);
                let len = npdu.encode_into(&mut self.scratch).ok();
                let target = match task.search_gap.gap {
                    Gap::None => Target::AllBut(task.port),
                    Gap::IsolatedPorts => Target::IsolatedBut(task.port),
                    Gap::OrdinaryPort => Target::Broadcast(task.search_gap.port),
                };
                Some((len, Kind::Solicit, Cause::Ingress, target))
            }
            TaskKind::Reject => {
                let data = RejectMessageToNetwork {
                    reason: task.reason,
                    network: task.network,
                }
                .to_bytes();
                let mut npdu = Npdu::message(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK, &data);
                npdu.destination = task.origin();
                let len = npdu.encode_into(&mut self.scratch).ok();
                let target = Target::Unicast(task.port, task.to.as_slice());
                Some((len, Kind::Reject(task.reason), Cause::Ingress, target))
            }
            TaskKind::IAmAnswer => {
                let ports = &self.ports;
                let mut others = self
                    .table
                    .routes()
                    .filter(move |(_, route)| {
                        route.port() != task.port
                            && !isolated_pair(ports, task.port, route.port())
                            && route.reachability() != Reachability::Unreachable
                    })
                    .map(|(network, _)| network)
                    .peekable();
                let kind = Kind::Answer(NetworkMessageType::I_AM_ROUTER_TO_NETWORK);
                let target = Target::Broadcast(task.port);
                let message = NetworkMessageType::I_AM_ROUTER_TO_NETWORK;
                if task.network != 0 {
                    let len = write_message(&mut self.scratch, message, |data| {
                        Networks::write([task.network], data).ok()
                    });
                    Some((len, kind, Cause::Ingress, target))
                } else if others.peek().is_some() {
                    let len = write_message(&mut self.scratch, message, |data| {
                        Networks::write(others, data).ok()
                    });
                    Some((len, kind, Cause::Ingress, target))
                } else {
                    // Nothing to list: the hosted router sends nothing.
                    None
                }
            }
            TaskKind::UpdateAck => {
                let npdu = Npdu::message(NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK, &[]);
                let len = npdu.encode_into(&mut self.scratch).ok();
                let kind = Kind::Answer(NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK);
                let target = Target::Unicast(task.port, task.to.as_slice());
                Some((len, kind, Cause::Ingress, target))
            }
            TaskKind::TableAck => {
                let message = NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK;
                let named = task
                    .named
                    .checked_sub(1)
                    .and_then(|index| self.ports[..self.port_count].get(usize::from(index)));
                let header = Npdu {
                    source: named.map(|port| Address {
                        network: port.network,
                        mac: port.mac.as_slice(),
                    }),
                    destination: task.origin(),
                    ..Npdu::message(message, &[])
                };
                let port = task.port;
                let link_limit = self.ports[usize::from(port.0)].max_npdu.get(false);
                let budget = link_limit.min(self.scratch.len()).min(501);
                // A relay may retain the destination and add its ingress source.
                // Reserve that header on the 501-octet path even on a larger link.
                let relayed = if header.destination.is_some() && header.source.is_none() {
                    Npdu {
                        source: Some(Address {
                            network: 1,
                            mac: &[0; crate::MAX_MAC_LEN],
                        }),
                        ..header
                    }
                    .encoded_len()
                } else {
                    header.encoded_len()
                };
                let room = budget
                    .saturating_sub(header.encoded_len() + 1)
                    .min(501usize.saturating_sub(relayed + 1));
                let per_ack = room / RoutingEntries::ENTRY_LEN;
                let entries = wire_entries(&self.table, &self.ports, port, task.cursor);
                let mut remaining = entries.clone();
                let last = remaining.by_ref().take(per_ack).last();
                let more = remaining.next().is_some();
                let len = if more && last.is_none() {
                    None
                } else {
                    write_with_header(&mut self.scratch[..budget], header, |data| {
                        RoutingEntries::write(entries.take(per_ack), data).ok()
                    })
                };
                if more && len.is_some() {
                    next = last.map(|(network, _)| u32::from(network) + 1);
                }
                Some((
                    len,
                    Kind::Answer(message),
                    Cause::Ingress,
                    Target::Unicast(port, task.to.as_slice()),
                ))
            }
            TaskKind::NumberIs => {
                let len = self.write_number_is(task.port);
                let kind = Kind::Answer(NetworkMessageType::NETWORK_NUMBER_IS);
                Some((len, kind, Cause::Ingress, Target::Broadcast(task.port)))
            }
            TaskKind::Announce => {
                self.next_announcement(task.cursor).map(|(at, len)| {
                    next = Some(at + 1).filter(|&cursor| cursor < self.announce_end());
                    let kind = if at & 1 == 0 {
                        Kind::Announce(NetworkMessageType::NETWORK_NUMBER_IS)
                    } else {
                        Kind::Announce(NetworkMessageType::I_AM_ROUTER_TO_NETWORK)
                    };
                    // At most 255 ports, so the index fits in a u8.
                    let port = PortId((at >> 1) as u8);
                    (len, kind, Cause::Local, Target::Broadcast(port))
                })
            }
        };
        if let Some((len, kind, cause, target)) = output {
            match len {
                Some(len) => {
                    let mut current =
                        Current::new(len, kind, cause, NetworkPriority::Normal, target);
                    // Only this exact task remains at the queue front until
                    // the next poll or refusal. A completed answer has no continuation.
                    current.table_ack = task.kind == TaskKind::TableAck && next.is_some();
                    self.out.current = current;
                }
                None => bump(&mut self.counters.write_failures),
            }
        }
        match next {
            Some(cursor) => self.out.advance(cursor),
            None => self.out.pop(),
        }
        true
    }

    /// Write a Network-Number-Is for `port`'s configured network.
    fn write_number_is(&mut self, port: PortId) -> Option<usize> {
        let network = self.ports.get(usize::from(port.0))?.network;
        let data = NetworkNumberIs {
            network,
            flag: NetworkNumberIs::CONFIGURED,
        }
        .to_bytes();
        Npdu::message(NetworkMessageType::NETWORK_NUMBER_IS, &data)
            .encode_into(&mut self.scratch)
            .ok()
    }

    /// One past the last announcement cursor.
    fn announce_end(&self) -> u32 {
        2 * self.port_count as u32
    }

    /// Write the next startup announcement from `cursor` (port index × 2 +
    /// step): a Network-Number-Is with the port's number (Clause 6.4.20);
    /// then an I-Am-Router listing every other port's network, in
    /// port order, when there is another port. Returns its cursor and its
    /// length (`None`: the writer refused it), or `None` when none is left.
    fn next_announcement(&mut self, cursor: u32) -> Option<(u32, Option<usize>)> {
        for at in cursor..self.announce_end() {
            let port = PortId((at >> 1) as u8);
            if at & 1 == 0 {
                return Some((at, self.write_number_is(port)));
            }
            if self.port_count > 1 {
                let mut others = other_networks(&self.ports[..self.port_count], port).peekable();
                if others.peek().is_none() {
                    continue;
                }
                let len = write_message(
                    &mut self.scratch,
                    NetworkMessageType::I_AM_ROUTER_TO_NETWORK,
                    |data| Networks::write(others, data).ok(),
                );
                return Some((at, len));
            }
        }
        None
    }
}

#[cfg(test)]
#[path = "table_reply_tests.rs"]
mod table_reply_tests;

/// A queued task is as small as before routed table answers: 52 octets,
/// where a task holding a whole reply plan (three MACs) took 76. Every
/// router's storage holds four (#2699).
#[cfg(test)]
#[test]
fn a_task_keeps_one_mac_per_address_it_cannot_rebuild() {
    assert!(
        core::mem::size_of::<Task>() <= 52,
        "{}",
        core::mem::size_of::<Task>()
    );
}
