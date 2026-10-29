//! What the router asks its caller to send.
//!
//! The scratch holds one NPDU at a time, the [`Current`] output, which goes
//! to one port or fans out to every port but one. An output that needs the
//! arriving frame (a relay or a rewrite) is written during the call that
//! received it, before the frame is let go. Everything else waits as a
//! small [`Task`] and is written into the scratch when
//! [`poll`](Storage::poll) reaches it, from the table as it is then. A call
//! queues at most two tasks, and a frame's own output always comes before
//! them, so the queue never fills.

use super::counters::bump;
use super::host::{Event, Host};
use super::tasks::{Task, TaskKind};
use super::{RejectOrigin, Storage};
use crate::npdu::{NetworkMessageType, RejectMessageReason};
use crate::npdu::{NetworkPriority, Npdu};
use crate::{Mac, PortId, swap_apart};

/// What to do next. Poll [`Router::poll_action`](super::Router::poll_action)
/// until [`Action::Idle`] after every call into the router.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "carry out each send, and poll until Idle"]
pub enum Action<'a> {
    /// Send an NPDU.
    Send(Send<'a>),
    /// Nothing more to do.
    Idle,
}

/// An NPDU for one port. If the port can't take it, say so with
/// [`Router::refused`](super::Router::refused) before the next poll.
///
/// It is `#[non_exhaustive]`, so a new field isn't a breaking change: a
/// pattern needs `..`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Send<'a> {
    /// The egress port.
    pub port: PortId,
    /// Where on that port's link it goes.
    pub to: Dest<'a>,
    /// The NPDU, in the router's scratch until the next poll.
    pub npdu: &'a [u8],
    /// The NPDU's network priority, for a port that queues by it.
    pub priority: NetworkPriority,
    /// Why the router sends it.
    pub kind: Kind,
    /// What prompted it.
    pub cause: Cause,
    /// Return-path record for an eligible routed send. Retain it with a
    /// queued send for [`Router::refused_later`](super::Router::refused_later).
    /// It is absent for fan-outs, forwarded rejects and local messages.
    pub reject_origin: Option<RejectOrigin>,
}

/// A link-layer destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dest<'a> {
    /// One node, by its MAC on the port.
    Unicast(&'a [u8]),
    /// Every node on the port.
    Broadcast,
}

/// Why the router sends an NPDU: what a host needs to log a refused send
/// the way it logs one today.
///
/// It is `#[non_exhaustive]`, so a new kind isn't a breaking change: a
/// `match` needs a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Kind {
    /// A message routed toward its DNET: an APDU, a network message
    /// directed elsewhere, or a reject passed on.
    Forward,
    /// A global broadcast passed on.
    Global,
    /// A network message of this type passed on: an I-Am-Router, a
    /// Router-Busy or Router-Available, or a Who-Is-Router searching
    /// further.
    Relay(NetworkMessageType),
    /// A reject this router sends, with this reason.
    Reject(RejectMessageReason),
    /// An answer of this type: I-Am-Router, Initialize-Routing-Table-Ack
    /// or Network-Number-Is.
    Answer(NetworkMessageType),
    /// A startup announcement of this type.
    Announce(NetworkMessageType),
    /// A Who-Is-Router for a network the router has no route to.
    Solicit,
    /// The router's own Router-Busy or Router-Available, of this type, for
    /// a port that clogged or drained (#1697).
    Congestion(NetworkMessageType),
    /// A request from the router's own application
    /// ([`Router::originate`](super::Router::originate), #1699).
    Originate,
}

/// What prompted a send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    /// A received frame: the send carries that frame's link data
    /// attributes, as every output an arrival prompts does today.
    Ingress,
    /// The router itself (the startup announcements, its congestion
    /// messages and its application's requests): no attributes.
    Local,
}

/// Why a port didn't take a send.
///
/// It is `#[non_exhaustive]`, so a new reason isn't a breaking change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SendRefusal {
    /// Its queue is full.
    Full,
    /// The NPDU or its link envelope was refused as too long before sending.
    TooLong,
    /// It is down.
    PortDown,
    /// Anything else.
    Other,
}

/// [`Kind`] without its octet, so storage stays all zero bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tag {
    Forward,
    Global,
    Relay,
    Reject,
    Answer,
    Announce,
    Solicit,
    Congestion,
    Originate,
}

