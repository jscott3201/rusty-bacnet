use bacnet_types::error::Error;

/// Configuration for SC transport reconnection with jittered exponential backoff.
///
/// The nominal backoff starts at `initial_delay_ms` and doubles after each failed
/// active-hub retry, capped at `max_delay_ms`. Each reconnect sleep uses fresh OS
/// randomness between `max(initial_delay_ms, backoff / 2)` and
/// `min(max_delay_ms, backoff + backoff / 2)`, inclusive. Thus the initial delay
/// remains a floor and the maximum remains a cap, including at the capped step.
/// Equal initial and maximum delays leave no room for jitter. If OS randomness
/// is unavailable, the nominal backoff is used. Jitter applies only to active-hub
/// retries, not the separate failover attempt or primary-restoration timer.
#[derive(Debug, Clone)]
pub struct ScReconnectConfig {
    /// Initial nominal backoff and minimum reconnect sleep (ms), nonzero and
    /// no greater than `max_delay_ms`.
    pub initial_delay_ms: u64,
    /// Maximum delay between reconnect attempts (ms), nonzero and at most
    /// 86_400_000 (24 hours). This is a local defensive cap, not a conformance limit.
    pub max_delay_ms: u64,
    /// Maximum reconnect attempts on the active hub after a disconnect.
    /// Zero skips these retries, not the initial connection, eligible failover,
    /// or primary restoration while connected to failover.
    pub max_retries: u32,
}

impl Default for ScReconnectConfig {
    fn default() -> Self {
        Self {
            initial_delay_ms: 10_000,
            max_delay_ms: 600_000,
            max_retries: 10,
        }
    }
}

impl ScReconnectConfig {
    /// Check that delays are nonzero, the initial delay does not exceed the
    /// maximum, and `max_delay_ms` is at most 86_400_000 (24 hours).
    ///
    /// This defensive guard applies even when `max_retries` is zero. It does not
    /// impose a production minimum delay or retry-count policy;
    /// acceptance does not establish deployment safety or Annex AB.6.1 conformance.
    /// Call this before dialing if supplying a socket to a raw SC transport.
    pub fn validate(&self) -> Result<(), Error> {
        if self.initial_delay_ms == 0 {
            return Err(Error::OutOfRange(
                "BACnet/SC reconnect initial_delay_ms must be greater than zero".into(),
            ));
        }
        if self.max_delay_ms == 0 {
            return Err(Error::OutOfRange(
                "BACnet/SC reconnect max_delay_ms must be greater than zero".into(),
            ));
        }
        if self.initial_delay_ms > self.max_delay_ms {
            return Err(Error::OutOfRange(format!(
                "BACnet/SC reconnect initial_delay_ms must not exceed max_delay_ms, \
                 got initial_delay_ms={} max_delay_ms={}",
                self.initial_delay_ms, self.max_delay_ms
            )));
        }
        if self.max_delay_ms > 86_400_000 {
            return Err(Error::OutOfRange(format!(
                "BACnet/SC reconnect max_delay_ms must not exceed 86400000 (24 hours), \
                 got max_delay_ms={}",
                self.max_delay_ms
            )));
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
                    max_retries,
                };
                assert!(
                    matches!(config.validate(), Err(Error::OutOfRange(message))
                        if message.contains("reconnect") && message.contains(field)),
                    "{config:?}"
                );
            }
        }
    }

    #[test]
    fn reconnect_validation_rejects_over_cap_delays_for_all_retry_counts() {
        for max_retries in [0, 1, 10, u32::MAX] {
            for max_delay_ms in [86_400_001, u64::MAX] {
                for initial_delay_ms in [1, max_delay_ms] {
                    let config = ScReconnectConfig {
                        initial_delay_ms,
                        max_delay_ms,
                        max_retries,
                    };
                    match config.validate() {
                        Err(Error::OutOfRange(message)) => assert_eq!(
                            message,
                            format!(
                                "BACnet/SC reconnect max_delay_ms must not exceed 86400000 (24 hours), \
                                 got max_delay_ms={max_delay_ms}"
                            )
                        ),
                        result => panic!("expected delay-cap rejection for {config:?}, got {result:?}"),
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
        assert_eq!(default.max_retries, 10);
        default.validate().unwrap();

        // Validate only: accepted values are not safe timer/deployment promises.
        for max_retries in [0, 1, 10, u32::MAX] {
            for (initial_delay_ms, max_delay_ms) in [
                (1, 1),
                (1, 2),
                (10_000, 600_000),
                (600_001, 600_001),
                (1, 86_400_000),
                (86_399_999, 86_400_000),
                (86_400_000, 86_400_000),
            ] {
                ScReconnectConfig {
                    initial_delay_ms,
                    max_delay_ms,
                    max_retries,
                }
                .validate()
                .unwrap();
            }
        }
    }
}
