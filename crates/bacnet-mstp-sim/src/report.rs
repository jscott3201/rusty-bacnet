//! What a run produced: the frames on the wire, what applications received,
//! and the invariant report.

use std::fmt;

use bacnet_mstp_codec::{FrameHeader, FrameType};
use bacnet_mstp_core::{Duration, Instant, TransmitId};

use crate::line::Driver;
use crate::trace::Millis;

/// One transmission on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireFrame {
    /// Who drove it.
    pub driver: Driver,
    /// When its first octet started.
    pub start: Instant,
    /// When its last octet ended, or when it was cut off.
    pub end: Instant,
    /// The octets as the driver sent them, before any injected corruption.
    pub octets: Vec<u8>,
    /// The header, when the octets decode as one whole frame.
    pub header: Option<FrameHeader>,
    /// The data field, when the octets decode.
    pub data: Vec<u8>,
    /// Another driver overlapped before this frame's final data stop,
    /// including during an inter-octet gap. A later overlap while
    /// this driver holds idle mark is recorded in [`Report::collisions`]
    /// without retroactively corrupting the completed data.
    pub collided: bool,
    /// An injected corruption damaged one of its octets.
    pub corrupted: bool,
    /// Transmission stopped before all data completed, due to power loss
    /// or an octet adapter's transmit watchdog.
    pub truncated: bool,
    /// A master sent it as a token-generation Poll For Master.
    pub generation: bool,
}

impl WireFrame {
    /// Whether the transmission was free of physical overlap, injected
    /// corruption and truncation. A receiver-specific intact capture does
    /// not make a globally collided transmission clean.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        !self.collided && !self.corrupted && !self.truncated
    }

    /// The frame type, if it decodes.
    #[must_use]
    pub fn frame_type(&self) -> Option<FrameType> {
        self.header.map(|header| header.frame_type)
    }

    /// The source station, if it decodes.
    #[must_use]
    pub fn source(&self) -> Option<u8> {
        self.header.map(|header| header.source)
    }

    /// The destination station, if it decodes.
    #[must_use]
    pub fn destination(&self) -> Option<u8> {
        self.header.map(|header| header.destination)
    }

    /// Whether this is a clean frame of `frame_type` from `source` to
    /// `destination`.
    #[must_use]
    pub fn is(&self, frame_type: FrameType, source: u8, destination: u8) -> bool {
        self.is_clean()
            && self.header
                == Some(FrameHeader {
                    frame_type,
                    destination,
                    source,
                })
    }
}

impl fmt::Display for WireFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{} ", Millis(self.start), Millis(self.end))?;
        match self.header {
            Some(header) => write!(
                f,
                "{:?} {}->{} ({} data octets)",
                header.frame_type,
                header.source,
                header.destination,
                self.data.len()
            )?,
            None => write!(
                f,
                "{:?}: {} undecodable octets",
                self.driver,
                self.octets.len()
            )?,
        }
        for (flag, text) in [
            (self.collided, " collided"),
            (self.corrupted, " corrupted"),
            (self.truncated, " truncated"),
            (self.generation, " generation"),
        ] {
            if flag {
                f.write_str(text)?;
            }
        }
        Ok(())
    }
}

/// Data a master's core delivered to its application: an NPDU, or a frame
/// delivered whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    /// When the core delivered it.
    pub at: Instant,
    /// Source station.
    pub source: u8,
    /// Destination station (this node or broadcast).
    pub destination: u8,
    /// `None` for an NPDU; for a frame delivered whole (a Test_Response or a
    /// proprietary frame), its type.
    pub frame_type: Option<FrameType>,
    /// The NPDU, or the whole frame's data.
    pub data: Vec<u8>,
    /// Whether the core bound a reply request to it.
    pub request: bool,
}

/// Which invariant failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViolationKind {
    /// A master sent octets that do not decode as one whole frame, with no
    /// fault to explain it.
    Undecodable,
    /// A clean frame was not counted by a master that heard all of it.
    ListenerMissedFrame,
    /// A master started a frame within its Tturnaround of line activity.
    Turnaround,
    /// Two drivers overlapped without a fault or a token-generation race to
    /// explain it.
    Collision,
    /// More than one master believed it held the token, outside the
    /// configured tolerance.
    DuplicateToken,
    /// The simulator's pending events grew past their bound.
    EventGrowth,
    /// The core or the adapter broke their contract: a refusal the contract
    /// does not expect, a start whose octets differ from the frame named, or
    /// too many actions from one event.
    ContractBroken,
    /// A core counter that must stay zero moved.
    CoreCounter,
    /// The core asked for a deadline already in the past, or kept asking
    /// for one it had just been given.
    Deadline,
    /// A master answered a Token or Poll For Master later than Tusage_delay.
    UsageDelay,
    /// A host's first reply octet exceeded the physical T_reply_delay.
    ReplyDelay,
}

