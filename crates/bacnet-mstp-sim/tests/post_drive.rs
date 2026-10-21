//! Post-drive ownership composes with complete standard and extended rings.

use bacnet_mstp_core::{Duration, Instant};
use bacnet_mstp_sim::{HostSpec, MasterSpec, Sim, SimConfig};

fn ms(value: u64) -> Instant {
    Instant::from_micros(value * 1_000)
}

fn ring(baud: u32, mode: u8, seed: u64, random_hold: bool) -> Sim {
    let mut config = SimConfig::new(baud);
    config.seed = seed;
    let mut sim = Sim::new(config);
    for station in [0, 1] {
        let mut spec = MasterSpec::new(station, 1, baud).extended_frames();
        spec.adapter.post_drive = if random_hold {
            let max_ns = sim.timing().max_post_drive().as_nanos();
            Duration::from_nanos(sim.rng().range(0, max_ns))
        } else {
            sim.timing().max_post_drive()
        };
        if mode == 1 || (mode == 2 && station == 0) {
            spec = spec.host(HostSpec {
                max_chunk: 7,
                random_chunks: true,
                echo_grace: Some(Duration::from_millis(40)),
                ..HostSpec::default()
            });
        }
        spec.adapter.echo = true;
        sim.add_master(spec);
    }
    sim
}

/// The conservative maximum hold fits the turnaround at each selected baud
/// rate, whether software receives octets, host chunks or a mixture.
#[test]
fn maximum_hold_preserves_standard_extended_and_mixed_ring_delivery() {
    for baud in [9_600, 38_400, 115_200] {
        for mode in [0, 1, 2] {
            let mut sim = ring(baud, mode, 0, false);
            let standard = vec![0x01, 0x00, 0x42];
            let extended = vec![0x11; 700];
            sim.enqueue(ms(800), 0, 1, &standard);
            sim.enqueue(ms(1_000), 1, 0, &extended);
            sim.run_until(ms(3_000));
            sim.assert_no_violations();
            assert!(sim.deliveries(1).iter().any(|d| d.data == standard));
            assert!(sim.deliveries(0).iter().any(|d| d.data == extended));
            assert!(sim.report().collisions.is_empty());
            assert!(sim.report().max_pending_events <= 12);
            for frame in sim.frames() {
                assert!(frame.is_clean());
                assert_eq!(
                    frame.end,
                    sim.timing().end_of(frame.start, frame.octets.len())
                );
                if frame.end + sim.timing().max_post_drive() <= sim.now() {
                    let releases: Vec<_> = sim
                        .report()
                        .driver_releases
                        .iter()
                        .filter(|r| r.driver == frame.driver && r.start == frame.start)
                        .collect();
                    assert_eq!(releases.len(), 1);
                    assert_eq!(releases[0].data_end, Some(frame.end));
                    assert_eq!(releases[0].at, frame.end + sim.timing().max_post_drive());
                }
            }
        }
    }
}

/// Seeded normal holds and payload lengths replay complete wire, release,
/// delivery and host evidence, with bounded pending events over long runs.
#[test]
fn randomized_holds_replay_without_receipt_or_event_growth() {
    let run = |seed, mode| {
        let baud = [9_600, 38_400, 115_200][seed as usize % 3];
        let mut sim = ring(baud, mode, seed, true);
        for index in 0..6 {
            let mut payload = sim.rng().bytes(if index % 2 == 0 { 3 } else { 700 });
            payload[0] = 0x01;
            payload[1] = 0x00;
            let source = index % 2;
            let at = ms(sim.rng().range(800, 2_000));
            sim.enqueue(at, source, 1 - source, &payload);
        }
        sim.run_until(ms(8_000));
        sim.assert_no_violations();
        assert_eq!(sim.deliveries(0).len() + sim.deliveries(1).len(), 6);
        assert!(sim.report().max_pending_events <= 12);
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
