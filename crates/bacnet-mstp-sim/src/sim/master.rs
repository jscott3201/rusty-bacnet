//! A master node: its core, the adapter that runs the core's actions, and
//! the application behind it.

use bacnet_mstp_codec::{FrameType, MAX_EXTENDED_FRAME_LENGTH, MAX_STANDARD_FRAME_LENGTH};
use bacnet_mstp_core::{
    Action, AllocQueues, CoreError, Direction, Instant, MasterCore, MasterState, MstpCounters,
    NetworkPriority, QueueRefusal, ReceiveError, ReplyDecision, RequestId, TransmitId,
};

use super::{
    decode_whole, describe, leak_action_buffers, leak_storage, Event, NodeKind, Sim, SimCore,
};
use crate::event::Class;
use crate::line::Driver;
use crate::report::{Delivery, ViolationKind};
use crate::spec::{MasterSpec, ReplyPolicy};
use crate::trace::{TraceKind, Who};

/// A deadline the core still reports as due this many times in a row at one
/// instant, after being serviced, means the adapter would spin.
const DEADLINE_SPIN_LIMIT: u32 = 64;
/// Actions one drain may return before the core is declared to be looping.
const ACTION_DRAIN_LIMIT: usize = 64;

pub(crate) struct MasterNode {
    pub(super) spec: MasterSpec,
    pub(super) core: Option<Box<SimCore>>,
    /// The core's next deadline, as it last reported it.
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
    /// A frame whose start the core deferred. When the core hands the same
    /// octets over again, before it gives any frame up, it is the same
    /// frame.
    withdrawn: Option<PendingTx>,
    /// Deadline misses already accounted for.
    misses_seen: u32,
    /// The master generated a token and has held it since: a collision with
    /// another master doing the same is the generation race.
    pub(super) generating: bool,
    pub(super) deliveries: Vec<Delivery>,
    pub(super) rejects: Vec<(Instant, QueueRefusal)>,
    spin: (Instant, u32),
}

pub(crate) struct PendingTx {
    pub(super) id: TransmitId,
    pub(super) octets: Vec<u8>,
    /// When the core handed the frame over.
    pub(super) dispatched: Instant,
    /// The line's last activity at that moment.
    pub(super) activity_at_dispatch: Option<Instant>,
    /// The frame is the core's answer to a request (a data frame or Reply
    /// Postponed, handed over while answering).
    pub(super) while_answering: bool,
    pub(super) generation: bool,
}

impl MasterNode {
    pub(super) fn new(spec: MasterSpec) -> Self {
        Self {
            spec,
            core: None,
            deadline: None,
            fire_at: None,
            pending: None,
            transmitting: None,
            regenerated: false,
            withdrawn: None,
            generating: false,
            misses_seen: 0,
            deliveries: Vec::new(),
            rejects: Vec::new(),
            spin: (Instant::ZERO, 0),
        }
    }

    /// A fresh core, as after INITIALIZE. Its receive storage holds the
    /// longest frame it takes: a standard one, or an extended one with the
    /// flag on, so a master without it treats extended frames as a node
    /// without COBS support does.
    pub(super) fn boot(&mut self, now: Instant) {
        let queues = AllocQueues::new(self.spec.unicast_capacity, self.spec.broadcast_capacity);
        let storage = if self.spec.config.extended_frames {
            MAX_EXTENDED_FRAME_LENGTH
        } else {
            MAX_STANDARD_FRAME_LENGTH
        };
        let core: SimCore = MasterCore::new(
            self.spec.config,
            queues,
            leak_storage(storage),
            leak_action_buffers(),
            now,
        )
        .expect("a valid master spec");
        self.deadline = core.next_deadline();
        self.fire_at = None;
        self.misses_seen = 0;
        self.core = Some(Box::new(core));
        self.pending = None;
        self.transmitting = None;
        self.regenerated = false;
        self.withdrawn = None;
        self.generating = false;
    }

