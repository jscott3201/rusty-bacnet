//! The throughput and summary arithmetic of the stress output (#1714).

use std::time::{Duration, Instant};

use super::*;

fn point(parameter: u64, successful: u64, errors: u64, secs: f64) -> DegradationPoint {
    DegradationPoint {
        parameter,
        p50_us: 0,
        p99_us: 0,
        throughput: rate(successful, Duration::from_secs_f64(secs)),
        successful,
        errors,
        elapsed_secs: secs,
    }
}

#[test]
fn rate_keeps_sub_millisecond_durations() {
    // 50 devices scanned in 600 microseconds: no whole millisecond elapsed.
    let per_sec = rate(50, Duration::from_micros(600));
    assert!((per_sec - 83_333.333).abs() < 0.01, "{per_sec}");
    // 1.5 milliseconds is not one millisecond.
    let per_sec = rate(3, Duration::from_micros(1_500));
    assert!((per_sec - 2_000.0).abs() < 1e-9, "{per_sec}");
}

#[test]
fn rate_of_nothing_timed_is_zero() {
    assert_eq!(rate(10, Duration::ZERO), 0.0);
    assert_eq!(rate(0, Duration::from_secs(1)), 0.0);
}

#[test]
fn summary_counts_what_each_step_did() {
    // Two 5-second steps: the totals are the steps' counts, not a rate times
    // a fixed 10 seconds.
    let curve = vec![point(1, 50_000, 0, 5.0), point(5, 150_000, 10, 5.0)];
    let m = StressMetrics::from_curve(curve, 1_024);
    assert_eq!(m.successful, 200_000);
    assert_eq!(m.failed, 10);
    assert_eq!(m.total_requests, 200_010);
    assert!((m.elapsed_secs - 10.0).abs() < 1e-9);
    assert!((m.throughput_ops_sec - 20_000.0).abs() < 1e-6);
    assert!((m.error_rate_pct - 10.0 * 100.0 / 200_010.0).abs() < 1e-9);
    assert_eq!(m.peak_rss_kb, 1_024);
}

#[test]
fn summary_names_the_fastest_step_not_the_last() {
    let curve = vec![
        point(1, 10_000, 0, 1.0),
        point(10, 70_000, 0, 1.0),
        point(100, 60_000, 0, 1.0),
    ];
    let m = StressMetrics::from_curve(curve, 0);
    assert_eq!(m.peak_parameter, Some(10));
    assert!((m.peak_throughput_ops_sec - 70_000.0).abs() < 1e-9);
    assert!((m.throughput_ops_sec - 140_000.0 / 3.0).abs() < 1e-6);
}

#[test]
fn summary_of_no_steps_is_empty() {
    let m = StressMetrics::from_curve(Vec::new(), 0);
    assert_eq!(m.total_requests, 0);
    assert_eq!(m.error_rate_pct, 0.0);
    assert_eq!(m.throughput_ops_sec, 0.0);
    assert_eq!(m.peak_parameter, None);
}

#[test]
fn a_step_takes_its_rate_from_its_measured_time() {
    let mut recorder = LatencyRecorder::new();
    for _ in 0..4 {
        recorder.record_latency(Duration::from_micros(100));
    }
    recorder.record_failure();
    let pt = DegradationPoint::from_recorder(7, &recorder, Duration::from_millis(250));
    assert_eq!(pt.parameter, 7);
    assert_eq!(pt.successful, 4);
    assert_eq!(pt.errors, 1);
    assert!((pt.throughput - 16.0).abs() < 1e-9);
    assert!((pt.elapsed_secs - 0.25).abs() < 1e-9);
    assert_eq!(pt.p50_us, 100);
}

#[test]
fn a_step_is_timed_to_its_last_client_not_the_nominal_duration() {
    let started = Instant::now();
    let client = |ok: u64, failed: u64, micros: u64| {
        let mut recorder = LatencyRecorder::new();
        for _ in 0..ok {
            recorder.record_latency(Duration::from_micros(micros));
        }
        for _ in 0..failed {
            recorder.record_failure();
        }
        recorder
    };
    // Two clients stop at 1.0 and 1.25 seconds: the step ran 1.25 seconds.
    let pt = DegradationPoint::from_clients(
        2,
        started,
        [
            (client(3, 1, 100), started + Duration::from_millis(1_000)),
            (client(5, 0, 300), started + Duration::from_millis(1_250)),
        ],
    );
    assert_eq!(pt.parameter, 2);
    assert_eq!(pt.successful, 8);
    assert_eq!(pt.errors, 1);
    assert!((pt.elapsed_secs - 1.25).abs() < 1e-9);
    assert!((pt.throughput - 6.4).abs() < 1e-9, "{}", pt.throughput);
}

#[test]
fn a_step_with_no_clients_has_no_time_and_no_rate() {
    let pt = DegradationPoint::from_clients(0, Instant::now(), []);
    assert_eq!(pt.successful, 0);
    assert_eq!(pt.elapsed_secs, 0.0);
    assert_eq!(pt.throughput, 0.0);
}
