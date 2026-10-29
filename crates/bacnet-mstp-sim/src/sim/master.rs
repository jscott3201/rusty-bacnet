//! A master node: its core, the adapter that runs the core's actions, and
//! the application behind it.

use bacnet_mstp_codec::{FrameHeader, FrameType};
use bacnet_mstp_core::npdu::NetworkPriority;
use bacnet_mstp_core::{
    Action, AllocQueues, Direction, Duration, Instant, MasterCore, MasterState, MstpCounters,
    QueueRefusal, ReceiveError, ReplyDecision, RequestError, RequestId, TransmitError, TransmitId,
};

use super::{Event, NodeKind, Sim, SimCore, decode_whole, describe, leak_core_storage};
use crate::app::{Answer, Outbox, Received};
use crate::event::Class;
use crate::line::Driver;
use crate::report::{Delivery, ViolationKind};
use crate::skew::Clock;
use crate::spec::MasterSpec;
use crate::trace::{TraceKind, Who};

/// A deadline the core still reports as due this many times in a row at one
/// instant, after being serviced, means the adapter would spin.
const DEADLINE_SPIN_LIMIT: u32 = 64;
/// Actions one drain may return before the core is declared to be looping.
const ACTION_DRAIN_LIMIT: usize = 64;

pub(crate) struct MasterNode {
    pub(super) host: Option<super::host::HostState>,
    pub(super) incarnation: u64,
    pub(super) spec: MasterSpec,
    pub(super) core: Option<Box<SimCore>>,
    /// The clock the core reads, which starts at the line's time at each
    /// boot and runs at the spec's clock error. Every time the core sees is
    /// on it; the simulator's own records stay on the line's clock.
    pub(super) clock: Clock,
    /// The core's next deadline, as it last reported it, on its clock.
    pub(super) deadline: Option<Instant>,
    /// When the adapter services it: the deadline plus its lateness and a
    /// jitter drawn once per deadline.
    pub(super) fire_at: Option<Instant>,
    /// A frame the core handed over that the adapter has not started yet.
    pub(super) pending: Option<PendingTx>,
    /// The transmit the adapter is sending.
    pub(super) transmitting: Option<TransmitId>,
    /// The core counted a token generation; its next frame is that poll.
    pub(super) regenerated: bool,
    /// A frame whose start the core deferred. When the core hands a frame
    /// with the same header and length over again, before it gives any
    /// frame up, it is the same frame.
    withdrawn: Option<PendingTx>,
    /// The octets the core handed over with the last confirmed start.
    started: Option<Vec<u8>>,
    /// Late deadlines already accounted for.
    pub(super) late_seen: u32,
    /// The largest deadline lateness the core had reported when last
    /// checked.
    pub(super) lateness_seen: Duration,
    /// The master generated a token and has held it since: a collision with
    /// another master doing the same is the generation race.
    pub(super) generating: bool,
    pub(super) deliveries: Vec<Delivery>,
    pub(super) rejects: Vec<(Instant, QueueRefusal)>,
    spin: (Instant, u32),
}

pub(crate) struct PendingTx {
    pub(super) id: TransmitId,
    /// The frame's type and addresses, as the action named them.
    pub(super) header: FrameHeader,
    /// Its length on the wire, as the action gave it.
    pub(super) length: usize,
    /// Its octets, which come with the confirmed start: empty until then.
    pub(super) octets: Vec<u8>,
    /// When the core handed the frame over.
    pub(super) dispatched: Instant,
    /// The line's last activity at that moment.
    pub(super) activity_at_dispatch: Option<Instant>,
    /// The frame is the core's answer to a request (a data frame or Reply
    /// Postponed, handed over while answering).
    pub(super) while_answering: bool,
    /// The accepted host request that caused this answer, on the wire clock.
    pub(super) reply_to: Option<super::host::WireRequest>,
    pub(super) generation: bool,
}

