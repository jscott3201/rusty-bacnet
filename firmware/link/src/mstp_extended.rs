//! The mstp-extended image's scenario (#1664).
//!
//! [`run`] drives `bacnet-mstp-core` with
//! [`MstpCoreConfig::extended_frames`] on, as a routing node needs it: the
//! frame types 32 and 33, which carry NPDUs of 502 to 1497 octets
//! COBS-encoded. Every buffer the core borrows is sized for them, as the
//! flag requires, in one [`ExtendedStorage`]: 1516 octets of receive
//! storage, the action buffers and queues whose slots hold 1497 octets. It
//! returns a [`Summary`] of what it saw.
//!
//! The scenario is one extended exchange with the peer, driven the way a
//! UART adapter would drive it, on injected `bacnet-time` instants:
//!
//! - **Type 32:** a queued request of [`REQUEST_LENGTH`] octets, too long
//!   for a standard frame, goes out in a BACnet Extended Data Expecting
//!   Reply frame once the peer passes the token. Decoded from the octets
//!   the core sent, it is the NPDU queued.
//! - **Type 33:** the peer answers with a BACnet Extended Data Not
//!   Expecting Reply frame of [`REPLY_LENGTH`] octets, fed to the core
//!   octet by octet, and the core delivers the NPDU it carried.
//! - Then the core polls for a master, the peer answers, and the core
//!   passes it the token.
//!
//! [`Memory`] holds only what the core borrows, so the image's `.bss` is
//! the core's storage. The scenario's own buffers, for the NPDUs and the
//! frames it builds or checks, live on the stack of functions kept out of
//! line, and only while those run. The core sends a queued NPDU from the
//! queue's slot, with nothing NPDU-sized on its stack (#1691), so the stack
//! peaks while the core takes in the peer's reply, with the scenario's
//! buffers for it live. On the 16 KiB boards of the emulation check, what
//! RAM the storage leaves is the stack's.
//!
//! `tests/mstp_extended.rs` runs it natively and compares the whole summary
//! with [`EXPECTED`]; the `mstp-extended` image
//! (`src/bin/mstp_extended.rs`) links it, and its emulation build checks the
//! emulated CPU's summary against the same.

mod expected;

pub use expected::EXPECTED;

use bacnet_mstp_codec::{decode_frame_in_place, Frame, FrameType, MAX_EXTENDED_MPDU_DATA};
use bacnet_mstp_core::npdu::NetworkPriority;
use bacnet_mstp_core::{
    Action, Duration, ExtendedStorage, FixedQueues, Instant, MasterCore, MasterState,
    MstpCoreConfig, TransmitId,
};

/// Unicast slots in the transmit queue: one, so the image fits a 16 KiB
/// part with room for its stack. Each slot holds 1497 octets.
pub const UNICAST_SLOTS: usize = 1;
/// Broadcast slots in the transmit queue.
pub const BROADCAST_SLOTS: usize = 1;
/// Slots for received data waiting to be polled. One is enough: the
/// scenario polls every action after each call into the core.
pub const DELIVERY_SLOTS: usize = 1;

/// The core's transmit queues: fixed arrays, with slots for extended frames.
pub type Queues = FixedQueues<UNICAST_SLOTS, BROADCAST_SLOTS, MAX_EXTENDED_MPDU_DATA>;

/// The storage the core borrows: its receive buffer, queues and action
/// buffers, all sized for extended frames.
pub type Storage = ExtendedStorage<UNICAST_SLOTS, BROADCAST_SLOTS, DELIVERY_SLOTS>;

/// The master core as the image configures it: it borrows its [`Storage`],
/// so that can live in a static and the core itself is small.
pub type Core<'m> = MasterCore<'m, &'m mut Queues, MAX_EXTENDED_MPDU_DATA>;

/// The request this station sends, in octets: more than a standard frame's
/// 501.
pub const REQUEST_LENGTH: usize = 512;
/// The peer's reply, in octets.
pub const REPLY_LENGTH: usize = 600;

/// The storage the core borrows, and nothing else.
///
/// [`Memory::new`] is all zero bits, so the image keeps it in a static that
/// the linker places in `.bss`, costing no flash for an initial image.
pub struct Memory {
    /// The core's [`Storage`].
    core: Storage,
}

