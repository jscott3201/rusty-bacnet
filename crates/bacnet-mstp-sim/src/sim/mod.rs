//! The simulator: nodes, the line, the event loop and the public API.

mod checks;
mod contention;
mod faults;
#[cfg(test)]
mod gap_tests;
mod host;
mod master;
mod medium;
mod post_drive;
#[cfg(test)]
mod post_drive_tests;

use std::collections::BTreeMap;

use bacnet_mstp_codec::{
    FrameHeader, MAX_EXTENDED_FRAME_LENGTH, MAX_EXTENDED_MPDU_DATA, StreamDecoder,
};
use bacnet_mstp_core::npdu::NetworkPriority;
use bacnet_mstp_core::{
    AllocQueues, DEFAULT_DELIVERY_SLOTS, Duration, Instant, MasterCore, MasterState, QueueRefusal,
    RequestId, Storage, TransmitId,
};

use crate::event::{Class, Key, Queue};
use crate::line::{Driver, Line, TxId};
use crate::report::{Delivery, Report, Tolerance, Violation, ViolationKind, WireFrame};
use crate::rng::SimRng;
use crate::spec::{Fault, MasterSpec, Script, ScriptSend};
use crate::trace::{Trace, TraceKind, Who};
use crate::wire::WireTiming;

use master::MasterNode;

/// The storage every simulated master's core borrows: queues and buffers
/// sized for extended frames whether the master's configuration takes them
/// or not, so masters with and without the flag share one type. The core
/// uses as much of the receive storage as the flag takes (see
/// [`MasterSpec::extended_frames`]).
pub(crate) type SimStorage = Storage<
    AllocQueues<MAX_EXTENDED_MPDU_DATA>,
    DEFAULT_DELIVERY_SLOTS,
    MAX_EXTENDED_MPDU_DATA,
    MAX_EXTENDED_FRAME_LENGTH,
>;

/// The core type every simulated master runs, over a `SimStorage`.
///
/// So a master without the flag runs with 1497-octet delivery and queue
/// slots, not the 501-octet default instantiation a firmware build uses.
/// Its behaviour is the same: the flag, not the slot size, decides what it
/// takes and sends, and the core refuses an NPDU over 501 octets at the
/// queue whatever the slots hold.
pub type SimCore =
    MasterCore<'static, &'static mut AllocQueues<MAX_EXTENDED_MPDU_DATA>, MAX_EXTENDED_MPDU_DATA>;

/// Run-wide settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimConfig {
    /// Line speed. Every master's configuration must use the same rate.
    pub baud: u32,
    /// Seed for the run's generators: the one scenarios draw from
    /// ([`Sim::rng`]) and the adapters' timer jitter.
    pub seed: u64,
    /// Trace entries kept for failure output.
    pub trace_capacity: usize,
    /// Panic, with the trace, on the first invariant failure. When off,
    /// failures collect in [`Report::violations`].
    pub panic_on_violation: bool,
    /// Departures from the strict invariants this run accepts.
    pub tolerance: Tolerance,
}

impl SimConfig {
    /// Strict checking at `baud`, seed 0, 4096 trace entries.
    #[must_use]
    pub fn new(baud: u32) -> Self {
        Self {
            baud,
            seed: 0,
            trace_capacity: 4_096,
            panic_on_violation: true,
            tolerance: Tolerance::default(),
        }
    }
}

/// Lines of trace a violation panic shows.
const PANIC_TRACE_LINES: usize = 120;
/// Events allowed at a single instant before the run is declared stuck.
const SAME_INSTANT_LIMIT: u32 = 100_000;
/// Separates the jitter generator's stream from the scenario's.
const JITTER_STREAM: u64 = 0x6A09_E667_F3BC_C908;