impl MasterNode {
    pub(super) fn new(spec: MasterSpec) -> Self {
        Self {
            host: None,
            incarnation: 0,
            spec,
            core: None,
            clock: Clock::PERFECT,
            deadline: None,
            fire_at: None,
            pending: None,
            transmitting: None,
            regenerated: false,
            withdrawn: None,
            started: None,
            generating: false,
            late_seen: 0,
            lateness_seen: Duration::ZERO,
            deliveries: Vec::new(),
            rejects: Vec::new(),
            spin: (Instant::ZERO, 0),
        }
    }

    /// A fresh core, as after INITIALIZE. It uses as much receive storage as
    /// the longest frame it takes: a standard one, or an extended one with
    /// the flag on, so a master without it treats extended frames as a node
    /// without COBS support does. Its timer process ticks from
    /// `timer_phase`.
    pub(super) fn boot(&mut self, now: Instant, timer_phase: u64) {
        self.incarnation += 1;
        self.host = self.spec.adapter.host.map(|spec| {
            spec.validate();
            super::host::HostState::new(self.spec.config.t_frame_abort)
        });
        let queues = AllocQueues::new(self.spec.unicast_capacity, self.spec.broadcast_capacity);
        // The clock reads `now` at boot, so the core starts at the line's
        // time and drifts from it.
        self.clock = Clock::starting(now, self.spec.skew.clock_ppm).ticking_from(timer_phase);
        let core: SimCore = MasterCore::new(self.spec.config, leak_core_storage(queues), now)
            .expect("a valid master spec");
        self.deadline = core.next_deadline();
        self.fire_at = None;
        self.late_seen = 0;
        self.lateness_seen = Duration::ZERO;
        self.core = Some(Box::new(core));
        self.pending = None;
        self.transmitting = None;
        self.regenerated = false;
        self.withdrawn = None;
        self.generating = false;
    }

    /// Drop the core, as on power loss.
    pub(super) fn shut_down(&mut self) {
        self.host = None;
        self.core = None;
        self.deadline = None;
        self.fire_at = None;
        self.pending = None;
        self.transmitting = None;
        self.regenerated = false;
        self.withdrawn = None;
        self.generating = false;
    }
}

/// An event the adapter feeds the core.
pub(crate) enum Input {
    Octet(u8),
    Garbled,
    Deadline,
    Started(TransmitId),
    Complete(TransmitId, Instant),
    Decide(RequestId, Option<Vec<u8>>),
}

/// What the core refused, for the two inputs that can be refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refused {
    /// The adapter's transmit start.
    Start(TransmitError),
    /// The application's decision on a request.
    Decision(RequestError),
}

/// An action, copied out of the core so the borrow ends.
enum Effect {
    Direction(Direction),
    Transmit(TransmitId, FrameHeader, usize),
    Deliver {
        source: u8,
        destination: u8,
        /// `None` for an NPDU, else the type of a frame delivered whole.
        frame_type: Option<FrameType>,
        data: Vec<u8>,
        request: Option<RequestId>,
    },
}

/// Whether a frame is an answer to a request: a data frame not expecting a
/// reply, standard or extended, or Reply Postponed.
fn is_answer(header: FrameHeader) -> bool {
    matches!(
        header.frame_type,
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY
            | FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY
            | FrameType::REPLY_POSTPONED
    )
}

/// A frame named by its header and length, as [`describe`] prints octets.
fn describe_named(header: FrameHeader, length: usize) -> String {
    format!(
        "{:?} {}->{} ({length} octets)",
        header.frame_type, header.source, header.destination
    )
}

/// Frames the core has given up before they went out.
fn given_up(counters: &MstpCounters) -> u32 {
    counters.cancelled_sends + counters.expired_requests
}

/// Poll actions until the core is idle.
fn drain(core: &mut SimCore) -> Result<Vec<Effect>, usize> {
    let mut effects = Vec::new();
    loop {
        let effect = match core.poll_action() {
            Action::Idle => return Ok(effects),
            Action::SetDirection(direction) => Effect::Direction(direction),
            Action::Transmit { id, header, length } => Effect::Transmit(id, header, length),
            Action::DeliverNpdu {
                source,
                destination,
                npdu,
                request_id,
            } => Effect::Deliver {
                source,
                destination,
                frame_type: None,
                data: npdu.to_vec(),
                request: request_id,
            },
            Action::DeliverFrame {
                frame_type,
                source,
                destination,
                data,
            } => Effect::Deliver {
                source,
                destination,
                frame_type: Some(frame_type),
                data: data.to_vec(),
                request: None,
            },
        };
        effects.push(effect);
        if effects.len() > ACTION_DRAIN_LIMIT {
            return Err(effects.len());
        }
    }
}

