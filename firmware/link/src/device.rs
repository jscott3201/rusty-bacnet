//! The device image's scenario (#1771).
//!
//! The `device` image is an MS/TP device with no allocator: the network,
//! application and data link layers a small product needs, linked
//! together. [`run`] drives it, returning a [`Summary`] of what it saw.
//!
//! - **Data link:** `bacnet-mstp-core`'s master node over a
//!   [`StandardStorage`] bundle (#1769) in [`Memory`], fed octet by octet
//!   with injected time, every action carried out the way a UART adapter
//!   carries it out (the `link` module).
//! - **Application:** [`app::answer`] reads each NPDU the core delivers
//!   through `bacnet-npdu`'s view (#1770) and serves Who-Is and ReadProperty
//!   for a fixed Device object and one Analog Input, through the
//!   allocation-free profile of `bacnet-encoding` and `bacnet-services`
//!   (#1614). A confirmed request's answer goes back as the core's reply to
//!   the data-expecting-reply frame; an I-Am is queued as a broadcast for
//!   the next token.
//! - **The script:** a client at station 2 asks for every device and then
//!   for devices 5000 to 6000, passes the token, so the I-Am goes out, and
//!   takes the token back. Then it sends the thirteen confirmed requests in
//!   [`REQUESTS`]: reads that succeed, among them one through the wildcard
//!   Device instance and one relayed by a router from network 5, reads that
//!   draw each Error the ReadProperty service lists, a malformed request, a
//!   service the device doesn't serve and a segmented request. Last, two
//!   confirmed requests by broadcast, one in a broadcast frame and one to
//!   every network, which the device leaves unanswered.
//!
//! The client is played by the scenario, from frames written by hand: the
//! octets of each request come from the Clause 6.2 and 20.1 layouts, not
//! from the encoders the device uses. The summary keeps, for every frame
//! the device sent, its type and destination, and for each answer what the
//! client reads in it and a hash over its NPDU. `expected()` holds the hash
//! of each answer as written out by hand, so a fault the device's encoders
//! and the client's decoders share can't cancel out.
//!
//! `tests/device.rs` runs it natively and compares the whole summary with
//! `expected()`; the `device` image (`src/bin/device.rs`) links it, and its
//! emulation build checks the emulated CPU's summary against the same.

pub mod app;
mod link;

// Only the host tests and the emulation build compare summaries, so the
// link test's image builds without it, as the apdu image's does.
#[cfg(any(not(target_os = "none"), feature = "emulate"))]
mod expected;

#[cfg(any(not(target_os = "none"), feature = "emulate"))]
pub use expected::expected;

use bacnet_encoding::apdu::header::ApduHeader;
use bacnet_mstp_codec::FrameType;
use bacnet_mstp_core::{MasterCore, MasterState, MstpCoreConfig, StandardStorage};
use bacnet_types::enums::{AbortReason, ErrorClass, ErrorCode, RejectReason};

use app::{Scratch, ANSWER_LENGTH, MAX_INFO_FRAMES, MAX_MASTER};

/// Unicast slots in the transmit queue. The device answers requests in
/// the reply window, not through the queue, so one is plenty.
pub const UNICAST_SLOTS: usize = 1;
/// Broadcast slots in the transmit queue, for I-Ams waiting for the token.
pub const BROADCAST_SLOTS: usize = 2;
/// Slots for received data waiting to be polled. The scenario polls every
/// action after each call into the core, so one is enough.
pub const DELIVERY_SLOTS: usize = 1;

/// The storage the core borrows: its receive buffer, queues and action
/// buffers, all for standard frames.
pub type Storage = StandardStorage<UNICAST_SLOTS, BROADCAST_SLOTS, DELIVERY_SLOTS>;

/// The master core as the image builds it, over borrowed [`Storage`].
pub type Core<'m> =
    MasterCore<'m, &'m mut bacnet_mstp_core::FixedQueues<UNICAST_SLOTS, BROADCAST_SLOTS>>;

