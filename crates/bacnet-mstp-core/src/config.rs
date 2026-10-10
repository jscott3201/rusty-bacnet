//! Configuration and Clause 9 timing validation for the MS/TP master core.

use crate::clock::Duration;

const MAX_MASTER_ADDRESS: u8 = 127;
const DEFAULT_BAUD_RATE: u32 = 9_600;
const MAX_FRAME_ABORT: Duration = Duration::from_millis(100);

/// A configuration value that violates a protocol or local safety bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigError {
    /// A baud rate of zero cannot be used to derive bit-time limits.
    ZeroBaudRate,
    /// A derived timing calculation overflowed the microsecond representation.
    TimingOverflow,
    /// Clause 9 fixes Npoll at 50 token uses.
    InvalidPollCount {
        /// Supplied poll count.
        value: u8,
    },
    /// Clause 9 fixes Nretry_token at one retry.
    InvalidTokenRetryCount {
        /// Supplied token retry count.
        value: u8,
    },
    /// Clause 9 fixes Nmin_octets at four octets.
    InvalidMinimumOctets {
        /// Supplied minimum idle-octet count.
        value: u8,
    },
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
    /// T_no_token is the fixed 500 ms lost-token deadline.
    InvalidNoToken {
        /// Supplied no-token duration.
        value: Duration,
    },
    /// T_reply_delay cannot exceed 250 ms.
    ReplyDelayTooLong {
        /// Supplied reply-delay duration.
        value: Duration,
        /// Maximum permitted duration.
        maximum: Duration,
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
    /// T_slot is the ten-millisecond maintenance slot.
    InvalidSlot {
        /// Supplied maintenance-slot duration.
        value: Duration,
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
}

/// Names of local duration settings used by [`ConfigError::ZeroLocalDuration`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalDurationField {
    QueueMaxAge,
    TransmitWatchdog,
}

/// Names of local count/ratio settings used by [`ConfigError::ZeroLocalLimit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalLimitField {
    BroadcastUnicastRatio,
    MultipleFrameRatio,
    StarvationLimit,
}

/// All timing, addressing, and bounded-queue parameters needed by an MS/TP
/// master.
///
/// Durations are represented in microseconds.  Values whose limits are stated
/// in bit times are still stored as absolute durations so a core instance can
/// keep absolute deadlines without retaining a platform clock or baud-rate
/// conversion helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MstpCoreConfig {
    /// This station's MAC address.
    pub station: u8,
    /// Max_Master, inclusive.
    pub max_master: u8,
    /// Max_Info_Frames, the number of information frames per token use.
    pub max_info_frames: u8,
    /// Serial baud rate used to validate bit-time-derived limits.
    pub baud_rate: u32,

    /// Npoll: token uses between maintenance polls.
    pub n_poll: u8,
    /// Nretry_token: token retries before finding a new successor.
    pub n_retry_token: u8,
    /// Nmin_octets: minimum idle octets before a new transmission.
    pub n_min_octets: u8,

    /// T_frame_abort.
    pub t_frame_abort: Duration,
    /// T_frame_gap.
    pub t_frame_gap: Duration,
    /// T_no_token.
    pub t_no_token: Duration,
    /// T_postdrive.
    pub t_postdrive: Duration,
    /// T_reply_delay.
    pub t_reply_delay: Duration,
    /// T_reply_timeout.
    pub t_reply_timeout: Duration,
    /// T_slot.
    pub t_slot: Duration,
    /// T_turnaround.
    pub t_turnaround: Duration,
    /// T_usage_delay.
    pub t_usage_delay: Duration,
    /// T_usage_timeout.
    pub t_usage_timeout: Duration,

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
    pub starvation_limit: u8,
    /// Maximum time allowed for one platform transmit operation.
    pub transmit_watchdog: Duration,
}

impl Default for MstpCoreConfig {
    fn default() -> Self {
        Self::for_baud_rate_unchecked(DEFAULT_BAUD_RATE)
    }
}

impl MstpCoreConfig {
    /// Construct standard-safe settings for a baud rate.
    pub fn for_baud_rate(baud_rate: u32) -> Result<Self, ConfigError> {
        if baud_rate == 0 {
            return Err(ConfigError::ZeroBaudRate);
        }
        let config = Self::for_baud_rate_unchecked(baud_rate);
        config.validate()?;
        Ok(config)
    }

    /// Return the standard 9600-baud configuration.
    #[must_use]
    pub const fn standard() -> Self {
        Self::for_baud_rate_unchecked(DEFAULT_BAUD_RATE)
    }

