//! Broadcast forwarding fanout budgeting, deduplication, and work queue.
//!
//! Provides fair, bounded, non-blocking broadcast distribution for BBMD mode
//! per Issue #531. Broadcast forwarding is offloaded from the UDP receive loop
//! to an asynchronous worker task through a bounded work queue with per-origin
//! and global packet and byte rate limits.

use std::collections::HashMap;
use std::net::SocketAddrV4;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tracing::warn;

use crate::bvll::encode_bvll_forwarded;

/// Policy governing broadcast forwarding fanout limits, rate budgets, and work queue capacity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FanoutPolicy {
    /// Maximum forwarded packets emitted per input broadcast.
    pub max_fanout_per_input: usize,
    /// Global rate limit for forwarded packets per second.
    pub max_packets_per_sec_global: u32,
    /// Global rate limit for forwarded bytes per second.
    pub max_bytes_per_sec_global: usize,
    /// Per-origin rate limit for forwarded packets per second.
    pub max_packets_per_sec_per_origin: u32,
    /// Capacity of the bounded fanout forwarding work queue.
    pub queue_capacity: usize,
}

impl Default for FanoutPolicy {
    fn default() -> Self {
        Self {
            max_fanout_per_input: 64,
            max_packets_per_sec_global: 1024,
            max_bytes_per_sec_global: 512_000,
            max_packets_per_sec_per_origin: 128,
            queue_capacity: 256,
        }
    }
}

/// Operational telemetry counters for broadcast forwarding fanout.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FanoutCounters {
    /// Total forwarded packets emitted to the network.
    pub packets_forwarded: u64,
    /// Total forwarded bytes emitted to the network.
    pub bytes_forwarded: u64,
    /// Number of forwarded packets throttled or dropped due to fanout budgets/rate limits.
    pub packets_throttled: u64,
    /// Number of duplicate forwarding destinations skipped during resolution.
    pub destinations_deduplicated: u64,
    /// Number of fanout forwarding jobs dropped due to work queue overflow.
    pub queue_overflow_drops: u64,
}

/// Atomic storage for thread-safe operational counters.
#[derive(Debug, Default)]
pub(crate) struct AtomicFanoutCounters {
    pub(crate) packets_forwarded: AtomicU64,
    pub(crate) bytes_forwarded: AtomicU64,
    pub(crate) packets_throttled: AtomicU64,
    pub(crate) destinations_deduplicated: AtomicU64,
    pub(crate) queue_overflow_drops: AtomicU64,
}

impl AtomicFanoutCounters {
    pub(crate) fn snapshot(&self) -> FanoutCounters {
        FanoutCounters {
            packets_forwarded: self.packets_forwarded.load(Ordering::Relaxed),
            bytes_forwarded: self.bytes_forwarded.load(Ordering::Relaxed),
            packets_throttled: self.packets_throttled.load(Ordering::Relaxed),
            destinations_deduplicated: self.destinations_deduplicated.load(Ordering::Relaxed),
            queue_overflow_drops: self.queue_overflow_drops.load(Ordering::Relaxed),
        }
    }
}

/// Fixed-window rate limiter enforcing global and per-origin packet and byte budgets.
#[derive(Debug)]
pub(crate) struct FanoutRateLimiter {
    policy: FanoutPolicy,
    window_start: Option<Instant>,
    rate_window: Duration,
    global_packets_in_window: u32,
    global_bytes_in_window: usize,
    origin_packets_in_window: HashMap<[u8; 4], u32>,
}

impl FanoutRateLimiter {
    /// Create a new rate limiter with the specified policy.
    pub(crate) fn new(policy: FanoutPolicy) -> Self {
        Self {
            policy,
            window_start: None,
            rate_window: Duration::from_secs(1),
            global_packets_in_window: 0,
            global_bytes_in_window: 0,
            origin_packets_in_window: HashMap::new(),
        }
    }

    /// Update the fanout policy.
    pub(crate) fn set_policy(&mut self, policy: FanoutPolicy) {
        self.policy = policy;
    }

    fn roll_window_if_expired(&mut self, now: Instant) {
        match self.window_start {
            None => {
                self.window_start = Some(now);
            }
            Some(start) => {
                if now.saturating_duration_since(start) >= self.rate_window {
                    self.window_start = Some(now);
                    self.global_packets_in_window = 0;
                    self.global_bytes_in_window = 0;
                    self.origin_packets_in_window.clear();
                }
            }
        }
    }

