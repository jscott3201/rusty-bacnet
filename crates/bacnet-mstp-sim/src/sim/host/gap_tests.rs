//! Host completion clocks preserve the production write/drain/echo await.

use bacnet_mstp_core::Duration;

use crate::sim::gap_tests::{outgoing, sending};
use crate::{Fault, HostBudgetOverrun, HostSpec, MasterSpec, Sim, WireTiming};

fn host(budget: Duration, drain: Duration, grace: Option<Duration>, echo: bool) -> Sim {
    let mut spec = MasterSpec::new(0, 1, 38_400).host(HostSpec {
        drain_duration: drain,
        echo_grace: grace,
        max_chunk: 4096,
        ..HostSpec::default()
    });
    spec.config.transmit_watchdog = budget;
    spec.adapter.inter_octet_gap = WireTiming::new(38_400).max_inter_octet_gap();
    spec.adapter.post_drive = WireTiming::new(38_400).max_post_drive();
    spec.adapter.echo = echo;
    sending(spec, 501)
}

/// C uses the actual gapped F, even if drain returns earlier. A later drain
/// may exceed the budget, but production's host await does not abort the wire.
#[test]
fn host_drain_before_and_after_gapped_data_keeps_independent_budget_clocks() {
    for drain_ms in [0, 600] {
        for echo in [false, true] {
            let mut sim = host(
                Duration::from_millis(500),
                Duration::from_millis(drain_ms),
                echo.then_some(Duration::from_millis(5)),
                echo,
            );
            let (_, start, end) = outgoing(&sim);
            let id = sim.master_mut(0).unwrap().transmitting.unwrap();
            let incarnation = sim.master_mut(0).unwrap().incarnation;
            assert_eq!(end, start + Duration::from_nanos(385_416_410));
            let completion = end.max(start + Duration::from_millis(drain_ms));
            sim.run_until(completion);
            let timing = sim
                .report
                .host_transmits
                .iter()
                .find(|timing| timing.transmit_id == id)
                .unwrap();
            assert_eq!(timing.station, 0);
            assert_eq!(timing.incarnation, incarnation);
            assert_eq!(timing.start, start);
            assert_eq!(timing.watchdog, start + Duration::from_millis(500));
            assert_eq!(timing.data_end, end);
            assert_eq!(timing.completion, completion);
            assert_eq!(timing.notification, completion);
            assert_eq!(
                timing.budget_overrun(),
                (drain_ms == 600).then_some(HostBudgetOverrun::CompletionAfterWatchdog)
            );
            assert_eq!(sim.core(0).unwrap().counters().transmit_timeouts, 0);
            assert_eq!(sim.core(0).unwrap().counters().own_address_frames, 0);
            assert!(
                sim.frames
                    .iter()
                    .filter(|frame| frame.driver == crate::Driver::Station(0))
                    .all(|frame| frame.is_clean())
            );
            sim.assert_no_violations();
        }
    }
}

/// C may meet W while notification N crosses it because echo is absent.
/// Matching echo completes at C, and missing echo keeps C rather than N.
#[test]
fn missing_echo_distinguishes_late_notification_from_late_completion() {
    for (echo, grace_ms, overrun) in [
        (true, 200, None),
        (false, 100, None),
        (
            false,
            200,
            Some(HostBudgetOverrun::NotificationAfterWatchdog),
        ),
    ] {
        let grace = Duration::from_millis(grace_ms);
        let mut sim = host(
            Duration::from_millis(500),
            Duration::ZERO,
            Some(grace),
            echo,
        );
        let (_, _, end) = outgoing(&sim);
        let id = sim.master_mut(0).unwrap().transmitting.unwrap();
        let notification = if echo { end } else { end + grace };
        sim.run_until(notification);
        let timing = sim
            .report
            .host_transmits
            .iter()
            .find(|timing| timing.transmit_id == id)
            .unwrap();
        assert_eq!(timing.completion, end);
        assert_eq!(timing.notification, notification);
        assert_eq!(timing.budget_overrun(), overrun);
        assert_eq!(sim.host_stats(0).unwrap().echo_missing, u32::from(!echo));
        assert_eq!(sim.host_stats(0).unwrap().last_completion, Some(end));
        assert_eq!(sim.core(0).unwrap().counters().transmit_timeouts, 0);
        assert_eq!(sim.core(0).unwrap().counters().transmit_aborts, 0);
        sim.assert_no_violations();
    }
}

