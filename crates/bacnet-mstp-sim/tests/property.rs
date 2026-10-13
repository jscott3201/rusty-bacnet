//! Scenario (i): randomized runs. Each seed builds a ring, some of whose
//! masters take extended frames, drives traffic through it and injects
//! noise, damaged octets, power cuts, joins and a slow adapter for most of a
//! run, then leaves it quiet; every invariant must hold throughout, and the
//! ring must be whole again at the end. Each seed runs with the shortest
//! timeouts Clause 9.5.3 allows, then with a host transport's longest ones
//! on every master and on every other master.
//!
//! Run it alone with `cargo nextest run -p bacnet-mstp-sim -E 'binary(property)'`.
//! `MSTP_SIM_SEEDS=<n>` runs more seeds; `MSTP_SIM_SEED=<seed>` runs one;
//! `MSTP_SIM_PANIC=1` stops at the first violation with the trace before it.

mod common;

use bacnet_mstp_codec::FrameType;
use bacnet_mstp_core::{Duration, Instant, MAX_FRAME_ABORT, MAX_REPLY_TIMEOUT, MAX_USAGE_TIMEOUT};
use bacnet_mstp_sim::analysis::token_users;
use bacnet_mstp_sim::{
    AdapterSpec, CollisionClass, Corruption, Fault, MasterSpec, ReplyPolicy, Sim, SimConfig, SimRng,
};

use common::*;

/// Seeds run by default: enough to cover the fault mix, few enough to stay
/// within a few seconds of wall time in a debug build.
const DEFAULT_SEEDS: u64 = 24;
const FAULTS_FROM: u64 = 1_000;
const FAULTS_UNTIL: u64 = 12_000;
const SETTLE_FROM: u64 = 14_000;
const END: u64 = 26_000;

struct Plan {
    sim: Sim,
    stations: Vec<u8>,
}

/// An NPDU of 502 to 1497 octets, which only an extended frame carries.
fn long_npdu(rng: &mut SimRng, expecting_reply: bool) -> Vec<u8> {
    let length = rng.range(502, 1_498) as usize;
    let mut npdu = rng.bytes(length);
    npdu[0] = 0x01;
    npdu[1] = if expecting_reply { 0x04 } else { 0x00 };
    npdu
}

/// How a station's application answers. One that takes extended frames
/// sometimes answers with an NPDU only an extended frame carries.
fn reply_policy(rng: &mut SimRng, extended: bool) -> ReplyPolicy {
    let payload = if extended && rng.chance(30) {
        long_npdu(rng, false)
    } else {
        vec![0x01, 0x00, rng.byte()]
    };
    match rng.below(4) {
        0 => ReplyPolicy::Answer {
            delay: Duration::from_millis(rng.range(1, 30)),
            payload,
        },
        1 => ReplyPolicy::Answer {
            delay: millis(240),
            payload,
        },
        2 => ReplyPolicy::Postpone {
            delay: Duration::from_millis(rng.range(1, 20)),
            payload,
        },
        _ => ReplyPolicy::Ignore,
    }
}

/// The timeouts every master in a run uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Timing {
    /// The baud-rate constructors' values: the shortest T_frame_abort,
    /// T_reply_timeout and T_usage_timeout Clause 9.5.3 allows.
    Standard,
    /// A host transport's values (#1655): the longest it allows, 100, 300
    /// and 35 ms.
    Host,
    /// Host values on every other master, in address order, and standard
    /// ones on the rest: hosts beside UART-driven devices.
    Mixed,
}

