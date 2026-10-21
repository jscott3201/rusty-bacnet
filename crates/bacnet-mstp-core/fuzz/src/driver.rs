//! Drives one core through the steps as an adapter would, and checks the
//! adapter contract on every action it returns.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use bacnet_mstp_codec::{
    FrameType, BROADCAST_MAC, MAX_EXTENDED_FRAME_LENGTH, MAX_EXTENDED_MPDU_DATA,
    MAX_STANDARD_FRAME_LENGTH, MAX_STANDARD_MPDU_DATA,
};
use bacnet_mstp_core::{
    Action, ActionBuffers, AllocQueues, Direction, FixedQueues, Instant, MasterCore,
    MstpCoreConfig, QueueStorage, RequestId, TransmitId, MIN_RX_STORAGE,
};

use crate::checker::{Checker, Delivered};
use crate::input::{Input, Layout, MAX_STEPS};
use crate::line::{Heard, Line};
use crate::model::{Builder, Params, Seen};
use crate::queues::{Logged, QueueLog};

mod steps;

/// Run one input.
pub fn run(input: &Input) {
    let config = input.setup.config();
    if let Err(error) = config.validate() {
        panic!("the setup made an invalid configuration: {error:?}");
    }
    let unicast = usize::from(input.setup.unicast_slots % 6);
    let broadcast = usize::from(input.setup.broadcast_slots % 4);
    let standard_rx = input
        .setup
        .short_rx
        .map_or(MAX_STANDARD_FRAME_LENGTH, |length| {
            MIN_RX_STORAGE + usize::from(length) % (MAX_STANDARD_FRAME_LENGTH - MIN_RX_STORAGE)
        });
    match input.setup.layout() {
        Layout::Standard => drive::<_, MAX_STANDARD_MPDU_DATA, 1, MAX_STANDARD_FRAME_LENGTH>(
            input,
            config,
            AllocQueues::<MAX_STANDARD_MPDU_DATA>::new(unicast, broadcast),
            [unicast, broadcast],
            standard_rx,
        ),
        Layout::Heapless => drive::<_, MAX_STANDARD_MPDU_DATA, 3, MAX_STANDARD_FRAME_LENGTH>(
            input,
            config,
            FixedQueues::<3, 2>::new(),
            [3, 2],
            standard_rx,
        ),
        Layout::Extended => drive::<_, MAX_EXTENDED_MPDU_DATA, 2, MAX_EXTENDED_FRAME_LENGTH>(
            input,
            config,
            AllocQueues::<MAX_EXTENDED_MPDU_DATA>::new(unicast, broadcast),
            [unicast, broadcast],
            MAX_EXTENDED_FRAME_LENGTH,
        ),
    }
}

fn drive<Q: QueueStorage, const NPDU: usize, const SLOTS: usize, const FRAME: usize>(
    input: &Input,
    config: MstpCoreConfig,
    queues: Q,
    capacity: [usize; 2],
    rx_length: usize,
) {
    let params = Params::new(&config);
    let log = Rc::new(RefCell::new(QueueLog::default()));
    let queues = Logged::new(
        queues,
        capacity[0],
        capacity[1],
        params.max_npdu,
        Rc::clone(&log),
    );
    let mut rx = vec![0; rx_length];
    let mut shadow = vec![0; rx_length];
    let mut buffers = Box::new(ActionBuffers::<SLOTS, NPDU, FRAME>::new());
    let start = std::ptr::addr_of!(*buffers) as usize;
    let span = start..start + std::mem::size_of::<ActionBuffers<SLOTS, NPDU, FRAME>>();
    let mut core = MasterCore::from_parts(config, queues, &mut rx, &mut *buffers, Instant::ZERO)
        .unwrap_or_else(|error| panic!("the core refused the setup: {error:?}"));
    let line = Line::new(&mut shadow, params.extended, params.frame_abort);
    let mut driver = Driver {
        peer: params.next(params.station),
        checker: Checker::new(params, line, Rc::clone(&log)),
        tx: None,
        requests: Vec::new(),
        now: 0,
        stamp: 0,
        skew: 0,
        slots: SLOTS,
        npdu: NPDU,
        frame: FRAME,
        span,
        lazy: input.setup.lazy,
        log,
    };
    for (index, step) in input.steps.iter().take(MAX_STEPS).enumerate() {
        driver.checker.step = index;
        if tracing() {
            eprintln!("step {index} at {} us: {step:?}", driver.now);
        }
        driver.step(&mut core, step);
        if tracing() {
            eprintln!(
                "  core {:?}; checker {}",
                core.snapshot(),
                driver.checker.describe()
            );
        }
        driver.check_queue();
    }
    driver.drain(&mut core);
}

