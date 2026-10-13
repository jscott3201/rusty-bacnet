//! Configuration and Clause 9 timing validation for the MS/TP master core.

use core::fmt;

use bacnet_mstp_codec::{
    MAX_EXTENDED_FRAME_LENGTH, MAX_EXTENDED_MPDU_DATA, MAX_MASTER, MAX_STANDARD_FRAME_LENGTH,
    MAX_STANDARD_MPDU_DATA,
};
use bacnet_time::Duration;

const DEFAULT_BAUD_RATE: u32 = 9_600;
/// Npoll (Clause 9.5.3): the token uses between two maintenance Poll For
/// Master cycles, 50. Fixed by the standard, so not a configuration field.
pub const N_POLL: u8 = 50;
/// Nretry_token (Clause 9.5.3): how many times the core sends its Token
/// again to a successor that shows no sign of using it before it looks for
/// another, once. Fixed by the standard.
pub const N_RETRY_TOKEN: u8 = 1;
/// Nmin_octets (Clause 9.5.3): a station that hears more receive events
/// (octets and receive errors) than this takes the line to be in use, 4.
/// Fixed by the standard.
pub const N_MIN_OCTETS: u8 = 4;
/// T_no_token (Clause 9.5.3): the silence after which a station takes the
/// token to be lost, 500 ms. Fixed by the standard.
pub const T_NO_TOKEN: Duration = Duration::from_millis(500);
/// T_slot (Clause 9.5.3): the width of each station's turn to generate a
/// lost token, 10 ms. Fixed by the standard.
pub const T_SLOT: Duration = Duration::from_millis(10);
/// The longest T_frame_abort a station may use (Clause 9.5.3): 100 ms.
/// [`MstpCoreConfig::validate`] refuses a longer one.
pub const MAX_FRAME_ABORT: Duration = Duration::from_millis(100);
/// The shortest T_reply_timeout a station may use (Clause 9.5.3): 255 ms,
/// the baud-rate constructors' value. [`MstpCoreConfig::validate`] refuses a
/// shorter one. No requester stops waiting for a reply sooner, so the core
/// also uses it to bound how late its own answer may start.
pub const MIN_REPLY_TIMEOUT: Duration = Duration::from_millis(255);
/// The longest T_reply_timeout a station may use (Clause 9.5.3): 300 ms.
/// [`MstpCoreConfig::validate`] refuses a longer one.
pub const MAX_REPLY_TIMEOUT: Duration = Duration::from_millis(300);
/// The shortest T_usage_timeout a station may use (Clause 9.5.3): 20 ms, the
/// baud-rate constructors' value. [`MstpCoreConfig::validate`] refuses a
/// shorter one. No poller stops waiting for the answer to its poll sooner,
/// so the core also uses it to bound how late its own answer may start.
pub const MIN_USAGE_TIMEOUT: Duration = Duration::from_millis(20);
/// The longest T_usage_timeout a station may use (Clause 9.5.3): 35 ms.
/// [`MstpCoreConfig::validate`] refuses a longer one.
pub const MAX_USAGE_TIMEOUT: Duration = Duration::from_millis(35);
/// The longest T_reply_delay (Clause 9.5.3), the constructors' value.
const MAX_REPLY_DELAY: Duration = Duration::from_millis(250);
/// The longest T_usage_delay (Clause 9.5.3), the constructors' value.
const MAX_USAGE_DELAY: Duration = Duration::from_millis(15);
/// Default for [`MstpCoreConfig::starvation_limit`], which queues also use
/// until a core hands them the configured one.
pub(crate) const DEFAULT_STARVATION_LIMIT: u8 = 8;
/// Default for [`MstpCoreConfig::reply_transmit_margin`], the value the host
/// MS/TP transport reserves.
const DEFAULT_REPLY_TRANSMIT_MARGIN: Duration = Duration::from_millis(25);
/// Default for [`MstpCoreConfig::transmit_watchdog`] where it clears the
/// longest frame.
const DEFAULT_TRANSMIT_WATCHDOG: Duration = Duration::from_secs(1);
/// Kept in [`MstpCoreConfig::minimum_transmit_watchdog`] beyond the longest
/// frame and T_postdrive, for the adapter to report the completion.
const WATCHDOG_COMPLETION_MARGIN: Duration = Duration::from_millis(1);