impl Memory {
    /// Zeroed storage.
    pub const fn new() -> Self {
        Self {
            core: Storage::new(),
        }
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

/// Room for the frames the core sends during the scenario.
pub const SENT_FRAMES: usize = 4;

/// The type of a frame the core sent, its destination, and its length on
/// the wire.
pub type Sent = (FrameType, u8, u16);

/// What [`run`] saw. Every field is plain data, so the host test can compare
/// the whole summary at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    /// The longest NPDU the core sends and takes.
    pub max_npdu: usize,
    /// Each frame the core sent, in order.
    pub sent: [Option<Sent>; SENT_FRAMES],
    /// Whether the request's frame, decoded from the octets the core sent,
    /// came from this station to the peer with the NPDU queued.
    pub request_intact: bool,
    /// The NPDU the core delivered: its source, its destination and its
    /// length.
    pub delivered: Option<(u8, u8, u16)>,
    /// Whether it is the NPDU the peer sent.
    pub reply_intact: bool,
    /// Requests it handed up for an answer.
    pub requests: u8,
    /// Direction changes it asked for.
    pub direction_changes: u8,
    /// Its state at the end.
    pub state: MasterState,
    /// Core counter: valid frames received.
    pub frames_received: u32,
    /// Core counter: frames transmitted.
    pub frames_transmitted: u32,
    /// Core counter: tokens passed to a successor.
    pub token_rotations: u32,
    /// Core counter: invalid frames received.
    pub parser_errors: u32,
    /// Calls that failed where the scenario expects success. Zero in a good
    /// run.
    pub errors: u8,
}

impl Summary {
    const fn new() -> Self {
        Self {
            max_npdu: 0,
            sent: [None; SENT_FRAMES],
            request_intact: false,
            delivered: None,
            reply_intact: false,
            requests: 0,
            direction_changes: 0,
            state: MasterState::Idle,
            frames_received: 0,
            frames_transmitted: 0,
            token_rotations: 0,
            parser_errors: 0,
            errors: 0,
        }
    }

    /// Count a call that failed where the scenario expects success.
    fn error(&mut self) {
        self.errors = self.errors.saturating_add(1);
    }
}

/// This station.
const STATION: u8 = 1;
/// The peer, which hands this station the token and answers its request.
const PEER: u8 = 2;

/// Station 1 of masters 0-2, at the standard 9600 baud, with extended
/// frames on.
const CONFIG: MstpCoreConfig = {
    let mut config = MstpCoreConfig::STANDARD;
    config.station = STATION;
    config.max_master = 2;
    config.with_extended_frames()
};

/// An NPDU the scenario sends: version 1, a control octet, then octets that
/// step through a pattern, so it can be checked without a copy.
struct Npdu {
    length: usize,
    control: u8,
    step: usize,
}

impl Npdu {
    /// The octet at `index`.
    fn octet(&self, index: usize) -> u8 {
        match index {
            0 => 0x01,
            1 => self.control,
            // Below 251, so the cast keeps every bit.
            _ => (index.wrapping_mul(self.step) % 251) as u8,
        }
    }

    /// Write it to the front of `out`, which must hold it, and return it.
    fn write<'o>(&self, out: &'o mut [u8]) -> &'o [u8] {
        let npdu = out.get_mut(..self.length).unwrap_or_default();
        for (index, octet) in npdu.iter_mut().enumerate() {
            *octet = self.octet(index);
        }
        npdu
    }

    /// Whether `npdu` is this one.
    fn is(&self, npdu: &[u8]) -> bool {
        npdu.len() == self.length
            && npdu
                .iter()
                .enumerate()
                .all(|(index, &octet)| octet == self.octet(index))
    }
}

/// The request: its control octet asks for a reply.
const REQUEST: Npdu = Npdu {
    length: REQUEST_LENGTH,
    control: 0x04,
    step: 1,
};
/// The reply.
const REPLY: Npdu = Npdu {
    length: REPLY_LENGTH,
    control: 0x00,
    step: 7,
};

