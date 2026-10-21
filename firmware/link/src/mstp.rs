//! The MS/TP image's scenario.
//!
//! [`run`] drives `bacnet-mstp-codec` and `bacnet-mstp-core`, both with
//! default features plus the core's fixed queues, through the paths a
//! small MS/TP device uses. It returns a [`Summary`] of what it saw.
//!
//! - **Codec:** encode a few frame types, refuse one bad frame, and decode the
//!   stream with a [`StreamDecoder`](bacnet_mstp_codec::StreamDecoder),
//!   noise and a corrupted frame included.
//! - **Core:** take a token, send a queued data frame, poll for a master,
//!   pass the token on, then receive a broadcast and a Test_Response and
//!   answer a request. Time is injected, and every action the core returns
//!   is carried out. The core is built on the stack over storage in
//!   [`Memory`], the pattern for firmware.
//! - **Capacity:** fill the unicast queue and offer an NPDU too long for a
//!   standard frame.
//!
//! `tests/mstp.rs` runs it natively and compares the whole summary with
//! [`EXPECTED`]; the `mstp` image (`src/bin/mstp.rs`) links it, and its
//! emulation build checks the emulated CPU's summary against the same.

mod codec;
mod expected;
mod link;

pub use expected::EXPECTED;

use bacnet_mstp_codec::{FrameType, MAX_STANDARD_FRAME_LENGTH, MAX_STANDARD_MPDU_DATA};
use bacnet_mstp_core::{FixedQueues, MasterCore, MasterState, StandardStorage};

/// Unicast slots in the transmit queue.
pub const UNICAST_SLOTS: usize = 4;
/// Broadcast slots in the transmit queue.
pub const BROADCAST_SLOTS: usize = 2;
/// Slots for received data waiting to be polled. One is enough: the
/// scenario polls every action after each call into the core.
pub const DELIVERY_SLOTS: usize = 1;

/// The core's transmit queues: fixed arrays, with standard-frame slots.
pub type Queues = FixedQueues<UNICAST_SLOTS, BROADCAST_SLOTS>;

/// The storage the core borrows: its receive buffer, queues and action
/// buffers, all for standard frames.
pub type Storage = StandardStorage<UNICAST_SLOTS, BROADCAST_SLOTS, DELIVERY_SLOTS>;

/// The master core as the image configures it: it borrows its [`Storage`],
/// so that can live in a static and the core itself is small.
pub type Core<'m> = MasterCore<'m, &'m mut Queues>;

/// An NPDU with an unconfirmed Who-Is: version 1, no addressing, no reply
/// expected.
const WHO_IS: [u8; 4] = [0x01, 0x00, 0x10, 0x08];

/// Octets of encoded frames the scenario keeps at once.
const WIRE_LENGTH: usize = 128;

/// Storage the scenario works in besides its stack: the [`Storage`] the
/// core borrows (its receive buffer, queues and action buffers), the stream
/// decoder's buffer and an encode buffer.
///
/// [`Memory::new`] is all zero bits, so the image keeps it in a static that
/// the linker places in `.bss`, costing no flash for an initial image. This
/// is the pattern for firmware: the large buffers in a zeroed static, and the
/// core, which borrows them, built on the stack.
pub struct Memory {
    /// What the core borrows.
    core: Storage,
    /// The standalone stream decoder's storage. Once the codec part is done,
    /// its first octets serve as the oversized NPDU.
    stream: [u8; MAX_STANDARD_FRAME_LENGTH],
    /// Encoded frames on their way to a decoder.
    wire: [u8; WIRE_LENGTH],
}

impl Memory {
    /// Zeroed storage.
    pub const fn new() -> Self {
        Self {
            core: Storage::new(),
            stream: [0; MAX_STANDARD_FRAME_LENGTH],
            wire: [0; WIRE_LENGTH],
        }
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

/// Room for the frames the core sends during the scenario.
pub const SENT_FRAMES: usize = 8;

/// The type and destination of a frame the core sent.
pub type Sent = (FrameType, u8);

/// What [`run`] saw. Every field is plain data, so the host test can compare
/// the whole summary at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    /// Codec: frames encoded for the stream.
    pub frames_encoded: u8,
    /// Codec: frames the encoder refused.
    pub encode_refusals: u8,
    /// Codec: frames the stream decoder returned, each equal to the frame
    /// encoded for it.
    pub frames_decoded: u8,
    /// Codec: candidates the stream decoder dropped as malformed.
    pub frames_malformed: u8,
    /// Codec: octets the stream decoder discarded as line noise.
    pub noise_octets: u8,
    /// Core: direction changes it asked for.
    pub direction_changes: u8,
    /// Core: the type and destination of each frame it sent, in order.
    pub sent: [Option<Sent>; SENT_FRAMES],
    /// Core: NPDUs it delivered.
    pub npdus_delivered: u8,
    /// Core: frames it delivered whole (Test_Response and proprietary).
    pub frames_delivered: u8,
    /// Core: requests it handed up for an answer.
    pub requests: u8,
    /// Core: its state at the end of the token exchange.
    pub state: MasterState,
    /// Core counter: valid frames received.
    pub frames_received: u32,
    /// Core counter: frames transmitted.
    pub frames_transmitted: u32,
    /// Core counter: tokens passed to a successor.
    pub token_rotations: u32,
    /// Core counter: invalid frames received.
    pub parser_errors: u32,
    /// Capacity: NPDUs the unicast queue accepted before it was full.
    pub queued_before_full: u8,
    /// Capacity, core counter: unicast NPDUs refused because the queue was
    /// full.
    pub drops_full: u32,
    /// Capacity, core counter: unicast NPDUs refused as too long.
    pub drops_oversize: u32,
    /// Calls that failed where the scenario expects success. Zero in a good
    /// run.
    pub errors: u8,
}

impl Summary {
    const fn new() -> Self {
        Self {
            frames_encoded: 0,
            encode_refusals: 0,
            frames_decoded: 0,
            frames_malformed: 0,
            noise_octets: 0,
            direction_changes: 0,
            sent: [None; SENT_FRAMES],
            npdus_delivered: 0,
            frames_delivered: 0,
            requests: 0,
            state: MasterState::Idle,
            frames_received: 0,
            frames_transmitted: 0,
            token_rotations: 0,
            parser_errors: 0,
            queued_before_full: 0,
            drops_full: 0,
            drops_oversize: 0,
            errors: 0,
        }
    }

    /// Count a call that failed where the scenario expects success.
    fn error(&mut self) {
        self.errors = self.errors.saturating_add(1);
    }
}

/// Run the whole scenario in `memory`. The core is built on this function's
/// stack, borrowing its large buffers from `memory`.
///
/// The scenario never panics on a failed call: it counts the failure in
/// [`Summary::errors`] and carries on, or returns early when it cannot
/// continue.
pub fn run(memory: &mut Memory) -> Summary {
    let Memory { core, stream, wire } = memory;
    let mut summary = Summary::new();
    codec::run(stream, wire, &mut summary);
    // The decoder is done with its storage, which now serves as an NPDU one
    // octet longer than a standard frame carries.
    let oversize = stream.get(..MAX_STANDARD_MPDU_DATA + 1).unwrap_or_default();
    link::run(core, wire, oversize, &mut summary);
    summary
}
