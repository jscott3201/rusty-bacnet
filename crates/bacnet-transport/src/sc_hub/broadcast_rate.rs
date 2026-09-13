//! Always-on local broadcast relay policy; see `broadcast-rate-policy.md`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use bacnet_types::error::Error;
use tokio::time::Instant;

use super::{HubRelayTarget, ScHub};
use crate::sc::diagnostic_throttle::DiagnosticThrottle;
use crate::sc_frame::Vmac;

const TOKEN: u64 = 1_000_000_000;

/// Always-on, continuously refilled token buckets for hub broadcast relay only.
///
/// Defaults: each sender gets **128 broadcasts/second, burst 1024**; the entire
/// hub gets **512 broadcasts/second, burst 4096**. One token admits one broadcast
/// request, not one recipient. The existing 256-client cap bounds each admitted
/// request to at most 255 recipient writes. Buckets start full and never queue.
///
/// These generous local defaults use a one-second tuning scale (the node's
/// solicited-advertisement interval), well below the existing 30-second heartbeat
/// interval and 6-second transaction retry timeout. Even a single sender may
/// relay 768 broadcasts per retry interval after its initial burst. They are not
/// normative BACnet rates or a measured site capacity guarantee.
/// The sender burst leaves headroom over the existing 513-broadcast retirement
/// test; eight seconds of credit covers that burst independently of test speed.
///
/// Tune to measured broadcast peaks and recipient count with
/// [`super::ScHubTlsConfig::with_broadcast_rate_policy`]. Reduce the global rate
/// for large/slow fanouts; increase burst capacity for legitimate discovery
/// bursts. Exhaustion silently drops; there is no disabled/zero setting.
/// Startup rejects every bound outside `1..=u64::MAX / 1_000_000_000`.
///
/// Per-sender state belongs to the connection handler holding the registered
/// VMAC lease, not to a peer-supplied origin. Reconnection resets that bucket,
/// but never the hub aggregate. NPDU, Unknown and Proprietary broadcasts share
/// both budgets. Unicast (including destination-stripped relays) and control
/// processing are excluded. Existing activity/heartbeat policy is unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScHubBroadcastRatePolicy {
    /// Maximum immediate burst per registered sender connection.
    pub sender_burst: u64,
    /// Continuous per-sender refill, in broadcast requests per second.
    pub sender_per_second: u64,
    /// Maximum immediate burst across all connections of one hub.
    pub global_burst: u64,
    /// Continuous aggregate refill, in broadcast requests per second.
    pub global_per_second: u64,
}

impl Default for ScHubBroadcastRatePolicy {
    fn default() -> Self {
        Self {
            sender_burst: 1024,
            sender_per_second: 128,
            global_burst: 4096,
            global_per_second: 512,
        }
    }
}

impl ScHubBroadcastRatePolicy {
    fn validate(self) -> Result<(), Error> {
        for (name, value) in [
            ("sender_burst", self.sender_burst),
            ("sender_per_second", self.sender_per_second),
            ("global_burst", self.global_burst),
            ("global_per_second", self.global_per_second),
        ] {
            if value == 0 || value.checked_mul(TOKEN).is_none() {
                return Err(Error::Encoding(format!(
                    "hub broadcast {name} must be in 1..={}",
                    u64::MAX / TOKEN
                )));
            }
        }
        Ok(())
    }
}

/// Saturating lifetime drop counts for one hub, independent of admission state.
///
/// Every rate drop increments exactly one field. Sender exhaustion is checked
/// first. A sender token is spent even if the aggregate subsequently rejects;
/// sender-rejected requests never spend aggregate tokens. Counts are not reset
/// on peer replacement/reconnection and are never inputs to limiting decisions.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ScHubBroadcastDropCounts {
    /// Broadcasts dropped because their sender's bucket was exhausted.
    pub sender_exhausted: u64,
    /// Broadcasts passing the sender budget but exceeding the hub budget.
    pub global_exhausted: u64,
}

impl ScHub {
    /// Read the two broadcast-drop counters, including after [`Self::stop`].
    ///
    /// Each counter is atomic; the pair is not a transactional snapshot while
    /// workers are active. No per-VMAC statistics/map or wire response is created.
    pub fn broadcast_drop_counts(&self) -> ScHubBroadcastDropCounts {
        self.tasks.broadcast.drop_counts()
    }
}

struct Bucket {
    credit: u64,
    capacity: u64,
    rate: u64,
    updated: Instant,
}