/// Octets of an extended frame carrying `npdu` octets: the preamble and
/// header (8), the data with COBS's overhead of one octet in 254, and the
/// encoded CRC-32K (5).
const fn extended_frame(npdu: usize) -> usize {
    8 + npdu + npdu.div_ceil(254) + 5
}

/// Octets of a frame without data.
const BARE_FRAME: usize = 8;

/// The most actions one stimulus may produce before the scenario counts an
/// error and moves on, so a runaway core can't hang it.
const MAX_ACTIONS: usize = 16;

/// Run the whole scenario in `memory`. The core is built on this function's
/// stack, borrowing its large buffers from `memory`.
///
/// The scenario never panics on a failed call: it counts the failure in
/// [`Summary::errors`] and carries on, or returns early when it cannot
/// continue.
pub fn run(memory: &mut Memory) -> Summary {
    let mut summary = Summary::new();
    summary.max_npdu = CONFIG.max_npdu();
    let mut line = Line {
        now: Instant::ZERO,
        octet: CONFIG.octet_time(),
        summary: &mut summary,
    };
    let Ok(mut core) = MasterCore::new(CONFIG, &mut memory.core, line.now) else {
        line.summary.error();
        return summary;
    };
    line.exchange(&mut core);
    record_counters(&mut core, &mut summary);
    summary
}

/// Take the core's counters, as firmware drains them, and copy the ones
/// the summary keeps: the scenario takes them once, so they are its totals.
fn record_counters(core: &mut Core<'_>, summary: &mut Summary) {
    let counters = core.take_counters();
    summary.frames_received = counters.frames_received;
    summary.frames_transmitted = counters.frames_transmitted;
    summary.token_rotations = counters.token_rotations;
    summary.parser_errors = counters.parser_errors;
    // Inputs the core ignored as broken: none in a good run.
    let broken = counters
        .adapter_errors
        .saturating_add(counters.internal_errors);
    summary.errors = summary
        .errors
        .saturating_add(u8::try_from(broken).unwrap_or(u8::MAX));
}

/// The line between this station and the peer: injected time, advanced by
/// the octets on it and by waits.
struct Line<'m> {
    now: Instant,
    /// One octet's time at the line's speed.
    octet: Duration,
    summary: &'m mut Summary,
}

/// What one polled action leaves for the adapter to do once the action's
/// borrow of the core has ended.
enum Then {
    Nothing,
    Send { id: TransmitId },
}

