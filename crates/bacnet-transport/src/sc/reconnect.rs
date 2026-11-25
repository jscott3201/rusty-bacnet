use bacnet_types::error::{Error, Refusal};

/// Configuration for SC transport reconnection with jittered exponential backoff.
///
/// Recovery after losing an established hub connection runs in rounds. Each
/// round waits a jittered backoff, then dials the primary hub and, when that
/// fails, the failover hub if one is configured. Both hubs get a turn from the
/// first round on, and the primary always goes first, whichever hub was lost
/// (Annex AB.5.2). Recovery ends when a hub accepts, when no hub can be dialled
/// any more, when a hub forbids further retries, or when the transport stops.
/// `max_retries` bounds the number of rounds; `None` keeps going.
///
/// The nominal backoff starts at `initial_delay_ms` and doubles after each failed
/// round, capped at `max_delay_ms`. Each round's sleep uses fresh OS
/// randomness between `max(initial_delay_ms, backoff / 2)` and
/// `min(max_delay_ms, backoff + backoff / 2)`, inclusive. Thus the initial delay
/// remains a floor and the maximum remains a cap, including at the capped step.
/// Equal initial and maximum delays leave no room for jitter. If OS randomness
/// is unavailable, the nominal backoff is used. A round sleeps once, before its
/// first dial; the failover dial follows a failed primary dial at once, since it
/// goes to a different hub.
///
/// The start-up dial (the primary, then the failover hub when the primary
/// cannot be reached) and primary restoration are separate and bounded; neither
/// counts against `max_retries`. While the failover hub is active, each attempt
/// to restore the primary starts `initial_delay_ms` after the failover connected
/// or the previous attempt ended.
#[derive(Debug, Clone)]
pub struct ScReconnectConfig {
    /// Initial nominal backoff and minimum reconnect sleep (ms), nonzero and
    /// no greater than `max_delay_ms`. With `max_retries: None` it must be at
    /// least [`Self::UNLIMITED_MIN_INITIAL_DELAY_MS`] (one second).
    pub initial_delay_ms: u64,
    /// Maximum delay between reconnect rounds (ms), nonzero and at most
    /// [`Self::MAX_DELAY_MS`] (ten minutes).
    pub max_delay_ms: u64,
    /// Reconnect rounds after an established connection is lost: `Some(n)`
    /// makes at most `n` rounds and `None` keeps going while a hub can still be
    /// dialled. `Some(0)` turns recovery off, so the transport gives up at the
    /// first loss; the start-up dial and primary restoration while on the
    /// failover hub still run. Defaults to `Some(10)`.
    pub max_retries: Option<u32>,
}

impl Default for ScReconnectConfig {
    fn default() -> Self {
        Self {
            initial_delay_ms: 10_000,
            max_delay_ms: 600_000,
            max_retries: Some(10),
        }
    }
}

impl ScReconnectConfig {
    /// The largest `max_delay_ms` [`validate`](Self::validate) accepts:
    /// 600,000 ms, ten minutes. Annex AB.6.1 lets the wait between reconnect
    /// attempts grow after each failure, but never past 600 seconds, and the
    /// jittered sleep never exceeds `max_delay_ms`.
    pub const MAX_DELAY_MS: u64 = 600_000;

    /// The smallest `initial_delay_ms` unlimited recovery (`max_retries:
    /// None`) accepts: one second, so a node retrying a hub that stays down
    /// for days cannot dial it in a tight loop. A local choice, not one the
    /// standard makes.
    pub const UNLIMITED_MIN_INITIAL_DELAY_MS: u64 = 1_000;

    /// Check that delays are nonzero, the initial delay does not exceed the
    /// maximum, `max_delay_ms` is at most [`Self::MAX_DELAY_MS`] (ten
    /// minutes), and that unlimited recovery (`max_retries: None`) has an
    /// initial delay of at least [`Self::UNLIMITED_MIN_INITIAL_DELAY_MS`].
    ///
    /// These are defensive guards. They impose no production minimum delay on
    /// bounded recovery, and acceptance does not establish deployment safety or
    /// Annex AB.6.1 conformance (whose range for the initial wait this does not
    /// check). The transport's `start` and the hub dialer
    /// call this before dialling; call it yourself before handing a socket to a
    /// raw SC transport.
    pub fn validate(&self) -> Result<(), Error> {
        if self.initial_delay_ms == 0 {
            return Err(Error::invalid_config(
                "BACnet/SC reconnect initial_delay_ms must be greater than zero",
            ));
        }
        if self.max_delay_ms == 0 {
            return Err(Error::invalid_config(
                "BACnet/SC reconnect max_delay_ms must be greater than zero",
            ));
        }
        if self.initial_delay_ms > self.max_delay_ms {
            return Err(Error::InvalidConfig(
                Refusal::new(
                    "BACnet/SC reconnect initial_delay_ms must not exceed max_delay_ms, \
                     got initial_delay_ms={} max_delay_ms={}",
                )
                .with_values(self.initial_delay_ms, self.max_delay_ms),
            ));
        }
        if self.max_delay_ms > Self::MAX_DELAY_MS {
            return Err(Error::InvalidConfig(
                Refusal::new(
                    "BACnet/SC reconnect max_delay_ms must not exceed {} (10 minutes), \
                     got max_delay_ms={}",
                )
                .with_values(Self::MAX_DELAY_MS, self.max_delay_ms),
            ));
        }
        if self.max_retries.is_none()
            && self.initial_delay_ms < Self::UNLIMITED_MIN_INITIAL_DELAY_MS
        {
            return Err(Error::InvalidConfig(
                Refusal::new(
                    "BACnet/SC reconnect initial_delay_ms must be at least {} with unlimited \
                     retries, got initial_delay_ms={}",
                )
                .with_values(Self::UNLIMITED_MIN_INITIAL_DELAY_MS, self.initial_delay_ms),
            ));
        }
        Ok(())
    }
}

