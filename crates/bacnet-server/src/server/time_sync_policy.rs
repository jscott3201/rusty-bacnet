//! Local opt-in inbound clock policy, independent of DCC and mutation policy.

use super::{BipServerBuilder, ServerBuilder, TransportPort};
use bacnet_encoding::npdu::NpduAddress;
use bacnet_network::layer::ReceivedApdu;
use bacnet_objects::clock::ClockFrame;
use bacnet_types::error::Error;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Exact claimed time-sync source, not an authenticated principal.
///
/// Unconfirmed dispatch exposes transport MAC and NPDU source only. On SC the
/// MAC is the source VMAC, not a certificate identity or proof of trust. Routed
/// identity takes precedence; a trusted router MAC does not trust its clients.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum TimeSyncSource {
    /// Complete transport-native MAC, only when no routed source is present.
    Direct(Vec<u8>),
    /// Complete claimed routed source, independent of the immediate router.
    Routed {
        /// Source network (1..=65534).
        network: u16,
        /// Source address (1..=255 octets).
        address: Vec<u8>,
    },
}

impl TimeSyncSource {
    fn validate(&self) -> Result<(), Error> {
        let address = match self {
            Self::Direct(address) => address,
            Self::Routed { network, address } => {
                if !(1..=65534).contains(network) {
                    return Err(denied("source network must be 1..=65534"));
                }
                address
            }
        };
        if !(1..=255).contains(&address.len()) {
            return Err(denied("source address must contain 1..=255 octets"));
        }
        Ok(())
    }

    fn from_received(received: &ReceivedApdu) -> Result<Self, Error> {
        let source = match &received.source_network {
            Some(source) => Self::Routed {
                network: source.network,
                address: source.mac_address.to_vec(),
            },
            None => Self::Direct(received.source_mac.to_vec()),
        };
        source.validate()?;
        Ok(source)
    }
}

/// Validated exact-source allowlist. A configured empty list denies everyone.
/// `None` in [`TimeSyncPolicy`] leaves sources unrestricted.
#[derive(Clone, Eq, PartialEq)]
pub struct TimeSyncSourceRestriction(Vec<TimeSyncSource>);

impl std::fmt::Debug for TimeSyncSourceRestriction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TimeSyncSourceRestriction")
            .field("entries", &self.0.len())
            .finish_non_exhaustive()
    }
}

impl TimeSyncSourceRestriction {
    /// Accept at most 256 entries with 1..=255 address octets and valid networks.
    /// These are local configuration limits, not authentication guarantees.
    pub fn new(sources: Vec<TimeSyncSource>) -> Result<Self, Error> {
        if sources.len() > 256 {
            return Err(denied("source restriction allows at most 256 entries"));
        }
        for source in &sources {
            source.validate()?;
        }
        Ok(Self(sources))
    }

    pub(super) fn allows(&self, mac: &[u8], routed: Option<&NpduAddress>) -> bool {
        // Do not use admission's malformed-routed fallback for authorization.
        match routed {
            Some(source) => self.0.iter().any(|entry| matches!(entry,
                TimeSyncSource::Routed { network, address }
                if *network == source.network && address.as_slice() == source.mac_address.as_slice())),
            None => self.0.iter().any(|entry|
                matches!(entry, TimeSyncSource::Direct(address) if address.as_slice() == mac)),
        }
    }
}

/// Opt-in token bucket for accepted local and UTC time-sync requests combined.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeSyncRateLimit {
    /// Positive finite refill rate in requests per second of monotonic time.
    pub max_per_second: f64,
    /// Positive initial and maximum token capacity.
    pub burst_capacity: u32,
}

impl TimeSyncRateLimit {
    fn validate(self) -> Result<(), Error> {
        if !self.max_per_second.is_finite()
            || self.max_per_second <= 0.0
            || self.burst_capacity == 0
        {
            return Err(denied(
                "rate must be positive and finite with a positive burst",
            ));
        }
        Ok(())
    }
}

