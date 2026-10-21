//! What a scenario configures: master nodes and their adapters, scripted
//! stations' replies, and faults. Applications are in `app`.

use bacnet_mstp_codec::{Frame, FrameType, MAX_COBS_FRAME_LENGTH};
use bacnet_mstp_core::{Duration, Instant, MstpCoreConfig};

/// How a simulated master's platform adapter behaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AdapterSpec {
    /// Extra idle time after each stop bit except the final one. Zero by
    /// default; the normal maximum is 19 bit times after stop end, because
    /// the 20-bit idle limit includes the stop bit (135-2020 9.2.3).
    pub inter_octet_gap: Duration,
    /// Idle-mark driver hold after the final data stop bit ends. Zero by
    /// default; must not exceed [`crate::WireTiming::max_post_drive`]. This
    /// delays physical release and receiver enable, not the core's final-stop
    /// completion timestamp or the host's drain observation.
    pub post_drive: Duration,
    /// Chunked host reads and completion timing, or octet delivery when absent.
    pub host: Option<HostSpec>,
    /// How long after the core hands it a frame the adapter confirms the
    /// start and enables its driver. Zero is an ideal adapter; anything more
    /// models a slow one (a busy executor, a DMA queue, a late interrupt).
    pub start_delay: Duration,
    /// Feed the node its own observations while it transmits, as a transceiver
    /// whose receiver stays enabled would. Octet mode delivers immediately;
    /// buffered hosts apply [`HostSpec::echo_return_delay`] and their filter.
    pub echo: bool,
    /// How late the adapter services each core deadline. Zero services it
    /// exactly when it falls due.
    pub deadline_lateness: Duration,
    /// Further lateness drawn per deadline, uniformly from zero to this, from
    /// the run's seeded generator, in whole microseconds: the granularity of
    /// a tick-driven timer. Below the core's `late_deadline` (1 ms by
    /// default) it is not counted late. Without
    /// any, two masters that start in step can stay in step for ever.
    pub deadline_jitter: Duration,
}

/// A buffered serial adapter. This is a deterministic model, not hardware
/// qualification. The default 1 ms latency fits the response budget at the
/// standard baud rates; 16 ms can exceed Tusage_delay on the physical wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostSpec {
    /// Local delay between observing an own transmitted character and making
    /// it available to the serial receive buffer. Captured at transmit start;
    /// ordinary receive latency follows it. Zero preserves immediate return.
    /// This does not change physical line timing or enable echo generation.
    /// Returns are FIFO: a shorter later profile cannot overtake queued data.
    /// Each host retains at most 4096 pending characters; overflow is counted
    /// and reported, and power loss cancels them. See [`crate::Report::local_echo`].
    pub echo_return_delay: Duration,
    /// Time from the first buffered character until the read becomes ready.
    /// Later characters can coalesce into that read. Must be 1 through 16 ms.
    pub receive_latency: Duration,
    /// Maximum characters returned by one read, in 1..=4096.
    pub max_chunk: usize,
    /// Draw each read size uniformly from 1 through `max_chunk`, seeded by
    /// the simulation seed, independently of scenario and deadline draws.
    pub random_chunks: bool,
    /// Time from write start until serial drain returns. Completion is the
    /// later of this and the frame's captured physical gap schedule.
    pub drain_duration: Duration,
    /// Optional content-matching echo hunt. Physical echo generation remains
    /// [`AdapterSpec::echo`], independently, to model missing echo.
    pub echo_grace: Option<Duration>,
}

impl Default for HostSpec {
    fn default() -> Self {
        Self {
            echo_return_delay: Duration::ZERO,
            receive_latency: Duration::from_millis(1),
            max_chunk: 512,
            random_chunks: false,
            drain_duration: Duration::ZERO,
            echo_grace: None,
        }
    }
}