impl super::ScConnection {
    pub(super) fn reset_for_connect_retry(&mut self) {
        *self = Self::new(self.local_vmac, self.device_uuid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnect_validation_rejects_zero_and_inverted_delays_for_all_retry_counts() {
        for max_retries in [0, 1, 10, u32::MAX] {
            for (initial_delay_ms, max_delay_ms, field) in [
                (0, 1, "initial_delay_ms"),
                (1, 0, "max_delay_ms"),
                (0, 0, "initial_delay_ms"),
                (2, 1, "initial_delay_ms"),
                (u64::MAX, u64::MAX - 1, "initial_delay_ms"),
            ] {
                let config = ScReconnectConfig {
                    initial_delay_ms,
                    max_delay_ms,
                    max_retries: Some(max_retries),
                };
                assert!(
                    matches!(config.validate(), Err(Error::InvalidConfig(message))
                        if message.to_string().contains("reconnect") && message.to_string().contains(field)),
                    "{config:?}"
                );
            }
        }
    }

    #[test]
    fn reconnect_validation_rejects_over_cap_delays_for_all_retry_counts() {
        for max_retries in [0, 1, 10, u32::MAX] {
            for max_delay_ms in [600_001, 86_400_000, u64::MAX] {
                for initial_delay_ms in [1, max_delay_ms] {
                    let config = ScReconnectConfig {
                        initial_delay_ms,
                        max_delay_ms,
                        max_retries: Some(max_retries),
                    };
                    match config.validate() {
                        Err(Error::InvalidConfig(message)) => assert_eq!(
                            message.to_string(),
                            format!(
                                "BACnet/SC reconnect max_delay_ms must not exceed 600000 (10 minutes), \
                                 got max_delay_ms={max_delay_ms}"
                            )
                        ),
                        result => {
                            panic!("expected delay-cap rejection for {config:?}, got {result:?}")
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn reconnect_validation_accepts_positive_ordered_delays_through_cap_without_retry_cap() {
        let default = ScReconnectConfig::default();
        assert_eq!(default.initial_delay_ms, 10_000);
        assert_eq!(default.max_delay_ms, 600_000);
        assert_eq!(default.max_retries, Some(10));
        default.validate().unwrap();

        // Validate only: accepted values are not safe timer/deployment promises.
        for max_retries in [0, 1, 10, u32::MAX] {
            for (initial_delay_ms, max_delay_ms) in [
                (1, 1),
                (1, 2),
                (10_000, 600_000),
                (1, 600_000),
                (599_999, 600_000),
                (600_000, 600_000),
            ] {
                ScReconnectConfig {
                    initial_delay_ms,
                    max_delay_ms,
                    max_retries: Some(max_retries),
                }
                .validate()
                .unwrap();
            }
        }
    }

    /// Unlimited recovery needs a one-second floor; bounded recovery keeps
    /// accepting shorter delays.
    #[test]
    fn unlimited_recovery_rejects_an_initial_delay_under_one_second() {
        let config = |initial_delay_ms, max_retries| ScReconnectConfig {
            initial_delay_ms,
            max_delay_ms: 600_000,
            max_retries,
        };
        for initial_delay_ms in [1, 999] {
            match config(initial_delay_ms, None).validate() {
                Err(Error::InvalidConfig(message)) => assert_eq!(
                    message.to_string(),
                    format!(
                        "BACnet/SC reconnect initial_delay_ms must be at least 1000 with \
                         unlimited retries, got initial_delay_ms={initial_delay_ms}"
                    )
                ),
                result => {
                    panic!("expected the unlimited floor for {initial_delay_ms}, got {result:?}")
                }
            }
            config(initial_delay_ms, Some(u32::MAX)).validate().unwrap();
        }
        for initial_delay_ms in [1_000, 10_000, 600_000] {
            config(initial_delay_ms, None).validate().unwrap();
        }
        // The ordinary checks still come first.
        assert!(matches!(
            config(0, None).validate(),
            Err(Error::InvalidConfig(message)) if message.to_string().contains("greater than zero")
        ));
    }
}