/// Default-allow inbound TimeSynchronization / UTCTimeSynchronization policy.
///
/// Every restriction is opt-in and independent. No default step cap is imposed:
/// a controller booting with an incorrect clock may need a large initial sync.
/// Operators should configure trusted claimed sources (not authentication), and
/// choose a cap only when their boot/recovery process can tolerate rejection.
/// This is local policy, not a change to the unconfirmed wire behavior (16.7/16.8).
/// Denials send no response and do not change the clock or invoke its observer.
/// Decoding/validation precedes source and rate policy, then step-check and clock
/// update are serialized. The observer runs afterwards outside the limiter lock;
/// a concurrent accepted request may already have advanced the clock again.
#[derive(Clone, Debug, PartialEq)]
pub struct TimeSyncPolicy {
    /// Accept inbound time sync by default. False disables clock touch and callback.
    pub enabled: bool,
    /// Optional exact-source allowlist; Some(empty) denies all, None allows all.
    pub source_restriction: Option<TimeSyncSourceRestriction>,
    /// Optional absolute step cap, inclusive at the boundary, for forward/backward
    /// corrections. Zero allows only an exact match. No readable clock means a
    /// configured cap fails closed; without a cap, boot synchronization still works.
    pub max_step: Option<Duration>,
    /// Optional per-source token bucket, shared across local and UTC requests.
    pub per_source_rate: Option<TimeSyncRateLimit>,
    /// Optional server-wide token bucket, including all sources and both services.
    pub global_rate: Option<TimeSyncRateLimit>,
    /// Minimum spacing between accepted requests from one source, regardless of
    /// value or local/UTC form. Zero disables coalescing. Drops do not extend it.
    pub coalesce_window: Duration,
    /// Minimum spacing between accepted requests globally, across all sources
    /// and both services. Zero disables global coalescing. Drops do not extend it.
    pub global_coalesce_window: Duration,
    /// Bound on rate/coalescing source state (1..=65536, default 256). A full
    /// table rejects new sources until an entry is fully refilled and outside its
    /// coalescing window; active entries are never evicted to reset their budget.
    pub max_sources: usize,
}

impl Default for TimeSyncPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            source_restriction: None,
            max_step: None,
            per_source_rate: None,
            global_rate: None,
            coalesce_window: Duration::ZERO,
            global_coalesce_window: Duration::ZERO,
            max_sources: 256,
        }
    }
}

impl TimeSyncPolicy {
    /// Validate before transport startup/dialing, even when acceptance is disabled.
    pub fn validate(&self) -> Result<(), Error> {
        for limit in [self.per_source_rate, self.global_rate]
            .into_iter()
            .flatten()
        {
            limit.validate()?;
        }
        if !(1..=65536).contains(&self.max_sources) {
            return Err(denied("max_sources must be 1..=65536"));
        }
        Ok(())
    }
}

impl<T: TransportPort + 'static> ServerBuilder<T> {
    /// Configure opt-in inbound clock restrictions; default is unrestricted allow.
    pub fn time_sync_policy(mut self, policy: TimeSyncPolicy) -> Self {
        self.config.time_sync_policy = policy;
        self
    }
}

impl BipServerBuilder {
    /// Configure opt-in inbound clock restrictions; see [`TimeSyncPolicy`].
    pub fn time_sync_policy(mut self, policy: TimeSyncPolicy) -> Self {
        self.config.time_sync_policy = policy;
        self
    }
}

#[cfg(feature = "sc-tls")]
impl super::ScServerBuilder {
    /// Configure inbound clock restrictions on claimed SC VMAC / routed sources.
    /// Dispatch does not expose certificate principals; see [`TimeSyncSource`].
    pub fn time_sync_policy(mut self, policy: TimeSyncPolicy) -> Self {
        self.config.time_sync_policy = policy;
        self
    }
}

pub(super) fn denied(reason: &str) -> Error {
    Error::Encoding(format!("time sync: {reason}"))
}

/// Compare in the request's time basis; no clock mutation or floating-point time.
pub(super) fn step_hundredths(supplied: i128, is_utc: bool, frame: ClockFrame) -> Option<u128> {
    let local = super::clock::date_time_to_hundredths(frame.local_date, frame.local_time).ok()?;
    let current = if is_utc {
        local + i128::from(frame.utc_offset) * 6000
            - if frame.daylight_savings_status {
                360_000
            } else {
                0
            }
    } else {
        local
    };
    Some(supplied.abs_diff(current))
}

pub(super) fn check_step(delta: Option<u128>, cap: Option<Duration>) -> Result<(), Error> {
    if let Some(cap) = cap {
        let delta = delta.ok_or_else(|| denied("clock unreadable for step cap"))?;
        if delta * 10_000_000 > cap.as_nanos() {
            return Err(denied("step cap exceeded"));
        }
    }
    Ok(())
}

struct Bucket {
    tokens: f64,
    last_refill: Instant,
}