    /// Drop the core, as on power loss.
    pub(super) fn shut_down(&mut self) {
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
    Complete(TransmitId),
    Decide(RequestId, Option<Vec<u8>>),
}

/// An action, copied out of the core so the borrow ends.
enum Effect {
    Direction(Direction),
    Transmit(TransmitId, Vec<u8>),
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
fn is_answer(octets: &[u8]) -> bool {
    decode_whole(octets).is_some_and(|(header, _)| {
        matches!(
            header.frame_type,
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY
                | FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY
                | FrameType::REPLY_POSTPONED
        )
    })
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
            Action::Transmit { id, frame } => Effect::Transmit(id, frame.to_vec()),
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
    pub(super) fn master_input(&mut self, node: usize, input: Input) -> Result<(), CoreError> {
        let now = self.now;
        let station = self.nodes[node].station;
        let NodeKind::Master(master) = &mut self.nodes[node].kind else {
            return Ok(());
        };
        let Some(core) = master.core.as_mut() else {
            return Ok(());
        };
        let before = core.state();
        let generations = core.counters().token_regenerations;
        let gave_up = given_up(&core.counters());
        let result = match &input {
            Input::Octet(byte) => core.on_octet(*byte, now),
            Input::Garbled => core.on_receive_error(ReceiveError::Framing, now),
            Input::Deadline => core.on_deadline(now),
            Input::Started(id) => core.on_transmit_started(*id, now),
            Input::Complete(id) => core.on_transmit_complete(*id, now),
            Input::Decide(request, Some(data)) => {
                core.complete_request(*request, ReplyDecision::Data(data), now)
            }
            Input::Decide(request, None) => {
                core.complete_request(*request, ReplyDecision::Postponed, now)
            }
        };
        let effects = drain(core);
        let after = core.state();
        let counters = core.counters();
        let deadline = core.next_deadline();
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
        let punctual = !adapter.may_miss_deadlines();
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
        self.check_core_result(station, &input, result);
        self.check_counters(node, &counters, punctual);
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
                ViolationKind::CoreError,
                format!("station {station}: {count} actions from one event"),
            ),
        }
        result
    }

    /// Work out when the adapter services the core's current deadline: on
    /// time, plus the adapter's lateness, plus a jitter drawn once per
    /// deadline.
    pub(super) fn arm_deadline(&mut self, node: usize) {
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
        master.fire_at = Some(
            deadline + adapter.deadline_lateness + bacnet_mstp_core::Duration::from_micros(drawn),
        );
    }

    /// Errors the adapter contract expects; anything else is a violation.
    fn check_core_result(&mut self, station: u8, input: &Input, result: Result<(), CoreError>) {
        let Err(error) = result else {
            return;
        };
        let expected = match input {
            // A reply whose window closed before the adapter got to it, or
            // line activity within Tturnaround of the start.
            Input::Started(_) => matches!(
                error,
                CoreError::RequestExpired | CoreError::TransmitDeferred
            ),
            // The application answered late, or for a request already gone.
            Input::Decide(..) => matches!(
                error,
                CoreError::RequestExpired | CoreError::NoPendingRequest | CoreError::StaleRequest
            ),
            _ => false,
        };
        if expected {
            self.note(
                Who::Station(station),
                TraceKind::Note(format!("core refused as expected: {error:?}")),
            );
        } else {
            self.violation(
                ViolationKind::CoreError,
                format!("station {station}: unexpected {error:?}"),
            );
        }
    }