#[derive(Debug)]
pub(crate) enum Event {
    WindowEnd,
    OctetStart {
        tx: TxId,
        index: usize,
    },
    TxEnd {
        tx: TxId,
    },
    DriverRelease {
        tx: TxId,
    },
    AdapterStart {
        node: usize,
        id: TransmitId,
        incarnation: u64,
    },
    Decide {
        node: usize,
        incarnation: u64,
        request: RequestId,
        requester: u8,
        postpone: bool,
        payload: Vec<u8>,
    },
    ScriptSend {
        node: usize,
        send: ScriptSend,
    },
    /// An NPDU the node's application sends, fenced to the incarnation
    /// that decided it: a node that lost power since never sends it.
    AppSend {
        node: usize,
        incarnation: u64,
        destination: u8,
        npdu: Vec<u8>,
    },
    Enqueue {
        node: usize,
        destination: u8,
        npdu: Vec<u8>,
        priority: NetworkPriority,
    },
    Fault(Fault),
}

pub(crate) struct ScriptedNode {
    script: Box<dyn Script>,
    decoder: StreamDecoder<'static>,
    last_character: Option<Instant>,
}

pub(crate) enum NodeKind {
    Master(Box<MasterNode>),
    Scripted(Box<ScriptedNode>),
}

pub(crate) struct Node {
    station: u8,
    receiver: contention::Receiver,
    kind: NodeKind,
    /// The transmission this node is driving.
    driving: Option<TxId>,
    /// Since when the node's receiver has been on; `None` while it drives
    /// or is unpowered.
    listening_since: Option<Instant>,
}

/// Facts about a transmission the line itself doesn't track.
#[derive(Debug, Default, Clone)]
pub(crate) struct TxMeta {
    /// Local return timing belongs to the physical operation, not its Write.
    echo_profile: Option<host::EchoProfile>,
    /// Core operation bound to this physical transmission, for abort fencing.
    transmit_id: Option<TransmitId>,
    /// When a master's core handed this frame over.
    dispatched: Option<Instant>,
    generation: bool,
    corrupted: bool,
    /// Power fails once this many octets have gone out.
    cut_after: Option<usize>,
    /// Masters listening at the start, with their valid-frame counts.
    listeners: Vec<(usize, u32)>,
    /// Drivers this transmission has already been reported colliding with.
    collided_with: Vec<Driver>,
}

enum Next {
    Dynamic(Key),
    Timeline(Key),
    Deadline(Key, usize),
    Host(Key, usize),
    LocalEcho(Key, usize),
}

/// A deterministic, virtual-time MS/TP bus with real [`MasterCore`]s on it.
///
/// See the crate documentation for the model and its limits.
pub struct Sim {
    config: SimConfig,
    timing: WireTiming,
    now: Instant,
    seq: u64,
    same_instant: (Instant, u32),
    /// Line, adapter, application and script events.
    dynamic: Queue<Event>,
    /// Events the scenario scheduled.
    timeline: Queue<Event>,
    nodes: Vec<Node>,
    line: Line,
    meta: BTreeMap<TxId, TxMeta>,
    frames: Vec<WireFrame>,
    trace: Trace,
    report: Report,
    rng: SimRng,
    /// Draws adapter timer jitter, apart from [`Self::rng`] so a scenario's
    /// own draws don't shift it.
    jitter: SimRng,
    /// Independent seeded stream for chunk boundaries.
    chunks: SimRng,
    /// A fault or collision has happened; see [`Self::perturbed`].
    perturbed: bool,
    noise_bursts: u64,
    armed: Vec<faults::Armed>,
    /// When more than one master began holding the token, and whether all
    /// of them had generated it.
    duplicate_since: Option<(Instant, bool)>,
}

impl Sim {
    /// An empty bus at virtual time zero.
    #[must_use]
    pub fn new(config: SimConfig) -> Self {
        let timing = WireTiming::new(config.baud);
        Self {
            config,
            timing,
            now: Instant::ZERO,
            seq: 0,
            same_instant: (Instant::ZERO, 0),
            dynamic: Queue::new(),
            timeline: Queue::new(),
            nodes: Vec::new(),
            line: Line::new(timing),
            meta: BTreeMap::new(),
            frames: Vec::new(),
            trace: Trace::new(config.trace_capacity),
            report: Report::default(),
            rng: SimRng::new(config.seed),
            jitter: SimRng::new(config.seed ^ JITTER_STREAM),
            chunks: SimRng::new(config.seed ^ 0xBB67_AE85_84CA_A73B),
            perturbed: false,
            noise_bursts: 0,
            armed: Vec::new(),
            duplicate_since: None,
        }
    }