impl Bucket {
    fn new(limit: Option<TimeSyncRateLimit>, now: Instant) -> Self {
        Self {
            tokens: limit.map_or(0.0, |l| f64::from(l.burst_capacity)),
            last_refill: now,
        }
    }

    fn refill(&mut self, limit: Option<TimeSyncRateLimit>, now: Instant) {
        if let Some(limit) = limit {
            let elapsed = now
                .saturating_duration_since(self.last_refill)
                .as_secs_f64();
            self.tokens =
                (self.tokens + elapsed * limit.max_per_second).min(f64::from(limit.burst_capacity));
        }
        self.last_refill = self.last_refill.max(now);
    }

    fn ready(&self, limit: Option<TimeSyncRateLimit>) -> bool {
        limit.is_none() || self.tokens >= 1.0
    }

    fn deduct(&mut self, limit: Option<TimeSyncRateLimit>) {
        if limit.is_some() {
            self.tokens -= 1.0;
        }
    }
}

struct SourceState {
    bucket: Bucket,
    last_accepted: Instant,
}

struct State {
    global: Bucket,
    last_accepted: Option<Instant>,
    sources: HashMap<TimeSyncSource, SourceState>,
}

/// Server-lifetime synchronization admission state, separate from discovery.
/// The mutex also serializes step-check + clock update across request tasks.
pub(super) struct TimeSyncLimiter {
    policy: TimeSyncPolicy,
    state: Mutex<State>,
}

// Keep construction plumbing out of the size-capped lifecycle. Policies and
// mutable state remain independent; both Arcs have the native server lifetime.
pub(super) fn request_limiters(
    config: &super::ServerConfig,
    device_instance: Option<u32>,
) -> (Arc<super::DiscoveryLimiter>, Arc<TimeSyncLimiter>) {
    (
        Arc::new(super::DiscoveryLimiter::new(
            config.discovery_policy.sanitized(),
            device_instance,
        )),
        Arc::new(TimeSyncLimiter::new(config.time_sync_policy.clone())),
    )
}

impl TimeSyncLimiter {
    pub(super) fn new(policy: TimeSyncPolicy) -> Self {
        Self {
            state: Mutex::new(State {
                global: Bucket::new(policy.global_rate, Instant::now()),
                last_accepted: None,
                sources: HashMap::new(),
            }),
            policy,
        }
    }

    pub(super) fn apply_at(
        &self,
        received: &ReceivedApdu,
        now: Instant,
        apply: impl FnOnce() -> Result<(), Error>,
    ) -> Result<(), Error> {
        let policy = &self.policy;
        let track_source = policy.per_source_rate.is_some() || !policy.coalesce_window.is_zero();
        let key = track_source
            .then(|| TimeSyncSource::from_received(received))
            .transpose()?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| denied("limiter lock poisoned"))?;
        if state
            .last_accepted
            .is_some_and(|last| now.saturating_duration_since(last) < policy.global_coalesce_window)
        {
            return Err(denied("coalesced globally"));
        }
        state.global.refill(policy.global_rate, now);
        if !state.global.ready(policy.global_rate) {
            return Err(denied("global rate limit"));
        }
        if let Some(key) = &key {
            if let Some(source) = state.sources.get_mut(key) {
                if now.saturating_duration_since(source.last_accepted) < policy.coalesce_window {
                    return Err(denied("coalesced source"));
                }
                source.bucket.refill(policy.per_source_rate, now);
                if !source.bucket.ready(policy.per_source_rate) {
                    return Err(denied("source rate limit"));
                }
            } else if state.sources.len() >= policy.max_sources {
                state.sources.retain(|_, source| {
                    source.bucket.refill(policy.per_source_rate, now);
                    let full = policy.per_source_rate.is_none_or(|limit| {
                        source.bucket.tokens >= f64::from(limit.burst_capacity)
                    });
                    !full
                        || now.saturating_duration_since(source.last_accepted)
                            < policy.coalesce_window
                });
                if state.sources.len() >= policy.max_sources {
                    return Err(denied("source table full"));
                }
            }
        }
        // No await or observer under this lock. Failed step/sync does not spend
        // tokens or postpone the next legitimate synchronization.
        apply()?;
        state.global.deduct(policy.global_rate);
        state.last_accepted = Some(now);
        if let Some(key) = key {
            let source = state.sources.entry(key).or_insert_with(|| SourceState {
                bucket: Bucket::new(policy.per_source_rate, now),
                last_accepted: now,
            });
            source.bucket.deduct(policy.per_source_rate);
            source.last_accepted = now;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "time_sync_policy_tests.rs"]
mod tests;