impl Sim {
    /// Feed one event to a master's core, then run the adapter over what it
    /// asked for and check the core's contract.
    pub(super) fn master_input(&mut self, node: usize, input: Input) -> Result<(), Refused> {
        let now = match input {
            Input::Complete(_, stamp) => stamp,
            _ => self.now,
        };
        let station = self.nodes[node].station;
        let NodeKind::Master(master) = &mut self.nodes[node].kind else {
            return Ok(());
        };
        let now = master.clock.local(now);
        let Some(core) = master.core.as_mut() else {
            return Ok(());
        };
        let before = core.state();
        let generations = core.counters().token_regenerations;
        let gave_up = given_up(&core.counters());
        let mut started = None;
        let result = match &input {
            Input::Octet(byte) => {
                core.on_octet(*byte, now);
                Ok(())
            }
            Input::Garbled => {
                core.on_receive_error(ReceiveError::Framing, now);
                Ok(())
            }
            Input::Deadline => {
                core.on_deadline(now);
                Ok(())
            }
            Input::Started(id) => core
                .on_transmit_started(*id, now)
                .map(|frame| started = Some(frame.to_vec()))
                .map_err(Refused::Start),
            Input::Complete(id, _) => {
                core.on_transmit_complete(*id, now);
                Ok(())
            }
            Input::Decide(request, Some(data)) => core
                .complete_request(*request, ReplyDecision::Data(data), now)
                .map_err(Refused::Decision),
            Input::Decide(request, None) => core
                .complete_request(*request, ReplyDecision::Postponed, now)
                .map_err(Refused::Decision),
        };
        let effects = drain(core);
        let after = core.state();
        let counters = core.counters();
        let deadline = core.next_deadline();
        let late_deadline = core.config().late_deadline;
        if started.is_some() {
            master.started = started;
        }
        let adapter = master.spec.adapter;
        let rearmed = deadline != master.deadline;
        if rearmed {
            master.deadline = deadline;
            master.fire_at = None;
        }
        // The core gave a frame up: a deferred one never comes back, so a
        // later frame with the same octets is not it.
        if given_up(&counters) > gave_up {
            master.withdrawn = None;
        }
        if counters.token_regenerations > generations {
            master.regenerated = true;
        }
        if !super::checks::holds_token(after) {
            master.generating = false;
        }
        let completion_exposed_timer = matches!(input, Input::Deadline)
            && master
                .host
                .as_ref()
                .is_some_and(|host| host.completion_reported == Some(self.now));
        let punctual = !adapter.may_miss_deadlines(late_deadline) && !completion_exposed_timer;
        // An octet adapter services each deadline within a known bound; a
        // host adapter can also hold one back while it transmits.
        let bound = (!punctual && master.host.is_none())
            .then(|| adapter.lateness_bound(master.spec.skew.clock_ppm));
        if master.fire_at.is_none() {
            self.arm_deadline(node);
        }

        if before != after {
            self.note(
                Who::Station(station),
                TraceKind::State {
                    from: before,
                    to: after,
                },
            );
        }
        self.check_core_result(station, result);
        self.check_counters(node, &counters, punctual, bound);
        // A deadline already due may wait for a late or jittery adapter; a
        // new one must not be set in the past.
        if rearmed && deadline.is_some_and(|deadline| deadline < now) {
            self.violation(
                ViolationKind::Deadline,
                format!("station {station}: new deadline {deadline:?} is before now"),
            );
        }
        let answering = before == MasterState::AnswerDataRequest;
        match effects {
            Ok(effects) => self.apply_effects(node, effects, answering),
            Err(count) => self.violation(
                ViolationKind::ContractBroken,
                format!("station {station}: {count} actions from one event"),
            ),
        }
        self.host_finish_request(node);
        result
    }