fn plan(seed: u64, timing: Timing) -> Plan {
    let mut rng = SimRng::new(seed);
    let baud = if rng.chance(30) { SLOW_BAUD } else { BAUD };
    let mut config = SimConfig::new(baud);
    config.seed = seed;
    config.trace_capacity = 600;
    // Collect violations to report them with the seed; MSTP_SIM_PANIC=1
    // stops at the first one instead, with the trace leading up to it.
    config.panic_on_violation = std::env::var_os("MSTP_SIM_PANIC").is_some();
    let mut sim = Sim::new(config);

    let max_master = 23;
    let mut stations: Vec<u8> = Vec::new();
    let count = rng.range(4, 6) as usize;
    while stations.len() < count {
        let station = rng.below(u64::from(max_master) + 1) as u8;
        if !stations.contains(&station) {
            stations.push(station);
        }
    }
    stations.sort_unstable();
    // At most 8 ms late, so start delay plus Tturnaround stays inside
    // Tusage_delay (15 ms) even at 9600 baud.
    let slow = stations[rng.below(count as u64) as usize];
    let joiner = stations[rng.below(count as u64) as usize];
    // About half the masters take extended frames (Types 32 and 33); the
    // rest count them as invalid frames, as nodes without COBS support do.
    let extended: Vec<bool> = stations.iter().map(|_| rng.chance(50)).collect();
    for (index, (&station, &extended)) in stations.iter().zip(&extended).enumerate() {
        let mut adapter = AdapterSpec {
            deadline_jitter: Duration::from_micros(rng.range(0, 900)),
            echo: rng.chance(20),
            ..AdapterSpec::default()
        };
        if station == slow {
            adapter.start_delay = Duration::from_micros(rng.range(500, 8_000));
        }
        let mut spec = MasterSpec::new(station, max_master, baud)
            .max_info_frames(rng.range(1, 4) as u8)
            .queues(8, 8)
            .adapter(adapter)
            .reply(reply_policy(&mut rng, extended));
        if station == joiner {
            spec = spec.unpowered();
        }
        if extended {
            spec = spec.extended_frames();
        }
        if timing == Timing::Host || (timing == Timing::Mixed && index % 2 == 0) {
            spec.config.t_frame_abort = MAX_FRAME_ABORT;
            spec.config.t_reply_timeout = MAX_REPLY_TIMEOUT;
            spec.config.t_usage_timeout = MAX_USAGE_TIMEOUT;
        }
        sim.add_master(spec);
    }
    sim.inject(
        ms(rng.range(800, 6_000)),
        Fault::PowerOn { station: joiner },
    );

    // Traffic while the faults last, a quarter of it requests to other
    // masters. A master that takes extended frames sends a tenth of its
    // NPDUs in them, whether the receiver takes them or not.
    for (&station, &extended) in stations.iter().zip(&extended) {
        let mut at = rng.range(600, 900);
        while at < FAULTS_UNTIL {
            let to = if rng.chance(15) {
                0xFF
            } else {
                stations[rng.below(count as u64) as usize]
            };
            let request = to != 0xFF && to != station && rng.chance(25);
            let npdu = if extended && rng.chance(10) {
                long_npdu(&mut rng, request)
            } else {
                npdu(request, rng.byte())
            };
            sim.enqueue(ms(at), station, to, &npdu);
            at += rng.range(60, 300);
        }
    }

    // Faults between FAULTS_FROM and FAULTS_UNTIL, when whatever is still
    // armed is withdrawn. A node powered off comes back within two seconds,
    // and every node is powered on again after the faults stop.
    let mut at = FAULTS_FROM;
    while at < FAULTS_UNTIL {
        let victim = stations[rng.below(count as u64) as usize];
        match rng.below(10) {
            0..=3 => {
                let length = rng.range(1, 16) as usize;
                sim.inject(
                    ms(at),
                    Fault::Noise {
                        octets: rng.bytes(length),
                    },
                );
            }
            4..=6 => {
                sim.inject(
                    ms(at),
                    Fault::CorruptNext {
                        station: victim,
                        frame_type: None,
                        offset: rng.range(0, 12) as usize,
                        corruption: if rng.chance(50) {
                            Corruption::Xor(1 << rng.below(8))
                        } else {
                            Corruption::FramingError
                        },
                    },
                );
            }
            7 | 8 if victim != joiner => {
                let fault = if rng.chance(50) {
                    Fault::PowerOff { station: victim }
                } else {
                    Fault::PowerOffMidFrame {
                        station: victim,
                        after_octets: rng.range(1, 10) as usize,
                    }
                };
                let back = at + rng.range(100, 2_000);
                sim.inject(ms(at), fault)
                    .inject(ms(back), Fault::PowerOn { station: victim });
            }
            _ => {}
        }
        at += rng.range(80, 500);
    }
    sim.inject(ms(FAULTS_UNTIL), Fault::Disarm);
    for &station in &stations {
        sim.inject(ms(FAULTS_UNTIL + 1_000), Fault::PowerOn { station });
    }
    Plan { sim, stations }
}

