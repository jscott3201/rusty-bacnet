//! Regression evidence for the first host-adapter review findings.
use bacnet_mstp_codec::FrameType;
use bacnet_mstp_core::{Duration, Instant};

use crate::line::Character;
use crate::{
    AdapterSpec, Fault, HostSpec, MasterSpec, ReplyPolicy, ScriptSend, Sim, SimConfig,
    ViolationKind,
};

fn ms(value: u64) -> Instant {
    Instant::from_micros(value * 1_000)
}

fn request(source: u8) -> ScriptSend {
    ScriptSend::frame(
        Duration::ZERO,
        FrameType::BACNET_DATA_EXPECTING_REPLY,
        0,
        source,
        &[1, 4, 9],
    )
}

/// A decision becomes ready inside the await for missing echo. Completion
/// must establish the core's idle deadline from the physical end, before
/// the now-late answer is queued without advancing the core's clock.
#[test]
fn ready_answer_during_echo_wait_preserves_idle_deadline_and_wire_start() {
    let mut sim = Sim::new(SimConfig::new(10_000));
    sim.add_master(
        MasterSpec::new(0, 1, 10_000)
            .host(HostSpec {
                echo_grace: Some(Duration::from_millis(40)),
                ..HostSpec::default()
            })
            .application(ReplyPolicy::Answer {
                delay: Duration::from_millis(240),
                payload: vec![1, 0, 0xA7],
            }),
    );
    for byte in request(1).octets {
        sim.host_character(0, Character::Clean(byte));
    }
    sim.master_mut(0).unwrap().host.as_mut().unwrap().ready = Some(ms(0));
    sim.run_until(ms(240));
    assert_eq!(
        sim.report().late_answers,
        0,
        "the host task is still awaiting echo"
    );
    sim.run_until(ms(273));
    assert!(sim.frames()[0].is(FrameType::REPLY_POSTPONED, 0, 1));
    assert_eq!(sim.frames()[0].start, ms(225));
    assert_eq!(sim.frames()[0].end, ms(233));
    assert_eq!(sim.core(0).unwrap().next_deadline(), Some(ms(733)));
    assert_eq!(sim.report().late_answers, 1);
    sim.run_until(ms(741));
    assert!(sim.frames()[1].is(FrameType::POLL_FOR_MASTER, 0, 1));
    assert_eq!(sim.frames()[1].start, ms(733));
    sim.assert_no_violations();
}

/// Drain awaits own ready application work just as an echo await does. A
/// queued decision and its request association disappear on power loss.
#[test]
fn drain_wait_defers_answers_and_reboot_discards_them() {
    for reboot in [false, true] {
        let mut sim = Sim::new(SimConfig::new(10_000));
        sim.add_master(
            MasterSpec::new(0, 1, 10_000)
                .host(HostSpec {
                    drain_duration: Duration::from_millis(60),
                    ..HostSpec::default()
                })
                .application(ReplyPolicy::Answer {
                    delay: Duration::from_millis(240),
                    payload: vec![1, 0, 0xA7],
                }),
        );
        for byte in request(1).octets {
            sim.host_character(0, Character::Clean(byte));
        }
        sim.master_mut(0).unwrap().host.as_mut().unwrap().ready = Some(ms(0));
        sim.run_until(ms(240));
        assert_eq!(sim.report().late_answers, 0);
        assert_eq!(
            sim.master_mut(0)
                .unwrap()
                .host
                .as_ref()
                .unwrap()
                .decisions
                .len(),
            1
        );
        if reboot {
            sim.inject(ms(250), Fault::PowerOff { station: 0 });
            sim.inject(ms(251), Fault::PowerOn { station: 0 });
        }
        sim.run_until(ms(285));
        let host = sim.master_mut(0).unwrap().host.as_ref().unwrap();
        assert!(host.decisions.is_empty());
        assert!(host.request.is_none());
        assert_eq!(sim.report().late_answers, u32::from(!reboot));
        assert_eq!(
            sim.core(0).unwrap().next_deadline(),
            Some(ms(if reboot { 751 } else { 785 }))
        );
        sim.assert_no_violations();
    }
}

/// An adversarial private-fixture event flood is diagnosed at the fixed
/// capacity; it cannot allocate an unbounded backlog behind a serial await.
#[test]
fn ready_application_backlog_is_bounded() {
    let mut config = SimConfig::new(10_000);
    config.panic_on_violation = false;
    let mut sim = Sim::new(config);
    sim.add_master(
        MasterSpec::new(0, 1, 10_000)
            .host(HostSpec {
                echo_grace: Some(Duration::from_millis(40)),
                ..HostSpec::default()
            })
            .application(ReplyPolicy::Ignore),
    );
    for byte in request(1).octets {
        sim.host_character(0, Character::Clean(byte));
    }
    sim.master_mut(0).unwrap().host.as_mut().unwrap().ready = Some(ms(0));
    sim.run_until(ms(0));
    let id = sim.core(0).unwrap().snapshot().request_id.unwrap();
    sim.run_until(ms(240));
    for _ in 0..9 {
        sim.decide(0, id, 1, false, vec![1, 0, 0xA7]);
    }
    assert_eq!(
        sim.master_mut(0)
            .unwrap()
            .host
            .as_ref()
            .unwrap()
            .decisions
            .len(),
        8
    );
    assert_eq!(sim.report().late_answers, 0);
    assert_eq!(sim.report().violations.len(), 1);
    assert_eq!(sim.report().violations[0].kind, ViolationKind::EventGrowth);
}