    /// Counters that must not move: adapter errors, delivery drops and
    /// frames from the station's own address ever, and deadline misses while
    /// the adapter is punctual.
    fn check_counters(&mut self, node: usize, counters: &MstpCounters, punctual: bool) {
        let station = self.nodes[node].station;
        let master = self.master_mut(node).expect("a master");
        let new_misses = counters.deadline_misses - master.misses_seen;
        master.misses_seen = counters.deadline_misses;
        let mut wrong = Vec::new();
        if counters.adapter_errors != 0 {
            wrong.push(format!("adapter_errors {}", counters.adapter_errors));
        }
        if counters.delivery_drops != 0 {
            wrong.push(format!("delivery_drops {}", counters.delivery_drops));
        }
        // Every simulated station has its own address, so a frame from this
        // one's is its own echo leaking past the adapter.
        if counters.own_address_frames != 0 {
            wrong.push(format!(
                "own_address_frames {}",
                counters.own_address_frames
            ));
        }
        if punctual && new_misses != 0 {
            wrong.push(format!("{new_misses} more deadline_misses"));
        }
        if !wrong.is_empty() {
            self.violation(
                ViolationKind::CoreCounter,
                format!("station {station}: {}", wrong.join(", ")),
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
                }
                Effect::Transmit(id, octets) => {
                    self.note(
                        who,
                        TraceKind::Action(format!(
                            "Transmit #{} {}",
                            id.generation(),
                            describe(&octets)
                        )),
                    );
                    let now = self.now;
                    let activity = self.line.last_activity();
                    let master = self.master_mut(node).expect("a master");
                    let replaced = master.pending.is_some();
                    let delay = master.spec.adapter.start_delay;
                    let again = master.withdrawn.take().is_some_and(|withdrawn| {
                        withdrawn.generation && withdrawn.octets == octets
                    });
                    let generation = std::mem::take(&mut master.regenerated) || again;
                    master.generating |= generation;
                    let answer = answering && is_answer(&octets);
                    master.pending = Some(PendingTx {
                        id,
                        octets,
                        dispatched: now,
                        activity_at_dispatch: activity,
                        while_answering: answer,
                        generation,
                    });
                    if replaced {
                        self.violation(
                            ViolationKind::CoreError,
                            format!("station {station}: a transmit before the last one started"),
                        );
                    }
                    self.push(
                        now + delay,
                        Class::AdapterStart,
                        station,
                        Event::AdapterStart { node, id },
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

    /// The application receives an NPDU and, for a request, plans its answer.
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
        let master = self.master_mut(node).expect("a master");
        master.deliveries.push(delivery);
        let Some(request) = request else {
            return;
        };
        let (delay, postpone, payload) = match master.spec.reply.clone() {
            ReplyPolicy::Answer { delay, payload } => (delay, false, payload),
            ReplyPolicy::Postpone { delay, payload } => (delay, true, payload),
            ReplyPolicy::Ignore => return,
        };
        self.push(
            now + delay,
            Class::Scheduled,
            station,
            Event::Decide {
                node,
                request,
                requester: source,
                postpone,
                payload,
            },
        );
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
        let Some(core) = master.core.as_mut() else {
            master.rejects.push((now, QueueRefusal::Full));
            return;
        };
        if let Err(reason) = core.enqueue_npdu(destination, npdu, priority, now) {
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
        let station = self.nodes[node].station;
        let Some(master) = self.master_mut(node) else {
            return;
        };
        let Some(pending) = master.pending.take() else {
            return;
        };
        if pending.id != id {
            master.pending = Some(pending);
            return;
        }
        match self.master_input(node, Input::Started(id)) {
            Ok(()) => {
                self.check_turnaround(node, &pending);
                self.check_usage_delay(node, &pending);
                let corruption = self.take_corruption(station, &pending.octets);
                let tx = self.begin_transmission(
                    Driver::Station(station),
                    pending.octets,
                    corruption,
                    Some((pending.dispatched, pending.generation)),
                );
                let master = self.master_mut(node).expect("a master");
                master.transmitting = Some(id);
                self.nodes[node].driving = Some(tx);
                self.nodes[node].listening_since = None;
                self.octet_start(tx, 0);
            }
            Err(error) => {
                self.report.refused_starts += 1;
                if error == CoreError::TransmitDeferred {
                    let master = self.master_mut(node).expect("a master");
                    master.withdrawn = Some(pending);
                }
            }
        }
    }

    /// The driver has sent the final stop bit.
    pub(super) fn master_tx_done(&mut self, node: usize) {
        let Some(master) = self.master_mut(node) else {
            return;
        };
        if let Some(id) = master.transmitting.take() {
            let _ = self.master_input(node, Input::Complete(id));
        }
    }
}