/// Run until the token has gone once round the ring with every frame clean,
/// from a Token some master sends to the next Token addressed back to that
/// master (a token sent again by the same master does not close the lap);
/// then declare the bus settled, so the checks a fault suspends apply
/// again.
fn settle_after_a_clean_rotation(sim: &mut Sim, limit: Instant, seed: u64) {
    let mut seen = sim.frames().len();
    let mut lap: Option<u8> = None;
    loop {
        assert!(sim.now() < limit, "seed {seed}: the bus never settled");
        sim.run_for(millis(1));
        let mut closed = false;
        for frame in &sim.frames()[seen..] {
            if !frame.is_clean() {
                lap = None;
            } else if frame.frame_type() == Some(FrameType::TOKEN) {
                match lap {
                    None => lap = frame.source(),
                    Some(first) if frame.destination() == Some(first) => closed = true,
                    Some(_) => {}
                }
            }
        }
        seen = sim.frames().len();
        if closed {
            sim.settle();
            return;
        }
    }
}

fn check(seed: u64, timing: Timing) {
    let Plan { mut sim, stations } = plan(seed, timing);
    // The faults stop at 12 s and every node is powered again at 13 s. From
    // 14 s, once the bus has settled, every listening master must count every
    // clean frame again for the rest of the run.
    sim.run_until(ms(SETTLE_FROM));
    settle_after_a_clean_rotation(&mut sim, ms(END - 6_000), seed);
    sim.run_until(ms(END));
    assert!(
        !sim.perturbed(),
        "seed {seed}: a fault after the bus settled"
    );
    let report = sim.report();
    assert!(
        report.violations.is_empty(),
        "seed {seed}: invariant violations\n{}",
        sim.dump(200)
    );
    // Slow-adapter collisions are not tolerated here: the slow adapter stays
    // inside Tusage_delay, so it should never cause one.
    for collision in &report.collisions {
        assert!(
            matches!(
                collision.class,
                CollisionClass::Noise | CollisionClass::GenerationRace
            ),
            "seed {seed}: {collision:?}"
        );
    }
    // Races are rare here (none in a 400-seed sweep); lost_token.rs drives
    // them on purpose. Any that happens must end within a second.
    for race in &report.generation_races {
        assert!(race.length < millis(1_000), "seed {seed}: {race:?}");
    }
    // The pending events stay small however long the run.
    assert!(
        report.max_pending_events <= 8 + 2 * stations.len(),
        "seed {seed}: {} pending events",
        report.max_pending_events
    );
    // A joiner waits up to Npoll token uses for a maintenance poll to reach
    // it, about 4 s at 9600 baud. By the last four seconds every master uses
    // the token again.
    let powered: Vec<u8> = stations
        .iter()
        .copied()
        .filter(|&station| sim.core(station).is_some())
        .collect();
    assert_eq!(powered, stations, "seed {seed}");
    let users: Vec<u8> = token_users(sim.frames(), ms(END - 4_000))
        .into_iter()
        .collect();
    assert_eq!(users, stations, "seed {seed}\n{}", sim.dump(60));
}

fn seeds() -> Vec<u64> {
    if let Some(seed) = std::env::var("MSTP_SIM_SEED")
        .ok()
        .and_then(|value| value.parse().ok())
    {
        return vec![seed];
    }
    let count = std::env::var("MSTP_SIM_SEEDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_SEEDS);
    (0..count).collect()
}

/// Clauses 9.5.4 to 9.5.6 under a random mix of faults: every invariant
/// holds for every seed, and each run ends with an intact ring.
#[test]
fn random_faults_never_break_the_invariants_or_the_ring() {
    for seed in seeds() {
        check(seed, Timing::Standard);
    }
}

/// The same with host transports' timeouts, the longest Clause 9.5.3
/// allows (#1655): T_frame_abort 100 ms, T_reply_timeout 300 ms and
/// T_usage_timeout 35 ms, on every master and then on every other one.
/// Waiting longer for a successor, a poll answer or a reply still keeps one
/// token on the bus and the ring whole.
#[test]
fn host_timeouts_never_break_the_invariants_or_the_ring() {
    for seed in seeds() {
        check(seed, Timing::Host);
        check(seed, Timing::Mixed);
    }
}

/// Every seed replays exactly: the same frames at the same instants.
#[test]
fn a_seed_replays_exactly() {
    let run = |seed| {
        let mut plan = plan(seed, Timing::Standard);
        plan.sim.run_until(ms(6_000));
        plan.sim
            .frames()
            .iter()
            .map(|frame| (frame.start, frame.end, frame.octets.clone()))
            .collect::<Vec<(Instant, Instant, Vec<u8>)>>()
    };
    let first = run(3);
    assert!(first.len() > 50);
    assert_eq!(first, run(3));
    assert_ne!(first, run(4));
}
