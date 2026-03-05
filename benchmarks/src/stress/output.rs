//! JSON output and HDR histogram latency tracking for stress tests.

use std::time::Instant;

use hdrhistogram::Histogram;
use serde::Serialize;

/// Top-level result for a stress test scenario.
#[derive(Debug, Serialize)]
pub struct StressResult {
    pub scenario: String,
    pub transport: String,
    pub parameters: serde_json::Value,
    pub results: StressMetrics,
}

/// Aggregate metrics for one scenario run.
#[derive(Debug, Serialize)]
pub struct StressMetrics {
    pub total_requests: u64,
    pub successful: u64,
    pub failed: u64,
    pub error_rate_pct: f64,
    pub throughput_ops_sec: f64,
    pub latency_us: LatencyStats,
    pub peak_rss_kb: u64,
    pub degradation_curve: Vec<DegradationPoint>,
}

/// Latency percentiles in microseconds.
#[derive(Debug, Serialize)]
pub struct LatencyStats {
    pub min: u64,
    pub p50: u64,
    pub p95: u64,
    pub p99: u64,
    pub p999: u64,
    pub max: u64,
}

/// One step in a degradation curve.
#[derive(Debug, Clone, Serialize)]
pub struct DegradationPoint {
    pub parameter: u64,
    pub p50_us: u64,
    pub p99_us: u64,
    pub throughput: f64,
    pub errors: u64,
}

/// HDR histogram-backed latency recorder.
pub struct LatencyRecorder {
    histogram: Histogram<u64>,
    successful: u64,
    failed: u64,
}

impl LatencyRecorder {
    pub fn new() -> Self {
        Self {
            // Track latencies from 1µs to 60s with 3 significant digits.
            histogram: Histogram::new_with_bounds(1, 60_000_000, 3).unwrap(),
            successful: 0,
            failed: 0,
        }
    }

    /// Record a successful operation. `start` is the instant before the call.
    pub fn record_success(&mut self, start: Instant) {
        let us = start.elapsed().as_micros() as u64;
        let _ = self.histogram.record(us.max(1));
        self.successful += 1;
    }

    /// Record a failed operation.
    pub fn record_failure(&mut self) {
        self.failed += 1;
    }

    /// Extract latency percentiles.
    pub fn stats(&self) -> LatencyStats {
        LatencyStats {
            min: self.histogram.min(),
            p50: self.histogram.value_at_quantile(0.50),
            p95: self.histogram.value_at_quantile(0.95),
            p99: self.histogram.value_at_quantile(0.99),
            p999: self.histogram.value_at_quantile(0.999),
            max: self.histogram.max(),
        }
    }

    pub fn successful(&self) -> u64 {
        self.successful
    }

    pub fn failed(&self) -> u64 {
        self.failed
    }

    pub fn total(&self) -> u64 {
        self.successful + self.failed
    }

    /// Merge another recorder into this one.
    pub fn merge(&mut self, other: &LatencyRecorder) {
        let _ = self.histogram.add(&other.histogram);
        self.successful += other.successful;
        self.failed += other.failed;
    }

    /// Reset for reuse.
    pub fn reset(&mut self) {
        self.histogram.reset();
        self.successful = 0;
        self.failed = 0;
    }
}

impl Default for LatencyRecorder {
    fn default() -> Self {
        Self::new()
    }
}

/// Print results: JSON to stdout, human-readable table to stderr.
pub fn print_results(result: &StressResult) {
    println!("{}", serde_json::to_string_pretty(result).unwrap());

    eprintln!();
    eprintln!("=== {} ({}) ===", result.scenario, result.transport);

    if result.results.degradation_curve.is_empty() {
        eprintln!(
            "Requests: {} ok / {} err | Throughput: {:.0} ops/s",
            result.results.successful, result.results.failed, result.results.throughput_ops_sec,
        );
        let l = &result.results.latency_us;
        eprintln!(
            "Latency: p50={} p95={} p99={} max={}µs",
            l.p50, l.p95, l.p99, l.max,
        );
    } else {
        eprintln!(
            "{:>10} | {:>8} | {:>8} | {:>12} | {:>6}",
            "Parameter", "p50", "p99", "Throughput", "Errors"
        );
        eprintln!("{}", "-".repeat(56));
        for pt in &result.results.degradation_curve {
            let warn = if pt.errors > 0 { " ⚠" } else { "" };
            eprintln!(
                "{:>10} | {:>6}µs | {:>6}µs | {:>10.0}/s | {:>5}{}",
                pt.parameter, pt.p50_us, pt.p99_us, pt.throughput, pt.errors, warn
            );
        }
    }

    eprintln!("Peak RSS: {}KB", result.results.peak_rss_kb);
    eprintln!();
}
