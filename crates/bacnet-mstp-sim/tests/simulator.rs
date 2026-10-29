//! Checks of the simulator itself: its collision model, trace and failure
//! reporting.

mod common;

use bacnet_mstp_codec::FrameType;
use bacnet_mstp_core::Duration;
use bacnet_mstp_sim::analysis::token_users;
use bacnet_mstp_sim::{
    AdapterSpec, CollisionClass, Driver, Fault, MasterSpec, ScriptSend, SeenFrame, Sim, SimConfig,
    TraceKind, ViolationKind,
};

use common::*;

/// Master 0 alone, with scripted stations 2 and 4 that both answer a poll of
/// station 2, at the same instant.
fn two_answers(config: SimConfig) -> Sim {
    let mut sim = Sim::new(config);
    sim.add_master(MasterSpec::new(0, 7, BAUD));
    for station in [2, 4] {
        sim.add_scripted(
            station,
            move |frame: &SeenFrame, sends: &mut Vec<ScriptSend>| {
                if frame.destination == 2 && frame.frame_type == FrameType::POLL_FOR_MASTER {
                    sends.push(ScriptSend::frame(
                        millis(1),
                        FrameType::REPLY_TO_POLL_FOR_MASTER,
                        frame.source,
                        station,
                        &[],
                    ));
                }
            },
        );
    }
    sim
}

#[test]
fn overlapping_drivers_collide_and_receivers_hear_garbage() {
    let mut config = SimConfig::new(BAUD);
    config.panic_on_violation = false;
    let mut sim = two_answers(config);
    sim.run_until(ms(600));
    let report = sim.report();
    assert_eq!(report.collisions.len(), 1);
    let collision = report.collisions[0];
    assert_eq!(collision.class, CollisionClass::Scripted);
    assert_eq!(
        (collision.driver, collision.other),
        (Driver::Station(4), Driver::Station(2))
    );
    assert_eq!(report.violations[0].kind, ViolationKind::Collision);
    let collided: Vec<Driver> = sim
        .frames()
        .iter()
        .filter(|frame| frame.collided)
        .map(|frame| frame.driver)
        .collect();
    assert_eq!(collided, [Driver::Station(2), Driver::Station(4)]);
    // The master heard eight garbled characters, not a reply, and polled on.
    let after = sim.frames()[4].clone();
    assert!(after.is(FrameType::POLL_FOR_MASTER, 0, 3), "{after}");
    assert!(sim.perturbed());
}

#[test]
fn scripted_collisions_can_be_tolerated() {
    let mut config = SimConfig::new(BAUD);
    config.tolerance.scripted_collisions = true;
    let mut sim = two_answers(config);
    sim.run_until(ms(600));
    assert_eq!(sim.report().collisions_of(CollisionClass::Scripted), 1);
    sim.assert_no_violations();
}

#[test]
#[should_panic(expected = "MS/TP simulator invariant failed")]
fn a_strict_run_stops_at_the_first_violation() {
    let mut sim = two_answers(SimConfig::new(BAUD));
    sim.run_until(ms(600));
}

#[test]
fn the_trace_records_octets_actions_and_state_changes() {
    let mut sim = ring(BAUD, &[0, 3], 3);
    sim.run_until(ms(560));
    let entries: Vec<String> = sim.trace().entries().map(ToString::to_string).collect();
    let has = |text: &str| entries.iter().any(|entry| entry.contains(text));
    assert!(
        has("    500.000ms st000  state Idle -> PollForMaster"),
        "{entries:#?}"
    );
    assert!(has("    500.000ms st000  SetDirection(Transmit)"));
    assert!(has(
        "    500.000ms st000  TX start FrameType::POLL_FOR_MASTER 0->1 (8 octets)"
    ));
    assert!(has("    500.000ms st000  octet[0] 55"));
    assert!(has("    500.261ms st000  octet[1] FF"));
    assert!(has("st003  state Idle -> NoToken"));
    assert!(
        sim.trace()
            .entries()
            .any(|entry| matches!(entry.kind, TraceKind::FrameEnd(_)))
    );
    let dump = sim.dump(5);
    assert!(dump.starts_with("trace at     560.000ms:\n"), "{dump}");
    assert_eq!(dump.lines().count(), 7);
}

