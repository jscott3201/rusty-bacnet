//! The accepting Hub's optional probe policy and one monotonic time origin.

use bacnet_types::error::Error;
use std::time::Duration;
use tokio::time::Instant;

/// Local, scan-driven Hub liveness probes; not the initiating node's AB.6.3 duty.
///
/// Defaults are 30s scans, 60s idle age, 5s pending-ACK age and 5s send budget.
/// A scan probes only when idle age strictly exceeds the configured age. Pending
/// age begins at reservation, before sink acquisition, and must also strictly
/// exceed its bound before a later scan retires the peer. This is not a hard ACK
/// deadline: serial sends and scheduler delay can postpone subsequent scans.
/// Missed scans are skipped. The send budget covers sink acquisition plus send.
///
/// All durations must be positive whole milliseconds, be at most `i64::MAX` milliseconds (reserving elapsed-tick headroom),
/// and be representable as a future monotonic instant on this platform. These
/// are local representation bounds, not BACnet's initiating-node 3–300s range.
/// Transit relays have a separate configured acquisition-plus-send budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScHubProbePolicy {
    scan_interval: Duration,
    idle_age: Duration,
    ack_age: Duration,
    send_budget: Duration,
}

impl ScHubProbePolicy {
    /// Validate all timing values without binding a socket or reading TLS files.
    pub fn new(
        scan_interval: Duration,
        idle_age: Duration,
        ack_age: Duration,
        send_budget: Duration,
    ) -> Result<Self, Error> {
        let policy = Self {
            scan_interval,
            idle_age,
            ack_age,
            send_budget,
        };
        policy.validate()?;
        Ok(policy)
    }

    pub(super) fn validate(self) -> Result<(), Error> {
        for (name, value) in [
            ("scan interval", self.scan_interval),
            ("idle age", self.idle_age),
            ("ACK age", self.ack_age),
            ("send budget", self.send_budget),
        ] {
            validate_milliseconds(&format!("probe {name}"), value)?;
        }
        Ok(())
    }

    /// Interval between scan opportunities; delayed ticks are skipped.
    pub fn scan_interval(self) -> Duration {
        self.scan_interval
    }
    /// Strictly exceeded idle age required to reserve a probe.
    pub fn idle_age(self) -> Duration {
        self.idle_age
    }
    /// Strictly exceeded pending age required for scan-driven retirement.
    pub fn ack_age(self) -> Duration {
        self.ack_age
    }
    /// Absolute budget around both sink acquisition and probe send.
    pub fn send_budget(self) -> Duration {
        self.send_budget
    }
}

impl Default for ScHubProbePolicy {
    fn default() -> Self {
        Self {
            scan_interval: Duration::from_secs(30),
            idle_age: Duration::from_secs(60),
            ack_age: Duration::from_secs(5),
            send_budget: Duration::from_secs(5),
        }
    }
}

/// Copied into owned tasks; every copy retains the same Hub origin and policy.
#[derive(Clone, Copy)]
pub(super) struct HubTiming {
    pub(super) origin: Instant,
    pub policy: ScHubProbePolicy,
    pub relay_send_budget: Duration,
}

impl HubTiming {
    pub fn new(policy: ScHubProbePolicy) -> Self {
        Self {
            origin: Instant::now(),
            policy,
            relay_send_budget: Duration::from_secs(5),
        }
    }
    pub fn now_ms(self) -> u64 {
        self.origin.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
    }
}

// Shared representation boundary for local Hub probe and unicast settings.
pub(super) fn validate_milliseconds(name: &str, value: Duration) -> Result<(), Error> {
    if value.is_zero()
        || !value.subsec_nanos().is_multiple_of(1_000_000)
        || value.as_millis() > i64::MAX as u128
        || Instant::now().checked_add(value).is_none()
    {
        return Err(Error::Encoding(format!("Hub {name} must be whole milliseconds in 1..=i64::MAX and representable by the monotonic clock")));
    }
    Ok(())
}