/// A configuration value that violates a protocol or local safety bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConfigError {
    /// A baud rate of zero cannot be used to derive bit-time limits.
    ZeroBaudRate,
    /// A derived timing calculation overflowed its arithmetic.
    TimingOverflow,
    /// Max_Master is a seven-bit station address.
    MaxMasterOutOfRange {
        /// Supplied Max_Master value.
        value: u8,
    },
    /// This station cannot be beyond Max_Master.
    StationExceedsMaxMaster {
        /// Local station address.
        station: u8,
        /// Configured Max_Master value.
        max_master: u8,
    },
    /// Max_Info_Frames must be in the inclusive 1..=255 range.
    MaxInfoFramesOutOfRange {
        /// Supplied Max_Info_Frames value.
        value: u8,
    },
    /// T_frame_abort is outside the 60-bit-time through 100 ms range.
    FrameAbortOutOfRange {
        /// Supplied frame-abort duration.
        value: Duration,
        /// Minimum permitted duration.
        minimum: Duration,
        /// Maximum permitted duration.
        maximum: Duration,
    },
    /// T_frame_gap is longer than 20 bit times.
    FrameGapTooLong {
        /// Supplied frame-gap duration.
        value: Duration,
        /// Maximum permitted duration.
        maximum: Duration,
    },
    /// T_postdrive is longer than 15 bit times.
    PostdriveTooLong {
        /// Supplied postdrive duration.
        value: Duration,
        /// Maximum permitted duration.
        maximum: Duration,
    },
    /// T_reply_delay cannot exceed 250 ms.
    ReplyDelayTooLong {
        /// Supplied reply-delay duration.
        value: Duration,
        /// Maximum permitted duration.
        maximum: Duration,
    },
    /// The reply transmit margin must leave part of T_reply_delay for the
    /// application to answer.
    ReplyMarginTooLong {
        /// Supplied margin.
        margin: Duration,
        /// Configured T_reply_delay.
        reply_delay: Duration,
    },
    /// T_reply_timeout must be between 255 and 300 ms.
    ReplyTimeoutOutOfRange {
        /// Supplied reply-timeout duration.
        value: Duration,
        /// Minimum permitted duration.
        minimum: Duration,
        /// Maximum permitted duration.
        maximum: Duration,
    },
    /// T_turnaround must cover at least 40 bit times.
    TurnaroundTooShort {
        /// Supplied turnaround duration.
        value: Duration,
        /// Minimum permitted duration.
        minimum: Duration,
    },
    /// T_turnaround cannot exceed the maximum T_usage_delay response window.
    TurnaroundExceedsUsageDelay {
        /// Supplied turnaround duration.
        turnaround: Duration,
        /// Configured usage-delay duration.
        usage_delay: Duration,
    },
    /// T_usage_delay cannot exceed 15 ms.
    UsageDelayTooLong {
        /// Supplied usage-delay duration.
        value: Duration,
        /// Maximum permitted duration.
        maximum: Duration,
    },
    /// T_usage_timeout must be between 20 and 35 ms.
    UsageTimeoutOutOfRange {
        /// Supplied usage-timeout duration.
        value: Duration,
        /// Minimum permitted duration.
        minimum: Duration,
        /// Maximum permitted duration.
        maximum: Duration,
    },
    /// A local queue age or watchdog of zero would disable the safety bound.
    ZeroLocalDuration {
        /// Local duration setting that was zero.
        field: LocalDurationField,
    },
    /// A local scheduling ratio or starvation limit of zero is invalid.
    ZeroLocalLimit {
        /// Local count or ratio setting that was zero.
        field: LocalLimitField,
    },
    /// Multiple-frame ratio is a percentage and cannot exceed 100.
    MultipleFrameRatioOutOfRange {
        /// Supplied percentage value.
        value: u8,
    },
    /// The transmit watchdog would abort the longest frame this
    /// configuration sends before the driver could go off: a standard frame
    /// of 511 octets takes 1.06 s at 4800 baud, and an extended one of 1516
    /// octets 1.58 s at 9600. See
    /// [`MstpCoreConfig::minimum_transmit_watchdog`]; the baud-rate
    /// constructors and [`MstpCoreConfig::with_extended_frames`] set one
    /// that clears it.
    TransmitWatchdogTooShort {
        /// Supplied watchdog.
        value: Duration,
        /// The shortest watchdog accepted,
        /// [`MstpCoreConfig::minimum_transmit_watchdog`].
        minimum: Duration,
    },
    /// [`MstpCoreConfig::extended_frames`] is on, but a buffer the core was
    /// given can't hold the largest extended frame or NPDU. Only
    /// [`MasterCore::new`](crate::MasterCore::new) reports it, since only it
    /// sees the buffers.
    ExtendedStorageTooSmall {
        /// The buffer that is too small.
        buffer: CoreBuffer,
        /// Its size in octets.
        length: usize,
        /// The size extended frames need: the longest extended frame on the
        /// wire for the receive storage and the transmit frame, the longest
        /// NPDU one carries for a delivery slot and a queue slot.
        minimum: usize,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::ZeroBaudRate => f.write_str("the baud rate is zero"),
            Self::TimingOverflow => f.write_str("a bit-time bound overflowed"),
            Self::MaxMasterOutOfRange { value } => {
                write!(f, "Max_Master {value} is over {MAX_MASTER}")
            }
            Self::StationExceedsMaxMaster {
                station,
                max_master,
            } => write!(f, "station {station} is over Max_Master {max_master}"),
            Self::MaxInfoFramesOutOfRange { value } => {
                write!(f, "Max_Info_Frames {value} is not 1 to 255")
            }
            Self::FrameAbortOutOfRange {
                value,
                minimum,
                maximum,
            } => write!(
                f,
                "T_frame_abort {value:?} is not {minimum:?} to {maximum:?}"
            ),
            Self::FrameGapTooLong { value, maximum } => {
                write!(f, "T_frame_gap {value:?} is over {maximum:?}")
            }
            Self::PostdriveTooLong { value, maximum } => {
                write!(f, "T_postdrive {value:?} is over {maximum:?}")
            }
            Self::ReplyDelayTooLong { value, maximum } => {
                write!(f, "T_reply_delay {value:?} is over {maximum:?}")
            }
            Self::ReplyMarginTooLong {
                margin,
                reply_delay,
            } => write!(
                f,
                "the reply transmit margin {margin:?} leaves nothing of T_reply_delay {reply_delay:?}"
            ),
            Self::ReplyTimeoutOutOfRange {
                value,
                minimum,
                maximum,
            } => write!(
                f,
                "T_reply_timeout {value:?} is not {minimum:?} to {maximum:?}"
            ),
            Self::TurnaroundTooShort { value, minimum } => {
                write!(f, "T_turnaround {value:?} is under {minimum:?}")
            }
            Self::TurnaroundExceedsUsageDelay {
                turnaround,
                usage_delay,
            } => write!(
                f,
                "T_turnaround {turnaround:?} is over T_usage_delay {usage_delay:?}"
            ),
            Self::UsageDelayTooLong { value, maximum } => {
                write!(f, "T_usage_delay {value:?} is over {maximum:?}")
            }
            Self::UsageTimeoutOutOfRange {
                value,
                minimum,
                maximum,
            } => write!(
                f,
                "T_usage_timeout {value:?} is not {minimum:?} to {maximum:?}"
            ),
            Self::ZeroLocalDuration { field } => write!(f, "{field} is zero"),
            Self::ZeroLocalLimit { field } => write!(f, "{field} is zero"),
            Self::MultipleFrameRatioOutOfRange { value } => {
                write!(f, "multiple_frame_ratio {value} is over 100")
            }
            Self::TransmitWatchdogTooShort { value, minimum } => write!(
                f,
                "the transmit watchdog {value:?} is under {minimum:?}, the longest frame's"
            ),
            Self::ExtendedStorageTooSmall {
                buffer,
                length,
                minimum,
            } => write!(
                f,
                "the {buffer} holds {length} octets; extended frames need {minimum}"
            ),
        }
    }
}

