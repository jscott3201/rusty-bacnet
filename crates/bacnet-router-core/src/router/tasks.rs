//! Outputs that wait in the queue, and how each is written when polled.

use super::config::PortConfig;
use super::counters::bump;
use super::output::{Cause, Current, Kind, Target};
use super::Storage;
use crate::message::{
    MessageType, NetworkNumberIs, Networks, Reject, RejectReason, RoutingEntries,
};
use crate::npdu::{Address, Npdu, GLOBAL_NETWORK};
use crate::table::{Reachability, Table};
use crate::{Mac, PortId};

/// What a queued task writes. Zero is an empty slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TaskKind {
    Empty,
    /// A Who-Is-Router for `network` on every port but `port`.
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
    /// `port`, from network `cursor` on.
    TableAck,
    /// A Network-Number-Is answering on `port`.
    NumberIs,
    /// The startup announcements, from `cursor` = port index × 2 + step.
    Announce,
}

/// A queued output: all zero bits when empty.
#[derive(Debug, Clone, Copy)]
pub(super) struct Task {
    pub(super) kind: TaskKind,
    port: PortId,
    network: u16,
    reason: RejectReason,
    to: Mac,
    origin_network: u16,
    origin: Mac,
    pub(super) cursor: u32,
}

impl Task {
    pub(super) const EMPTY: Self = Self {
        kind: TaskKind::Empty,
        port: PortId(0),
        network: 0,
        reason: RejectReason::OTHER,
        to: Mac::EMPTY,
        origin_network: 0,
        origin: Mac::EMPTY,
        cursor: 0,
    };

    const fn of(kind: TaskKind, port: PortId) -> Self {
        let mut task = Self::EMPTY;
        task.kind = kind;
        task.port = port;
        task
    }

