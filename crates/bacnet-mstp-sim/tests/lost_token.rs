//! Scenario (e): the token is lost and regenerated.

mod common;

use bacnet_mstp_codec::FrameType;
use bacnet_mstp_core::{Duration, Instant};
use bacnet_mstp_sim::analysis::{token_passes, token_users};
use bacnet_mstp_sim::{
    AdapterSpec, CollisionClass, Fault, MasterSpec, Sim, SimConfig, ViolationKind, WireFrame,
};

use common::*;

/// Kill `station` part way through its next frame, sent while it holds the
/// token, and return the cut-off frame.
fn kill_holder_mid_frame(sim: &mut Sim, at: Instant, station: u8) -> WireFrame {
    sim.inject(
        at,
        Fault::PowerOffMidFrame {
            station,
            after_octets: 6,
        },
    );
    sim.run_until(at + millis(200));
    sim.frames()
        .iter()
        .find(|frame| frame.start >= at && frame.truncated)
        .cloned()
        .expect("the holder is cut off")
}

/// Clauses 9.5.6.6 (SawTokenUser), 9.5.6.2 (LostToken) and 9.5.6.7
/// (GenerateToken).
///
/// The holder dies after six octets of a frame. Its predecessor has seen
/// more than Nmin_octets, so it has let go; nobody holds the token and the
/// line falls silent. Every master moves to NO_TOKEN after Tno_token, and
/// station 0, whose slot comes first, generates a new token exactly
/// Tno_token after the last character it heard.
#[test]
fn a_holder_dying_mid_frame_leaves_the_lowest_station_to_regenerate() {
    let mut sim = ring(BAUD, &[0, 3, 10, 30], 31);
    sim.run_until(ms(2_000));
    let cut = kill_holder_mid_frame(&mut sim, ms(2_000), 10);
    assert_eq!(cut.driver, bacnet_mstp_sim::Driver::Station(10));
    sim.run_until(ms(5_000));
    let frames = sim.frames();
    let next = first_after(frames, cut.end, |_| true).expect("a frame after the silence");
    assert!(next.is(FrameType::POLL_FOR_MASTER, 0, 1), "{next}");
    assert!(next.generation);
    assert_eq!(next.start, cut.end + millis(500));
    // Nothing at all in between: the line was silent.
    assert!(
        frames
            .iter()
            .all(|frame| frame.end <= cut.end || frame.start >= next.start)
    );
    assert_eq!(sim.core(0).unwrap().counters().token_regenerations, 2);
    assert_eq!(
        token_users(frames, next.end)
            .into_iter()
            .collect::<Vec<u8>>(),
        [0, 3, 30]
    );
    sim.assert_no_violations();
}

/// Clause 9.5.6.7: generation slots go in station order, Tslot (10 ms)
/// apart, so with station 0 gone station 3 regenerates 30 ms later than 0
/// would have.
#[test]
fn regeneration_falls_to_the_lowest_surviving_slot() {
    let mut sim = ring(BAUD, &[0, 3, 10, 30], 31);
    sim.inject(ms(1_500), Fault::PowerOff { station: 0 });
    sim.run_until(ms(3_000));
    let cut = kill_holder_mid_frame(&mut sim, ms(3_000), 10);
    sim.run_until(ms(5_000));
    let frames = sim.frames();
    let next = first_after(frames, cut.end, |_| true).expect("a frame after the silence");
    assert!(next.is(FrameType::POLL_FOR_MASTER, 3, 4), "{next}");
    assert!(next.generation);
    assert_eq!(next.start, cut.end + millis(500) + millis(30));
    assert_eq!(
        token_users(frames, next.end)
            .into_iter()
            .collect::<Vec<u8>>(),
        [3, 30]
    );
    sim.assert_no_violations();
}

