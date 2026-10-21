//! The fuzz input: one station's configuration, and the steps that drive it.

use arbitrary::Arbitrary;
use bacnet_mstp_core::npdu::NetworkPriority;
use bacnet_mstp_core::{Duration, MstpCoreConfig, ReceiveError, TransmitFailure};

/// Steps past this many are ignored, to keep runs quick.
pub const MAX_STEPS: usize = 2048;
/// Octets one step may put on the line.
pub const MAX_OCTETS: usize = 256;

/// One run: a station, and what happens to it.
#[derive(Debug, Arbitrary)]
pub struct Input {
    /// How the station is configured and built.
    pub setup: Setup,
    /// What the line, the application and the adapter do, in order.
    pub steps: Vec<Step>,
}

/// The station's configuration and storage, each field reduced to a valid
/// value by [`Setup::config`] and its neighbours.
#[derive(Debug, Arbitrary)]
pub struct Setup {
    /// This station: with bit 7 set any address 0-127, otherwise 0-7.
    pub station: u8,
    /// How far Max_Master lies above the station: with bit 7 set up to 127,
    /// otherwise up to 3. It is at least 1.
    pub max_master: u8,
    /// Max_Info_Frames: 1 to 4, or 192 to 255.
    pub max_info_frames: u8,
    /// One of the standard baud rates.
    pub baud: u8,
    /// Extended frames on, which needs the extended-size buffers.
    pub extended: bool,
    /// Which buffers and queues the core gets; see [`Layout`].
    pub layout: u8,
    /// Unicast queue slots for the allocating queues, 0 to 5.
    pub unicast_slots: u8,
    /// Broadcast queue slots for the allocating queues, 0 to 3.
    pub broadcast_slots: u8,
    /// Receive storage shorter than a standard frame (standard layouts only).
    pub short_rx: Option<u16>,
    /// The adapter polls the actions a line input queues only before its
    /// next other call, as the contract allows (a line input never hands
    /// over a frame), so received data waits in the delivery slots.
    pub lazy: bool,
    /// The configurable timers, within their Clause 9.5.3 ranges.
    pub timing: Timing,
    /// The local scheduling settings.
    pub policy: Policy,
}

/// The timers a configuration may set, within their bounds.
#[derive(Debug, Arbitrary)]
pub struct Timing {
    /// T_reply_timeout: 255 ms plus up to 45.
    pub reply_timeout: u8,
    /// T_usage_timeout: 20 ms plus up to 15.
    pub usage_timeout: u8,
    /// T_frame_abort: 60 bit times plus 2 µs each, up to 100 ms.
    pub frame_abort: u16,
    /// T_turnaround: 40 bit times plus up to 63 times 50 µs.
    pub turnaround: u8,
    /// T_usage_delay: 15 ms less up to 14, at least T_turnaround.
    pub usage_delay: u8,
    /// T_reply_delay: 250 ms less up to 199.
    pub reply_delay: u8,
    /// The reply transmit margin: 1 to 50 ms.
    pub margin: u8,
}

/// The core's local scheduling settings.
#[derive(Debug, Arbitrary)]
pub struct Policy {
    /// 1 to 100, or 100 from 128 up.
    pub multiple_frame_ratio: u8,
    /// 1 to 4.
    pub broadcast_unicast_ratio: u8,
    /// 1 to 16.
    pub starvation_limit: u8,
    /// 1 ms to 20 s.
    pub queue_max_age: u16,
}

/// The core's storage: its queues, action buffers and receive storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// Standard sizes, allocating queues, one delivery slot.
    Standard,
    /// Standard sizes, fixed queues (3 unicast, 2 broadcast), three
    /// delivery slots.
    Heapless,
    /// Extended sizes, allocating queues, two delivery slots: what extended
    /// frames need, and a core without them may have too.
    Extended,
}

impl Setup {
    /// This station's address.
    pub fn station(&self) -> u8 {
        if self.station & 0x80 != 0 {
            self.station & 0x7F
        } else {
            self.station % 8
        }
    }