    /// Put a master on the bus. A powered one starts its core now.
    ///
    /// # Panics
    ///
    /// If the station is taken or the spec's baud rate differs from the
    /// line's, or the inter-octet gap, post-drive hold or skew exceeds its
    /// limit.
    pub fn add_master(&mut self, spec: MasterSpec) -> &mut Self {
        let station = spec.station();
        self.claim_station(station);
        assert_eq!(
            spec.config.baud_rate, self.config.baud,
            "station {station}: core baud rate differs from the line's"
        );
        let powered = spec.powered;
        spec.skew.validate();
        self.timing.validate_post_drive(spec.adapter.post_drive);
        self.timing
            .validate_inter_octet_gap(spec.adapter.inter_octet_gap);
        self.nodes.push(Node {
            station,
            receiver: contention::Receiver::new(self.config.seed, station),
            kind: NodeKind::Master(Box::new(MasterNode::new(spec))),
            driving: None,
            listening_since: None,
        });
        if powered {
            let index = self.nodes.len() - 1;
            self.power_on(index);
        }
        self
    }

    /// Put a scripted station on the bus. It hears every frame and reacts
    /// through `script`; it runs no MS/TP state machine of its own.
    ///
    /// # Panics
    ///
    /// If the station is taken.
    pub fn add_scripted(&mut self, station: u8, script: impl Script + 'static) -> &mut Self {
        self.claim_station(station);
        self.nodes.push(Node {
            station,
            receiver: contention::Receiver::new(self.config.seed, station),
            kind: NodeKind::Scripted(Box::new(ScriptedNode {
                script: Box::new(script),
                decoder: StreamDecoder::new(leak_storage(2 * MAX_EXTENDED_FRAME_LENGTH)),
                last_character: None,
            })),
            driving: None,
            listening_since: Some(self.now),
        });
        self
    }

    /// Select this receiver's synthetic digital contention policy.
    /// Uncontended characters and physical line timing are unchanged.
    /// If enabled after unobserved core input, receipt qualification waits
    /// for a known parser reset (silence, receive error, transmit or reboot).
    ///
    /// # Panics
    ///
    /// If the station has not been added.
    pub fn set_contention(&mut self, station: u8, policy: crate::ContentionPolicy) -> &mut Self {
        let node = self.index_of(station);
        self.nodes[node].receiver.policy = policy;
        self
    }

    /// Queue an NPDU at a master at `at`. Refusals are kept in
    /// [`Self::rejects`].
    pub fn enqueue(&mut self, at: Instant, station: u8, destination: u8, npdu: &[u8]) -> &mut Self {
        let node = self.index_of(station);
        self.schedule(
            at,
            Class::Scheduled,
            station,
            Event::Enqueue {
                node,
                destination,
                npdu: npdu.to_vec(),
                priority: NetworkPriority::Normal,
            },
        );
        self
    }

    /// Inject `fault` at `at`.
    pub fn inject(&mut self, at: Instant, fault: Fault) -> &mut Self {
        self.schedule(at, Class::Scheduled, 300u16, Event::Fault(fault));
        self
    }

    /// Run every event up to and including `end`, then set the clock to
    /// `end`.
    pub fn run_until(&mut self, end: Instant) {
        while let Some(next) = self.next() {
            let key = match next {
                Next::Dynamic(key)
                | Next::Timeline(key)
                | Next::Deadline(key, _)
                | Next::Host(key, _)
                | Next::LocalEcho(key, _) => key,
            };
            if key.at > end {
                break;
            }
            self.advance(key.at);
            match next {
                Next::Dynamic(_) => {
                    let (_, event) = self.dynamic.pop().expect("peeked");
                    self.dispatch(event);
                }
                Next::Timeline(_) => {
                    let (_, event) = self.timeline.pop().expect("peeked");
                    self.dispatch(event);
                }
                Next::Deadline(_, node) => self.service_deadline(node),
                Next::Host(_, node) => self.host_service(node),
                Next::LocalEcho(_, node) => self.local_echo_service(node),
            }
            self.check_holders();
        }
        self.advance(end.max(self.now));
    }

