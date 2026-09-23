//! Fixed, diagnostic-only per-start Hub outcomes.
use std::sync::atomic::{AtomicU64, Ordering};

/// Saturating lifetime outcomes for one started Hub, shared by its workers.
///
/// Refusals count selected decisions, not delivered NAKs. A refusal followed by
/// expiry can increment both counters. Unicast counts exclude malformed,
/// pre-registration, stale-source, self/local, broadcast and forwarded Result
/// traffic. No successful-send count is inferred from retired-sink skips.
/// Fields are individually sampled, not a transactional snapshot; none controls
/// policy. A new Hub starts at zero even when it reuses the same configuration.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ScHubOutcomeCounts {
    /// Committed incumbent UUID replacements.
    pub uuid_replacements: u64,
    /// Selected VMAC collision refusals, including the Hub VMAC.
    pub vmac_collision_rejections: u64,
    /// Selected registered-client capacity refusals.
    pub registered_capacity_rejections: u64,
    /// TCP accepts dropped at the total-active limit.
    pub total_active_accept_drops: u64,
    /// TCP accepts dropped at the handshake limit, after the total-active check.
    pub handshake_accept_drops: u64,
    /// Expired TLS handshake deadlines.
    pub tls_timeouts: u64,
    /// Expired WebSocket upgrade deadlines.
    pub websocket_timeouts: u64,
    /// Expired SC Connect-Request deadlines.
    pub connect_timeouts: u64,
    /// Eligible NPDU or opaque unicast requests without a registered destination.
    pub unicast_no_target: u64,
    /// Eligible unicast requests exceeding a destination length limit.
    pub unicast_target_limit: u64,
    /// Eligible unicast attempts whose sink acquisition or send budget expired.
    pub unicast_send_timeout: u64,
    /// Eligible unicast attempts returning a send error.
    pub unicast_send_error: u64,
    /// Matching-registration heartbeat removals, including generation exhaustion.
    pub heartbeat_retirements: u64,
}

#[derive(Default)]
pub(super) struct OutcomeCounters {
    pub(super) uuid_replacements: AtomicU64,
    pub(super) vmac_collision_rejections: AtomicU64,
    pub(super) registered_capacity_rejections: AtomicU64,
    pub(super) total_active_accept_drops: AtomicU64,
    pub(super) handshake_accept_drops: AtomicU64,
    pub(super) tls_timeouts: AtomicU64,
    pub(super) websocket_timeouts: AtomicU64,
    pub(super) connect_timeouts: AtomicU64,
    pub(super) unicast_no_target: AtomicU64,
    pub(super) unicast_target_limit: AtomicU64,
    pub(super) unicast_send_timeout: AtomicU64,
    pub(super) unicast_send_error: AtomicU64,
    pub(super) heartbeat_retirements: AtomicU64,
}

impl OutcomeCounters {
    pub(super) fn snapshot(&self) -> ScHubOutcomeCounts {
        ScHubOutcomeCounts {
            uuid_replacements: self.uuid_replacements.load(Ordering::Relaxed),
            vmac_collision_rejections: self.vmac_collision_rejections.load(Ordering::Relaxed),
            registered_capacity_rejections: self
                .registered_capacity_rejections
                .load(Ordering::Relaxed),
            total_active_accept_drops: self.total_active_accept_drops.load(Ordering::Relaxed),
            handshake_accept_drops: self.handshake_accept_drops.load(Ordering::Relaxed),
            tls_timeouts: self.tls_timeouts.load(Ordering::Relaxed),
            websocket_timeouts: self.websocket_timeouts.load(Ordering::Relaxed),
            connect_timeouts: self.connect_timeouts.load(Ordering::Relaxed),
            unicast_no_target: self.unicast_no_target.load(Ordering::Relaxed),
            unicast_target_limit: self.unicast_target_limit.load(Ordering::Relaxed),
            unicast_send_timeout: self.unicast_send_timeout.load(Ordering::Relaxed),
            unicast_send_error: self.unicast_send_error.load(Ordering::Relaxed),
            heartbeat_retirements: self.heartbeat_retirements.load(Ordering::Relaxed),
        }
    }
}

pub(super) fn increment(counter: &AtomicU64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        Some(value.saturating_add(1))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcomes_saturate_without_affecting_other_fields_or_instances() {
        let counts = OutcomeCounters::default();
        counts
            .unicast_send_error
            .store(u64::MAX - 1, Ordering::Relaxed);
        increment(&counts.unicast_send_error);
        increment(&counts.unicast_send_error);
        assert_eq!(
            counts.snapshot(),
            ScHubOutcomeCounts {
                unicast_send_error: u64::MAX,
                ..ScHubOutcomeCounts::default()
            }
        );
        assert_eq!(
            OutcomeCounters::default().snapshot(),
            ScHubOutcomeCounts::default()
        );
    }
}