    /// Validate all protocol and local bounds.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.baud_rate == 0 {
            return Err(ConfigError::ZeroBaudRate);
        }
        if self.n_poll != 50 {
            return Err(ConfigError::InvalidPollCount { value: self.n_poll });
        }
        if self.n_retry_token != 1 {
            return Err(ConfigError::InvalidTokenRetryCount {
                value: self.n_retry_token,
            });
        }
        if self.n_min_octets != 4 {
            return Err(ConfigError::InvalidMinimumOctets {
                value: self.n_min_octets,
            });
        }
        if self.max_master > MAX_MASTER_ADDRESS {
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

        let frame_abort_min = ceil_bit_times(self.baud_rate, 60)?;
        let frame_gap_max = floor_bit_times(self.baud_rate, 20)?;
        let postdrive_max = floor_bit_times(self.baud_rate, 15)?;
        let turnaround_min = ceil_bit_times(self.baud_rate, 40)?;
        if self.t_frame_abort < frame_abort_min || self.t_frame_abort > MAX_FRAME_ABORT {
            return Err(ConfigError::FrameAbortOutOfRange {
                value: self.t_frame_abort,
                minimum: frame_abort_min,
                maximum: MAX_FRAME_ABORT,
            });
        }
        if self.t_frame_gap > frame_gap_max {
            return Err(ConfigError::FrameGapTooLong {
                value: self.t_frame_gap,
                maximum: frame_gap_max,
            });
        }
        if self.t_postdrive > postdrive_max {
            return Err(ConfigError::PostdriveTooLong {
                value: self.t_postdrive,
                maximum: postdrive_max,
            });
        }
        if self.t_reply_delay > Duration::from_millis(250) {
            return Err(ConfigError::ReplyDelayTooLong {
                value: self.t_reply_delay,
                maximum: Duration::from_millis(250),
            });
        }
        if self.t_no_token != Duration::from_millis(500) {
            return Err(ConfigError::InvalidNoToken {
                value: self.t_no_token,
            });
        }
        let reply_timeout_min = Duration::from_millis(255);
        let reply_timeout_max = Duration::from_millis(300);
        if self.t_reply_timeout < reply_timeout_min || self.t_reply_timeout > reply_timeout_max {
            return Err(ConfigError::ReplyTimeoutOutOfRange {
                value: self.t_reply_timeout,
                minimum: reply_timeout_min,
                maximum: reply_timeout_max,
            });
        }
        if self.t_slot != Duration::from_millis(10) {
            return Err(ConfigError::InvalidSlot { value: self.t_slot });
        }
        if self.t_turnaround < turnaround_min {
            return Err(ConfigError::TurnaroundTooShort {
                value: self.t_turnaround,
                minimum: turnaround_min,
            });
        }
        if self.t_turnaround > self.t_usage_delay {
            return Err(ConfigError::TurnaroundExceedsUsageDelay {
                turnaround: self.t_turnaround,
                usage_delay: self.t_usage_delay,
            });
        }
        if self.t_usage_delay > Duration::from_millis(15) {
            return Err(ConfigError::UsageDelayTooLong {
                value: self.t_usage_delay,
                maximum: Duration::from_millis(15),
            });
        }
        let usage_timeout_min = Duration::from_millis(20);
        let usage_timeout_max = Duration::from_millis(35);
        if self.t_usage_timeout < usage_timeout_min || self.t_usage_timeout > usage_timeout_max {
            return Err(ConfigError::UsageTimeoutOutOfRange {
                value: self.t_usage_timeout,
                minimum: usage_timeout_min,
                maximum: usage_timeout_max,
            });
        }
        if self.queue_max_age == Duration::ZERO {
            return Err(ConfigError::ZeroLocalDuration {
                field: LocalDurationField::QueueMaxAge,
            });
        }
        if self.transmit_watchdog == Duration::ZERO {
            return Err(ConfigError::ZeroLocalDuration {
                field: LocalDurationField::TransmitWatchdog,
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
        Self {
            station: 0,
            max_master: MAX_MASTER_ADDRESS,
            max_info_frames: 1,
            baud_rate,
            n_poll: 50,
            n_retry_token: 1,
            n_min_octets: 4,
            t_frame_abort: ceil_bit_times_const(baud_rate, 60),
            t_frame_gap: floor_bit_times_const(baud_rate, 20),
            t_no_token: Duration::from_millis(500),
            t_postdrive: floor_bit_times_const(baud_rate, 15),
            t_reply_delay: Duration::from_millis(250),
            t_reply_timeout: Duration::from_millis(255),
            t_slot: Duration::from_millis(10),
            t_turnaround: ceil_bit_times_const(baud_rate, 40),
            t_usage_delay: Duration::from_millis(15),
            t_usage_timeout: Duration::from_millis(20),
            queue_max_age: Duration::from_secs(5),
            broadcast_unicast_ratio: 1,
            multiple_frame_ratio: 100,
            starvation_limit: 8,
            transmit_watchdog: Duration::from_secs(1),
        }
    }
}

/// Return the ceiling of `bits / baud` in microseconds.
fn ceil_bit_times(baud_rate: u32, bits: u64) -> Result<Duration, ConfigError> {
    if baud_rate == 0 {
        return Err(ConfigError::ZeroBaudRate);
    }
    let numerator = bits
        .checked_mul(1_000_000)
        .ok_or(ConfigError::TimingOverflow)?;
    let baud = u64::from(baud_rate);
    let micros = numerator
        .checked_add(baud - 1)
        .ok_or(ConfigError::TimingOverflow)?
        / baud;
    Ok(Duration::from_micros(micros))
}

/// Return the floor of `bits / baud` in microseconds for maximum bounds.
fn floor_bit_times(baud_rate: u32, bits: u64) -> Result<Duration, ConfigError> {
    if baud_rate == 0 {
        return Err(ConfigError::ZeroBaudRate);
    }
    let numerator = bits
        .checked_mul(1_000_000)
        .ok_or(ConfigError::TimingOverflow)?;
    Ok(Duration::from_micros(numerator / u64::from(baud_rate)))
}

const fn ceil_bit_times_const(baud_rate: u32, bits: u64) -> Duration {
    let numerator = bits * 1_000_000;
    let baud = baud_rate as u64;
    if baud == 0 {
        Duration::MAX
    } else {
        Duration::from_micros((numerator + baud - 1) / baud)
    }
}

const fn floor_bit_times_const(baud_rate: u32, bits: u64) -> Duration {
    let baud = baud_rate as u64;
    if baud == 0 {
        Duration::MAX
    } else {
        Duration::from_micros((bits * 1_000_000) / baud)
    }
}

#[cfg(test)]
mod tests {
    use super::{ConfigError, MstpCoreConfig};
    use crate::clock::Duration;

    #[test]
    fn defaults_are_standard_safe() {
        let config = MstpCoreConfig::default();
        assert_eq!(config.baud_rate, 9_600);
        assert_eq!(config.n_poll, 50);
        assert_eq!(config.n_retry_token, 1);
        assert_eq!(config.n_min_octets, 4);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validation_rejects_zero_baud_and_bad_addresses() {
        let mut config = MstpCoreConfig::default();
        config.baud_rate = 0;
        assert_eq!(config.validate(), Err(ConfigError::ZeroBaudRate));

        config = MstpCoreConfig::default();
        config.max_master = 127;
        config.station = 128;
        assert!(matches!(
            config.validate(),
            Err(ConfigError::StationExceedsMaxMaster { .. })
        ));
    }

    #[test]
    fn validation_enforces_bit_time_edges_without_overflow() {
        let mut config = MstpCoreConfig::default();
        config.t_frame_abort = Duration::from_millis(1);
        assert!(matches!(
            config.validate(),
            Err(ConfigError::FrameAbortOutOfRange { .. })
        ));

        config = MstpCoreConfig::default();
        config.baud_rate = 1;
        assert!(matches!(
            config.validate(),
            Err(ConfigError::FrameAbortOutOfRange { .. })
        ));
    }

    #[test]
    fn baud_constructor_recomputes_bit_time_fields() {
        let config = MstpCoreConfig::for_baud_rate(19_200).expect("valid baud");
        assert!(config.validate().is_ok());
        assert!(config.t_turnaround.as_micros() >= 40_000_000 / 19_200);
    }

    #[test]
    fn validation_rejects_infeasible_response_and_ratio_policies() {
        let mut config = MstpCoreConfig::default();
        config.t_turnaround = Duration::from_millis(16);
        assert!(matches!(
            config.validate(),
            Err(ConfigError::TurnaroundExceedsUsageDelay { .. })
        ));

        config = MstpCoreConfig::default();
        config.multiple_frame_ratio = 101;
        assert_eq!(
            config.validate(),
            Err(ConfigError::MultipleFrameRatioOutOfRange { value: 101 })
        );
    }
}