    /// The configuration, which always validates.
    pub fn config(&self) -> MstpCoreConfig {
        const BAUDS: [u32; 6] = [9_600, 19_200, 38_400, 57_600, 76_800, 115_200];
        let baud = BAUDS[usize::from(self.baud) % BAUDS.len()];
        let mut config = MstpCoreConfig::for_baud_rate(baud).expect("a standard baud rate");
        if self.extended {
            config = config.with_extended_frames();
        }
        let station = self.station();
        let span = if self.max_master & 0x80 != 0 {
            self.max_master & 0x7F
        } else {
            self.max_master % 4
        };
        config.station = station;
        config.max_master = station.saturating_add(span).clamp(1, 127);
        config.max_info_frames = match self.max_info_frames {
            raw @ 192.. => raw,
            raw => raw % 4 + 1,
        };
        self.timing.apply(&mut config);
        self.policy.apply(&mut config);
        config
    }

    /// The storage the core gets.
    pub fn layout(&self) -> Layout {
        match (self.extended, self.layout % 3) {
            (true, _) | (false, 2) => Layout::Extended,
            (false, 0) => Layout::Standard,
            (false, _) => Layout::Heapless,
        }
    }
}

impl Timing {
    fn apply(&self, config: &mut MstpCoreConfig) {
        config.t_reply_timeout = Duration::from_millis(255 + u64::from(self.reply_timeout % 46));
        config.t_usage_timeout = Duration::from_millis(20 + u64::from(self.usage_timeout % 16));
        config.t_frame_abort = (config.t_frame_abort
            + Duration::from_micros(2 * u64::from(self.frame_abort)))
        .min(Duration::from_millis(100));
        config.t_turnaround += Duration::from_micros(50 * u64::from(self.turnaround % 64));
        config.t_usage_delay =
            Duration::from_millis(15 - u64::from(self.usage_delay % 15)).max(config.t_turnaround);
        config.t_reply_delay = Duration::from_millis(250 - u64::from(self.reply_delay % 200));
        config.reply_transmit_margin = Duration::from_millis(1 + u64::from(self.margin % 50));
    }
}

impl Policy {
    fn apply(&self, config: &mut MstpCoreConfig) {
        config.multiple_frame_ratio = match self.multiple_frame_ratio {
            128.. => 100,
            raw => raw % 100 + 1,
        };
        config.broadcast_unicast_ratio = self.broadcast_unicast_ratio % 4 + 1;
        config.starvation_limit = self.starvation_limit % 16 + 1;
        config.queue_max_age = Duration::from_millis(1 + u64::from(self.queue_max_age) % 20_000);
    }
}

