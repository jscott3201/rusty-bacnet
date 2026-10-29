//! Gap schedules compose across real core rings with explicit local budgets.

use bacnet_mstp_codec::FrameType;
use bacnet_mstp_core::{Duration, Instant};
use bacnet_mstp_sim::{DriverReleaseCause, HostSpec, MasterSpec, Sim, SimConfig};

fn ms(value: u64) -> Instant {
    Instant::from_micros(value * 1_000)
}

fn ring(baud: u32, mode: u8, seed: u64, random_gap: bool) -> Sim {
    let mut config = SimConfig::new(baud);
    config.seed = seed;
    let mut sim = Sim::new(config);
    for station in [0, 1] {
        let mut spec = MasterSpec::new(station, 1, baud).extended_frames();
        // Six seconds explicitly covers the 4.578-second maximum schedule
        // at 9600 plus this fixture's matching-echo notification allowance.
        spec.config.transmit_watchdog = Duration::from_secs(6);
        // Six queued long frames can take more than the default five-second
        // queue age to service. Preserve them for this positive replay run.
        spec.config.queue_max_age = Duration::from_secs(15);
        let maximum = sim.timing().max_inter_octet_gap();
        spec.adapter.inter_octet_gap = if random_gap {
            Duration::from_nanos(sim.rng().range(0, maximum.as_nanos()))
        } else {
            maximum
        };
        spec.adapter.post_drive = sim.timing().max_post_drive();
        spec.adapter.echo = true;
        if mode == 1 || (mode == 2 && station == 0) {
            spec = spec.host(HostSpec {
                max_chunk: 31,
                random_chunks: true,
                echo_grace: Some(Duration::from_millis(40)),
                ..HostSpec::default()
            });
        }
        sim.add_master(spec);
    }
    sim
}

/// Standard and actual maximum-size extended frames retain their receipts
/// through maximal legal gaps in octet, host and mixed rings at each rate.
#[test]
fn budgeted_maximum_gap_rings_deliver_standard_and_extended_frames() {
    for baud in [9_600, 38_400, 115_200] {
        for mode in [0, 1, 2] {
            let mut sim = ring(baud, mode, 0, false);
            let standard = vec![0x11; 501];
            let extended = vec![0x11; 1497];
            sim.enqueue(ms(800), 0, 1, &standard);
            sim.enqueue(ms(1_000), 1, 0, &extended);
            sim.run_until(ms(10_000));
            sim.assert_no_violations();
            assert!(
                sim.deliveries(1)
                    .iter()
                    .any(|delivery| delivery.data == standard)
            );
            assert!(
                sim.deliveries(0)
                    .iter()
                    .any(|delivery| delivery.data == extended)
            );
            assert!(sim.frames().iter().any(|frame| frame.frame_type()
                == Some(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY)
                && frame.octets.len() == 511));
            assert!(sim.frames().iter().any(|frame| frame.frame_type()
                == Some(FrameType::BACNET_EXTENDED_DATA_NOT_EXPECTING_REPLY)
                && frame.octets.len() == 1516));
            assert!(sim.frames().iter().all(|frame| frame.is_clean()));
            assert!(sim.report().collisions.is_empty());
            assert!(sim.report().max_pending_events <= 12);
            assert!(
                sim.report()
                    .host_transmits
                    .iter()
                    .all(|timing| timing.budget_overrun().is_none())
            );
            for station in [0, 1] {
                assert_eq!(sim.core(station).unwrap().counters().transmit_timeouts, 0);
                assert_eq!(sim.core(station).unwrap().counters().own_address_frames, 0);
            }
            for frame in sim.frames() {
                let end = frame.start
                    + sim
                        .timing()
                        .duration_with_gap(frame.octets.len(), sim.timing().max_inter_octet_gap());
                assert_eq!(frame.end, end);
                if end + sim.timing().max_post_drive() <= sim.now() {
                    let releases: Vec<_> = sim
                        .report()
                        .driver_releases
                        .iter()
                        .filter(|release| {
                            release.driver == frame.driver && release.start == frame.start
                        })
                        .collect();
                    assert_eq!(releases.len(), 1);
                    assert_eq!(releases[0].data_end, Some(end));
                    assert_eq!(releases[0].at, end + sim.timing().max_post_drive());
                    assert_eq!(releases[0].cause, DriverReleaseCause::Completed);
                }
            }
        }
    }
}

/// Each seed captures independent normal gap profiles. Wire frames, physical
/// releases, host budget observations, deliveries and host statistics replay.
#[test]
fn seeded_gap_profiles_replay_with_bounded_events_and_receipts() {
    let run = |seed, mode| {
        let baud = [9_600, 38_400, 115_200][seed as usize % 3];
        let mut sim = ring(baud, mode, seed, true);
        for index in 0..6 {
            let mut payload = sim.rng().bytes(if index % 2 == 0 { 501 } else { 700 });
            payload[0] = 0x01;
            payload[1] = 0x00;
            let source = index % 2;
            let at = ms(sim.rng().range(800, 2_000));
            sim.enqueue(at, source, 1 - source, &payload);
        }
        sim.run_until(ms(15_000));
        sim.assert_no_violations();
        assert_eq!(
            sim.deliveries(0).len() + sim.deliveries(1).len(),
            6,
            "seed {seed}, mode {mode}, baud {baud}",
        );
        for station in [0, 1] {
            assert_eq!(
                sim.core(station).unwrap().counters().unicast.drops_expired,
                0
            );
        }
        assert!(sim.report().max_pending_events <= 12);
        assert!(
            sim.report()
                .host_transmits
                .iter()
                .all(|timing| timing.budget_overrun().is_none())
        );
        assert!(sim.report().driver_releases.len() > 20);
        (
            sim.frames().to_vec(),
            sim.report().clone(),
            [sim.deliveries(0).to_vec(), sim.deliveries(1).to_vec()],
            [sim.host_stats(0).copied(), sim.host_stats(1).copied()],
        )
    };
    for seed in 0..8 {
        for mode in [0, 1, 2] {
            assert_eq!(run(seed, mode), run(seed, mode));
        }
    }
}