impl core::error::Error for ConfigError {}

/// The buffers [`ConfigError::ExtendedStorageTooSmall`] names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CoreBuffer {
    /// The receive storage, which must hold [`MAX_EXTENDED_FRAME_LENGTH`]
    /// octets.
    ReceiveStorage,
    /// The [`ActionBuffers`](crate::ActionBuffers) frame buffer, its `FRAME`,
    /// which must hold as much.
    TransmitFrame,
    /// Each [`ActionBuffers`](crate::ActionBuffers) delivery slot, its
    /// `NPDU`, which must hold
    /// [`MAX_EXTENDED_MPDU_DATA`]
    /// octets.
    DeliverySlot,
    /// Each queue slot, as
    /// [`QueueStorage::max_npdu`](crate::QueueStorage::max_npdu)
    /// reports it, which must hold as much.
    QueueSlot,
}

/// Names of local duration settings used by [`ConfigError::ZeroLocalDuration`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LocalDurationField {
    /// [`MstpCoreConfig::queue_max_age`].
    QueueMaxAge,
    /// [`MstpCoreConfig::transmit_watchdog`].
    TransmitWatchdog,
}

/// Names of local count/ratio settings used by [`ConfigError::ZeroLocalLimit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LocalLimitField {
    /// [`MstpCoreConfig::broadcast_unicast_ratio`].
    BroadcastUnicastRatio,
    /// [`MstpCoreConfig::multiple_frame_ratio`].
    MultipleFrameRatio,
    /// [`MstpCoreConfig::starvation_limit`].
    StarvationLimit,
}