    /// A Who-Is-Router for `network` on every port but `except`.
    pub(super) fn solicit(except: PortId, network: u16) -> Self {
        Self {
            network,
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
        reason: RejectReason,
    ) -> Self {
        // A parsed address is never over the bound.
        let mac = |octets: &[u8]| Mac::from_slice(octets).unwrap_or(Mac::EMPTY);
        Self {
            network,
            reason,
            to: mac(to),
            origin_network: origin.map_or(0, |origin| origin.network),
            origin: origin.map_or(Mac::EMPTY, |origin| mac(origin.mac)),
            ..Self::of(TaskKind::Reject, port)
        }
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

    /// The whole table, in Acks to `to` on `port`.
    pub(super) fn table_ack(port: PortId, to: Mac) -> Self {
        Self {
            to,
            ..Self::of(TaskKind::TableAck, port)
        }
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

/// Entries per Ack of a table answer: as many as fit an NPDU of 501
/// octets, the most a standard MS/TP frame carries (Clause 9.3), so every
/// link can take the answer. That is 124.
const ACK_ENTRIES: usize = (501 - 3 - 1) / RoutingEntries::ENTRY_LEN;

/// Write a network message of type `kind` with no addresses into `scratch`:
/// the three-octet header, then the data `data` writes behind it.
fn write_message(
    scratch: &mut [u8],
    kind: MessageType,
    data: impl FnOnce(&mut [u8]) -> Option<usize>,
) -> Option<usize> {
    let header = Npdu::message(kind, &[]);
    let at = header.encoded_len();
    let (head, rest) = scratch.split_at_mut_checked(at)?;
    let len = data(rest)?;
    header.write(head).ok()?;
    Some(at + len)
}

/// The routes from network `from` on that have a wire Port ID, ascending,
/// as (network, Port ID).
fn wire_entries<const R: usize, const H: usize>(
    table: &Table<R, H>,
    from: u32,
) -> impl Iterator<Item = (u16, u8)> + Clone + '_ {
    table
        .routes()
        .filter(move |(network, _)| u32::from(*network) >= from)
        .filter_map(|(network, route)| Some((network, route.port().wire_id()?)))
}

/// Every other port's network, in port order.
fn other_networks(ports: &[PortConfig], port: PortId) -> impl Iterator<Item = u16> + Clone + '_ {
    ports
        .iter()
        .enumerate()
        .filter(move |(index, _)| *index != usize::from(port.0))
        .map(|(_, config)| config.network)
}

impl<const P: usize, const R: usize, const H: usize, const D: usize, const N: usize>
    Storage<P, R, H, D, N>
{
    /// Write the front task's next output into the scratch as the current
    /// one. `false` when no task is left.
    pub(super) fn write_task(&mut self) -> bool {
        let Some(task) = self.out.front() else {
            return false;
        };
        let mut next = None;
        // The output's length (`None`: the writer refused it), kind, cause
        // and target, or `None` when the task has nothing to send.
        let output = match task.kind {
            TaskKind::Empty => None,
            TaskKind::Solicit => {
                let data = task.network.to_be_bytes();
                let npdu = Npdu::message(MessageType::WHO_IS_ROUTER_TO_NETWORK, &data);
                let len = npdu.write(&mut self.scratch).ok();
                Some((
                    len,
                    Kind::Solicit,
                    Cause::Ingress,
                    Target::AllBut(task.port),
                ))
            }
            TaskKind::Reject => {
                let data = Reject {
                    reason: task.reason,
                    network: task.network,
                }
                .to_bytes();
                let mut npdu = Npdu::message(MessageType::REJECT_MESSAGE_TO_NETWORK, &data);
                if task.origin_network != 0 {
                    npdu.destination = Some(Address {
                        network: task.origin_network,
                        mac: task.origin.as_slice(),
                    });
                }
                let len = npdu.write(&mut self.scratch).ok();
                let target = Target::Unicast(task.port, task.to.as_slice());
                Some((len, Kind::Reject(task.reason), Cause::Ingress, target))
            }
            TaskKind::IAmAnswer => {
                let mut others = self
                    .table
                    .routes()
                    .filter(move |(_, route)| {
                        route.port() != task.port
                            && route.reachability() != Reachability::Unreachable
                    })
                    .map(|(network, _)| network)
                    .peekable();
                let kind = Kind::Answer(MessageType::I_AM_ROUTER_TO_NETWORK);
                let target = Target::Broadcast(task.port);
                let message = MessageType::I_AM_ROUTER_TO_NETWORK;
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
                let npdu = Npdu::message(MessageType::INITIALIZE_ROUTING_TABLE_ACK, &[]);
                let len = npdu.write(&mut self.scratch).ok();
                let kind = Kind::Answer(MessageType::INITIALIZE_ROUTING_TABLE_ACK);
                let target = Target::Unicast(task.port, task.to.as_slice());
                Some((len, kind, Cause::Ingress, target))
            }
            TaskKind::TableAck => {
                let entries = wire_entries(&self.table, task.cursor).take(ACK_ENTRIES);
                let last = entries.clone().last().map(|(network, _)| network);
                // The first Ack goes even for an empty table, with a count
                // of 0; later ones only while routes remain.
                if task.cursor != 0 && last.is_none() {
                    None
                } else {
                    let message = MessageType::INITIALIZE_ROUTING_TABLE_ACK;
                    let len = write_message(&mut self.scratch, message, |data| {
                        RoutingEntries::write(entries, data).ok()
                    });
                    // A write that fails ends the answer, as an encode
                    // failure does in the hosted router.
                    next = last
                        .filter(|_| len.is_some())
                        .map(|network| u32::from(network) + 1);
                    let kind = Kind::Answer(message);
                    let target = Target::Unicast(task.port, task.to.as_slice());
                    Some((len, kind, Cause::Ingress, target))
                }
            }
            TaskKind::NumberIs => {
                let len = self.write_number_is(task.port);
                let kind = Kind::Answer(MessageType::NETWORK_NUMBER_IS);
                Some((len, kind, Cause::Ingress, Target::Broadcast(task.port)))
            }
            TaskKind::Announce => {
                self.next_announcement(task.cursor).map(|(at, len)| {
                    next = Some(at + 1).filter(|&cursor| cursor < self.announce_end());
                    let kind = if at & 1 == 0 {
                        Kind::Announce(MessageType::NETWORK_NUMBER_IS)
                    } else {
                        Kind::Announce(MessageType::I_AM_ROUTER_TO_NETWORK)
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
                    let mut current = Current::new(len, kind, cause, 0, target);
                    current.table_ack = task.kind == TaskKind::TableAck;
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
        Npdu::message(MessageType::NETWORK_NUMBER_IS, &data)
            .write(&mut self.scratch)
            .ok()
    }

    /// One past the last announcement cursor.
    fn announce_end(&self) -> u32 {
        2 * self.port_count as u32
    }

    /// Write the next startup announcement from `cursor` (port index × 2 +
    /// step): a Network-Number-Is for a port with a usable number (Clause
    /// 6.4.20); then an I-Am-Router listing every other port's network, in
    /// port order, when there is another port. Returns its cursor and its
    /// length (`None`: the writer refused it), or `None` when none is left.
    fn next_announcement(&mut self, cursor: u32) -> Option<(u32, Option<usize>)> {
        for at in cursor..self.announce_end() {
            let port = PortId((at >> 1) as u8);
            if at & 1 == 0 {
                let network = self.ports[usize::from(port.0)].network;
                if network != 0 && network != GLOBAL_NETWORK {
                    return Some((at, self.write_number_is(port)));
                }
            } else if self.port_count > 1 {
                let others = other_networks(&self.ports[..self.port_count], port);
                let len = write_message(
                    &mut self.scratch,
                    MessageType::I_AM_ROUTER_TO_NETWORK,
                    |data| Networks::write(others, data).ok(),
                );
                return Some((at, len));
            }
        }
        None
    }
}