/// One thing that happens to the station.
#[derive(Debug, Arbitrary)]
pub enum Step {
    /// Octets on the line, each an octet time after the last plus `gap`
    /// times 16 µs.
    Octets {
        /// The octets.
        octets: Vec<u8>,
        /// Extra time between octets.
        gap: u16,
    },
    /// A whole valid frame, on the line or from a host assembler.
    Frame {
        /// The frame.
        frame: FrameSpec,
        /// How it reaches the core.
        path: Path,
    },
    /// A frame with one bit flipped past its header, on the line.
    Damaged {
        /// The frame, given at least one data octet.
        frame: FrameSpec,
        /// The octet, after the header, modulo what follows it.
        position: u16,
        /// The bit, modulo 8.
        bit: u8,
    },
    /// The first octets of a frame (1 up to its length less one, from
    /// `at`), so the next step finds it part way received: an error inside
    /// it, a start over it, the silence that ends it.
    Cut {
        /// The frame.
        frame: FrameSpec,
        /// Where it stops.
        at: u16,
    },
    /// A UART receive error, an octet time from now.
    ReceiveError(Fault),
    /// A host assembler reports an invalid frame `after` µs from now.
    InvalidFrame {
        /// Delay in µs.
        after: u16,
    },
    /// A host assembler reports octets `after` µs from now.
    HostActivity {
        /// Octets, 0 included.
        octets: u16,
        /// Delay in µs.
        after: u16,
    },
    /// `on_silence` after this many µs.
    Silence(u32),
    /// Time passes with no call.
    Wait(u32),
    /// The adapter services the core's deadline, on time or late.
    Deadline {
        /// How late.
        late: Late,
        /// Report a start for the frame the call queued before polling it.
        start_before_poll: bool,
    },
    /// The adapter confirms the start of the frame it holds, `after` µs
    /// from now; with none, or one already started, a start the core must
    /// refuse.
    Start {
        /// Delay in µs.
        after: u16,
    },
    /// The adapter reports its frame's last stop bit, `after` µs after it
    /// went out, having heard `echo` meanwhile.
    Complete {
        /// Delay in µs past the frame's time on the wire.
        after: u16,
        /// What the receiver heard while the driver was on.
        echo: Echo,
    },
    /// The adapter aborts the frame it holds.
    Abort {
        /// Delay in µs.
        after: u16,
        /// The reason given.
        reason: Failure,
    },
    /// A transmit event for an id the adapter doesn't hold.
    Bogus {
        /// Which event.
        event: TransmitEvent,
        /// The id, moved off the one the adapter holds.
        id: u32,
    },
    /// The application queues an NPDU.
    Enqueue {
        /// Its destination.
        to: Address,
        /// Bit 15 set: any length up to a little over the longest NPDU the
        /// station sends; otherwise 0 to 23 octets.
        length: u16,
        /// The NPDU's data-expecting-reply bit.
        expecting: bool,
        /// Network priority, modulo 4.
        priority: u8,
        /// Fills the NPDU.
        pattern: u8,
        /// How many copies, modulo 4, plus one.
        copies: u8,
    },
    /// The application decides a request.
    Decide {
        /// Which request: the newest, an older one, or an id nobody issued.
        pick: u8,
        /// The decision.
        decision: Decision,
        /// Delay in units of 8 µs, up to about half a second.
        after: u16,
    },
    /// The adapter polls every waiting action.
    Drain,
    /// A punctual adapter on a quiet line, for up to `rounds` (modulo 32,
    /// plus one) of its own events: it starts the frame it holds at once,
    /// completes a started one when its last octet is out, and otherwise
    /// services the next deadline on time.
    Run {
        /// How many events.
        rounds: u8,
    },
    /// A call stamped earlier than the core's last event.
    Regress {
        /// How much earlier, modulo the time so far.
        back: u32,
        /// Which call.
        event: u8,
    },
}

/// How late a deadline is serviced.
#[derive(Debug, Arbitrary)]
pub enum Late {
    /// At the deadline.
    OnTime,
    /// Microseconds late.
    Micros(u16),
    /// Milliseconds late, up to a minute.
    Millis(u16),
}

impl Late {
    /// The lateness in µs.
    pub fn micros(&self) -> u64 {
        match *self {
            Self::OnTime => 0,
            Self::Micros(micros) => u64::from(micros),
            Self::Millis(millis) => 1_000 * u64::from(millis),
        }
    }
}

/// How a whole frame reaches the core.
#[derive(Debug, Arbitrary)]
pub enum Path {
    /// Octet by octet, back to back.
    Octets,
    /// Octet by octet, with a gap of `micros` before octet `at` (modulo the
    /// frame's length).
    Gapped {
        /// Where.
        at: u16,
        /// How long.
        micros: u32,
    },
    /// Through `on_validated_frame`, after its octets through
    /// `on_host_activity` when `with_octets` is set.
    Validated {
        /// Report the octets first.
        with_octets: bool,
    },
}

/// What the adapter's receiver hears while its own frame goes out.
#[derive(Debug, Arbitrary)]
pub enum Echo {
    /// Nothing: the receiver is off.
    Silent,
    /// The frame's own octets.
    Octets,
    /// The frame, assembled by a host assembler.
    Frame,
    /// Other octets: a collision.
    Noise(Vec<u8>),
}

/// A UART receive error.
#[derive(Debug, Clone, Copy, Arbitrary)]
pub enum Fault {
    /// Framing.
    Framing,
    /// Overrun.
    Overrun,
    /// Parity.
    Parity,
    /// Break.
    Break,
}