    /// Check and admit target destinations up to configured budgets.
    ///
    /// Returns `(admitted_count, throttled_count)`.
    pub(crate) fn check_and_admit(
        &mut self,
        origin_ip: [u8; 4],
        candidate_count: usize,
        packet_bytes: usize,
        now: Instant,
    ) -> (usize, usize) {
        if candidate_count == 0 {
            return (0, 0);
        }
        self.roll_window_if_expired(now);

        // 1. Cap to per-input budget
        let per_input_capped = candidate_count.min(self.policy.max_fanout_per_input);
        let per_input_excess = candidate_count.saturating_sub(per_input_capped);

        // 2. Global packet budget remaining
        let global_packets_remaining = (self
            .policy
            .max_packets_per_sec_global
            .saturating_sub(self.global_packets_in_window))
            as usize;

        // 3. Global byte budget remaining
        let global_bytes_remaining = self
            .policy
            .max_bytes_per_sec_global
            .saturating_sub(self.global_bytes_in_window);
        let global_packets_by_bytes = if packet_bytes > 0 {
            global_bytes_remaining / packet_bytes
        } else {
            usize::MAX
        };

        // 4. Per-origin packet budget remaining
        let current_origin = self
            .origin_packets_in_window
            .get(&origin_ip)
            .copied()
            .unwrap_or(0);
        let origin_packets_remaining = (self
            .policy
            .max_packets_per_sec_per_origin
            .saturating_sub(current_origin)) as usize;

        let admitted = per_input_capped
            .min(global_packets_remaining)
            .min(global_packets_by_bytes)
            .min(origin_packets_remaining);

        let throttled = per_input_excess + (per_input_capped - admitted);

        if admitted > 0 {
            self.global_packets_in_window = self
                .global_packets_in_window
                .saturating_add(admitted as u32);
            self.global_bytes_in_window = self
                .global_bytes_in_window
                .saturating_add(admitted.saturating_mul(packet_bytes));
            let new_origin = current_origin.saturating_add(admitted as u32);
            self.origin_packets_in_window.insert(origin_ip, new_origin);
        }

        (admitted, throttled)
    }
}

/// A batched job queued for the background fanout worker.
#[derive(Debug)]
pub(crate) struct FanoutJob {
    pub(crate) frame: Bytes,
    pub(crate) targets: Vec<SocketAddrV4>,
}

/// Dispatcher used by the receive loop to enqueue fanout jobs without blocking.
#[derive(Clone)]
pub(crate) struct FanoutDispatcher {
    tx: mpsc::Sender<FanoutJob>,
    limiter: Arc<std::sync::Mutex<FanoutRateLimiter>>,
    counters: Arc<AtomicFanoutCounters>,
}

impl FanoutDispatcher {
    pub(crate) fn new(
        tx: mpsc::Sender<FanoutJob>,
        limiter: Arc<std::sync::Mutex<FanoutRateLimiter>>,
        counters: Arc<AtomicFanoutCounters>,
    ) -> Self {
        Self {
            tx,
            limiter,
            counters,
        }
    }

    /// Dispatch pre-encoded frame to target destinations.
    pub(crate) fn dispatch(
        &self,
        origin_ip: [u8; 4],
        targets: Vec<SocketAddrV4>,
        frame: Bytes,
        dedup_count: u64,
    ) -> bool {
        if dedup_count > 0 {
            self.counters
                .destinations_deduplicated
                .fetch_add(dedup_count, Ordering::Relaxed);
        }
        if targets.is_empty() {
            return true;
        }

        let packet_bytes = frame.len();
        let (admitted_count, throttled_count) = {
            let mut limiter = match self.limiter.lock() {
                Ok(l) => l,
                Err(p) => p.into_inner(),
            };
            limiter.check_and_admit(origin_ip, targets.len(), packet_bytes, Instant::now())
        };

        if throttled_count > 0 {
            self.counters
                .packets_throttled
                .fetch_add(throttled_count as u64, Ordering::Relaxed);
        }

        if admitted_count == 0 {
            return false;
        }

        let mut job_targets = targets;
        job_targets.truncate(admitted_count);

        let job = FanoutJob {
            frame,
            targets: job_targets,
        };

        match self.tx.try_send(job) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.counters
                    .queue_overflow_drops
                    .fetch_add(1, Ordering::Relaxed);
                warn!("BIP BBMD: fanout work queue full, dropping broadcast forwarding job");
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                warn!("BIP BBMD: fanout worker task closed");
                false
            }
        }
    }

    /// Encode a Forwarded-NPDU and dispatch to target destinations.
    pub(crate) fn dispatch_forwarded_npdu(
        &self,
        orig_ip: [u8; 4],
        orig_port: u16,
        npdu: &[u8],
        targets: Vec<SocketAddrV4>,
        dedup_count: u64,
    ) -> bool {
        if targets.is_empty() {
            if dedup_count > 0 {
                self.counters
                    .destinations_deduplicated
                    .fetch_add(dedup_count, Ordering::Relaxed);
            }
            return true;
        }
        let mut buf = bytes::BytesMut::with_capacity(10 + npdu.len());
        if let Err(e) = encode_bvll_forwarded(&mut buf, orig_ip, orig_port, npdu) {
            warn!(error = %e, "Failed to encode Forwarded-NPDU for fanout");
            return false;
        }
        let frame = buf.freeze();
        self.dispatch(orig_ip, targets, frame, dedup_count)
    }
}

/// Background worker loop dequeuing fanout jobs and sending them asynchronously.
pub(crate) async fn run_fanout_worker(
    socket: Arc<UdpSocket>,
    mut rx: mpsc::Receiver<FanoutJob>,
    counters: Arc<AtomicFanoutCounters>,
) {
    while let Some(job) = rx.recv().await {
        for dest in job.targets {
            match socket.send_to(&job.frame, dest).await {
                Ok(bytes_sent) => {
                    counters.packets_forwarded.fetch_add(1, Ordering::Relaxed);
                    counters
                        .bytes_forwarded
                        .fetch_add(bytes_sent as u64, Ordering::Relaxed);
                }
                Err(e) => {
                    warn!(error = %e, %dest, "BIP BBMD: failed to send forwarded NPDU");
                }
            }
        }
    }
}
