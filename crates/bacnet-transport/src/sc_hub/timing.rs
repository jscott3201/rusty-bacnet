//! The accepting Hub's optional probe policy and one monotonic time origin.

use bacnet_types::error::Error;
use std::time::Duration;
use tokio::time::Instant;

/// Local, scan-driven Hub liveness probes; not the initiating node's AB.6.3 duty.
///
/// Defaults are 30s scans, 60s idle age, 5s pending-ACK age, 5s send budget
/// and a 360s liveness bound ([`SC_HUB_PROBE_LIVENESS_BOUND`]).
/// A scan probes only when idle age strictly exceeds the configured age. Pending
/// age begins at reservation, before sink acquisition. A probe the connector
/// hasn't answered does not retire it on its own (#2141): a later scan
/// retires the peer only once nothing at all has arrived from it since the
/// probe for longer than both the ACK age and the liveness bound. Any inbound
/// BVLC message that counts as activity, including the connector's own
/// Heartbeat-Request, shows it is alive and lets the next idle period probe
/// again; until then an unanswered probe is not repeated. Neither bound is a
/// hard deadline: serial sends and scheduler delay can postpone subsequent
/// scans. Missed scans are skipped. The send budget covers sink acquisition
/// plus send, and a probe send that fails or overruns it still retires the
/// peer at once.
///
/// All durations must be positive whole milliseconds, be at most `i64::MAX` milliseconds (reserving elapsed-tick headroom),
/// and be representable as a future monotonic instant on this platform. These
/// are local representation bounds, not BACnet's initiating-node 3–300s range.
/// The liveness bound is also capped at [`SC_HUB_PROBE_MAX_LIVENESS_BOUND`].
/// Transit relays have a separate configured acquisition-plus-send budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScHubProbePolicy {
    scan_interval: Duration,
    idle_age: Duration,
    ack_age: Duration,
    send_budget: Duration,
    liveness_bound: Duration,
}

/// Default probe liveness bound (#2141): 360 seconds.
///
/// A connector is the initiating peer of its hub connection, so Annex AB.6.3
/// makes keep-alive its job and obliges only the accepting peer (the hub) to
/// answer a Heartbeat-Request. A conforming connector may therefore ignore the
/// hub's probes. It sends its own Heartbeat-Request once its heartbeat timeout
/// passes without receiving a BVLC message, and the hub's probe is such a
/// message, so the connector's next heartbeat can come up to one full timeout
/// after the probe. AB.6.3 holds a fixed heartbeat timeout to at most 300
/// seconds and requires a configurable one to offer at least 3 to 300 seconds,
/// so this default, counted from the probe, covers all of those plus 60
/// seconds for reply lag, scheduling and network delay, as the direct
/// listener's idle bound does (#2133).
///
/// Only frames received from the connector count; frames the hub sends it,
/// probes and relayed traffic alike, don't, because the hub keeps relaying to
/// a connector that may already be dead. The trade-off: a connector that
/// receives relayed traffic often enough never needs to heartbeat, so if it
/// also sends nothing and ignores probes it is retired after this bound.
///
/// Cost: a dead connector is retired only after this bound, counted from the
/// unanswered probe, instead of a few seconds after it. With the default
/// policy that is about seven and a half to eight minutes (450–480 s) after its last frame: idle
/// age, up to one scan to send the probe, this bound, and up to one more scan
/// to notice. A connector configured for a heartbeat timeout above 300
/// seconds needs a larger bound ([`ScHubProbePolicy::with_liveness_bound`]);
/// a bound no larger than the ACK age brings back retirement on the first
/// unanswered probe, unless the connector sends something first.
pub const SC_HUB_PROBE_LIVENESS_BOUND: Duration = Duration::from_secs(360);

/// Largest liveness bound [`ScHubProbePolicy::with_liveness_bound`] accepts:
/// one day, as for the direct listener's idle bound.
pub const SC_HUB_PROBE_MAX_LIVENESS_BOUND: Duration = Duration::from_secs(24 * 60 * 60);

impl ScHubProbePolicy {
    /// Validate all timing values without binding a socket or reading TLS files.
    ///
    /// The liveness bound starts at [`SC_HUB_PROBE_LIVENESS_BOUND`]; change it
    /// with [`Self::with_liveness_bound`].
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
            liveness_bound: SC_HUB_PROBE_LIVENESS_BOUND,
        };
        policy.validate()?;
        Ok(policy)
    }

    /// Set the liveness bound (builder-style), validated like the other
    /// durations and at most [`SC_HUB_PROBE_MAX_LIVENESS_BOUND`].
    ///
    /// A connector that hasn't answered a probe is retired only once nothing
    /// has arrived from it since that probe for longer than this bound (and
    /// the ACK age). See [`SC_HUB_PROBE_LIVENESS_BOUND`] for the default's
    /// reasoning: a value under 300 seconds can retire a conforming connector
    /// that ignores probes and heartbeats on a longer timeout.
    pub fn with_liveness_bound(mut self, bound: Duration) -> Result<Self, Error> {
        self.liveness_bound = bound;
        self.validate()?;
        Ok(self)
    }

    pub(super) fn validate(self) -> Result<(), Error> {
        for (name, value) in [
            ("scan interval", self.scan_interval),
            ("idle age", self.idle_age),
            ("ACK age", self.ack_age),
            ("send budget", self.send_budget),
            ("liveness bound", self.liveness_bound),
        ] {
            validate_milliseconds(&format!("probe {name}"), value)?;
        }
        if self.liveness_bound > SC_HUB_PROBE_MAX_LIVENESS_BOUND {
            return Err(Error::Encoding(format!(
                "Hub probe liveness bound must be at most {SC_HUB_PROBE_MAX_LIVENESS_BOUND:?}"
            )));
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
    /// Strictly exceeded pending age after which a probe counts as unanswered.
    pub fn ack_age(self) -> Duration {
        self.ack_age
    }
    /// Absolute budget around both sink acquisition and probe send.
    pub fn send_budget(self) -> Duration {
        self.send_budget
    }
    /// Strictly exceeded time since an unanswered probe, with nothing received
    /// from the connector in between, required for scan-driven retirement.
    pub fn liveness_bound(self) -> Duration {
        self.liveness_bound
    }
}

impl Default for ScHubProbePolicy {
    fn default() -> Self {
        Self {
            scan_interval: Duration::from_secs(30),
            idle_age: Duration::from_secs(60),
            ack_age: Duration::from_secs(5),
            send_budget: Duration::from_secs(5),
            liveness_bound: SC_HUB_PROBE_LIVENESS_BOUND,
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
        return Err(Error::Encoding(format!(
            "Hub {name} must be whole milliseconds in 1..=i64::MAX and representable by the monotonic clock"
        )));
    }
    Ok(())
}