    /// Work out when the adapter services the core's current deadline: on
    /// the first tick of its timer at or after it, plus the adapter's
    /// lateness, plus a jitter drawn once per deadline.
    pub(super) fn arm_deadline(&mut self, node: usize) {
        let now = self.now;
        let Some(master) = self.master_mut(node) else {
            return;
        };
        let adapter = master.spec.adapter;
        let Some(deadline) = master.deadline else {
            master.fire_at = None;
            return;
        };
        let jitter = adapter.deadline_jitter.as_micros();
        let drawn = if jitter == 0 {
            0
        } else {
            self.jitter.range(0, jitter)
        };
        let master = self.master_mut(node).expect("a master");
        // The deadline is on the node's clock: service it once that clock
        // reaches the timer's next tick.
        let tick = master.clock.next_tick(deadline, adapter.timer_tick);
        master.fire_at = Some(
            (master.clock.line_time(tick)
                + adapter.deadline_lateness
                + bacnet_mstp_core::Duration::from_micros(drawn))
            .max(now),
        );
    }

    /// Errors the adapter contract expects; anything else is a violation.
    fn check_core_result(&mut self, station: u8, result: Result<(), Refused>) {
        let Err(error) = result else {
            return;
        };
        let expected = matches!(
            error,
            // A reply whose window closed before the adapter got to it, or
            // line activity within Tturnaround of the start.
            Refused::Start(TransmitError::Expired | TransmitError::Deferred)
            // The application answered late, or for a request already gone.
                | Refused::Decision(
                    RequestError::Expired | RequestError::NoPending | RequestError::Stale
                )
        );
        if expected {
            self.note(
                Who::Station(station),
                TraceKind::Note(format!("core refused as expected: {error:?}")),
            );
        } else {
            self.violation(
                ViolationKind::ContractBroken,
                format!("station {station}: unexpected {error:?}"),
            );
        }
    }

    fn apply_effects(&mut self, node: usize, effects: Vec<Effect>, answering: bool) {
        let station = self.nodes[node].station;
        let who = Who::Station(station);
        for effect in effects {
            match effect {
                Effect::Direction(direction) => {
                    self.note(
                        who,
                        TraceKind::Action(format!("SetDirection({direction:?})")),
                    );
                    // Back to receive before the adapter started the frame:
                    // the core gave the frame up, so the adapter drops it. It
                    // never comes back, so nothing carries over to a later
                    // frame (only a deferred start does that).
                    let master = self.master_mut(node).expect("a master");
                    if direction == Direction::Receive && master.pending.take().is_some() {
                        self.note(who, TraceKind::Note("frame taken back".into()));
                    }
                    if direction == Direction::Receive {
                        self.abort_withdrawn_transmission(node);
                    }
                }
                Effect::Transmit(id, header, length) => {
                    self.note(
                        who,
                        TraceKind::Action(format!(
                            "Transmit #{} {}",
                            id.generation(),
                            describe_named(header, length)
                        )),
                    );
                    let now = self.now;
                    let activity = self.line.last_activity();
                    let activity = self
                        .master_mut(node)
                        .and_then(|master| master.host.as_ref())
                        .map_or(activity, |host| host.activity);
                    let master = self.master_mut(node).expect("a master");
                    let replaced = master.pending.is_some();
                    let delay = master.spec.adapter.start_delay;
                    let again = master.withdrawn.take().is_some_and(|withdrawn| {
                        withdrawn.generation
                            && (withdrawn.header, withdrawn.length) == (header, length)
                    });
                    let generation = std::mem::take(&mut master.regenerated) || again;
                    master.generating |= generation;
                    let answer = answering && is_answer(header);
                    master.pending = Some(PendingTx {
                        id,
                        header,
                        length,
                        octets: Vec::new(),
                        dispatched: now,
                        activity_at_dispatch: activity,
                        while_answering: answer,
                        reply_to: answer
                            .then(|| master.host.as_ref().and_then(|host| host.request))
                            .flatten(),
                        generation,
                    });
                    let incarnation = master.incarnation;
                    if replaced {
                        self.violation(
                            ViolationKind::ContractBroken,
                            format!("station {station}: a transmit before the last one started"),
                        );
                    }
                    self.push(
                        now + delay,
                        Class::AdapterStart,
                        station,
                        Event::AdapterStart {
                            node,
                            id,
                            incarnation,
                        },
                    );
                }
                Effect::Deliver {
                    source,
                    destination,
                    frame_type,
                    data,
                    request,
                } => {
                    let delivery = Delivery {
                        at: self.now,
                        source,
                        destination,
                        frame_type,
                        data,
                        request: request.is_some(),
                    };
                    self.deliver(node, delivery, request);
                }
            }
        }
    }