/// Where a written NPDU goes.
#[derive(Debug, Clone, Copy)]
pub(super) enum Target<'m> {
    /// Unicast to this MAC on the port, or a broadcast for an empty one.
    Toward(PortId, &'m [u8]),
    /// Unicast to this MAC on the port, even an empty one.
    Unicast(PortId, &'m [u8]),
    /// A broadcast on the port.
    Broadcast(PortId),
    /// A broadcast on every port but this one.
    AllBut(PortId),
}

/// The NPDU in the scratch and where it goes next.
#[derive(Debug, Clone, Copy)]
pub(super) struct Current {
    active: bool,
    len: usize,
    tag: Tag,
    /// The [`Kind`]'s octet, its message type or reject reason.
    code: u8,
    /// [`Cause::Local`]; zero is [`Cause::Ingress`].
    local: bool,
    priority: NetworkPriority,
    /// To every port but `port`, rather than to `port`.
    fan_out: bool,
    port: PortId,
    /// The next port a fan-out tries.
    next: u16,
    /// Where the NPDU starts in the scratch: 0 but for a tail.
    start: u16,
    /// A fan-out's trailing send to `port`, of the same NPDU without its
    /// SNET and SADR, which starts this far into the scratch; 0 for none.
    /// See [`Current::with_tail`].
    tail: u16,
    /// The tail's control octet: the NPDU's, without the SNET flag.
    tail_control: u8,
    /// A single-port send that is a broadcast rather than to `to`.
    broadcast: bool,
    to: Mac,
    /// Written for a table answer, which a refusal ends.
    pub(super) table_ack: bool,
    pub(super) reject_origin: RejectOrigin,
}

impl Current {
    const NONE: Self = Self {
        active: false,
        len: 0,
        tag: Tag::Forward,
        code: 0,
        local: false,
        priority: NetworkPriority::Normal,
        fan_out: false,
        port: PortId(0),
        next: 0,
        start: 0,
        tail: 0,
        tail_control: 0,
        broadcast: false,
        to: Mac::EMPTY,
        table_ack: false,
        reject_origin: RejectOrigin::NONE,
    };

    /// `len` octets in the scratch, going to `target`.
    pub(super) fn new(
        len: usize,
        kind: Kind,
        cause: Cause,
        priority: NetworkPriority,
        target: Target<'_>,
    ) -> Self {
        let (tag, code) = match kind {
            Kind::Forward => (Tag::Forward, 0),
            Kind::Global => (Tag::Global, 0),
            Kind::Relay(code) => (Tag::Relay, code.to_raw()),
            Kind::Reject(reason) => (Tag::Reject, reason.to_raw()),
            Kind::Answer(code) => (Tag::Answer, code.to_raw()),
            Kind::Announce(code) => (Tag::Announce, code.to_raw()),
            Kind::Solicit => (Tag::Solicit, 0),
            Kind::Congestion(code) => (Tag::Congestion, code.to_raw()),
            Kind::Originate => (Tag::Originate, 0),
        };
        let mut current = Self {
            active: true,
            len,
            tag,
            code,
            local: cause == Cause::Local,
            priority,
            ..Self::NONE
        };
        // Every MAC here came from a parsed frame, a port or a stored
        // route, all within the bound, so none is lost.
        let mac = |octets: &[u8]| Mac::from_slice(octets).unwrap_or(Mac::EMPTY);
        match target {
            Target::Toward(port, to) => {
                current.port = port;
                current.broadcast = to.is_empty();
                current.to = mac(to);
            }
            Target::Unicast(port, to) => {
                current.port = port;
                current.to = mac(to);
            }
            Target::Broadcast(port) => {
                current.port = port;
                current.broadcast = true;
            }
            Target::AllBut(port) => {
                current.port = port;
                current.fan_out = true;
            }
        }
        current
    }

    /// End a fan-out to every port but `port` with a send to `port` too,
    /// of the same global broadcast without its SNET and SADR: the router's
    /// own, from its address on that port (#1809). The scratch holds the
    /// NPDU with them, as DNET 0xFFFF, DLEN 0, SNET, SLEN, SADR, then the
    /// hop count and the rest (Clause 6.2, Figure 6-1). The one without
    /// them is the same from the hop count on, so once the fan-out is done
    /// its five octets of version, control, DNET and DLEN are written over
    /// the end of the source fields, just before the hop count, and the
    /// send starts there: `offset` octets in, the SNET, SLEN and SADR's
    /// length. Nothing is moved, so no memmove is linked.
    pub(super) fn with_tail(mut self, offset: u16, control: u8) -> Self {
        self.tail = offset;
        self.tail_control = control;
        self
    }

    /// Whether it has a send left on a router of `ports` ports.
    fn has_more(&self, ports: usize) -> bool {
        self.active
            && (!self.fan_out
                || self.tail != 0
                || (usize::from(self.next)..ports).any(|port| port != usize::from(self.port.0)))
    }

    fn kind(&self) -> Kind {
        match self.tag {
            Tag::Forward => Kind::Forward,
            Tag::Global => Kind::Global,
            Tag::Relay => Kind::Relay(NetworkMessageType::from_raw(self.code)),
            Tag::Reject => Kind::Reject(RejectMessageReason::from_raw(self.code)),
            Tag::Answer => Kind::Answer(NetworkMessageType::from_raw(self.code)),
            Tag::Announce => Kind::Announce(NetworkMessageType::from_raw(self.code)),
            Tag::Solicit => Kind::Solicit,
            Tag::Congestion => Kind::Congestion(NetworkMessageType::from_raw(self.code)),
            Tag::Originate => Kind::Originate,
        }
    }
}

/// The tasks a queue holds at most: twice what one call queues.
const QUEUE: usize = 4;

/// The current output and the queued tasks: all zero bits when empty.
#[derive(Debug, Clone, Copy)]
pub(super) struct Outputs {
    pub(super) current: Current,
    tasks: [Task; QUEUE],
    len: usize,
    /// The last send polled came from a table answer.
    last_table_ack: bool,
    /// The port of the last send polled.
    last_port: PortId,
}

impl Outputs {
    pub(super) const NEW: Self = Self {
        current: Current::NONE,
        tasks: [Task::EMPTY; QUEUE],
        len: 0,
        last_table_ack: false,
        last_port: PortId(0),
    };

    /// How many outputs wait on a router of `ports` ports: the current
    /// one, if it has sends left, and the tasks.
    pub(super) fn pending(&self, ports: usize) -> usize {
        usize::from(self.current.has_more(ports)) + self.len
    }

    /// Queue `task` behind the others.
    pub(super) fn push(&mut self, task: Task) {
        debug_assert!(self.len < QUEUE, "a call queues at most two tasks");
        if let Some(slot) = self.tasks.get_mut(self.len) {
            *slot = task;
            self.len += 1;
        }
    }

    /// The task at the front.
    pub(super) fn front(&self) -> Option<Task> {
        self.tasks[..self.len].first().copied()
    }

    /// Move the front task's cursor on.
    pub(super) fn advance(&mut self, cursor: u32) {
        if let Some(task) = self.tasks[..self.len].first_mut() {
            task.cursor = cursor;
        }
    }

    /// Drop the front task.
    pub(super) fn pop(&mut self) {
        if self.len > 0 {
            // The front slot moves to the back one swap at a time, with no
            // memmove (`swap_apart`).
            for at in 1..self.len {
                swap_apart(&mut self.tasks, at - 1, at);
            }
            self.len -= 1;
            self.tasks[self.len] = Task::EMPTY;
        }
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
    /// Drop whatever the caller left unpolled, counting each output, a
    /// waiting congestion announcement included.
    pub(super) fn drop_undrained(&mut self) {
        let announcements = self.flows[..self.port_count]
            .iter_mut()
            .fold(0, |count, flow| count + usize::from(flow.take_pending()));
        let pending = self.out.pending(self.port_count) + announcements;
        if pending > 0 {
            let pending = u32::try_from(pending).unwrap_or(u32::MAX);
            self.counters.undrained = self.counters.undrained.saturating_add(pending);
            self.out = Outputs::NEW;
        }
    }

    /// Write a frame's own output, `npdu`, into the scratch. A write the
    /// writer refuses is counted and sends nothing.
    pub(super) fn write_now(&mut self, npdu: &Npdu<'_>, kind: Kind, target: Target<'_>) -> bool {
        debug_assert_eq!(
            self.out.pending(self.port_count),
            0,
            "a frame's output comes first"
        );
        match npdu.encode_into(&mut self.scratch) {
            Ok(len) => {
                self.out.current = Current::new(len, kind, Cause::Ingress, npdu.priority, target);
                true
            }
            Err(_) => {
                bump(&mut self.counters.write_failures);
                false
            }
        }
    }

    /// [`write_now`](Self::write_now) a frame's own fan-out to every port
    /// but `except`. `host` hears of each port that will leave it out as too
    /// long for its broadcast limit; the skip itself, and its count, come
    /// when the send is polled.
    pub(super) fn write_fan_out(
        &mut self,
        npdu: &Npdu<'_>,
        kind: Kind,
        except: PortId,
        host: &mut impl Host,
    ) -> bool {
        if !self.write_now(npdu, kind, Target::AllBut(except)) {
            return false;
        }
        let octets = self.out.current.len;
        let ports = &self.ports[..self.port_count];
        for (index, config) in ports.iter().enumerate() {
            let limit = config.max_npdu.get(true);
            if index != usize::from(except.0) && octets > limit {
                host.observe(Event::TooLong {
                    // At most 255 ports, so the index fits in a u8.
                    port: PortId(index as u8),
                    octets,
                    limit,
                    rejected: None,
                });
            }
        }
        true
    }

    /// The next action: the current output's next send, or the next task's
    /// first, or [`Action::Idle`].
    pub(super) fn poll(&mut self) -> Action<'_> {
        loop {
            if let Some((port, broadcast)) = self.next_target() {
                self.out.last_table_ack = self.out.current.table_ack;
                self.out.last_port = port;
                let current = &self.out.current;
                return Action::Send(Send {
                    port,
                    to: if broadcast {
                        Dest::Broadcast
                    } else {
                        Dest::Unicast(current.to.as_slice())
                    },
                    npdu: &self.scratch[usize::from(current.start)..current.len],
                    priority: current.priority,
                    kind: current.kind(),
                    reject_origin: current.reject_origin.as_option(),
                    cause: if current.local {
                        Cause::Local
                    } else {
                        Cause::Ingress
                    },
                });
            }
            if !self.write_task() {
                return Action::Idle;
            }
        }
    }

    /// The current output's next port, and whether it is a broadcast there.
    /// A port that can't carry it is left out and counted.
    fn next_target(&mut self) -> Option<(PortId, bool)> {
        let current = &mut self.out.current;
        if !current.active {
            return None;
        }
        let ports = &self.ports[..self.port_count];
        let len = current.len;
        let fits_from = |start: usize, port: PortId, broadcast: bool| {
            ports
                .get(usize::from(port.0))
                .is_some_and(|config| len - start <= config.max_npdu.get(broadcast))
        };
        let fits = |port: PortId, broadcast: bool| fits_from(0, port, broadcast);
        if !current.fan_out {
            current.active = false;
            if fits(current.port, current.broadcast) {
                return Some((current.port, current.broadcast));
            }
            bump(&mut self.counters.egress_too_long);
            return None;
        }
        while usize::from(current.next) < ports.len() {
            // At most 255 ports, so the index fits in a u8.
            let port = PortId(current.next as u8);
            current.next += 1;
            if port == current.port {
                continue;
            }
            if fits(port, true) {
                return Some((port, true));
            }
            bump(&mut self.counters.egress_too_long);
        }
        let tail = usize::from(core::mem::take(&mut current.tail));
        if tail != 0 {
            // The global broadcast's version, control, DNET 0xFFFF and
            // DLEN 0, just before its hop count (Current::with_tail).
            let header = [crate::npdu::VERSION, current.tail_control, 0xFF, 0xFF, 0];
            // The offset is the SNET, SLEN and SADR's length, so the header
            // ends at the hop count, inside the NPDU and the scratch: the
            // lookup below can't fail.
            debug_assert!(
                tail + header.len() < len,
                "a tail's header ends before its hop count"
            );
            if let Some(slot) = self.scratch.get_mut(tail..tail + header.len()) {
                slot.copy_from_slice(&header);
                current.start = tail as u16;
                if fits_from(tail, current.port, true) {
                    return Some((current.port, true));
                }
                bump(&mut self.counters.egress_too_long);
            }
        }
        current.active = false;
        None
    }

    /// The send just polled was refused for a full queue: when its port is
    /// clogged and it carries an origin, answer that origin with reason 2
    /// (Clause 6.6.3.6).
    pub(super) fn refused_while_clogged(&mut self) {
        let port = usize::from(self.out.last_port.0);
        let clogged = self.flows[..self.port_count]
            .get(port)
            .is_some_and(|flow| flow.clogged());
        if let Some(origin) = self.out.current.reject_origin.as_option()
            && clogged
        {
            self.queue_reject(origin, RejectMessageReason::ROUTER_BUSY);
        }
    }

    /// The caller refused the send just polled: end the table answer it
    /// belonged to. [`Router::refused`](super::Router::refused) counts it.
    pub(super) fn end_refused_answer(&mut self) {
        if core::mem::take(&mut self.out.last_table_ack)
            && self
                .out
                .front()
                .is_some_and(|task| task.kind == TaskKind::TableAck)
        {
            self.out.pop();
        }
    }
}
