//! Foreign device registration policy, quotas, and operational counters.
//!
//! Implements explicit foreign device registration enablement, fail-closed
//! admission, per-source quotas, reserved capacity, broadcast fanout bounding,
//! and rate limiting per ASHRAE Standard 135-2020 Annex J.5.2.2.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Foreign device registration policy for a BBMD.
///
/// Controls admission, quotas, rate limiting, and fanout limits for foreign
/// device registrations. By default, BBMD mode operates fail-closed with no
/// policy enabled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignDevicePolicy {
    /// Minimum allowed TTL in seconds.
    pub min_ttl: u16,
    /// Maximum allowed TTL in seconds.
    pub max_ttl: u16,
    /// Maximum active FDT entries allowed per source IPv4 address.
    pub max_entries_per_source: usize,
    /// Optional ACL of allowed source IPv4 addresses.
    /// `None` allows any source IP subject to quotas and rates.
    /// `Some(list)` requires the source IP to be present in `list`; an empty
    /// list denies all sources.
    pub allowed_sources: Option<Vec<[u8; 4]>>,
    /// Number of FDT slots reserved exclusively for `reserved_sources`.
    pub reserved_capacity: usize,
    /// Source IPv4 addresses authorized to use reserved capacity.
    pub reserved_sources: Vec<[u8; 4]>,
    /// Broadcast forwarding fanout budget for FDT targets.
    pub max_fdt_fanout: usize,
    /// Maximum registrations admitted per source IP per rate window.
    pub registration_rate_per_source: u32,
    /// Maximum registrations admitted globally per rate window.
    pub registration_rate_global: u32,
    /// Duration of the rate window.
    pub rate_window: Duration,
}

impl Default for ForeignDevicePolicy {
    fn default() -> Self {
        Self {
            min_ttl: 1,
            max_ttl: 3600,
            max_entries_per_source: 8,
            allowed_sources: None,
            reserved_capacity: 0,
            reserved_sources: Vec::new(),
            max_fdt_fanout: 32,
            registration_rate_per_source: 8,
            registration_rate_global: 64,
            rate_window: Duration::from_secs(1),
        }
    }
}

/// Operational counters for BBMD Foreign Device Table management and traffic.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FdtCounters {
    /// Number of foreign device registrations successfully admitted.
    pub registrations_accepted: u64,
    /// Number of foreign device registrations rejected.
    pub registrations_rejected: u64,
    /// Number of expired foreign device registrations purged.
    pub registrations_expired: u64,
    /// Number of registrations rejected due to FDT capacity exhaustion.
    pub capacity_exhausted: u64,
    /// Number of times the broadcast forwarding fanout budget for FDT targets was reached.
    pub fanout_budget_reached: u64,
}

/// Window-based registration rate limiter for foreign device registrations.
#[derive(Debug, Clone)]
pub(crate) struct ForeignDeviceRateTracker {
    window_start: Option<Instant>,
    global_count: u32,
    source_counts: HashMap<[u8; 4], u32>,
}

impl ForeignDeviceRateTracker {
    pub(crate) fn new() -> Self {
        Self {
            window_start: None,
            global_count: 0,
            source_counts: HashMap::new(),
        }
    }

    /// Check whether admitting another registration from `ip` at `now` would
    /// exceed either the per-source or global rate limit.
    pub(crate) fn is_rate_exceeded(
        &mut self,
        ip: [u8; 4],
        now: Instant,
        rate_window: Duration,
        max_per_source: u32,
        max_global: u32,
    ) -> bool {
        match self.window_start {
            None => {
                self.window_start = Some(now);
                self.global_count = 0;
                self.source_counts.clear();
            }
            Some(start) => {
                if now.saturating_duration_since(start) >= rate_window {
                    self.window_start = Some(now);
                    self.global_count = 0;
                    self.source_counts.clear();
                }
            }
        }

        let source_count = self.source_counts.get(&ip).copied().unwrap_or(0);
        source_count >= max_per_source || self.global_count >= max_global
    }

    /// Record an admitted registration.
    pub(crate) fn record(&mut self, ip: [u8; 4]) {
        self.global_count = self.global_count.saturating_add(1);
        let entry = self.source_counts.entry(ip).or_insert(0);
        *entry = entry.saturating_add(1);
    }
}
