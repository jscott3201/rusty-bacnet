//! What a scenario configures: master nodes, their adapters and
//! applications, scripted stations' replies, and faults.

use bacnet_mstp_codec::{Frame, FrameType, MAX_COBS_FRAME_LENGTH};
use bacnet_mstp_core::{Duration, Instant, MstpCoreConfig};

/// How a simulated master's platform adapter behaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AdapterSpec {
    /// How long after the core hands it a frame the adapter confirms the
    /// start and enables its driver. Zero is an ideal adapter; anything more
    /// models a slow one (a busy executor, a DMA queue, a late interrupt).
    pub start_delay: Duration,
    /// Feed the node its own octets while it transmits, as a transceiver
    /// whose receiver stays enabled would. The core takes what it hears
    /// after confirming a start for its own echo, and ignores it.
    pub echo: bool,
    /// How late the adapter services each core deadline. Zero services it
    /// exactly when it falls due.
    pub deadline_lateness: Duration,
    /// Further lateness drawn per deadline, uniformly from zero to this, from
    /// the run's seeded generator, in whole microseconds: the granularity of
    /// a tick-driven timer. Below the core's `LATE_DEADLINE` (1 ms) it is not
    /// a miss. Without
    /// any, two masters that start in step can stay in step for ever.
    pub deadline_jitter: Duration,
}

impl AdapterSpec {
    /// Whether this adapter is slow: it starts frames late, or services
    /// deadlines late by more than timer jitter.
    #[must_use]
    pub fn is_slow(&self) -> bool {
        self.start_delay > Duration::ZERO || self.deadline_lateness > Duration::ZERO
    }

    /// Whether the core may count this adapter's deadline service as late
    /// (`LATE_DEADLINE`, 1 ms, or more).
    #[must_use]
    pub fn may_miss_deadlines(&self) -> bool {
        self.deadline_lateness + self.deadline_jitter >= Duration::from_millis(1)
    }
}

/// How a simulated application answers the requests its core delivers.
///
/// Whatever the policy, an answer the core refuses as too late (it already
/// sent Reply Postponed, or the requester may have stopped waiting) is queued
/// as an ordinary NPDU for the next token, as a real stack would send it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplyPolicy {
    /// Answer every request with `payload` after `delay`.
    Answer {
        /// Time from the delivery to the answer.
        delay: Duration,
        /// Reply NPDU; it must not expect a reply itself.
        payload: Vec<u8>,
    },
    /// Decide to postpone after `delay`, then queue `payload` for the next
    /// token.
    Postpone {
        /// Time from the delivery to the decision.
        delay: Duration,
        /// Reply NPDU sent later.
        payload: Vec<u8>,
    },
    /// Never decide. The core sends Reply Postponed when its decision
    /// deadline passes.
    Ignore,
}

impl Default for ReplyPolicy {
    fn default() -> Self {
        Self::Answer {
            delay: Duration::from_millis(2),
            payload: vec![0x01, 0x00, 0xAC],
        }
    }
}

/// A master node to put on the bus.
#[derive(Debug, Clone)]
pub struct MasterSpec {
    /// The core's configuration. Its `station` is the node's address; its
    /// `baud_rate` must match the simulator's.
    pub config: MstpCoreConfig,
    /// Unicast queue capacity.
    pub unicast_capacity: usize,
    /// Broadcast queue capacity.
    pub broadcast_capacity: usize,
    /// Adapter timing and echo.
    pub adapter: AdapterSpec,
    /// How the node's application answers requests.
    pub reply: ReplyPolicy,
    /// Whether the node is powered at the start. A node added unpowered
    /// joins with [`Fault::PowerOn`].
    pub powered: bool,
}

impl MasterSpec {
    /// A master at `station` with standard timing for `baud` and the given
    /// Max_Master, one information frame per token, and 16-entry queues.
    ///
    /// # Panics
    ///
    /// If the resulting configuration is invalid.
    #[must_use]
    pub fn new(station: u8, max_master: u8, baud: u32) -> Self {
        let mut config = MstpCoreConfig::for_baud_rate(baud).expect("a standard baud rate");
        config.station = station;
        config.max_master = max_master;
        config.validate().expect("a valid master configuration");
        Self {
            config,
            unicast_capacity: 16,
            broadcast_capacity: 16,
            adapter: AdapterSpec::default(),
            reply: ReplyPolicy::default(),
            powered: true,
        }
    }

    /// The node's station address.
    #[must_use]
    pub fn station(&self) -> u8 {
        self.config.station
    }

    /// Set Max_Info_Frames.
    #[must_use]
    pub fn max_info_frames(mut self, frames: u8) -> Self {
        self.config.max_info_frames = frames;
        self
    }

    /// Set both queue capacities.
    #[must_use]
    pub fn queues(mut self, unicast: usize, broadcast: usize) -> Self {
        self.unicast_capacity = unicast;
        self.broadcast_capacity = broadcast;
        self
    }