/// One invariant failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// When it was detected.
    pub at: Instant,
    /// Which invariant.
    pub kind: ViolationKind,
    /// What exactly.
    pub detail: String,
    /// Wire and read timestamps when unavailable host input caused a
    /// physical turnaround failure. The failure remains a violation.
    pub host_timing: Option<HostTimingFailure>,
}

/// Causal evidence for a physical start before buffered input became known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostTimingFailure {
    /// Station that admitted the transmission.
    pub station: u8,
    /// Last character's physical end.
    pub wire_activity: Instant,
    /// Physical transmit admission.
    pub start: Instant,
    /// When the buffered input would first become readable.
    pub readable_at: Instant,
    /// Required silence before admission.
    pub turnaround: Duration,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {:?}: {}", Millis(self.at), self.kind, self.detail)
    }
}

/// Why two drivers overlapped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollisionClass {
    /// Injected noise was one of them.
    Noise,
    /// Both drivers were masters that had each generated a token and still
    /// held it: they measured the silence from different points (one powered
    /// on part way through it, say), so their generation slots coincided.
    GenerationRace,
    /// A master with a slow or late adapter was one of them, and its core
    /// handed its frame over before the other frame's first character
    /// arrived, so only the adapter's delay brought them together. Tolerated
    /// only under [`Tolerance::slow_adapter_collisions`].
    SlowAdapter,
    /// A scripted station was one of them.
    Scripted,
    /// None of the above: an invariant failure.
    Unexplained,
}

/// Two drivers on the line at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Collision {
    /// When the second driver's octet started.
    pub at: Instant,
    /// The driver that started an octet over another.
    pub driver: Driver,
    /// The driver already on the line.
    pub other: Driver,
    /// The explanation.
    pub class: CollisionClass,
    /// An observed host turnaround failure at this exact start, when
    /// present. This diagnoses the overlap; it does not tolerate it.
    pub host_timing: Option<HostTimingFailure>,
    /// The other driver was holding idle mark after its final data stop.
    /// The completed frame remains unchanged; this overlap damages the new
    /// transmission and is observable independently of its classification.
    pub post_drive: Option<PostDriveOverlap>,
}

/// Physical edges of the held driver involved in an overlap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PostDriveOverlap {
    /// When the held driver's final data stop bit ended.
    pub data_end: Instant,
    /// When that driver is scheduled to release the line.
    pub release: Instant,
}

/// A physical driver release, separate from data and software completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DriverRelease {
    /// Which station or noise source released the line.
    pub driver: Driver,
    /// First octet start; together with `driver`, identifies the wire frame.
    pub start: Instant,
    /// Final data stop, absent if power loss or watchdog expiry cut it off.
    pub data_end: Option<Instant>,
    /// Release originally scheduled when this transmission started.
    pub scheduled: Instant,
    /// Actual release instant, possibly earlier due to power loss or expiry.
    pub at: Instant,
    /// Why physical ownership ended.
    pub cause: DriverReleaseCause,
}

/// Why a driver stopped owning the physical line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverReleaseCause {
    /// Data and any post-drive hold completed normally.
    Completed,
    /// The station lost power, during data or post-drive hold.
    PowerLoss,
    /// The octet adapter serviced the core's transmit watchdog.
    Watchdog,
}

/// A host operation exceeded its configured local watchdog budget. This is
/// observational evidence: the host awaits completion without polling that
/// watchdog, so these cases do not imply a core timeout or wire abort.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostBudgetOverrun {
    /// The completion stamp itself is later than the watchdog deadline.
    CompletionAfterWatchdog,
    /// Completion was within budget, but echo delayed its notification.
    NotificationAfterWatchdog,
}

/// Physical and software clocks for one completed host transmission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostTransmitTiming {
    /// Transmitting station.
    pub station: u8,
    /// Powered incarnation, fencing a core ID reused after reboot.
    pub incarnation: u64,
    /// The core operation confirmed by the host adapter.
    pub transmit_id: TransmitId,
    /// Confirmed start S.
    pub start: Instant,
    /// Configured watchdog deadline W, captured at S.
    pub watchdog: Instant,
    /// Final physical data stop F from the captured gap schedule.
    pub data_end: Instant,
    /// Core completion stamp C = max(F, drain return).
    pub completion: Instant,
    /// Actual completion notification N, possibly delayed by echo handling.
    pub notification: Instant,
}