/// The buffer, as an error message names it.
impl fmt::Display for CoreBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ReceiveStorage => "receive storage",
            Self::TransmitFrame => "transmit frame buffer",
            Self::DeliverySlot => "delivery slot",
            Self::QueueSlot => "queue slot",
        })
    }
}

/// The field's name.
impl fmt::Display for LocalDurationField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::QueueMaxAge => "queue_max_age",
            Self::TransmitWatchdog => "transmit_watchdog",
        })
    }
}

/// The field's name.
impl fmt::Display for LocalLimitField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::BroadcastUnicastRatio => "broadcast_unicast_ratio",
            Self::MultipleFrameRatio => "multiple_frame_ratio",
            Self::StarvationLimit => "starvation_limit",
        })
    }
}

/// All timing, addressing, and bounded-queue parameters needed by an MS/TP
/// master.
///
/// Durations are `bacnet-time` [`Duration`]s, in nanoseconds. Values whose
/// limits are stated in bit times are still stored as absolute durations so a
/// core instance can keep absolute deadlines without retaining a platform
/// clock or baud-rate conversion helper. The bit-time values the constructors
/// derive from the baud rate, and the bounds [`Self::validate`] checks them
/// against, are whole microseconds (see [`Self::octet_time`]).
///
/// The values Clause 9.5.3 fixes are constants, not fields: [`N_POLL`],
/// [`N_RETRY_TOKEN`], [`N_MIN_OCTETS`], [`T_NO_TOKEN`] and [`T_SLOT`].
///
/// It is `#[non_exhaustive]`, so a new setting isn't a breaking change:
/// start from [`Self::STANDARD`] or [`Self::for_baud_rate`] and set the
/// fields to change, then let [`MasterCore::new`](crate::MasterCore::new)
/// or [`Self::validate`] check the result.
///
/// ```
/// use bacnet_mstp_core::MstpCoreConfig;
///
/// const CONFIG: MstpCoreConfig = {
///     let mut config = MstpCoreConfig::STANDARD;
///     config.station = 3;
///     config.max_master = 15;
///     config
/// };
/// assert_eq!(CONFIG.validate(), Ok(()));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct MstpCoreConfig {
    /// This station's MAC address.
    pub station: u8,
    /// Max_Master, inclusive.
    pub max_master: u8,
    /// Max_Info_Frames, the number of information frames per token use.
    pub max_info_frames: u8,
    /// Serial baud rate used to validate bit-time-derived limits.
    pub baud_rate: u32,

    /// T_frame_abort.
    pub t_frame_abort: Duration,
    /// T_frame_gap.
    pub t_frame_gap: Duration,
    /// T_postdrive.
    pub t_postdrive: Duration,
    /// T_reply_delay: the longest a reply to a data-expecting-reply frame may
    /// take to start.
    pub t_reply_delay: Duration,
    /// T_reply_timeout.
    pub t_reply_timeout: Duration,
    /// T_turnaround.
    pub t_turnaround: Duration,
    /// T_usage_delay.
    pub t_usage_delay: Duration,
    /// T_usage_timeout.
    pub t_usage_timeout: Duration,

    /// Time kept back from T_reply_delay for the platform to get a reply onto
    /// the wire.
    ///
    /// The core waits for the application's answer until T_reply_delay less
    /// this margin, then sends Reply Postponed itself, so that adapter and
    /// UART latency still lets the frame start within T_reply_delay. It must
    /// be shorter than T_reply_delay. Default 25 ms.
    pub reply_transmit_margin: Duration,
    /// Maximum time an item may remain in a local transmit queue.
    pub queue_max_age: Duration,
    /// Number of broadcast frames scheduled per unicast frame while both
    /// classes remain eligible.
    pub broadcast_unicast_ratio: u8,
    /// Percentage of `max_info_frames` usable during one token hold.
    ///
    /// The effective limit is rounded up and is always at least one. A value
    /// of 100 leaves Clause 9 `Max_Info_Frames` unchanged.
    pub multiple_frame_ratio: u8,
    /// Number of service opportunities after which an item is promoted.
    ///
    /// The one source of the queues' starvation limit:
    /// [`MasterCore::new`](crate::MasterCore::new) hands it to them through
    /// [`QueueStorage::set_starvation_limit`](crate::QueueStorage::set_starvation_limit).
    /// Default 8.
    pub starvation_limit: u8,
    /// Maximum time allowed for one platform transmit operation, from its
    /// confirmed start to its completion.
    ///
    /// It must clear [`Self::minimum_transmit_watchdog`], the longest
    /// frame's time on the wire plus T_postdrive, or validation refuses it
    /// with [`ConfigError::TransmitWatchdogTooShort`]. The baud-rate
    /// constructors give one second, or, where the longest standard frame
    /// would not fit in that (below about 5.1 kbaud), the longest frame's
    /// time plus one second.
    pub transmit_watchdog: Duration,

    /// Take and send extended frames, Types 32 and 33, which carry NPDUs of
    /// 502 to 1497 octets COBS-encoded (Clauses 9.3.9, 9.3.10). Off by
    /// default.
    ///
    /// Clause 9.3 requires COBS-encoded frames of a routing node and leaves
    /// them optional for any other node, so a router turns this on for
    /// every MS/TP port it routes and a device leaves it off unless it wants
    /// the larger NPDUs.
    ///
    /// - **Off**, the core is a node without COBS support. It refuses an
    ///   NPDU over 501 octets, and an extended frame it hears counts as an
    ///   invalid frame when it is for this station or broadcast.
    /// - **On**, a Type 32 for this station is a data request like a Type 5,
    ///   and a Type 33 data like a Type 6, a reply to this station's request
    ///   included; broadcasts of either go up as for Types 5 and 6. A
    ///   reserved COBS type (34-127) is checked and, with a good CRC-32K,
    ///   taken as a valid frame of a type the core doesn't know (Clause
    ///   9.5.4.9). An NPDU of 502 to 1497 octets, queued or as a reply, goes
    ///   out as Type 32 when it expects a reply and Type 33 otherwise.
    ///   [`MasterCore::new`](crate::MasterCore::new) then needs buffers
    ///   sized for extended frames, and refuses smaller ones with
    ///   [`ConfigError::ExtendedStorageTooSmall`]. The transmit watchdog
    ///   must clear the longest extended frame
    ///   ([`ConfigError::TransmitWatchdogTooShort`]);
    ///   [`Self::with_extended_frames`] sets both.
    pub extended_frames: bool,
}