    /// Set the adapter.
    #[must_use]
    pub fn adapter(mut self, adapter: AdapterSpec) -> Self {
        self.adapter = adapter;
        self
    }

    /// Set the reply policy.
    #[must_use]
    pub fn reply(mut self, reply: ReplyPolicy) -> Self {
        self.reply = reply;
        self
    }

    /// Start the node unpowered.
    #[must_use]
    pub fn unpowered(mut self) -> Self {
        self.powered = false;
        self
    }

    /// Turn on extended frames (Types 32 and 33), as a routing node must
    /// have them: the core's `with_extended_frames`, with receive storage
    /// for the longest extended frame. Without it a node takes extended
    /// frames as a node without COBS support does, with standard receive
    /// storage.
    #[must_use]
    pub fn extended_frames(mut self) -> Self {
        self.config = self.config.with_extended_frames();
        self
    }
}

/// How an injected corruption damages one octet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corruption {
    /// Receivers get the octet XORed with this mask, as a clean character.
    Xor(u8),
    /// Receivers get a framing error instead of the octet.
    FramingError,
}

/// A fault the simulator injects at a scheduled time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    /// Drive these octets onto the line back to back, whoever else is
    /// driving it. On an idle line receivers get them as clean characters;
    /// overlapping another driver they get framing errors.
    Noise {
        /// The octets the noise source sends.
        octets: Vec<u8>,
    },
    /// Damage one octet of the next frame `station` transmits, optionally
    /// only a frame of the given type. It replaces any corruption still
    /// armed for that station.
    CorruptNext {
        /// The transmitting station.
        station: u8,
        /// Only a frame of this type, if set.
        frame_type: Option<FrameType>,
        /// Which octet of the frame, counting the preamble from 0.
        offset: usize,
        /// What happens to it.
        corruption: Corruption,
    },
    /// Power a node off: it stops driving at once and its core is dropped.
    PowerOff {
        /// The node's station.
        station: u8,
    },
    /// Power a node on with a fresh core, as after a reset.
    PowerOn {
        /// The node's station.
        station: u8,
    },
    /// Power a node off part way through its next transmission, once it
    /// has started `after_octets` octets of it. It replaces any cut-off
    /// still armed for that node.
    PowerOffMidFrame {
        /// The node's station.
        station: u8,
        /// Octets of the frame that go out before the power fails.
        after_octets: usize,
    },
    /// Withdraw every corruption and cut-off armed but not yet fired.
    Disarm,
    /// Replace a node's adapter timing from now on.
    SetAdapter {
        /// The node's station.
        station: u8,
        /// The new adapter behaviour.
        adapter: AdapterSpec,
    },
}

/// A frame a scripted station saw complete on the line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeenFrame {
    /// When its last octet arrived.
    pub at: Instant,
    /// Frame type.
    pub frame_type: FrameType,
    /// Destination address.
    pub destination: u8,
    /// Source address.
    pub source: u8,
    /// Data field.
    pub data: Vec<u8>,
}

/// Octets a scripted station sends in reaction to a frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptSend {
    /// Delay from the frame that triggered it to the first octet.
    pub after: Duration,
    /// The octets, preamble included.
    pub octets: Vec<u8>,
    /// Whether the octets are deliberately not a valid frame. Such a send
    /// counts as an injected fault.
    pub malformed: bool,
}

impl ScriptSend {
    /// A well-formed frame, encoded by the codec.
    ///
    /// # Panics
    ///
    /// If the codec refuses the frame.
    #[must_use]
    pub fn frame(
        after: Duration,
        frame_type: FrameType,
        destination: u8,
        source: u8,
        data: &[u8],
    ) -> Self {
        let mut octets = vec![0u8; MAX_COBS_FRAME_LENGTH];
        let length = Frame::new(frame_type, destination, source, data)
            .encode_into(&mut octets)
            .expect("a frame the codec accepts");
        octets.truncate(length);
        Self {
            after,
            octets,
            malformed: false,
        }
    }

    /// A well-formed frame with one octet XORed by `mask` afterwards, so its
    /// header or data CRC fails.
    #[must_use]
    pub fn corrupted(mut self, offset: usize, mask: u8) -> Self {
        self.octets[offset] ^= mask;
        self.malformed = true;
        self
    }
}

/// A scripted station's behaviour: it is shown every frame it receives and
/// pushes whatever it sends in reply.
pub trait Script {
    /// React to a frame that completed on the line.
    fn on_frame(&mut self, frame: &SeenFrame, sends: &mut Vec<ScriptSend>);
}

impl<F: FnMut(&SeenFrame, &mut Vec<ScriptSend>)> Script for F {
    fn on_frame(&mut self, frame: &SeenFrame, sends: &mut Vec<ScriptSend>) {
        self(frame, sends);
    }
}