impl HostTransmitTiming {
    /// Classify strict budget excess while preserving the separate clocks.
    #[must_use]
    pub fn budget_overrun(&self) -> Option<HostBudgetOverrun> {
        if self.completion > self.watchdog {
            Some(HostBudgetOverrun::CompletionAfterWatchdog)
        } else if self.notification > self.watchdog {
            Some(HostBudgetOverrun::NotificationAfterWatchdog)
        } else {
            None
        }
    }
}

/// A stretch of time in which more than one master believed it held the
/// token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Episode {
    /// When it began.
    pub start: Instant,
    /// How long it lasted.
    pub length: Duration,
}

/// What the invariant checks found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// Bounded local echo-return accounting, retained across node reboots.
    pub local_echo: LocalEchoStats,
    /// Completed host operations and their independent physical/budget clocks.
    pub host_transmits: Vec<HostTransmitTiming>,
    /// Physical releases in event order, including power loss and expiry.
    pub driver_releases: Vec<DriverRelease>,
    /// Invariant failures, in order.
    pub violations: Vec<Violation>,
    /// Every overlap of two drivers, explained or not.
    pub collisions: Vec<Collision>,
    /// Stretches in which more than one master held the token after a
    /// fault, each within [`Tolerance::duplicate_token`].
    pub duplicate_token_episodes: Vec<Episode>,
    /// Stretches in which more than one master held a token it had generated
    /// itself, each within [`Tolerance::generation_race`].
    pub generation_races: Vec<Episode>,
    /// Transmit starts the core refused: a reply's window had closed, or the
    /// line was active within Tturnaround of the start, so the frame waited
    /// again.
    pub refused_starts: u32,
    /// Application answers the core refused as late, queued instead.
    pub late_answers: u32,
    /// The most dynamic events ever pending at once.
    pub max_pending_events: usize,
}

/// Run-wide accounting for the independent local echo-return queues.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LocalEchoStats {
    /// Physical characters accepted into local return queues.
    pub captured: u64,
    /// Characters whose local availability time ran, before host reads.
    pub released: u64,
    /// Characters cancelled on power loss or rejected as a stale incarnation.
    pub cancelled: u64,
    /// Characters refused because a host's 4096-character return queue was full.
    /// One invariant failure is recorded per sustained overflow episode.
    pub overflows: u64,
    /// Currently pending characters across all hosts.
    pub pending: usize,
    /// Largest pending queue at any single host, at most 4096 characters.
    pub max_pending: usize,
}

/// Observable work at a host adapter, reset when its node reboots.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HostStats {
    /// Nonempty serial reads.
    pub reads: u64,
    /// Characters returned across those reads, including echo.
    pub characters: u64,
    /// Most buffered characters at once.
    pub max_buffered: usize,
    /// Echo hunts ending their grace with no copy or mismatch.
    pub echo_missing: u32,
    /// Hunts that passed nonmatching characters while expecting echo.
    pub echo_mismatches: u32,
    /// Stale or superseded held echo fragments discarded.
    pub echo_fragments_dropped: u32,
    /// Most recent completion timestamp passed to the core.
    pub last_completion: Option<Instant>,
}

impl Report {
    /// Collisions of one class.
    #[must_use]
    pub fn collisions_of(&self, class: CollisionClass) -> usize {
        self.collisions
            .iter()
            .filter(|collision| collision.class == class)
            .count()
    }
}

/// Which departures from the strict invariants a run accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tolerance {
    /// Once a fault has been injected, how long more than one master may
    /// believe it holds the token. Without a fault it may never happen.
    pub duplicate_token: Duration,
    /// How long masters that each generated a token may go on holding them
    /// together. Only masters whose silence began at different instants can
    /// share a generation slot. Scenarios can use timing variation to test
    /// bounded recovery; see [`AdapterSpec::deadline_jitter`](crate::AdapterSpec::deadline_jitter).
    /// This configured tolerance is not a guarantee that every physical
    /// duplicate-token collision eventually resolves (135-2020 9.4).
    pub generation_race: Duration,
    /// Accept collisions a slow or late adapter causes: its core handed the
    /// frame over before the other frame's first character arrived, and only
    /// the adapter's delay made them overlap.
    pub slow_adapter_collisions: bool,
    /// Accept collisions involving scripted stations.
    pub scripted_collisions: bool,
}

impl Default for Tolerance {
    /// Strict: no duplicate token, generation races resolved within two
    /// seconds, and no slow-adapter or scripted collisions.
    fn default() -> Self {
        Self {
            duplicate_token: Duration::ZERO,
            generation_race: Duration::from_secs(2),
            slow_adapter_collisions: false,
            scripted_collisions: false,
        }
    }
}