/// This station.
pub const STATION: u8 = 1;
/// The client, which passes this station the token.
pub const CLIENT: u8 = 2;

/// Station 1 of masters 0 to [`MAX_MASTER`], at 9600 baud, taking
/// [`MAX_INFO_FRAMES`] frames per token: what the Device object reports.
pub const CONFIG: MstpCoreConfig = {
    let mut config = MstpCoreConfig::STANDARD;
    config.station = STATION;
    config.max_master = MAX_MASTER;
    config.max_info_frames = MAX_INFO_FRAMES;
    config
};

/// Octets of an encoded frame from the client: the longest request is 31
/// octets of NPDU in an 8-octet header with a 2-octet data CRC.
const WIRE_LENGTH: usize = 64;

/// Storage the device works in besides its stack: what the core borrows,
/// the application's scratch and answer buffers, and where the client's
/// frames are encoded.
///
/// [`Memory::new`] is all zero bits, so the image keeps it in a static that
/// the linker places in `.bss`, costing no flash for an initial image.
pub struct Memory {
    /// What the core borrows.
    core: Storage,
    /// The application's working buffers.
    scratch: Scratch,
    /// The NPDU that answers the latest request.
    answer: [u8; ANSWER_LENGTH],
    /// The client's frames on their way to the core.
    wire: [u8; WIRE_LENGTH],
}

impl Memory {
    /// Zeroed storage.
    pub const fn new() -> Self {
        Self {
            core: Storage::new(),
            scratch: Scratch::new(),
            answer: [0; ANSWER_LENGTH],
            wire: [0; WIRE_LENGTH],
        }
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

/// What the client finds in one of the device's answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pdu {
    /// An I-Am.
    IAm,
    /// A ComplexACK for this invoke ID.
    ComplexAck(u8),
    /// An Error for this invoke ID, with its class and code.
    Error(u8, ErrorClass, ErrorCode),
    /// A Reject of this invoke ID.
    Reject(u8, RejectReason),
    /// An Abort of this invoke ID, sent by the server.
    Abort(u8, AbortReason),
    /// Anything the client can't read.
    Unreadable,
}

/// One data frame the device sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reply {
    /// The frame's type.
    pub frame_type: FrameType,
    /// Its destination station.
    pub destination: u8,
    /// What the client reads in it.
    pub pdu: Pdu,
    /// The NPDU's length in octets.
    pub length: u8,
    /// A 32-bit FNV-1a hash over the NPDU.
    pub fnv: u32,
}

/// Room for the device's answers: the I-Am and one per request.
pub const REPLIES: usize = 1 + REQUESTS.len();
/// Room for the frames the device sends that carry no data.
pub const LINK_FRAMES: usize = 4;

/// What [`run`] saw. Every field is plain data, so the host test can compare
/// the whole summary at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    /// Each data frame the device sent, in order.
    pub replies: [Option<Reply>; REPLIES],
    /// The type and destination of each other frame it sent, in order.
    pub link_frames: [Option<(FrameType, u8)>; LINK_FRAMES],
    /// NPDUs the core delivered.
    pub delivered: u8,
    /// Of those, the ones the application left unanswered.
    pub unanswered: u8,
    /// The core's state at the end.
    pub state: MasterState,
    /// Core counter: valid frames received.
    pub frames_received: u32,
    /// Core counter: frames transmitted.
    pub frames_transmitted: u32,
    /// Core counter: tokens passed to a successor.
    pub token_rotations: u32,
    /// Core counter: invalid frames received.
    pub parser_errors: u32,
    /// Calls that failed where the scenario expects success, and inputs the
    /// core counted as broken. Zero in a good run.
    pub errors: u8,
}