    /// Run for `length` of virtual time.
    pub fn run_for(&mut self, length: Duration) {
        self.run_until(self.now + length);
    }

    /// Current virtual time.
    #[must_use]
    pub fn now(&self) -> Instant {
        self.now
    }

    /// The line's bit timing.
    #[must_use]
    pub fn timing(&self) -> WireTiming {
        self.timing
    }

    /// The run's seeded generator, for scenario choices.
    pub fn rng(&mut self) -> &mut SimRng {
        &mut self.rng
    }

    /// Every transmission that has ended, in order of its end.
    #[must_use]
    pub fn frames(&self) -> &[WireFrame] {
        &self.frames
    }

    /// The NPDUs a master's core has delivered.
    #[must_use]
    pub fn deliveries(&self, station: u8) -> &[Delivery] {
        &self.master(station).deliveries
    }

    /// The NPDUs a master's core refused to queue.
    #[must_use]
    pub fn rejects(&self, station: u8) -> &[(Instant, QueueRefusal)] {
        &self.master(station).rejects
    }

    /// A master's core, while it is powered.
    #[must_use]
    pub fn core(&self, station: u8) -> Option<&SimCore> {
        self.master(station).core.as_deref()
    }

    /// A master's state, while it is powered.
    #[must_use]
    pub fn state(&self, station: u8) -> Option<MasterState> {
        self.core(station).map(SimCore::state)
    }

    /// The invariant report so far.
    #[must_use]
    pub fn report(&self) -> &Report {
        &self.report
    }

    /// The trace.
    #[must_use]
    pub fn trace(&self) -> &Trace {
        &self.trace
    }

    /// Whether a fault, a malformed scripted frame or a collision has
    /// happened since the run began or was last [settled](Self::settle).
    /// Until then every listening master must count every clean frame, and
    /// no two masters may hold the token at once.
    #[must_use]
    pub fn perturbed(&self) -> bool {
        self.perturbed
    }

    /// Declare the bus settled after a fault, and re-arm the checks a fault
    /// suspends: from now on every listening master must count every clean
    /// frame again, and a duplicate token is a violation outright.
    ///
    /// Call it once nothing is in flight: no armed fault, no node about to
    /// power up, and the token has gone round the ring with every frame
    /// clean, so every receiver's decoder is back in step.
    ///
    /// # Panics
    ///
    /// If a corruption or cut-off is still armed; withdraw it first with
    /// [`Fault::Disarm`].
    pub fn settle(&mut self) {
        assert!(
            self.armed.is_empty(),
            "settle() with faults still armed: {:?}",
            self.armed
        );
        if self.perturbed {
            self.perturbed = false;
            self.note(
                Who::Sim,
                TraceKind::Note("bus settled: strict checks re-armed".into()),
            );
        }
    }

    /// The last `lines` trace entries, with any violations, for a failure
    /// message.
    #[must_use]
    pub fn dump(&self, lines: usize) -> String {
        let mut text = String::new();
        for violation in &self.report.violations {
            text.push_str(&format!("violation: {violation}\n"));
        }
        text.push_str(&format!("trace at {}:\n", crate::trace::Millis(self.now)));
        text.push_str(&self.trace.tail(lines));
        text
    }

    /// Panic with the trace if any invariant failed.
    ///
    /// # Panics
    ///
    /// When [`Report::violations`] is not empty.
    pub fn assert_no_violations(&self) {
        assert!(
            self.report.violations.is_empty(),
            "invariant violations:\n{}",
            self.dump(PANIC_TRACE_LINES)
        );
    }

    fn claim_station(&self, station: u8) {
        assert!(
            self.nodes.iter().all(|node| node.station != station),
            "station {station} is already on the bus"
        );
    }

    fn index_of(&self, station: u8) -> usize {
        self.nodes
            .iter()
            .position(|node| node.station == station)
            .unwrap_or_else(|| panic!("no station {station} on the bus"))
    }