/// Clause 9.5.6.7 with silence measured from different instants.
///
/// Each master times its slot from the last activity it heard. A master
/// powered up during the silence starts its own count later, so its slot can
/// coincide with another's: station 5 powered at 20 ms has its slot at
/// 20 + 500 + 50 = 570 ms, the same as station 7's 500 + 70. Both generate,
/// collide, and keep restarting each other's timers with their colliding
/// octets; the race ends only when one master's timer fires late enough to
/// hear the other first. With timer jitter below 1 ms it does, quickly.
#[test]
fn masters_whose_silence_began_apart_race_until_timing_variation_ends_it() {
    let jitter = AdapterSpec {
        deadline_jitter: Duration::from_micros(900),
        ..AdapterSpec::default()
    };
    let mut raced = 0;
    for seed in 0..16 {
        let mut config = SimConfig::new(BAUD);
        config.seed = seed;
        let mut sim = Sim::new(config);
        sim.add_master(MasterSpec::new(7, 20, BAUD).adapter(jitter))
            .add_master(MasterSpec::new(12, 20, BAUD).adapter(jitter))
            .add_master(MasterSpec::new(5, 20, BAUD).adapter(jitter).unpowered())
            .inject(ms(20), Fault::PowerOn { station: 5 });
        sim.run_until(ms(4_000));
        let report = sim.report();
        assert_eq!(
            report.collisions.len(),
            report.collisions_of(CollisionClass::GenerationRace),
            "seed {seed}"
        );
        if !report.collisions.is_empty() {
            raced += 1;
            assert_eq!(report.generation_races.len(), 1, "seed {seed}");
            assert!(
                report.generation_races[0].length < millis(1_000),
                "seed {seed}"
            );
        }
        assert_eq!(
            token_users(sim.frames(), ms(3_000))
                .into_iter()
                .collect::<Vec<u8>>(),
            [5, 7, 12],
            "seed {seed}"
        );
        sim.assert_no_violations();
    }
    assert!(raced >= 4, "only {raced} of 16 seeds raced");
}

/// The same race with ideal timers never ends: the two generators stay in
/// step, each restarting the other's Tusage_timeout with its own colliding
/// octets. The simulator reports it once it outlasts the race tolerance.
#[test]
fn without_timing_variation_a_generation_race_never_ends() {
    let mut config = SimConfig::new(BAUD);
    config.panic_on_violation = false;
    let mut sim = Sim::new(config);
    sim.add_master(MasterSpec::new(7, 20, BAUD))
        .add_master(MasterSpec::new(5, 20, BAUD).unpowered())
        .inject(ms(20), Fault::PowerOn { station: 5 });
    sim.run_until(ms(3_000));
    let report = sim.report();
    assert!(report.collisions.len() > 50);
    assert_eq!(
        report.collisions.len(),
        report.collisions_of(CollisionClass::GenerationRace)
    );
    assert_eq!(report.violations[0].kind, ViolationKind::DuplicateToken);
    // The tolerance is two seconds from the start of the race at 570 ms.
    assert!(report.violations[0].at > ms(2_570));
    assert!(report.violations[0].at < ms(2_600));
    assert!(token_passes(sim.frames()).is_empty());
}

/// Clause 9.5.6.7 defines no transition once a master's slot has passed
/// unused. The core then opens the master's window again every
/// Tslot x (Max_Master + 1), a local recovery policy kept by #1618 item 1
/// and counted in `missed_generation_slots`. A lone master whose adapter
/// served the slot deadline too late generates one cycle later.
#[test]
fn a_missed_generation_slot_comes_round_again_a_cycle_later() {
    let mut sim = Sim::new(SimConfig::new(BAUD));
    sim.add_master(MasterSpec::new(3, 31, BAUD));
    let late = AdapterSpec {
        deadline_lateness: millis(15),
        ..AdapterSpec::default()
    };
    sim.inject(
        ms(400),
        Fault::SetAdapter {
            station: 3,
            adapter: late,
        },
    )
    .inject(
        ms(560),
        Fault::SetAdapter {
            station: 3,
            adapter: AdapterSpec::default(),
        },
    );
    sim.run_until(ms(1_000));
    let frames = sim.frames();
    // The slot was 530..540 ms; the next window opens 32 slots later.
    assert!(
        frames[0].is(FrameType::POLL_FOR_MASTER, 3, 4),
        "{}",
        frames[0]
    );
    assert!(frames[0].generation);
    assert_eq!(frames[0].start, ms(530) + millis(10 * 32));
    let counters = sim.core(3).unwrap().counters();
    assert!(counters.late_deadlines > 0);
    assert_eq!(counters.missed_generation_slots, 1);
    assert_eq!(counters.token_regenerations, 1);
    sim.assert_no_violations();
}