/// Masters 0 and 3, run until 3 has just received the token; from then on
/// station 3's adapter behaves as `adapter`. Station 0 sends the token again
/// 20 ms after the first one ended, so a frame from 3 that starts then
/// collides with it.
fn collide_with_the_second_token(config: SimConfig, adapter: AdapterSpec) -> Sim {
    let mut sim = Sim::new(config);
    sim.add_master(MasterSpec::new(0, 3, BAUD))
        .add_master(MasterSpec::new(3, 3, BAUD));
    run_until_frame(&mut sim, ms(1_500), ms(3_000), |frame| {
        frame.is(FrameType::TOKEN, 0, 3)
    });
    let now = sim.now();
    sim.inject(
        now,
        Fault::SetAdapter {
            station: 3,
            adapter,
        },
    );
    sim.run_for(millis(100));
    sim
}

/// Station 3's core hands its frame over Tturnaround (1042 us) after the
/// token, and its adapter starts it 18958 us later: exactly when station 0
/// sends the token again. The core could not have known, so the collision is
/// the slow adapter's; a strict run still fails on it.
#[test]
fn a_slow_adapter_collision_fails_a_strict_run() {
    let slow = AdapterSpec {
        start_delay: Duration::from_micros(18_958),
        ..AdapterSpec::default()
    };
    let mut config = SimConfig::new(BAUD);
    config.panic_on_violation = false;
    let sim = collide_with_the_second_token(config, slow);
    let report = sim.report();
    assert_eq!(report.collisions[0].class, CollisionClass::SlowAdapter);
    assert_eq!(
        (report.collisions[0].driver, report.collisions[0].other),
        (Driver::Station(3), Driver::Station(0))
    );
    assert_eq!(report.violations[0].kind, ViolationKind::Collision);

    config.tolerance.slow_adapter_collisions = true;
    let sim = collide_with_the_second_token(config, slow);
    assert_eq!(sim.report().collisions_of(CollisionClass::SlowAdapter), 1);
    assert!(
        sim.report()
            .violations
            .iter()
            .all(|violation| violation.kind != ViolationKind::Collision)
    );
}

/// Station 3's adapter serves deadlines 19 ms late, so its core hands its
/// frame over 42 us after station 0's second token began, but before that
/// token's first character arrives (260 us in): the core could not have
/// heard it, and the collision is the late adapter's. The tolerance covers
/// it; a strict run fails on it. A core that hands a frame over after
/// hearing a character is never excused; `sim::checks` tests that case.
#[test]
fn a_late_core_that_dispatches_before_hearing_the_frame_is_the_adapters() {
    let late = AdapterSpec {
        deadline_lateness: Duration::from_micros(19_000),
        ..AdapterSpec::default()
    };
    let mut config = SimConfig::new(BAUD);
    config.panic_on_violation = false;
    let sim = collide_with_the_second_token(config, late);
    let report = sim.report();
    assert_eq!(report.collisions[0].class, CollisionClass::SlowAdapter);
    assert_eq!(report.violations[0].kind, ViolationKind::Collision);

    config.tolerance.slow_adapter_collisions = true;
    let sim = collide_with_the_second_token(config, late);
    assert_eq!(sim.report().collisions_of(CollisionClass::SlowAdapter), 1);
    assert!(
        sim.report()
            .violations
            .iter()
            .all(|violation| violation.kind != ViolationKind::Collision)
    );
}

/// Every adapter here starts frames 4 ms after its core hands them over,
/// which would hide a core that dispatched with no turnaround at all: each
/// start would still come 4 ms after the last activity. The simulator checks
/// the hand-over too. With a correct core this test only shows that check
/// raises nothing falsely, the collision tolerance on; its failing half
/// needs a core mutated to skip the turnaround, which then fails here even
/// with the tolerance on (checked by hand, PR #1627).
#[test]
fn slow_adapters_do_not_hide_a_dispatch_inside_turnaround() {
    let slow = AdapterSpec {
        start_delay: Duration::from_millis(4),
        ..AdapterSpec::default()
    };
    let mut config = SimConfig::new(BAUD);
    config.tolerance.slow_adapter_collisions = true;
    let mut sim = Sim::new(config);
    for station in [0, 3, 10] {
        sim.add_master(MasterSpec::new(station, 15, BAUD).adapter(slow));
    }
    sim.run_until(ms(3_000));
    assert_eq!(
        token_users(sim.frames(), ms(2_000))
            .into_iter()
            .collect::<Vec<u8>>(),
        [0, 3, 10]
    );
    sim.assert_no_violations();
}