/// Whether `MSTP_CORE_FUZZ_TRACE` is set: each step and each frame handed
/// over then goes to stderr, for replaying a failing input.
fn tracing() -> bool {
    static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *TRACE.get_or_init(|| std::env::var_os("MSTP_CORE_FUZZ_TRACE").is_some())
}

/// The frame the adapter holds.
#[derive(Debug)]
struct Tx {
    id: TransmitId,
    octets: Vec<u8>,
    /// When its start was confirmed.
    started: Option<u64>,
}

/// The adapter, and the checker it reports to.
struct Driver<'s> {
    checker: Checker<'s>,
    tx: Option<Tx>,
    /// Request ids the core handed up, newest last.
    requests: Vec<RequestId>,
    /// The time, in whole microseconds; the core gets it in nanoseconds.
    now: u64,
    /// The time of the last call the core took a time from.
    stamp: u64,
    /// How much earlier than `now` the core is told a call happens: zero,
    /// except for a regressed call (see `Step::Regress`).
    skew: u64,
    slots: usize,
    npdu: usize,
    frame: usize,
    /// Where the action buffers lie in memory.
    span: Range<usize>,
    /// Line inputs leave their actions waiting until the next other call.
    lazy: bool,
    log: Rc<RefCell<QueueLog>>,
    /// The station this one last sent a frame to.
    peer: u8,
}

impl<'s> Driver<'s> {
    fn fail(&self, what: &str) -> ! {
        self.checker.fail(what)
    }

    /// The time the core is told: `now`, less the skew of a regressed call.
    fn at(&self) -> Instant {
        Instant::from_micros(self.now - self.skew)
    }

    fn params(&self) -> Params {
        self.checker.params
    }

    fn builder(&self) -> Builder {
        Builder {
            station: self.params().station,
            peer: self.peer,
        }
    }

    /// Whether the station's driver is on: what the receiver hears is echo.
    fn transmitting(&self) -> bool {
        self.tx.as_ref().is_some_and(|tx| tx.started.is_some())
    }

    fn check_queue(&self) {
        if let Some(broken) = self.log.borrow_mut().broken.take() {
            self.fail(&broken);
        }
    }