impl HostSpec {
    pub(crate) fn validate(self) {
        assert!(
            (Duration::from_millis(1)..=Duration::from_millis(16)).contains(&self.receive_latency),
            "host receive latency must be 1 through 16 ms"
        );
        assert!(
            (1..=4096).contains(&self.max_chunk),
            "host chunk size must be 1 through 4096"
        );
        assert!(
            self.echo_grace.is_none_or(|grace| grace > Duration::ZERO),
            "echo grace must be positive"
        );
    }
}

impl AdapterSpec {
    /// Whether this adapter is slow: it starts frames late, or services
    /// deadlines late by more than timer jitter.
    #[must_use]
    pub fn is_slow(&self) -> bool {
        self.start_delay > Duration::ZERO || self.deadline_lateness > Duration::ZERO
    }

    /// Whether a core that counts a deadline late at `late_deadline` or
    /// more ([`MstpCoreConfig::late_deadline`]) may count this adapter's
    /// deadline service late.
    #[must_use]
    pub fn may_miss_deadlines(&self, late_deadline: Duration) -> bool {
        self.deadline_lateness + self.deadline_jitter >= late_deadline
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
    /// The node's application. Clones of the spec share it.
    pub(crate) application: crate::app::Shared,
    /// Whether the node is powered at the start. A node added unpowered
    /// joins with [`Fault::PowerOn`].
    pub powered: bool,
    /// How far its UART and its clock run from nominal. None by default.
    pub skew: crate::Skew,
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
            application: crate::app::Shared::new(crate::ReplyPolicy::default()),
            powered: true,
            skew: crate::Skew::default(),
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

    /// Use buffered host input and the host timeout defaults: 100 ms frame
    /// abort, 300 ms reply timeout and 35 ms usage timeout. These receiver
    /// allowances do not extend the 15 ms/250 ms peer response obligations.
    ///
    /// # Panics
    ///
    /// If the host read latency, chunk size or echo grace is invalid.
    #[must_use]
    pub fn host(mut self, host: HostSpec) -> Self {
        host.validate();
        self.adapter.host = Some(host);
        self.config.t_frame_abort = bacnet_mstp_core::MAX_FRAME_ABORT;
        self.config.t_reply_timeout = bacnet_mstp_core::MAX_REPLY_TIMEOUT;
        self.config.t_usage_timeout = bacnet_mstp_core::MAX_USAGE_TIMEOUT;
        self
    }

    /// Set the node's application: a [`ReplyPolicy`](crate::ReplyPolicy)
    /// or any [`Application`](crate::Application). The default answers
    /// every request with a short fixed NPDU after 2 ms.
    #[must_use]
    pub fn application(mut self, application: impl crate::Application + 'static) -> Self {
        self.application = crate::app::Shared::new(application);
        self
    }

    /// Run the node's UART and clock off nominal, within the one percent
    /// Clause 9.2.3 allows; see [`Skew`](crate::Skew).
    ///
    /// # Panics
    ///
    /// If either rate is off by more than one percent.
    #[must_use]
    pub fn skew(mut self, skew: crate::Skew) -> Self {
        skew.validate();
        self.skew = skew;
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
    /// Replace a node's adapter timing from now on. Changing between host
    /// and octet delivery requires the node to be powered off; buffered data
    /// and in-flight completion cannot migrate between the two adapters.
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
    /// Uniform extra idle time between octets, as in
    /// [`AdapterSpec::inter_octet_gap`].
    pub inter_octet_gap: Duration,
    /// Idle-mark driver hold after the last stop bit, with the same limit
    /// as [`AdapterSpec::post_drive`].
    pub post_drive: Duration,
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
            inter_octet_gap: Duration::ZERO,
            post_drive: Duration::ZERO,
            octets,
            malformed: false,
        }
    }

    /// Hold the driver after the final stop bit. The simulator validates
    /// the limit against its baud rate before transmitting.
    #[must_use]
    pub fn post_drive(mut self, hold: Duration) -> Self {
        self.post_drive = hold;
        self
    }

    /// Insert the same extra idle time after every non-final stop bit.
    /// The simulator validates the gap against the line's baud rate.
    #[must_use]
    pub fn inter_octet_gap(mut self, gap: Duration) -> Self {
        self.inter_octet_gap = gap;
        self
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