    fn master(&self, station: u8) -> &MasterNode {
        match &self.nodes[self.index_of(station)].kind {
            NodeKind::Master(master) => master,
            NodeKind::Scripted(_) => panic!("station {station} is scripted, not a master"),
        }
    }

    fn master_mut(&mut self, node: usize) -> Option<&mut MasterNode> {
        match &mut self.nodes[node].kind {
            NodeKind::Master(master) => Some(master),
            NodeKind::Scripted(_) => None,
        }
    }

    fn schedule(&mut self, at: Instant, class: Class, tie: impl Into<u16>, event: Event) {
        let key = self.key(at, class, tie.into());
        self.timeline.push(key, event);
    }

    /// Queue a line, adapter, application or script event, and check the
    /// queue stays bounded.
    fn push(&mut self, at: Instant, class: Class, tie: impl Into<u16>, event: Event) {
        let key = self.key(at, class, tie.into());
        self.dynamic.push(key, event);
        let pending = self.dynamic.len();
        self.report.max_pending_events = self.report.max_pending_events.max(pending);
        let bound = 16 + 4 * self.nodes.len() + self.line.live_count();
        if pending > bound {
            self.violation(
                ViolationKind::EventGrowth,
                format!("{pending} pending events, bound {bound}"),
            );
        }
    }

    fn key(&mut self, at: Instant, class: Class, tie: u16) -> Key {
        self.seq += 1;
        Key {
            at,
            class,
            tie,
            seq: self.seq,
        }
    }

    fn next(&self) -> Option<Next> {
        let mut best = self.dynamic.peek_key().map(Next::Dynamic);
        let mut consider = |candidate: Next| {
            let key = |next: &Next| match next {
                Next::Dynamic(key)
                | Next::Timeline(key)
                | Next::Deadline(key, _)
                | Next::Host(key, _)
                | Next::LocalEcho(key, _) => *key,
            };
            if best
                .as_ref()
                .is_none_or(|current| key(&candidate) < key(current))
            {
                best = Some(candidate);
            }
        };
        if let Some(key) = self.timeline.peek_key() {
            consider(Next::Timeline(key));
        }
        for (index, node) in self.nodes.iter().enumerate() {
            let NodeKind::Master(master) = &node.kind else {
                continue;
            };
            if let Some(at) = master
                .host
                .as_ref()
                .and_then(host::HostState::next_local_echo)
            {
                consider(Next::LocalEcho(
                    Key {
                        at: at.max(self.now),
                        class: Class::LocalEcho,
                        tie: u16::from(node.station),
                        seq: 0,
                    },
                    index,
                ));
            }
            if let Some(at) = master.host.as_ref().and_then(host::HostState::next) {
                consider(Next::Host(
                    Key {
                        at: at.max(self.now),
                        class: Class::HostRead,
                        tie: u16::from(node.station),
                        seq: 0,
                    },
                    index,
                ));
            }
            // The host transport awaits write/drain/echo without polling core
            // deadlines. An octet adapter must still service its watchdog.
            if master.transmitting.is_some() && master.host.is_some() {
                continue;
            }
            if let Some(fire_at) = master.fire_at {
                let key = Key {
                    at: fire_at,
                    class: Class::Deadline,
                    tie: u16::from(node.station),
                    seq: 0,
                };
                consider(Next::Deadline(key, index));
            }
        }
        best
    }

    fn advance(&mut self, at: Instant) {
        assert!(at >= self.now, "virtual time ran backwards");
        if at == self.same_instant.0 {
            self.same_instant.1 += 1;
            assert!(
                self.same_instant.1 < SAME_INSTANT_LIMIT,
                "the run is stuck at one instant\n{}",
                self.dump(PANIC_TRACE_LINES)
            );
        } else {
            self.same_instant = (at, 0);
        }
        self.now = at;
    }

