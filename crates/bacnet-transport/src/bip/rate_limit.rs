//! Bounded inbound BBMD management rate limiter.
//!
//! Transport-local, fixed-window quota for the four covered inbound BVLC
//! management requests. Private to B/IP receive handling; no public
//! configuration, no counters, and no effect on data traffic or the
//! unconditional Write-BDT not-supported answer.

use std::time::{Duration, Instant};

use bacnet_types::enums::BvlcFunction;

/// Fixed window length for management quota accounting.
pub(super) const MANAGEMENT_RATE_WINDOW: Duration = Duration::from_secs(1);
/// Maximum covered requests admitted per source IPv4 address per window.
pub(super) const MANAGEMENT_RATE_PER_SOURCE_IP: u32 = 16;
/// Maximum covered requests admitted globally per transport per window.
pub(super) const MANAGEMENT_RATE_GLOBAL: u32 = 256;
/// Maximum distinct source IPv4 addresses tracked per window.
pub(super) const MANAGEMENT_RATE_MAX_SOURCES: usize = 256;
/// Maximum management response bytes admitted per source IPv4 address per window.
pub(super) const MANAGEMENT_RESPONSE_BYTES_PER_SOURCE_IP: usize = 4096;
/// Maximum management response bytes admitted globally per transport per window.
pub(super) const MANAGEMENT_RESPONSE_BYTES_GLOBAL: usize = 32768;

/// Operational counters for BACnet/IP BBMD management traffic.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ManagementCounters {
    /// Number of Read-BDT-ACK responses sent.
    pub read_bdt_responses: u64,
    /// Number of Read-FDT-ACK responses sent.
    pub read_fdt_responses: u64,
    /// Total bytes of Read-BDT-ACK and Read-FDT-ACK responses sent.
    pub response_bytes_sent: u64,
    /// Number of management responses throttled due to byte budget exhaustion.
    pub response_throttled: u64,
}

/// Tracking entry for a source IPv4 address within the current window.
#[derive(Clone, Copy, Debug, Default)]
struct SourceQuota {
    ip: [u8; 4],
    requests: u32,
    response_bytes: usize,
}

/// Whether an inbound BVLC function shares the combined management quota.
///
/// Covered: Read-BDT, Read-FDT, Register-Foreign-Device,
/// Delete-FDT-Entry. Everything else — including Write-BDT (which must
/// always answer not-supported), Distribute-Broadcast-To-Network, NPDU
/// data functions, BVLC-Result, Read ACKs, and unknown functions — is
/// outside the limiter.
pub(super) fn is_covered_management_request(function: BvlcFunction) -> bool {
    function == BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE
        || function == BvlcFunction::READ_FOREIGN_DEVICE_TABLE
        || function == BvlcFunction::REGISTER_FOREIGN_DEVICE
        || function == BvlcFunction::DELETE_FOREIGN_DEVICE_TABLE_ENTRY
}

/// Fixed-window, strictly bounded management quota.
///
/// Tracks at most [`MANAGEMENT_RATE_MAX_SOURCES`] source IPv4 addresses in
/// a preallocated array; the window reset clears accounting without
/// allocating. Time is supplied by the caller so accounting stays
/// deterministic in tests without sleeps.
#[derive(Debug)]
pub(super) struct ManagementRateLimiter {
    window_start: Option<Instant>,
    admitted: u32,
    response_bytes_in_window: usize,
    entries: [SourceQuota; MANAGEMENT_RATE_MAX_SOURCES],
    len: usize,
    counters: ManagementCounters,
}

impl ManagementRateLimiter {
    pub(super) fn new() -> Self {
        Self {
            window_start: None,
            admitted: 0,
            response_bytes_in_window: 0,
            entries: [SourceQuota {
                ip: [0; 4],
                requests: 0,
                response_bytes: 0,
            }; MANAGEMENT_RATE_MAX_SOURCES],
            len: 0,
            counters: ManagementCounters::default(),
        }
    }

    fn roll_window_if_expired(&mut self, now: Instant) {
        match self.window_start {
            None => {
                self.window_start = Some(now);
            }
            Some(start) => {
                if now.saturating_duration_since(start) >= MANAGEMENT_RATE_WINDOW {
                    self.window_start = Some(now);
                    self.admitted = 0;
                    self.response_bytes_in_window = 0;
                    self.len = 0;
                }
            }
        }
    }

