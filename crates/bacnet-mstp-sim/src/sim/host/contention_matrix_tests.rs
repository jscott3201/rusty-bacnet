//! Bounded contention and exact traffic once the injected overlap has ended.
use bacnet_mstp_core::{Duration, Instant};

use super::contention_tests::{bus, capture, frame, send};
use crate::{ContentionPolicy, Driver, HostSpec};

fn scenario(baud: u32, mode: u8, extended: bool, enabled: bool) -> crate::Sim {
    let mut sim = bus(baud, mode == 1, extended);
    if mode == 2 {
        for station in [10, 12] {
            let node = sim.index_of(station);
            sim.power_off(node);
            sim.master_mut(node).unwrap().spec =
                sim.master(station).spec.clone().host(HostSpec::default());
            sim.power_on(node);
        }
    }
    if enabled {
        capture(&mut sim, 10);
        sim.set_contention(13, ContentionPolicy::Capture(Driver::Station(2)));
    }
    let mut payload = vec![0x6B; if extended { 1497 } else { 501 }];
    payload[..2].copy_from_slice(&[1, 0]);
    let first = frame(1, extended, &payload);
    let end =
        Instant::from_micros((first.octets.len() as u64 * 10_000_000).div_ceil(u64::from(baud)));
    send(&mut sim, 1, Instant::ZERO, first);
    send(&mut sim, 2, Instant::ZERO, frame(2, extended, &payload));
    // Every receiver is idle after bounded traffic and a receive-abort
    // interval. This fixture makes no duplicate-token convergence claim.
    sim.run_until(end + Duration::from_millis(120));
    for station in [10, 11, 12, 13] {
        assert_eq!(sim.contention_stats(station).pending, 0);
    }
    sim.settle();
    let clean_at = sim.now() + Duration::from_millis(1);
    send(&mut sim, 3, clean_at, frame(3, false, &[1, 0, 0xC3]));
    sim.run_until(clean_at + Duration::from_millis(30));
    for station in [10, 11, 12, 13] {
        let deliveries = sim.deliveries(station);
        let won = enabled && matches!(station, 10 | 13);
        assert_eq!(
            deliveries.len(),
            if won { 2 } else { 1 },
            "baud {baud}, mode {mode}, station {station}"
        );
        assert_eq!(deliveries.last().unwrap().data, [1, 0, 0xC3]);
        if won {
            assert_eq!(deliveries[0].data, payload);
            assert_eq!(sim.contention_stats(station).checked, 1);
            assert_eq!(sim.contention_stats(station).matched, 1);
            assert_eq!(sim.contention_stats(station).max_pending, 1);
        }
    }
    assert_eq!(sim.frames().len(), 3);
    assert!(sim.frames()[0].collided && sim.frames()[1].collided);
    assert!(sim.frames()[2].is_clean());
    assert!(sim.report().max_pending_events <= 12);
    sim.assert_no_violations();
    sim
}

#[test]
fn standard_extended_and_mixed_adapters_keep_identical_physical_evidence() {
    for baud in [9_600, 38_400, 115_200] {
        for mode in 0..3 {
            for extended in [false, true] {
                let default = scenario(baud, mode, extended, false);
                let captured = scenario(baud, mode, extended, true);
                assert_eq!(captured.frames(), default.frames());
                assert_eq!(captured.report().collisions, default.report().collisions);
                assert_eq!(
                    captured.report().driver_releases,
                    default.report().driver_releases
                );
                assert_eq!(captured.line.last_activity(), default.line.last_activity());
            }
        }
    }
}

#[test]
fn explicit_conservative_policy_is_the_default_trace_and_report() {
    let first = scenario(38_400, 2, false, false);
    let mut second = bus(38_400, false, false);
    for station in [10, 12] {
        let node = second.index_of(station);
        second.power_off(node);
        second.master_mut(node).unwrap().spec = second
            .master(station)
            .spec
            .clone()
            .host(HostSpec::default());
        second.power_on(node);
    }
    for station in [10, 11, 12, 13] {
        second.set_contention(station, ContentionPolicy::FramingError);
    }
    let mut payload = vec![0x6B; 501];
    payload[..2].copy_from_slice(&[1, 0]);
    let send_one = frame(1, false, &payload);
    let end = Instant::from_micros((send_one.octets.len() as u64 * 10_000_000).div_ceil(38_400));
    send(&mut second, 1, Instant::ZERO, send_one);
    send(&mut second, 2, Instant::ZERO, frame(2, false, &payload));
    second.run_until(end + Duration::from_millis(120));
    second.settle();
    let clean_at = second.now() + Duration::from_millis(1);
    send(&mut second, 3, clean_at, frame(3, false, &[1, 0, 0xC3]));
    second.run_until(clean_at + Duration::from_millis(30));
    assert_eq!(first.frames(), second.frames());
    assert_eq!(first.report(), second.report());
    assert_eq!(
        first.trace().entries().collect::<Vec<_>>(),
        second.trace().entries().collect::<Vec<_>>()
    );
}