    /// Line input: echo while transmitting, which must change nothing;
    /// otherwise activity the checker hears too.
    fn line_input<'c, Q: QueueStorage, const N: usize>(
        &mut self,
        core: &mut MasterCore<'c, Q, N>,
        feed: impl FnOnce(&mut MasterCore<'c, Q, N>, Instant),
        hear: impl FnOnce(&mut Line<'s>, u64, &mut Vec<Heard>),
    ) {
        let now = self.now;
        if self.transmitting() {
            let before = (core.snapshot(), core.counters());
            feed(core, self.at());
            self.stamp = now;
            if (core.snapshot(), core.counters()) != before {
                self.fail("the station's own echo changed the core");
            }
            return;
        }
        self.checker.tick(now);
        feed(core, self.at());
        self.stamp = now;
        let mut heard = Vec::new();
        hear(&mut self.checker.line, now, &mut heard);
        self.checker.heard(heard, now);
        if !self.lazy {
            self.drain(core);
        }
    }

    fn octet<Q: QueueStorage, const N: usize>(
        &mut self,
        core: &mut MasterCore<'_, Q, N>,
        octet: u8,
    ) {
        self.line_input(
            core,
            |core, now| core.on_octet(octet, now),
            |line, now, heard| line.octet(octet, now, heard),
        );
    }

    /// A frame from a host assembler, which the core must take exactly when
    /// the codec would have decoded it: source 255 and, without extended
    /// frames, a COBS type are refused, as is more data than its type takes.
    fn validated<Q: QueueStorage, const N: usize>(
        &mut self,
        core: &mut MasterCore<'_, Q, N>,
        frame: Seen,
    ) {
        let params = self.params();
        let frame_type = frame.frame_type;
        let longest = match frame_type {
            _ if !(params.extended && frame_type.is_cobs()) => MAX_STANDARD_MPDU_DATA,
            _ if frame_type.is_extended_data() => MAX_EXTENDED_MPDU_DATA,
            _ => usize::MAX,
        };
        let taken = frame.source != BROADCAST_MAC
            && !(frame_type.is_cobs() && !params.extended)
            && frame.data.len() <= longest;
        let now = self.now;
        let echo = self.transmitting();
        if !echo && taken {
            self.checker.tick(now);
        }
        let before = (core.snapshot(), core.counters());
        core.on_validated_frame(frame.frame(), self.at());
        if taken {
            self.stamp = now;
        } else {
            // Refused: counted as the adapter's error, and nothing else.
            let mut expected = before;
            expected.1.adapter_errors += 1;
            if (core.snapshot(), core.counters()) != expected {
                self.fail(&format!(
                    "a host assembler's {frame:?}, which the codec would refuse, \
                     was not refused as the adapter's error alone"
                ));
            }
            return;
        }
        if echo {
            if (core.snapshot(), core.counters()) != before {
                self.fail("the station's echo changed the core");
            }
            return;
        }
        let mut heard = Vec::new();
        self.checker.line.validated(frame, now, &mut heard);
        self.checker.heard(heard, now);
        if !self.lazy {
            self.drain(core);
        }
    }

    /// Poll actions until idle.
    fn drain<Q: QueueStorage, const N: usize>(&mut self, core: &mut MasterCore<'_, Q, N>) {
        let mut deliveries = 0;
        let mut direction = false;
        loop {
            let action = core.poll_action();
            if direction && !matches!(action, Action::Transmit { .. }) {
                self.fail(&format!("a switch to transmit, then {action:?}"));
            }
            match action {
                Action::Idle => return,
                Action::SetDirection(Direction::Transmit) => {
                    if let Some(tx) = &self.tx {
                        self.fail(&format!("a second frame while {tx:?} is pending or active"));
                    }
                    direction = true;
                }
                Action::SetDirection(Direction::Receive) => match self.tx.take() {
                    Some(Tx {
                        started: Some(_), ..
                    }) => self.checker.aborted(self.now),
                    Some(_) => self.checker.withdrawn(),
                    None => {}
                },
                Action::Transmit { id, header, length } => {
                    direction = false;
                    if length > self.frame {
                        self.fail(&format!(
                            "a transmit of {length} octets, over {}",
                            self.frame
                        ));
                    }
                    // An adapter gets the octets with the confirmed start;
                    // the checker sees them now, through `inspect`, and the
                    // start must hand over the same.
                    let octets = match core.handed_over_frame() {
                        Some((handed, frame)) if handed == id => {
                            self.check_span(frame, self.frame, "a transmit");
                            frame.to_vec()
                        }
                        other => self.fail(&format!("transmit {id:?} handed over as {other:?}")),
                    };
                    if (
                        octets.len(),
                        Seen::decode(&octets).map(|seen| seen.frame().header),
                    ) != (length, Ok(header))
                    {
                        self.fail("the transmit action misnames its frame");
                    }
                    if tracing() {
                        eprintln!("  {} us: transmit {:?}", self.now, Seen::decode(&octets));
                    }
                    self.checker.handover(&octets, self.now);
                    if let Ok(frame) = Seen::decode(&octets) {
                        if frame.destination != BROADCAST_MAC {
                            self.peer = frame.destination;
                        }
                    }
                    self.tx = Some(Tx {
                        id,
                        octets,
                        started: None,
                    });
                }
                Action::DeliverNpdu {
                    source,
                    destination,
                    npdu,
                    request_id,
                } => {
                    deliveries += 1;
                    self.check_delivery(deliveries, npdu);
                    self.checker
                        .delivered(Delivered::Npdu, source, destination, npdu, request_id);
                    if let Some(id) = request_id {
                        if self.requests.len() == 8 {
                            self.requests.remove(0);
                        }
                        self.requests.push(id);
                    }
                }
                Action::DeliverFrame {
                    frame_type,
                    source,
                    destination,
                    data,
                } => {
                    deliveries += 1;
                    self.check_delivery(deliveries, data);
                    self.checker.delivered(
                        Delivered::Frame(frame_type),
                        source,
                        destination,
                        data,
                        None,
                    );
                }
            }
        }
    }

    fn check_delivery(&self, deliveries: usize, data: &[u8]) {
        if deliveries > self.slots {
            self.fail(&format!(
                "{deliveries} deliveries came out between two calls, with {} slots",
                self.slots
            ));
        }
        self.check_span(data, self.npdu, "a delivery");
    }

    /// An action's octets lie in the action buffers and fit their slot.
    fn check_span(&self, octets: &[u8], longest: usize, what: &str) {
        let start = octets.as_ptr() as usize;
        if octets.len() > longest || start < self.span.start || start + octets.len() > self.span.end
        {
            self.fail(&format!(
                "{what} of {} octets at {start:#x}, outside the action buffers {:#x?} or longer than {longest}",
                octets.len(),
                self.span
            ));
        }
    }

    /// A Token for this station from the next station, for a regressed call.
    fn token_frame(&self) -> Seen {
        let params = self.params();
        Seen {
            frame_type: FrameType::TOKEN,
            destination: params.station,
            source: params.next(params.station),
            data: Vec::new(),
        }
    }
}