impl Line<'_> {
    /// The exchange: the request out in a Type 32 frame, the reply in a
    /// Type 33, then the token on to the peer.
    fn exchange(&mut self, core: &mut Core<'_>) {
        self.enqueue_request(core);
        // The peer passes the token, and the request goes out once the line
        // has been quiet long enough.
        self.receive_bare(core, FrameType::TOKEN);
        self.wait(core);
        // The peer's reply, a longer NPDU in an extended frame.
        self.receive_reply(core);
        // With no successor known, the core polls the next station for a
        // master, which is the peer. It answers, so the token goes to it.
        self.wait(core);
        self.receive_bare(core, FrameType::REPLY_TO_POLL_FOR_MASTER);
        self.wait(core);
        self.summary.state = core.state();
    }

    /// Queue the request for the peer, built in a buffer that lasts only
    /// for this call: the queue keeps a copy.
    #[inline(never)]
    fn enqueue_request(&mut self, core: &mut Core<'_>) {
        let mut buffer = [0; REQUEST_LENGTH];
        let request = REQUEST.write(&mut buffer);
        if core
            .enqueue_npdu(PEER, request, NetworkPriority::Normal, self.now)
            .is_err()
        {
            self.summary.error();
        }
    }

    /// The peer sends a frame of `frame_type`, without data.
    fn receive_bare(&mut self, core: &mut Core<'_>, frame_type: FrameType) {
        let mut wire = [0; BARE_FRAME];
        self.receive(core, Frame::new(frame_type, STATION, PEER, &[]), &mut wire);
    }

    /// The peer sends its reply in a Type 33 frame, built in buffers that
    /// last only for this call.
    #[inline(never)]
    fn receive_reply(&mut self, core: &mut Core<'_>) {
        let mut buffer = [0; REPLY_LENGTH];
        let mut wire = [0; extended_frame(REPLY_LENGTH)];
        let reply = REPLY.write(&mut buffer);
        let frame = Frame::new(
            FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY,
            STATION,
            PEER,
            reply,
        );
        self.receive(core, frame, &mut wire);
    }

    /// Encode `frame` into `wire` and feed it to the core octet by octet, as
    /// the peer sends it, then carry out what the core asks for.
    fn receive(&mut self, core: &mut Core<'_>, frame: Frame<'_>, wire: &mut [u8]) {
        let Ok(length) = frame.encode_into(wire) else {
            self.summary.error();
            return;
        };
        for &octet in wire.iter().take(length) {
            self.now += self.octet;
            core.on_octet(octet, self.now);
        }
        self.drain(core);
    }

    /// Let time run to the core's next deadline, service it, and carry out
    /// what the core asks for.
    fn wait(&mut self, core: &mut Core<'_>) {
        let Some(deadline) = core.next_deadline() else {
            self.summary.error();
            return;
        };
        self.now = self.now.max(deadline);
        core.on_deadline(self.now);
        self.drain(core);
    }

    /// Poll actions until the core has none, carrying out each one: a
    /// transmit starts at once and takes its octets' time on the line.
    fn drain(&mut self, core: &mut Core<'_>) {
        for _ in 0..MAX_ACTIONS {
            let then = match core.poll_action() {
                Action::Idle => return,
                Action::SetDirection(_) => {
                    self.summary.direction_changes += 1;
                    Then::Nothing
                }
                Action::Transmit { id, .. } => Then::Send { id },
                Action::DeliverNpdu {
                    source,
                    destination,
                    npdu,
                    request_id,
                } => {
                    let length = u16::try_from(npdu.len()).unwrap_or(u16::MAX);
                    self.summary.delivered = Some((source, destination, length));
                    self.summary.reply_intact = REPLY.is(npdu);
                    if request_id.is_some() {
                        self.summary.requests += 1;
                    }
                    Then::Nothing
                }
                Action::DeliverFrame { .. } => {
                    self.summary.error();
                    Then::Nothing
                }
            };
            if let Then::Send { id } = then {
                // The octets come with the confirmed start: send them from
                // the core's frame buffer.
                let octets = match core.on_transmit_started(id, self.now) {
                    Ok(frame) => {
                        self.record_sent(frame);
                        frame.len()
                    }
                    Err(_) => {
                        self.summary.error();
                        0
                    }
                };
                let octets = u32::try_from(octets).unwrap_or(u32::MAX);
                self.now += self.octet.saturating_mul(octets);
                core.on_transmit_complete(id, self.now);
            }
        }
        self.summary.error();
    }

    /// Note the type, destination and length of a frame the core sends,
    /// and for the request's frame, whether it carries the request.
    fn record_sent(&mut self, frame: &[u8]) {
        let (Some(&raw_type), Some(&destination)) = (frame.get(2), frame.get(3)) else {
            self.summary.error();
            return;
        };
        let frame_type = FrameType::from_raw(raw_type);
        if frame_type == FrameType::BACNET_EXTENDED_DATA_EXPECTING_REPLY {
            self.summary.request_intact = carries_request(frame);
        }
        let length = u16::try_from(frame.len()).unwrap_or(u16::MAX);
        match self.summary.sent.iter_mut().find(|slot| slot.is_none()) {
            Some(slot) => *slot = Some((frame_type, destination, length)),
            None => self.summary.error(),
        }
    }
}

/// Whether `frame`, decoded from a copy that lasts only for this call, goes
/// from this station to the peer with the request as its data.
#[inline(never)]
fn carries_request(frame: &[u8]) -> bool {
    let mut copy = [0; extended_frame(REQUEST_LENGTH)];
    let Some(copy) = copy.get_mut(..frame.len()) else {
        return false;
    };
    copy.copy_from_slice(frame);
    match decode_frame_in_place(copy) {
        Ok((decoded, used)) => {
            used == frame.len()
                && (decoded.header.destination, decoded.header.source) == (PEER, STATION)
                && REQUEST.is(decoded.data)
        }
        Err(_) => false,
    }
}