impl Summary {
    const fn new() -> Self {
        Self {
            replies: [None; REPLIES],
            link_frames: [None; LINK_FRAMES],
            delivered: 0,
            unanswered: 0,
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

/// The client's Who-Is for every device: a local broadcast, no range.
pub const WHO_IS_EVERY_DEVICE: [u8; 4] = [0x01, 0x00, 0x10, 0x08];
/// The client's Who-Is for devices 5000 (0x1388) to 6000 (0x1770), context
/// tags 0 and 1 of two octets each, which leaves this device out.
pub const WHO_IS_OTHERS: [u8; 10] = [0x01, 0x00, 0x10, 0x08, 0x0A, 0x13, 0x88, 0x1A, 0x17, 0x70];

/// The client's confirmed requests, each an NPDU that expects a reply
/// (control 0x04), with a Confirmed-Request header of no segmentation and
/// APDUs up to 480 octets (0x00 0x03), the invoke ID 1 to 13 in order, and
/// ReadProperty (0x0C) unless noted. The object identifiers are Device 1234
/// (0x020004D2), Device 4194303 (0x023FFFFF), Analog Input 1 (0x00000001)
/// and Analog Input 2; the properties Object_Name (77, 0x4D),
/// Object_Identifier (75, 0x4B), Object_List (76, 0x4C), Present_Value
/// (85, 0x55), Priority_Array (87, 0x57) and Protocol_Services_Supported
/// (97, 0x61).
pub const REQUESTS: [&[u8]; 13] = [
    // 1: the Device's Object_Name.
    &[
        0x01, 0x04, 0x00, 0x03, 0x01, 0x0C, 0x0C, 0x02, 0x00, 0x04, 0xD2, 0x19, 0x4D,
    ],
    // 2: the Object_Identifier of Device 4194303, which stands for this one.
    &[
        0x01, 0x04, 0x00, 0x03, 0x02, 0x0C, 0x0C, 0x02, 0x3F, 0xFF, 0xFF, 0x19, 0x4B,
    ],
    // 3: Object_List's length, array index 0 (context tag 2).
    &[
        0x01, 0x04, 0x00, 0x03, 0x03, 0x0C, 0x0C, 0x02, 0x00, 0x04, 0xD2, 0x19, 0x4C, 0x29, 0x00,
    ],
    // 4: Object_List's second element.
    &[
        0x01, 0x04, 0x00, 0x03, 0x04, 0x0C, 0x0C, 0x02, 0x00, 0x04, 0xD2, 0x19, 0x4C, 0x29, 0x02,
    ],
    // 5: Object_List's third element, past its end.
    &[
        0x01, 0x04, 0x00, 0x03, 0x05, 0x0C, 0x0C, 0x02, 0x00, 0x04, 0xD2, 0x19, 0x4C, 0x29, 0x03,
    ],
    // 6: the Analog Input's Present_Value.
    &[
        0x01, 0x04, 0x00, 0x03, 0x06, 0x0C, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55,
    ],
    // 7: the same with an array index, though it isn't an array.
    &[
        0x01, 0x04, 0x00, 0x03, 0x07, 0x0C, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55, 0x29, 0x01,
    ],
    // 8: Analog Input 2, which doesn't exist.
    &[
        0x01, 0x04, 0x00, 0x03, 0x08, 0x0C, 0x0C, 0x00, 0x00, 0x00, 0x02, 0x19, 0x55,
    ],
    // 9: the Analog Input's Priority_Array, which it doesn't have.
    &[
        0x01, 0x04, 0x00, 0x03, 0x09, 0x0C, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x57,
    ],
    // 10: a ReadProperty without its property identifier.
    &[
        0x01, 0x04, 0x00, 0x03, 0x0A, 0x0C, 0x0C, 0x00, 0x00, 0x00, 0x01,
    ],
    // 11: WriteProperty (0x0F) of 100.0 (a Real, 0x42C80000) to the
    // Present_Value, a service the device doesn't serve.
    &[
        0x01, 0x04, 0x00, 0x03, 0x0B, 0x0F, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55, 0x3E, 0x44,
        0x42, 0xC8, 0x00, 0x00, 0x3F,
    ],
    // 12: a segmented request (0x08), its first segment: sequence number 0,
    // proposed window size 1, then the service and parameters.
    &[
        0x01, 0x04, 0x08, 0x03, 0x0C, 0x00, 0x01, 0x0C, 0x0C, 0x02, 0x00, 0x04, 0xD2, 0x19, 0x4D,
    ],
    // 13: Protocol_Services_Supported, relayed by a router from station 7
    // on network 5 (control 0x0C: SNET 5, SLEN 1, SADR 7).
    &[
        0x01, 0x0C, 0x00, 0x05, 0x01, 0x07, 0x00, 0x03, 0x0D, 0x0C, 0x0C, 0x02, 0x00, 0x04, 0xD2,
        0x19, 0x61,
    ],
];

/// A ReadProperty of the Device's Object_Name sent to every network
/// (control 0x24: DNET 0xFFFF, DLEN 0, hop count 255), which a device must
/// leave unanswered: confirmed requests don't travel by broadcast.
pub const GLOBAL_READ_PROPERTY: [u8; 17] = [
    0x01, 0x24, 0xFF, 0xFF, 0x00, 0xFF, 0x00, 0x03, 0x0E, 0x0C, 0x0C, 0x02, 0x00, 0x04, 0xD2, 0x19,
    0x4D,
];

/// Run the whole scenario in `memory`. The core is built on this function's
/// stack, borrowing its large buffers from `memory`.
///
/// The scenario never panics on a failed call: it counts the failure in
/// [`Summary::errors`] and carries on, or returns early when it cannot
/// continue.
pub fn run(memory: &mut Memory) -> Summary {
    let Memory {
        core,
        scratch,
        answer,
        wire,
    } = memory;
    let mut summary = Summary::new();
    link::run(core, scratch, answer, wire, &mut summary);
    summary
}

/// What the client reads in `npdu`, an answer from the device: its header
/// through `bacnet-npdu`'s view and the APDU header, and an Error's class
/// and code.
fn read_answer(npdu: &[u8]) -> Pdu {
    let Ok(npdu) = bacnet_npdu::Npdu::decode(npdu) else {
        return Pdu::Unreadable;
    };
    match ApduHeader::decode(npdu.payload) {
        Ok((ApduHeader::UnconfirmedRequest(_), _)) => Pdu::IAm,
        Ok((ApduHeader::ComplexAck { invoke_id, .. }, _)) => Pdu::ComplexAck(invoke_id),
        Ok((ApduHeader::Error { invoke_id, .. }, parameters)) => match error_type(parameters) {
            Some((class, code)) => Pdu::Error(invoke_id, class, code),
            None => Pdu::Unreadable,
        },
        Ok((ApduHeader::Reject { invoke_id, reason }, [])) => Pdu::Reject(invoke_id, reason),
        Ok((
            ApduHeader::Abort {
                server: true,
                invoke_id,
                reason,
            },
            [],
        )) => Pdu::Abort(invoke_id, reason),
        _ => Pdu::Unreadable,
    }
}

/// An Error's class and code: two application-tagged Enumerated values,
/// and nothing after them.
fn error_type(parameters: &[u8]) -> Option<(ErrorClass, ErrorCode)> {
    use bacnet_encoding::primitives::{decode_application_value_ref, ApplicationValueRef};
    let (ApplicationValueRef::Enumerated(class), next) =
        decode_application_value_ref(parameters, 0).ok()?
    else {
        return None;
    };
    let (ApplicationValueRef::Enumerated(code), end) =
        decode_application_value_ref(parameters, next).ok()?
    else {
        return None;
    };
    (end == parameters.len()).then_some((
        ErrorClass::from_raw(u16::try_from(class).ok()?),
        ErrorCode::from_raw(u16::try_from(code).ok()?),
    ))
}