impl From<Fault> for ReceiveError {
    fn from(fault: Fault) -> Self {
        match fault {
            Fault::Framing => Self::Framing,
            Fault::Overrun => Self::Overrun,
            Fault::Parity => Self::Parity,
            Fault::Break => Self::Break,
        }
    }
}

/// A transmit failure.
#[derive(Debug, Clone, Copy, Arbitrary)]
pub enum Failure {
    /// Aborted.
    Aborted,
    /// Timed out.
    Timeout,
    /// I/O.
    Io,
    /// Rejected.
    Rejected,
}

impl From<Failure> for TransmitFailure {
    fn from(failure: Failure) -> Self {
        match failure {
            Failure::Aborted => Self::Aborted,
            Failure::Timeout => Self::Timeout,
            Failure::Io => Self::Io,
            Failure::Rejected => Self::Rejected,
        }
    }
}

/// A transmit event.
#[derive(Debug, Clone, Copy, Arbitrary)]
pub enum TransmitEvent {
    /// `on_transmit_started`.
    Start,
    /// `on_transmit_complete`.
    Complete,
    /// `on_transmit_aborted`.
    Abort,
}

/// An address, resolved against the station.
#[derive(Debug, Clone, Copy, Arbitrary)]
pub enum Address {
    /// This station.
    This,
    /// Broadcast, 255; as a source, which no station may use, 254 on the
    /// line and 255 from a host assembler, which the core must refuse.
    Broadcast,
    /// The station this one last sent a frame to, or its next address.
    Peer,
    /// Any address.
    Other(u8),
}

/// A frame type.
#[derive(Debug, Clone, Copy, Arbitrary)]
pub enum Kind {
    /// Token.
    Token,
    /// Poll For Master.
    PollForMaster,
    /// Reply To Poll For Master.
    ReplyToPollForMaster,
    /// Test_Request.
    TestRequest,
    /// Test_Response.
    TestResponse,
    /// BACnet Data Expecting Reply.
    DataExpectingReply,
    /// BACnet Data Not Expecting Reply.
    DataNotExpectingReply,
    /// Reply Postponed.
    ReplyPostponed,
    /// BACnet Extended Data Expecting Reply.
    ExtendedExpectingReply,
    /// BACnet Extended Data Not Expecting Reply.
    ExtendedNotExpectingReply,
    /// A reserved standard type, 8-31.
    Reserved(u8),
    /// A reserved COBS type, 34-127.
    ReservedCobs(u8),
    /// A proprietary type, 128-255.
    Proprietary(u8),
}

/// A frame another station sends.
#[derive(Debug, Clone, Arbitrary)]
pub struct FrameSpec {
    /// Its type.
    pub kind: Kind,
    /// Its destination.
    pub to: Address,
    /// Its source.
    pub from: Address,
    /// Bit 15 set: any length its type takes; otherwise a short one.
    pub length: u16,
    /// Fills its data.
    pub pattern: u8,
}

/// An application decision.
#[derive(Debug, Clone, Copy, Arbitrary)]
pub enum Decision {
    /// Answer with data: bit 15 set, any length up to a little over the
    /// longest the station sends; otherwise 0 to 15 octets.
    Data {
        /// The length.
        length: u16,
        /// Fills the data.
        pattern: u8,
    },
    /// Reply Postponed.
    Postponed,
    /// Reply Postponed too: the core's `ReplyDecision::Cancelled`, which
    /// acted as `Postponed`, is gone (#1684), and the variant stays so that
    /// saved inputs decode as they did.
    Cancelled,
    /// Abandoned.
    Abandoned,
}

/// The network priority for `raw`.
pub fn priority(raw: u8) -> NetworkPriority {
    NetworkPriority::try_from(raw % 4).expect("0-3 are priorities")
}

/// `length` octets: `pattern`, then each one more than the last.
pub fn fill(length: usize, pattern: u8) -> Vec<u8> {
    (0..length)
        .map(|index| pattern.wrapping_add(index as u8))
        .collect()
}
