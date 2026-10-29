//! JSON output and HDR histogram latency tracking for stress tests.

use std::time::{Duration, Instant};

use hdrhistogram::Histogram;
use serde::Serialize;

/// Top-level result for a stress test scenario.
#[derive(Debug, Serialize)]
pub struct StressResult {
    pub scenario: String,
    pub transport: String,
    pub parameters: serde_json::Value,
    pub results: StressMetrics,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clients_diagnostics: Option<super::client_diagnostics::ClientsDiagnostics>,
}

/// Aggregate metrics for one scenario run, summed over every step of its
/// degradation curve. Latency percentiles stay per step: one step's
/// percentiles don't combine with another's.
#[derive(Debug, Serialize)]
pub struct StressMetrics {
    pub total_requests: u64,
    pub successful: u64,
    pub failed: u64,
    pub error_rate_pct: f64,
    /// Successful operations per second over all steps together.
    pub throughput_ops_sec: f64,
    /// The fastest step's rate.
    pub peak_throughput_ops_sec: f64,
    /// The fastest step's parameter.
    pub peak_parameter: Option<u64>,
    /// Measured time of all steps, in seconds.
    pub elapsed_secs: f64,
    pub peak_rss_kb: u64,
    pub degradation_curve: Vec<DegradationPoint>,
}

impl StressMetrics {
    /// Summarise every step of `curve`: the totals of its operations, the
    /// rate over the steps' measured time, and the fastest step.
    pub fn from_curve(curve: Vec<DegradationPoint>, peak_rss_kb: u64) -> Self {
        let successful: u64 = curve.iter().map(|pt| pt.successful).sum();
        let failed: u64 = curve.iter().map(|pt| pt.errors).sum();
        let elapsed_secs: f64 = curve.iter().map(|pt| pt.elapsed_secs).sum();
        let total = successful + failed;
        let peak = curve
            .iter()
            .max_by(|a, b| a.throughput.total_cmp(&b.throughput));
        let (peak_throughput_ops_sec, peak_parameter) =
            peak.map_or((0.0, None), |pt| (pt.throughput, Some(pt.parameter)));
        Self {
            total_requests: total,
            successful,
            failed,
            error_rate_pct: if total > 0 {
                failed as f64 * 100.0 / total as f64
            } else {
                0.0
            },
            throughput_ops_sec: rate(successful, Duration::from_secs_f64(elapsed_secs)),
            peak_throughput_ops_sec,
            peak_parameter,
            elapsed_secs,
            peak_rss_kb,
            degradation_curve: curve,
        }
    }
}

/// `count` operations over `elapsed`, per second, at the duration's full
/// resolution, so a scan of a few hundred microseconds still gets a rate.
/// Zero when nothing was timed.
pub fn rate(count: u64, elapsed: Duration) -> f64 {
    let secs = elapsed.as_secs_f64();
    if secs > 0.0 { count as f64 / secs } else { 0.0 }
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
    /// Successful operations per second over `elapsed_secs`.
    pub throughput: f64,
    pub successful: u64,
    pub errors: u64,
    /// The step's measured time, in seconds.
    pub elapsed_secs: f64,
}

impl DegradationPoint {
    /// A step from `recorder`'s operations, timed over `elapsed`.
    pub fn from_recorder(parameter: u64, recorder: &LatencyRecorder, elapsed: Duration) -> Self {
        let stats = recorder.stats();
        Self {
            parameter,
            p50_us: stats.p50,
            p99_us: stats.p99,
            throughput: rate(recorder.successful(), elapsed),
            successful: recorder.successful(),
            errors: recorder.failed(),
            elapsed_secs: elapsed.as_secs_f64(),
        }
    }

    /// A step from every client's own recorder and the instant its loop
    /// ended, timed from `started` to the last of those instants.
    pub fn from_clients(
        parameter: u64,
        started: Instant,
        clients: impl IntoIterator<Item = (LatencyRecorder, Instant)>,
    ) -> Self {
        let mut recorder = LatencyRecorder::new();
        let mut ended = started;
        for (client, end) in clients {
            recorder.merge(&client);
            ended = ended.max(end);
        }
        Self::from_recorder(parameter, &recorder, ended - started)
    }

    /// Print the step's line: its counts, time, rate and percentiles.
    pub fn log(&self) {
        eprintln!(
            "  ok={} err={} in {:.3} s = {:.0} per second, p50={} µs p99={} µs",
            self.successful,
            self.errors,
            self.elapsed_secs,
            self.throughput,
            self.p50_us,
            self.p99_us,
        );
    }
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
        self.record_latency(start.elapsed());
    }

    /// Record a successful operation that took `latency`.
    pub fn record_latency(&mut self, latency: Duration) {
        let us = latency.as_micros() as u64;
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

/// Print results: JSON to stdout, then to stderr a table of every step and
/// the summary of all of them.
pub fn print_results(result: &StressResult) {
    println!("{}", serde_json::to_string_pretty(result).unwrap());

    let r = &result.results;
    eprintln!();
    eprintln!("=== {} ({}) ===", result.scenario, result.transport);
    eprintln!(
        "{:>10} | {:>9} | {:>9} | {:>11} | {:>9} | {:>7} | {:>8}",
        "Parameter", "p50 (µs)", "p99 (µs)", "Per second", "OK", "Errors", "Seconds"
    );
    eprintln!("{}", "-".repeat(82));
    for pt in &r.degradation_curve {
        let warn = if pt.errors > 0 { " ⚠" } else { "" };
        eprintln!(
            "{:>10} | {:>9} | {:>9} | {:>11.0} | {:>9} | {:>7} | {:>8.3}{}",
            pt.parameter,
            pt.p50_us,
            pt.p99_us,
            pt.throughput,
            pt.successful,
            pt.errors,
            pt.elapsed_secs,
            warn
        );
    }
    eprintln!(
        "All steps: {} ok, {} failed ({:.2}%) in {:.2} s, {:.0} per second overall",
        r.successful, r.failed, r.error_rate_pct, r.elapsed_secs, r.throughput_ops_sec,
    );
    if let Some(parameter) = r.peak_parameter {
        eprintln!(
            "Fastest step: {:.0} per second at parameter {}",
            r.peak_throughput_ops_sec, parameter
        );
    }
    eprintln!("Peak resident memory: {} KB", r.peak_rss_kb);
    eprintln!();
}

#[cfg(test)]
#[path = "output_tests.rs"]
mod tests;
