use super::{BipServerBuilder, ServerBuilder, TransportPort};
use bacnet_types::error::Error;
use std::sync::Mutex;
use tokio::time::Instant;

/// Optional global authorization budget for DISABLE_INITIATION, not ingress protection.
/// Use `Some(Default::default())` to enable; server configuration defaults to None.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DccDisableRateLimit {
    /// Initial and maximum whole tokens (1..=65535).
    pub capacity: u32,
    /// Time to earn one token, in milliseconds (1..=86400000).
    pub refill_interval_ms: u64,
}

impl Default for DccDisableRateLimit {
    fn default() -> Self {
        Self {
            capacity: 3,
            refill_interval_ms: 20_000,
        }
    }
}

impl DccDisableRateLimit {
    /// Validate local operator limits before startup/dialing, without time arithmetic.
    pub fn validate(self) -> Result<(), Error> {
        if !(1..=65535).contains(&self.capacity)
            || !(1..=86_400_000).contains(&self.refill_interval_ms)
        {
            return Err(Error::Encoding(
                "DCC disable rate requires capacity 1..=65535 and refill_interval_ms 1..=86400000"
                    .into(),
            ));
        }
        Ok(())
    }
}

pub(super) struct Bucket {
    interval_ns: u128,
    maximum: u128,
    state: Mutex<State>,
}

struct State {
    // Credit is measured in nanoseconds: one token costs interval_ns. This
    // retains fractional progress even across denied checks, without floats.
    credit: u128,
    last: Instant,
}

impl Bucket {
    pub(super) fn new(config: DccDisableRateLimit) -> Result<Self, Error> {
        config.validate()?;
        let interval_ns = u128::from(config.refill_interval_ms) * 1_000_000;
        let maximum = interval_ns * u128::from(config.capacity);
        Ok(Self {
            interval_ns,
            maximum,
            state: Mutex::new(State {
                credit: maximum,
                last: Instant::now(),
            }),
        })
    }

    pub(super) fn admit(&self) -> bool {
        let mut state = self.state.lock().unwrap();
        // Read time under the lock: concurrent callers cannot move last backwards.
        let now = Instant::now();
        state.credit = state
            .credit
            .saturating_add(now.duration_since(state.last).as_nanos())
            .min(self.maximum);
        state.last = now;
        if state.credit < self.interval_ns {
            return false;
        }
        state.credit -= self.interval_ns;
        // Admission is charged now, with no rollback on later cancellation.
        true
    }
}

impl<T: TransportPort + 'static> ServerBuilder<T> {
    /// Limit authorized DISABLE_INITIATION globally; None (default) disables it.
    /// ENABLE is exempt. Each new native server starts with a full bucket.
    pub fn dcc_disable_rate_limit(mut self, limit: Option<DccDisableRateLimit>) -> Self {
        self.config.dcc_disable_rate_limit = limit;
        self
    }
}

impl BipServerBuilder {
    /// Limit authorized DISABLE_INITIATION globally; None (default) disables it.
    pub fn dcc_disable_rate_limit(mut self, limit: Option<DccDisableRateLimit>) -> Self {
        self.config.dcc_disable_rate_limit = limit;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use tokio::time::{advance, Duration};

    #[tokio::test(start_paused = true)]
    async fn dcc_disable_rate_fractional_boundaries_and_idle_cap() {
        let bucket = Bucket::new(DccDisableRateLimit::default()).unwrap();
        for _ in 0..3 {
            assert!(bucket.admit());
        }
        assert!(!bucket.admit());
        // Repeated denials must preserve all accumulated fractional credit.
        for _ in 0..19 {
            advance(Duration::from_secs(1)).await;
            assert!(!bucket.admit());
        }
        advance(Duration::from_millis(999)).await;
        assert!(!bucket.admit());
        advance(Duration::from_micros(999)).await;
        assert!(!bucket.admit());
        advance(Duration::from_micros(1)).await;
        assert!(bucket.admit());
        assert!(!bucket.admit());
        advance(Duration::from_millis(45_500)).await;
        assert!(bucket.admit());
        assert!(bucket.admit());
        assert!(!bucket.admit());
        advance(Duration::from_millis(14_500)).await;
        assert!(bucket.admit());
        assert!(!bucket.admit());
        advance(Duration::from_secs(86400 * 365)).await;
        for _ in 0..3 {
            assert!(bucket.admit());
        }
        assert!(!bucket.admit());
    }

    #[test]
    fn dcc_disable_rate_concurrent_global_burst() {
        let bucket = Arc::new(
            Bucket::new(DccDisableRateLimit {
                capacity: 3,
                refill_interval_ms: 86_400_000,
            })
            .unwrap(),
        );
        let barrier = Arc::new(Barrier::new(32));
        let threads: Vec<_> = (0..32)
            .map(|_| {
                let bucket = bucket.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    bucket.admit()
                })
            })
            .collect();
        assert_eq!(
            threads
                .into_iter()
                .map(|t| usize::from(t.join().unwrap()))
                .sum::<usize>(),
            3
        );
        assert!(!bucket.admit());
    }
}