    fn dispatch(&mut self, event: Event) {
        match event {
            Event::WindowEnd => self.window_end(),
            Event::OctetStart { tx, index } => self.octet_start(tx, index),
            Event::TxEnd { tx } => self.tx_end(tx),
            Event::DriverRelease { tx } => {
                self.release_driver(tx, crate::DriverReleaseCause::Completed)
            }
            Event::AdapterStart {
                node,
                id,
                incarnation,
            } => {
                if self
                    .master_mut(node)
                    .is_some_and(|master| master.incarnation == incarnation)
                {
                    self.adapter_start(node, id);
                }
            }
            Event::Decide {
                node,
                incarnation,
                request,
                requester,
                postpone,
                payload,
            } => {
                if self
                    .master_mut(node)
                    .is_some_and(|master| master.incarnation == incarnation)
                {
                    self.decide(node, request, requester, postpone, payload);
                }
            }
            Event::ScriptSend { node, send } => self.script_send(node, send),
            Event::AppSend {
                node,
                incarnation,
                destination,
                npdu,
            } => {
                // Dropped, not refused, if the node lost power since: its
                // application's state went with it.
                if self.master_mut(node).is_some_and(|master| {
                    master.incarnation == incarnation && master.core.is_some()
                }) {
                    self.enqueue_now(node, destination, &npdu, NetworkPriority::Normal);
                }
            }
            Event::Enqueue {
                node,
                destination,
                npdu,
                priority,
            } => self.enqueue_now(node, destination, &npdu, priority),
            Event::Fault(fault) => self.apply_fault(fault),
        }
    }

    fn note(&mut self, who: Who, kind: TraceKind) {
        self.trace.push(self.now, who, kind);
    }

    fn perturb(&mut self, reason: &str) {
        if !self.perturbed {
            self.perturbed = true;
            self.note(
                Who::Sim,
                TraceKind::Note(format!("run perturbed: {reason}")),
            );
        }
    }

    fn violation(&mut self, kind: ViolationKind, detail: String) {
        self.violation_with_host(kind, detail, None);
    }

    fn violation_with_host(
        &mut self,
        kind: ViolationKind,
        detail: String,
        host_timing: Option<crate::HostTimingFailure>,
    ) {
        let violation = Violation {
            at: self.now,
            kind,
            detail,
            host_timing,
        };
        self.note(Who::Sim, TraceKind::Violation(violation.to_string()));
        self.report.violations.push(violation);
        if self.config.panic_on_violation {
            panic!(
                "MS/TP simulator invariant failed\n{}",
                self.dump(PANIC_TRACE_LINES)
            );
        }
    }
}

/// Caller-owned storage that lives as long as the run. The core borrows its
/// receive buffer for its whole life, so each powered-on core gets one; a
/// scenario powers nodes on a bounded number of times.
fn leak_storage(length: usize) -> &'static mut [u8] {
    Box::leak(vec![0u8; length].into_boxed_slice())
}

/// The storage a powered-on core borrows for its whole life, kept like
/// [`leak_storage`]'s, around `queues`.
fn leak_core_storage(queues: AllocQueues<MAX_EXTENDED_MPDU_DATA>) -> &'static mut SimStorage {
    Box::leak(Box::new(SimStorage::with_queues(queues)))
}

/// Ties at one instant go to the lower station; the noise source and the line
/// sort after every station.
fn tie_of(driver: Driver) -> u16 {
    match driver {
        Driver::Station(station) => u16::from(station),
        Driver::Noise(_) => 300,
    }
}

/// The header and decoded data of the octets of one whole frame, standard
/// or extended (decoded on a copy, since COBS decodes in place), or `None`
/// when they aren't exactly one valid frame of a type the codec decodes.
pub(crate) fn decode_whole(octets: &[u8]) -> Option<(FrameHeader, Vec<u8>)> {
    let mut copy = octets.to_vec();
    match bacnet_mstp_codec::decode_frame_in_place(&mut copy) {
        Ok((frame, used)) if used == octets.len() => Some((frame.header, frame.data.to_vec())),
        _ => None,
    }
}

/// A one-line summary of a frame's octets for the trace.
fn describe(octets: &[u8]) -> String {
    match decode_whole(octets) {
        Some((header, _)) => format!(
            "{:?} {}->{} ({} octets)",
            header.frame_type,
            header.source,
            header.destination,
            octets.len()
        ),
        None => format!("{} undecodable octets", octets.len()),
    }
}