/// [`MstpCoreConfig::STANDARD`].
impl Default for MstpCoreConfig {
    fn default() -> Self {
        Self::STANDARD
    }
}

impl MstpCoreConfig {
    /// The standard settings at 9600 baud: station 0, Max_Master 127, one
    /// information frame per token use, the bit-time timers derived from the
    /// baud rate, the shortest T_reply_timeout and T_usage_timeout Clause
    /// 9.5.3 allows, the longest T_reply_delay and T_usage_delay, and the
    /// local defaults below; extended frames off.
    pub const STANDARD: Self = Self::for_baud_rate_unchecked(DEFAULT_BAUD_RATE);

    /// The standard settings at `baud_rate`, as [`Self::STANDARD`] has them
    /// at 9600, validated.
    ///
    /// # Errors
    ///
    /// [`ConfigError::ZeroBaudRate`] for zero, and what [`Self::validate`]
    /// refuses at a rate too low for Clause 9 timing (below 2667 baud).
    pub const fn for_baud_rate(baud_rate: u32) -> Result<Self, ConfigError> {
        if baud_rate == 0 {
            return Err(ConfigError::ZeroBaudRate);
        }
        let config = Self::for_baud_rate_unchecked(baud_rate);
        match config.validate() {
            Ok(()) => Ok(config),
            Err(error) => Err(error),
        }
    }