    /// The application receives an NPDU, plans its answer and queues what
    /// it sends of its own accord.
    fn deliver(&mut self, node: usize, delivery: Delivery, request: Option<RequestId>) {
        let now = self.now;
        let station = self.nodes[node].station;
        let source = delivery.source;
        let destination = delivery.destination;
        let kind = match delivery.frame_type {
            None => String::new(),
            Some(frame_type) => format!(" {frame_type:?} frame"),
        };
        self.note(
            Who::Station(station),
            TraceKind::Action(format!(
                "Deliver{kind} {source}->{destination} {} octets{}",
                delivery.data.len(),
                if request.is_some() { " (request)" } else { "" }
            )),
        );
        if let Some(request) = request {
            self.host_accept_request(node, request, source);
        }
        let master = self.master_mut(node).expect("a master");
        let mut outbox = Outbox::default();
        // Only NPDUs reach the application, not frames delivered whole.
        let answer = if delivery.frame_type.is_none() {
            let received = Received {
                at: now,
                source,
                destination,
                npdu: &delivery.data,
                expects_reply: request.is_some(),
            };
            let mut application = master.spec.application.0.borrow_mut();
            application.on_npdu(&received, &mut outbox)
        } else {
            None
        };
        master.deliveries.push(delivery);
        let incarnation = master.incarnation;
        for (destination, npdu) in outbox.sends {
            self.push(
                now,
                Class::Scheduled,
                station,
                Event::AppSend {
                    node,
                    incarnation,
                    destination,
                    npdu,
                },
            );
        }
        let Some(answer) = answer else {
            return;
        };
        let (delay, postpone, payload) = match answer {
            Answer::Reply { delay, npdu } => (delay, false, npdu),
            Answer::Postpone { delay, npdu } => (delay, true, npdu),
        };
        let event = match request {
            Some(request) => Event::Decide {
                node,
                incarnation,
                request,
                requester: source,
                postpone,
                payload,
            },
            // Nothing holds the line open for it: it waits for the token.
            None => Event::AppSend {
                node,
                incarnation,
                destination: source,
                npdu: payload,
            },
        };
        self.push(now + delay, Class::Scheduled, station, event);
    }

    /// The application answers a request. An answer the core no longer
    /// takes goes out later as an ordinary NPDU.
    pub(super) fn decide(
        &mut self,
        node: usize,
        request: RequestId,
        requester: u8,
        postpone: bool,
        payload: Vec<u8>,
    ) {
        let master = self.master_mut(node).expect("a master");
        if let Some(host) = master.host.as_ref() {
            if host.writing() {
                self.host_defer_decision(node, request, requester, postpone, payload);
                return;
            }
            // Production moves a finished request's channel to its deferred
            // queue. A ready late answer queues data without entering the
            // core (even a refused complete_request would advance its clock).
            if master.core.as_ref().unwrap().snapshot().request_id != Some(request) {
                self.report.late_answers += 1;
                self.enqueue_now(node, requester, &payload, NetworkPriority::Normal);
                return;
            }
        }
        let data = if postpone {
            None
        } else {
            Some(payload.clone())
        };
        let result = self.master_input(node, Input::Decide(request, data));
        if postpone || result.is_err() {
            if result.is_err() {
                self.report.late_answers += 1;
            }
            self.enqueue_now(node, requester, &payload, NetworkPriority::Normal);
        }
    }