#[test]
fn host_completion_equal_to_budget_is_distinct_from_notification_excess() {
    for echo in [false, true] {
        let grace = Duration::from_millis(5);
        let mut sim = host(
            Duration::from_nanos(385_416_410),
            Duration::ZERO,
            Some(grace),
            echo,
        );
        let (_, _, end) = outgoing(&sim);
        sim.run_until(if echo { end } else { end + grace });
        let timing = sim.report.host_transmits.first().unwrap();
        assert_eq!(timing.completion, timing.watchdog);
        assert_eq!(
            timing.budget_overrun(),
            (!echo).then_some(HostBudgetOverrun::NotificationAfterWatchdog)
        );
        assert_eq!(sim.core(0).unwrap().counters().transmit_timeouts, 0);
        sim.assert_no_violations();
    }
}

/// The actual worst-case 1516-octet frame at 38400 exceeds the valid default
/// one-second budget when maximum legal gaps are selected. The host observes
/// that fact after the await; it does not claim an octet-style watchdog abort.
#[test]
fn host_reports_a_gapped_frame_beyond_budget_without_inventing_a_timeout() {
    let mut spec = MasterSpec::new(0, 1, 38_400)
        .extended_frames()
        .host(HostSpec::default());
    assert_eq!(spec.config.transmit_watchdog, Duration::from_secs(1));
    spec.adapter.inter_octet_gap = WireTiming::new(38_400).max_inter_octet_gap();
    let mut sim = sending(spec, 1497);
    let (tx, start, end) = outgoing(&sim);
    assert_eq!(sim.line.get(tx).unwrap().octets.len(), 1516);
    assert_eq!(end, start + Duration::from_nanos(1_144_400_365));
    sim.run_until(start + Duration::from_secs(1));
    assert_eq!(sim.nodes[0].driving, Some(tx));
    assert_eq!(sim.core(0).unwrap().counters().transmit_timeouts, 0);
    sim.run_until(end);
    let timing = sim.report.host_transmits.first().unwrap();
    assert_eq!(
        timing.budget_overrun(),
        Some(HostBudgetOverrun::CompletionAfterWatchdog)
    );
    assert_eq!(timing.data_end, end);
    assert_eq!(timing.notification, end);
    assert!(sim.frames.last().unwrap().is_clean());
    assert_eq!(sim.core(0).unwrap().counters().transmit_timeouts, 0);
    sim.assert_no_violations();
}

#[test]
fn host_completion_and_release_keep_the_captured_gap_profile() {
    let mut sim = host(Duration::from_millis(500), Duration::ZERO, None, false);
    let (_, start, end) = outgoing(&sim);
    let id = sim.master_mut(0).unwrap().transmitting.unwrap();
    let mut replacement = sim.master_mut(0).unwrap().spec.adapter;
    replacement.inter_octet_gap = Duration::ZERO;
    replacement.post_drive = Duration::ZERO;
    sim.apply_fault(Fault::SetAdapter {
        station: 0,
        adapter: replacement,
    });
    sim.run_until(end + sim.timing.max_post_drive());
    let timing = sim
        .report
        .host_transmits
        .iter()
        .find(|timing| timing.transmit_id == id)
        .unwrap();
    assert_eq!(timing.data_end, start + Duration::from_nanos(385_416_410));
    assert_eq!(timing.completion, end);
    assert_eq!(timing.notification, end);
    assert_eq!(
        sim.report.driver_releases.last().unwrap().at,
        end + sim.timing.max_post_drive()
    );
    assert_eq!(sim.core(0).unwrap().counters().transmit_timeouts, 0);
    sim.assert_no_violations();
}