    /// The time one octet takes on the wire: ten bit times, rounded up to a
    /// whole microsecond.
    ///
    /// Every time derived from the baud rate is a whole number of
    /// microseconds, rounded up for a lower bound and down for an upper one:
    /// the octet time, T_frame_abort, T_frame_gap, T_postdrive and
    /// T_turnaround, and validation's bounds on them. That keeps the
    /// arithmetic in 32 bits, with no 64-bit division on a 32-bit target.
    #[must_use]
    pub const fn octet_time(&self) -> Duration {
        or_max(ceil_bit_times(self.baud_rate, 10))
    }

    /// Validate all protocol and local bounds.
    ///
    /// # Errors
    ///
    /// The first bound the configuration breaks, as a [`ConfigError`].
    pub const fn validate(&self) -> Result<(), ConfigError> {
        if self.baud_rate == 0 {
            return Err(ConfigError::ZeroBaudRate);
        }
        if self.max_master > MAX_MASTER {
            return Err(ConfigError::MaxMasterOutOfRange {
                value: self.max_master,
            });
        }
        if self.station > self.max_master {
            return Err(ConfigError::StationExceedsMaxMaster {
                station: self.station,
                max_master: self.max_master,
            });
        }
        if self.max_info_frames == 0 {
            return Err(ConfigError::MaxInfoFramesOutOfRange {
                value: self.max_info_frames,
            });
        }

        let (Some(frame_abort_min), Some(frame_gap_max), Some(postdrive_max), Some(turnaround_min)) = (
            ceil_bit_times(self.baud_rate, 60),
            floor_bit_times(self.baud_rate, 20),
            floor_bit_times(self.baud_rate, 15),
            ceil_bit_times(self.baud_rate, 40),
        ) else {
            return Err(ConfigError::TimingOverflow);
        };
        if shorter(self.t_frame_abort, frame_abort_min)
            || longer(self.t_frame_abort, MAX_FRAME_ABORT)
        {
            return Err(ConfigError::FrameAbortOutOfRange {
                value: self.t_frame_abort,
                minimum: frame_abort_min,
                maximum: MAX_FRAME_ABORT,
            });
        }
        if longer(self.t_frame_gap, frame_gap_max) {
            return Err(ConfigError::FrameGapTooLong {
                value: self.t_frame_gap,
                maximum: frame_gap_max,
            });
        }
        if longer(self.t_postdrive, postdrive_max) {
            return Err(ConfigError::PostdriveTooLong {
                value: self.t_postdrive,
                maximum: postdrive_max,
            });
        }
        if longer(self.t_reply_delay, MAX_REPLY_DELAY) {
            return Err(ConfigError::ReplyDelayTooLong {
                value: self.t_reply_delay,
                maximum: MAX_REPLY_DELAY,
            });
        }
        if !shorter(self.reply_transmit_margin, self.t_reply_delay) {
            return Err(ConfigError::ReplyMarginTooLong {
                margin: self.reply_transmit_margin,
                reply_delay: self.t_reply_delay,
            });
        }
        if shorter(self.t_reply_timeout, MIN_REPLY_TIMEOUT)
            || longer(self.t_reply_timeout, MAX_REPLY_TIMEOUT)
        {
            return Err(ConfigError::ReplyTimeoutOutOfRange {
                value: self.t_reply_timeout,
                minimum: MIN_REPLY_TIMEOUT,
                maximum: MAX_REPLY_TIMEOUT,
            });
        }
        if shorter(self.t_turnaround, turnaround_min) {
            return Err(ConfigError::TurnaroundTooShort {
                value: self.t_turnaround,
                minimum: turnaround_min,
            });
        }
        if longer(self.t_turnaround, self.t_usage_delay) {
            return Err(ConfigError::TurnaroundExceedsUsageDelay {
                turnaround: self.t_turnaround,
                usage_delay: self.t_usage_delay,
            });
        }
        if longer(self.t_usage_delay, MAX_USAGE_DELAY) {
            return Err(ConfigError::UsageDelayTooLong {
                value: self.t_usage_delay,
                maximum: MAX_USAGE_DELAY,
            });
        }
        if shorter(self.t_usage_timeout, MIN_USAGE_TIMEOUT)
            || longer(self.t_usage_timeout, MAX_USAGE_TIMEOUT)
        {
            return Err(ConfigError::UsageTimeoutOutOfRange {
                value: self.t_usage_timeout,
                minimum: MIN_USAGE_TIMEOUT,
                maximum: MAX_USAGE_TIMEOUT,
            });
        }
        if self.queue_max_age.is_zero() {
            return Err(ConfigError::ZeroLocalDuration {
                field: LocalDurationField::QueueMaxAge,
            });
        }
        if self.transmit_watchdog.is_zero() {
            return Err(ConfigError::ZeroLocalDuration {
                field: LocalDurationField::TransmitWatchdog,
            });
        }
        if shorter(self.transmit_watchdog, self.minimum_transmit_watchdog()) {
            return Err(ConfigError::TransmitWatchdogTooShort {
                value: self.transmit_watchdog,
                minimum: self.minimum_transmit_watchdog(),
            });
        }
        if self.broadcast_unicast_ratio == 0 {
            return Err(ConfigError::ZeroLocalLimit {
                field: LocalLimitField::BroadcastUnicastRatio,
            });
        }
        if self.multiple_frame_ratio == 0 {
            return Err(ConfigError::ZeroLocalLimit {
                field: LocalLimitField::MultipleFrameRatio,
            });
        }
        if self.multiple_frame_ratio > 100 {
            return Err(ConfigError::MultipleFrameRatioOutOfRange {
                value: self.multiple_frame_ratio,
            });
        }
        if self.starvation_limit == 0 {
            return Err(ConfigError::ZeroLocalLimit {
                field: LocalLimitField::StarvationLimit,
            });
        }
        Ok(())
    }