    pub(super) fn enqueue_now(
        &mut self,
        node: usize,
        destination: u8,
        npdu: &[u8],
        priority: NetworkPriority,
    ) {
        let now = self.now;
        let station = self.nodes[node].station;
        let Some(master) = self.master_mut(node) else {
            return;
        };
        let local = master.clock.local(now);
        let Some(core) = master.core.as_mut() else {
            master.rejects.push((now, QueueRefusal::Full));
            return;
        };
        if let Err(reason) = core.enqueue_npdu(destination, npdu, priority, local) {
            master.rejects.push((now, reason));
            self.note(
                Who::Station(station),
                TraceKind::Note(format!(
                    "enqueue of {} octets for {destination} refused: {reason:?}",
                    npdu.len()
                )),
            );
        }
    }

    pub(super) fn service_deadline(&mut self, node: usize) {
        let now = self.now;
        // Errors are reported by `master_input`.
        let _ = self.master_input(node, Input::Deadline);
        let station = self.nodes[node].station;
        let master = self.master_mut(node).expect("a master");
        let still_due = master.fire_at.is_some_and(|fire_at| fire_at <= now);
        if !still_due {
            master.spin = (now, 0);
            return;
        }
        if master.spin.0 == now {
            master.spin.1 += 1;
        } else {
            master.spin = (now, 1);
        }
        if master.spin.1 >= DEADLINE_SPIN_LIMIT {
            master.fire_at = None;
            let deadline = master.deadline.take();
            self.violation(
                ViolationKind::Deadline,
                format!(
                    "station {station}: deadline {deadline:?} still due after {DEADLINE_SPIN_LIMIT} services; an adapter would spin"
                ),
            );
        }
    }

    /// The adapter gets to a frame the core handed it: confirm the start,
    /// then drive the line.
    pub(super) fn adapter_start(&mut self, node: usize, id: TransmitId) {
        if self.wait_for_release(node, id) {
            return;
        }
        self.host_before_start(node);
        let station = self.nodes[node].station;
        let Some(master) = self.master_mut(node) else {
            return;
        };
        let Some(mut pending) = master.pending.take() else {
            return;
        };
        if pending.id != id {
            master.pending = Some(pending);
            return;
        }
        match self.master_input(node, Input::Started(id)) {
            Ok(()) => {
                // The octets come with the confirmed start, and must be the
                // frame the action named.
                let master = self.master_mut(node).expect("a master");
                pending.octets = master.started.take().unwrap_or_default();
                let named = decode_whole(&pending.octets)
                    .is_some_and(|(header, _)| header == pending.header)
                    && pending.octets.len() == pending.length;
                if !named {
                    self.violation(
                        ViolationKind::ContractBroken,
                        format!(
                            "station {station}: the start handed over {}, not the {} the action named",
                            describe(&pending.octets),
                            describe_named(pending.header, pending.length)
                        ),
                    );
                }
                self.receiver_started(node);
                self.check_turnaround(node, &pending);
                self.check_usage_delay(node, &pending);
                self.check_host_reply_delay(node, &pending);
                let corruption = self.take_corruption(station, &pending.octets);
                let adapter = self.master_mut(node).expect("a master").spec.adapter;
                let tx = self.begin_transmission(
                    Driver::Station(station),
                    pending.octets,
                    corruption,
                    Some((pending.dispatched, pending.generation)),
                    adapter.post_drive,
                    adapter.inter_octet_gap,
                );
                let master = self.master_mut(node).expect("a master");
                master.transmitting = Some(id);
                self.meta
                    .get_mut(&tx)
                    .expect("transmission metadata")
                    .transmit_id = Some(id);
                self.nodes[node].driving = Some(tx);
                self.nodes[node].listening_since = None;
                self.host_started(node, tx, id);
                self.octet_start(tx, 0);
            }
            Err(error) => {
                self.report.refused_starts += 1;
                if error == Refused::Start(TransmitError::Deferred) {
                    let master = self.master_mut(node).expect("a master");
                    master.withdrawn = Some(pending);
                }
            }
        }
    }

    /// The driver has sent the final stop bit.
    pub(super) fn master_tx_done(&mut self, node: usize) {
        let now = self.now;
        let Some(master) = self.master_mut(node) else {
            return;
        };
        if master.host.is_some() {
            return;
        }
        if let Some(id) = master.transmitting.take() {
            let _ = self.master_input(node, Input::Complete(id, now));
        }
    }
}
