//! Delayed local returns compose with real host/octet rings and long frames.

use bacnet_mstp_core::{Duration, Instant};
use bacnet_mstp_sim::{HostSpec, MasterSpec, Sim, SimConfig};

fn ms(value: u64) -> Instant {
    Instant::from_micros(value * 1000)
}

fn ring(baud: u32, mode: u8, seed: u64, random: bool) -> Sim {
    let mut config = SimConfig::new(baud);
    config.seed = seed;
    let mut sim = Sim::new(config);
    for station in [0, 1] {
        let mut spec = MasterSpec::new(station, 1, baud).extended_frames();
        spec.config.transmit_watchdog = Duration::from_secs(6);
        spec.config.queue_max_age = Duration::from_secs(15);
        spec.adapter.echo = true;
        spec.adapter.post_drive = sim.timing().max_post_drive();
        if random {
            spec.adapter.inter_octet_gap =
                Duration::from_nanos(sim.rng().range(0, 19_000_000_000 / u64::from(baud)));
        }
        if mode == 1 || (mode == 2 && station == 0) {
            // This admitted clean profile returns before the earliest peer
            // character after turnaround. More delayed interleaving and
            // expired copies are separate, intentionally diagnostic cases.
            let delay = if random {
                Duration::from_nanos(sim.rng().range(1, 2_000_000_000 / u64::from(baud)))
            } else {
                sim.timing().bits(2)
            };
            spec = spec.host(HostSpec {
                echo_return_delay: delay,
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

fn assert_clean(sim: &Sim, receipts: usize) {
    sim.assert_no_violations();
    assert_eq!(sim.deliveries(0).len() + sim.deliveries(1).len(), receipts);
    assert!(sim.frames().iter().all(|frame| frame.is_clean()));
    assert!(sim.report().collisions.is_empty());
    assert!(sim.report().max_pending_events <= 12);
    assert!(sim.report().local_echo.max_pending <= 1);
    assert_eq!(sim.report().local_echo.overflows, 0);
    assert_eq!(sim.report().local_echo.cancelled, 0);
    assert_eq!(
        sim.report().local_echo.captured,
        sim.report().local_echo.released + sim.report().local_echo.pending as u64
    );
    for station in [0, 1] {
        let counters = sim.core(station).unwrap().counters();
        assert_eq!(counters.own_address_frames, 0);
        assert_eq!(counters.transmit_timeouts, 0);
        assert_eq!(counters.unicast.drops_expired, 0);
        if let Some(host) = sim.host_stats(station) {
            assert_eq!(host.echo_missing, 0);
            assert_eq!(host.echo_mismatches, 0);
        }
    }
}

#[test]
fn delayed_returns_preserve_standard_extended_and_mixed_ring_receipts() {
    for baud in [9600, 38400, 115200] {
        for mode in [0, 1, 2] {
            let mut sim = ring(baud, mode, 0, false);
            let standard = vec![0x11; 501];
            let extended = vec![0x11; 1497];
            sim.enqueue(ms(800), 0, 1, &standard);
            sim.enqueue(ms(1000), 1, 0, &extended);
            sim.run_until(ms(7000));
            assert_clean(&sim, 2);
            assert_eq!(sim.deliveries(1)[0].data, standard);
            assert_eq!(sim.deliveries(0)[0].data, extended);
            assert!(sim.frames().iter().any(|frame| frame.octets.len() == 511));
            assert!(sim.frames().iter().any(|frame| frame.octets.len() == 1516));
            if mode == 0 {
                assert_eq!(sim.report().local_echo.captured, 0);
            } else {
                assert!(sim.report().local_echo.released > 0);
            }
        }
    }
}

#[test]
fn seeded_delays_gaps_holds_and_host_chunks_replay_with_bounded_pending_work() {
    let run = |seed: u64, mode| {
        let baud = [9600, 38400, 115200][seed as usize % 3];
        let mut sim = ring(baud, mode, seed, true);
        for index in 0..6 {
            let mut payload = sim.rng().bytes(if index % 2 == 0 { 501 } else { 700 });
            payload[0] = 1;
            payload[1] = 0;
            let at = ms(sim.rng().range(800, 2000));
            sim.enqueue(at, index % 2, 1 - index % 2, &payload);
        }
        sim.run_until(ms(15_000));
        assert_clean(&sim, 6);
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