    const fn for_baud_rate_unchecked(baud_rate: u32) -> Self {
        let config = Self {
            station: 0,
            max_master: MAX_MASTER,
            max_info_frames: 1,
            baud_rate,
            t_frame_abort: or_max(ceil_bit_times(baud_rate, 60)),
            t_frame_gap: or_max(floor_bit_times(baud_rate, 20)),
            t_postdrive: or_max(floor_bit_times(baud_rate, 15)),
            t_reply_delay: MAX_REPLY_DELAY,
            t_reply_timeout: MIN_REPLY_TIMEOUT,
            t_turnaround: or_max(ceil_bit_times(baud_rate, 40)),
            t_usage_delay: MAX_USAGE_DELAY,
            t_usage_timeout: MIN_USAGE_TIMEOUT,
            reply_transmit_margin: DEFAULT_REPLY_TRANSMIT_MARGIN,
            queue_max_age: Duration::from_secs(5),
            broadcast_unicast_ratio: 1,
            multiple_frame_ratio: 100,
            starvation_limit: DEFAULT_STARVATION_LIMIT,
            transmit_watchdog: DEFAULT_TRANSMIT_WATCHDOG,
            extended_frames: false,
        };
        config.with_covering_watchdog()
    }

    /// The longest NPDU this configuration sends or takes: 1497 octets with
    /// [`Self::extended_frames`] on (Clauses 9.3.9, 9.3.10), else 501
    /// (Clauses 9.3.6, 9.3.7).
    #[must_use]
    pub const fn max_npdu(&self) -> usize {
        if self.extended_frames {
            MAX_EXTENDED_MPDU_DATA
        } else {
            MAX_STANDARD_MPDU_DATA
        }
    }