impl Bucket {
    // Bounds are validated once before construction. Nanotokens retain partial
    // refills exactly: frequent rejected requests cannot postpone replenishment.
    fn new(burst: u64, rate: u64, now: Instant) -> Self {
        Self {
            credit: burst * TOKEN,
            capacity: burst * TOKEN,
            rate,
            updated: now,
        }
    }

    fn take(&mut self, now: Instant) -> bool {
        if now > self.updated {
            let refill = now
                .duration_since(self.updated)
                .as_nanos()
                .saturating_mul(u128::from(self.rate))
                .min(u128::from(self.capacity - self.credit));
            self.credit += refill as u64;
            self.updated = now;
        }
        if self.credit < TOKEN {
            return false;
        }
        self.credit -= TOKEN;
        true
    }
}

pub(super) struct HubBudget {
    policy: ScHubBroadcastRatePolicy,
    bucket: Mutex<Bucket>,
    sender_drops: AtomicU64,
    global_drops: AtomicU64,
}

impl HubBudget {
    pub(super) fn new(policy: ScHubBroadcastRatePolicy) -> Result<Self, Error> {
        policy.validate()?;
        Ok(Self {
            policy,
            bucket: Mutex::new(Bucket::new(
                policy.global_burst,
                policy.global_per_second,
                Instant::now(),
            )),
            sender_drops: AtomicU64::new(0),
            global_drops: AtomicU64::new(0),
        })
    }

    fn drop_counts(&self) -> ScHubBroadcastDropCounts {
        ScHubBroadcastDropCounts {
            sender_exhausted: self.sender_drops.load(Ordering::Relaxed),
            global_exhausted: self.global_drops.load(Ordering::Relaxed),
        }
    }
}

impl Default for HubBudget {
    fn default() -> Self {
        Self::new(ScHubBroadcastRatePolicy::default()).expect("valid default broadcast policy")
    }
}

tokio::task_local! {
    // Explicit connection-scoped injection keeps rate policy out of handshake,
    // heartbeat and retirement APIs. The hub's Tasks owner retains the Arc.
    pub(super) static HUB_BUDGET: Arc<HubBudget>;
}

pub(super) struct SenderBudget {
    hub: Arc<HubBudget>,
    bucket: Bucket,
    diagnostic: DiagnosticThrottle,
}

fn increment(counter: &AtomicU64) {
    // Statistics only: no state publication or admission decision uses them.
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        Some(value.saturating_add(1))
    });
}

impl SenderBudget {
    pub(super) fn for_connection() -> Self {
        #[cfg(not(test))]
        let hub = HUB_BUDGET.with(Arc::clone);
        #[cfg(test)]
        let hub = HUB_BUDGET.try_with(Arc::clone).unwrap_or_else(|_| {
            // Only private direct-handler test harnesses bypass accept_loop.
            // Public startup always supplies a hub-owned shared budget.
            Arc::new(HubBudget::default())
        });
        Self::new(hub, Instant::now())
    }

    fn new(hub: Arc<HubBudget>, now: Instant) -> Self {
        let bucket = Bucket::new(hub.policy.sender_burst, hub.policy.sender_per_second, now);
        Self {
            hub,
            bucket,
            diagnostic: DiagnosticThrottle::new(),
        }
    }

    pub(super) fn admit(&mut self, target: HubRelayTarget, registered_vmac: Vmac) -> bool {
        if self.admit_at(target, Instant::now()) {
            return true;
        }
        // Same bounded warning idiom as malformed_diag, separate from the
        // saturating lifetime counters. AB.2 disallows broadcast responses.
        super::malformed_diag::emit(&mut self.diagnostic, |suppressed| {
            tracing::warn!(
                ?registered_vmac,
                suppressed,
                "Hub: broadcast relay rate limit exceeded, dropping"
            );
        });
        false
    }

    fn admit_at(&mut self, target: HubRelayTarget, now: Instant) -> bool {
        if target != HubRelayTarget::Broadcast {
            return true;
        }
        if !self.bucket.take(now) {
            increment(&self.hub.sender_drops);
            return false;
        }
        // One fixed-shape synchronous critical section, no I/O/await or other
        // lock held. The sender quota limits contention from any single task.
        if !self.hub.bucket.lock().unwrap().take(now) {
            increment(&self.hub.global_drops);
            return false;
        }
        true
    }
}

#[cfg(test)]
#[path = "broadcast_rate_live_tests.rs"]
mod live_tests;
#[cfg(test)]
#[path = "broadcast_rate_tests.rs"]
mod tests;