/// RequestId starts over after boot. A new accepted request must carry its
/// own physical end, even when the sender and payload match the old one.
#[test]
fn reply_wire_provenance_is_fresh_after_reboot() {
    let mut sim = Sim::new(SimConfig::new(10_000));
    sim.add_master(
        MasterSpec::new(0, 1, 10_000)
            .host(HostSpec::default())
            .application(ReplyPolicy::Answer {
                delay: Duration::from_millis(5),
                payload: vec![1, 0, 0xA7],
            }),
    );
    sim.add_scripted(1, |_: &crate::SeenFrame, _: &mut Vec<ScriptSend>| {});
    sim.script_send(1, request(1));
    sim.inject(ms(300), Fault::PowerOff { station: 0 });
    sim.inject(ms(390), Fault::PowerOn { station: 0 });
    sim.run_until(ms(400));
    sim.script_send(1, request(1));
    sim.run_until(ms(450));
    let answers: Vec<_> = sim
        .frames()
        .iter()
        .filter(|frame| frame.is(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 0, 1))
        .collect();
    assert_eq!(answers.len(), 2);
    assert!(answers[1].start - sim.frames()[0].end > Duration::from_millis(250));
    sim.assert_no_violations();
}

/// The application ID ends when it chooses its answer, before turnaround
/// permits transmission. Its wire association lasts through that wait and
/// through a refused admission followed by the reissued Transmit action.
#[test]
fn reply_wire_provenance_survives_turnaround_and_admission_retry() {
    let mut sim = Sim::new(SimConfig::new(10_000));
    sim.add_master(
        MasterSpec::new(0, 1, 10_000)
            .host(HostSpec::default())
            .adapter(AdapterSpec {
                host: Some(HostSpec::default()),
                start_delay: Duration::from_millis(5),
                ..AdapterSpec::default()
            })
            .application(ReplyPolicy::Answer {
                delay: Duration::ZERO,
                payload: vec![1, 0, 0xA7],
            }),
    );
    for byte in request(1).octets {
        sim.host_character(0, Character::Clean(byte));
    }
    sim.master_mut(0).unwrap().host.as_mut().unwrap().ready = Some(ms(0));
    sim.run_until(ms(8));
    assert!(sim.core(0).unwrap().snapshot().request_id.is_none());
    assert!(sim
        .master_mut(0)
        .unwrap()
        .host
        .as_ref()
        .unwrap()
        .request
        .is_some());
    // At 9 ms this ready input wins the tie against the first admission.
    sim.host_character(0, Character::Clean(0x55));
    sim.run_until(ms(35));
    assert_eq!(sim.report().refused_starts, 1);
    assert!(sim.frames()[0].is(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, 0, 1));
    assert_eq!(sim.frames()[0].start, ms(18));
    assert!(sim
        .master_mut(0)
        .unwrap()
        .host
        .as_ref()
        .unwrap()
        .request
        .is_none());
    sim.assert_no_violations();
}

/// Unrelated traffic, including an unaccepted request from the same source,
/// cannot replace the original request's physical end in the reply oracle.
#[test]
fn late_reply_keeps_original_wire_request_across_intervening_frames() {
    for kind in [
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        FrameType::BACNET_DATA_EXPECTING_REPLY,
    ] {
        for source in [1, 2] {
            let host = HostSpec {
                receive_latency: Duration::from_millis(16),
                ..HostSpec::default()
            };
            let mut config = SimConfig::new(38_400);
            config.panic_on_violation = false;
            let mut sim = Sim::new(config);
            sim.add_master(
                MasterSpec::new(0, 2, 38_400)
                    .host(host)
                    .adapter(AdapterSpec {
                        host: Some(host),
                        start_delay: Duration::from_millis(20),
                        ..AdapterSpec::default()
                    })
                    .application(ReplyPolicy::Ignore),
            );
            for station in [1, 2] {
                sim.add_scripted(station, |_: &crate::SeenFrame, _: &mut Vec<ScriptSend>| {});
            }
            sim.script_send(1, request(1));
            sim.run_until(ms(9));
            sim.script_send(
                usize::from(source),
                ScriptSend::frame(
                    Duration::ZERO,
                    kind,
                    if kind == FrameType::BACNET_DATA_NOT_EXPECTING_REPLY {
                        255
                    } else {
                        0
                    },
                    source,
                    &[1, 4, 9],
                ),
            );
            sim.run_until(ms(270));
            let response = sim
                .frames()
                .iter()
                .find(|f| f.is(FrameType::REPLY_POSTPONED, 0, 1))
                .unwrap();
            assert!(response.start - sim.frames()[0].end > Duration::from_millis(250));
            // Measuring from the latest DER instead would hide this failure.
            // Both frames finished before the same 16 ms buffered read.
            assert!(response.start - sim.frames()[1].end < Duration::from_millis(250));
            let replies: Vec<_> = sim
                .report()
                .violations
                .iter()
                .filter(|v| v.kind == ViolationKind::ReplyDelay)
                .collect();
            assert_eq!(
                replies.len(),
                1,
                "{kind:?}, source {source}: {}",
                sim.dump(40)
            );
            assert!(sim
                .report()
                .violations
                .iter()
                .all(|v| v.kind == ViolationKind::ReplyDelay));
            assert_eq!(sim.deliveries(0).iter().filter(|d| d.request).count(), 1);
        }
    }
}