    /// The time the longest frame this configuration sends takes on the
    /// wire: [`MAX_EXTENDED_FRAME_LENGTH`] octets with
    /// [`Self::extended_frames`] on, else [`MAX_STANDARD_FRAME_LENGTH`].
    #[must_use]
    pub const fn longest_frame_time(&self) -> Duration {
        let octets = if self.extended_frames {
            MAX_EXTENDED_FRAME_LENGTH
        } else {
            MAX_STANDARD_FRAME_LENGTH
        };
        // At most 1516 octets, so the cast is exact.
        self.octet_time().saturating_mul(octets as u32)
    }

    /// The shortest [`Self::transmit_watchdog`] validation accepts: the
    /// longest frame's time on the wire ([`Self::longest_frame_time`]) plus
    /// T_postdrive, which the watchdog's span from the confirmed start to
    /// completion also covers, plus 1 ms for the adapter to report the
    /// completion, the lateness at which the core counts a deadline missed
    /// ([`LATE_DEADLINE`](crate::LATE_DEADLINE)). A watchdog at this floor
    /// leaves nothing more for a slow adapter; the constructors' defaults
    /// leave a second or more.
    #[must_use]
    pub const fn minimum_transmit_watchdog(&self) -> Duration {
        self.longest_frame_time()
            .saturating_add(self.t_postdrive)
            .saturating_add(WATCHDOG_COMPLETION_MARGIN)
    }

    /// This configuration with [`Self::extended_frames`] on, and a transmit
    /// watchdog that clears the longest extended frame: the one it has if
    /// that clears it already, otherwise that frame's time on the wire plus
    /// one second.
    #[must_use]
    pub const fn with_extended_frames(mut self) -> Self {
        self.extended_frames = true;
        self.with_covering_watchdog()
    }

    /// This configuration, with a transmit watchdog that clears
    /// [`Self::minimum_transmit_watchdog`]: the one it has if it does, else
    /// the longest frame's time plus the default one second.
    const fn with_covering_watchdog(mut self) -> Self {
        if self.transmit_watchdog.as_nanos() < self.minimum_transmit_watchdog().as_nanos() {
            self.transmit_watchdog = self
                .longest_frame_time()
                .saturating_add(DEFAULT_TRANSMIT_WATCHDOG);
        }
        self
    }
}

/// `bits` bit times at `baud_rate`, rounded up to a whole microsecond, for
/// minimum bounds. `None` for a zero baud rate or on overflow.
///
/// The arithmetic is 32-bit: every caller asks for at most 60 bit times,
/// whose 60,000,000 microsecond-bits fit a `u32`, so the result is exact and
/// a 32-bit target links no 64-bit division routine for it. Nanosecond steps
/// would need 60,000,000,000, past a `u32`, and whole microseconds keep each
/// value what it was before the core counted nanoseconds.
const fn ceil_bit_times(baud_rate: u32, bits: u32) -> Option<Duration> {
    match bits.checked_mul(1_000_000) {
        Some(numerator) if baud_rate != 0 => {
            Some(Duration::from_micros(numerator.div_ceil(baud_rate) as u64))
        }
        _ => None,
    }
}

/// `bits` bit times at `baud_rate`, rounded down to a whole microsecond, for
/// maximum bounds. `None` for a zero baud rate or on overflow. 32-bit, as
/// [`ceil_bit_times`] is.
const fn floor_bit_times(baud_rate: u32, bits: u32) -> Option<Duration> {
    match bits.checked_mul(1_000_000) {
        Some(numerator) => match numerator.checked_div(baud_rate) {
            Some(micros) => Some(Duration::from_micros(micros as u64)),
            None => None,
        },
        None => None,
    }
}

/// Whether `value` is shorter than `bound`: `<`, which a `const fn` can't
/// call on [`Duration`].
const fn shorter(value: Duration, bound: Duration) -> bool {
    value.as_nanos() < bound.as_nanos()
}

/// Whether `value` is longer than `bound`.
const fn longer(value: Duration, bound: Duration) -> bool {
    value.as_nanos() > bound.as_nanos()
}

/// A bit-time default for [`MstpCoreConfig::for_baud_rate_unchecked`]: a
/// zero baud rate gives [`Duration::MAX`], which `validate` then rejects.
const fn or_max(duration: Option<Duration>) -> Duration {
    match duration {
        Some(duration) => duration,
        None => Duration::MAX,
    }
}

#[cfg(test)]
mod tests;