    /// Admit (`true`) or silently discard (`false`) one covered request.
    ///
    /// Keyed by source IPv4 address only. Rejected requests leave all
    /// accounting unchanged and never create tracking entries.
    pub(super) fn check(&mut self, ip: [u8; 4], now: Instant) -> bool {
        self.roll_window_if_expired(now);

        if self.admitted >= MANAGEMENT_RATE_GLOBAL {
            return false;
        }

        for i in 0..self.len {
            if self.entries[i].ip == ip {
                if self.entries[i].requests >= MANAGEMENT_RATE_PER_SOURCE_IP {
                    return false;
                }
                self.entries[i].requests += 1;
                self.admitted += 1;
                return true;
            }
        }

        if self.len >= MANAGEMENT_RATE_MAX_SOURCES {
            return false;
        }
        self.entries[self.len] = SourceQuota {
            ip,
            requests: 1,
            response_bytes: 0,
        };
        self.len += 1;
        self.admitted += 1;
        true
    }

    /// Production entry point using monotonic time.
    pub(super) fn check_now(&mut self, ip: [u8; 4]) -> bool {
        self.check(ip, Instant::now())
    }

    /// Admit (`true`) or throttle (`false`) response bytes for a source IPv4 address.
    ///
    /// Enforces per-source and global response byte budgets within the current window.
    pub(super) fn check_response(&mut self, ip: [u8; 4], bytes: usize, now: Instant) -> bool {
        self.roll_window_if_expired(now);

        if self.response_bytes_in_window.saturating_add(bytes) > MANAGEMENT_RESPONSE_BYTES_GLOBAL {
            return false;
        }

        for i in 0..self.len {
            if self.entries[i].ip == ip {
                if self.entries[i].response_bytes.saturating_add(bytes)
                    > MANAGEMENT_RESPONSE_BYTES_PER_SOURCE_IP
                {
                    return false;
                }
                self.entries[i].response_bytes += bytes;
                self.response_bytes_in_window += bytes;
                return true;
            }
        }

        if bytes > MANAGEMENT_RESPONSE_BYTES_PER_SOURCE_IP {
            return false;
        }

        if self.len >= MANAGEMENT_RATE_MAX_SOURCES {
            return false;
        }

        self.entries[self.len] = SourceQuota {
            ip,
            requests: 0,
            response_bytes: bytes,
        };
        self.len += 1;
        self.response_bytes_in_window += bytes;
        true
    }

    /// Check response byte admission using monotonic time.
    pub(super) fn check_response_now(&mut self, ip: [u8; 4], bytes: usize) -> bool {
        self.check_response(ip, bytes, Instant::now())
    }

    /// Record an admitted Read-BDT-ACK response.
    pub(super) fn record_read_bdt_response(&mut self, bytes: usize) {
        self.counters.read_bdt_responses += 1;
        self.counters.response_bytes_sent += bytes as u64;
    }

    /// Record an admitted Read-FDT-ACK response.
    pub(super) fn record_read_fdt_response(&mut self, bytes: usize) {
        self.counters.read_fdt_responses += 1;
        self.counters.response_bytes_sent += bytes as u64;
    }

    /// Record a throttled management response.
    pub(super) fn record_response_throttled(&mut self) {
        self.counters.response_throttled += 1;
    }

    /// Check response budget and record counters for a Read-BDT-ACK response.
    pub(super) fn check_and_record_bdt_response(&mut self, ip: [u8; 4], bytes: usize) -> bool {
        if self.check_response_now(ip, bytes) {
            self.record_read_bdt_response(bytes);
            true
        } else {
            self.record_response_throttled();
            false
        }
    }

    /// Check response budget and record counters for a Read-FDT-ACK response.
    pub(super) fn check_and_record_fdt_response(&mut self, ip: [u8; 4], bytes: usize) -> bool {
        if self.check_response_now(ip, bytes) {
            self.record_read_fdt_response(bytes);
            true
        } else {
            self.record_response_throttled();
            false
        }
    }

    /// Query the cumulative management counters.
    pub(super) fn counters(&self) -> ManagementCounters {
        self.counters
    }

    #[cfg(test)]
    pub(super) fn tracked_source_count(&self) -> usize {
        self.len
    }

    #[cfg(test)]
    pub(super) fn admitted_in_window(&self) -> u32 {
        self.admitted
    }

    #[cfg(test)]
    pub(super) fn response_bytes_in_window(&self) -> usize {
        self.response_bytes_in_window
    }
}

impl Default for ManagementRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}
