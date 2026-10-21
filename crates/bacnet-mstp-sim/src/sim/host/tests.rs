//! Adapter seams that require exact same-instant scheduling or buffer access.
use bacnet_mstp_codec::FrameType;
use bacnet_mstp_core::{Duration, Instant};

use crate::line::Character;
use crate::{AdapterSpec, Fault, HostSpec, MasterSpec, ScriptSend, Sim, SimConfig};

fn ms(value: u64) -> Instant {
    Instant::from_micros(value * 1000)
}

fn host(spec: HostSpec) -> Sim {
    let mut sim = Sim::new(SimConfig::new(38_400));
    sim.add_master(MasterSpec::new(0, 1, 38_400).host(spec));
    sim
}

fn raw_read(sim: &mut Sim, at: Instant, bytes: &[u8]) {
    sim.run_until(at);
    for &byte in bytes {
        sim.host_character(0, Character::Clean(byte));
    }
    sim.master_mut(0).unwrap().host.as_mut().unwrap().ready = Some(at);
    sim.run_until(at);
}

/// Two physical frames coalesce into one host read; deferred receive
/// accounting must count both without requiring one counter change per read.
#[test]
fn two_frames_in_one_read_keep_delivery_stamps_and_receipts() {
    let mut sim = host(HostSpec {
        receive_latency: Duration::from_millis(16),
        ..HostSpec::default()
    });
    sim.add_scripted(1, |_: &crate::SeenFrame, _: &mut Vec<ScriptSend>| {});
    sim.run_until(ms(10));
    let send = ScriptSend::frame(
        Duration::ZERO,
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        0,
        1,
        &[1, 0, 1],
    );
    let first_end = sim.timing().end_of(sim.now(), send.octets.len());
    sim.script_send(1, send.clone());
    sim.run_until(first_end);
    sim.script_send(1, send);
    sim.run_until(ms(30));
    let deliveries = sim.deliveries(0);
    assert_eq!(deliveries.len(), 2);
    assert_eq!(
        deliveries[0].at,
        ms(10) + sim.timing().octet() + Duration::from_millis(16)
    );
    assert_eq!(deliveries[1].at, deliveries[0].at);
    assert_eq!(sim.host_stats(0).unwrap().reads, 1);
    assert_eq!(sim.core(0).unwrap().counters().frames_received, 2);
    sim.assert_no_violations();
}

/// The adapter's own pre-start drain works even when readiness was discovered
/// after selecting the transmit action, before calling admission.
#[test]
fn available_input_is_drained_before_confirming_a_start() {
    let mut sim = Sim::new(SimConfig::new(38_400));
    let spec = MasterSpec::new(0, 1, 38_400)
        .host(HostSpec::default())
        .adapter(AdapterSpec {
            host: Some(HostSpec::default()),
            start_delay: Duration::from_millis(5),
            ..AdapterSpec::default()
        });
    sim.add_master(spec);
    sim.run_until(ms(500));
    let id = sim.master_mut(0).unwrap().pending.as_ref().unwrap().id;
    sim.advance(ms(505));
    sim.host_character(0, Character::Clean(0x55));
    sim.master_mut(0).unwrap().host.as_mut().unwrap().ready = Some(ms(505));
    sim.adapter_start(0, id);
    assert!(sim.nodes[0].driving.is_none());
    assert_eq!(sim.host_stats(0).unwrap().reads, 1);
    assert_eq!(sim.report().refused_starts, 1);
    sim.run_until(ms(520));
    assert!(sim.frames()[0].start >= ms(505) + sim.core(0).unwrap().config().t_turnaround);
    sim.assert_no_violations();
}

/// A read completing at the exact scheduled start wins the event tie.
#[test]
fn a_ready_read_wins_a_transmit_admission_tie() {
    let mut sim = Sim::new(SimConfig::new(38_400));
    sim.add_master(
        MasterSpec::new(0, 1, 38_400)
            .host(HostSpec::default())
            .adapter(AdapterSpec {
                host: Some(HostSpec::default()),
                start_delay: Duration::from_millis(5),
                ..AdapterSpec::default()
            }),
    );
    sim.run_until(ms(504));
    sim.host_character(0, Character::Clean(0x55)); // read due at 505 ms
    sim.run_until(ms(505));
    assert_eq!(sim.host_stats(0).unwrap().reads, 1);
    assert!(sim.nodes[0].driving.is_none());
    sim.assert_no_violations();
}

/// Serial drain is a completion floor, never permission to finish before
/// the last physical stop bit. Reads during drain wait for completion.
#[test]
fn completion_is_the_later_of_drain_and_wire_time() {
    for drain_ms in [0, 1, 20] {
        let mut sim = host(HostSpec {
            drain_duration: Duration::from_millis(drain_ms),
            ..HostSpec::default()
        });
        let wire_end = sim.timing().end_of(ms(500), 8);
        let complete = wire_end.max(ms(500 + drain_ms));
        sim.run_until(complete);
        assert_eq!(sim.frames()[0].end, wire_end);
        assert_eq!(sim.host_stats(0).unwrap().last_completion, Some(complete));
        assert!(sim.nodes[0].driving.is_none());
        assert!(sim.master_mut(0).unwrap().transmitting.is_none());
        sim.assert_no_violations();
    }
}

/// A missing echo delays notification, but the completion timestamp passed
/// to the core remains the frame end. An exposed overdue timer is counted.
#[test]
fn echo_grace_does_not_move_the_completion_stamp() {
    let mut sim = host(HostSpec {
        echo_grace: Some(Duration::from_millis(40)),
        ..HostSpec::default()
    });
    let wire_end = sim.timing().end_of(ms(500), 8);
    sim.run_until(wire_end + Duration::from_millis(40));
    assert_eq!(sim.host_stats(0).unwrap().last_completion, Some(wire_end));
    assert_eq!(sim.host_stats(0).unwrap().echo_missing, 1);
    assert_eq!(sim.core(0).unwrap().counters().late_deadlines, 1);
    sim.assert_no_violations();
}

/// Buffered bytes and pending write completion belong to the old powered
/// incarnation. Reboot cannot deliver them or finish the new core's write.
#[test]
fn reboot_discards_buffered_input_and_completion() {
    let mut sim = host(HostSpec {
        receive_latency: Duration::from_millis(16),
        drain_duration: Duration::from_millis(20),
        ..HostSpec::default()
    });
    sim.run_until(ms(500));
    let peer = ScriptSend::frame(
        Duration::ZERO,
        FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        0,
        1,
        &[1, 0, 5],
    );
    for byte in peer.octets {
        sim.host_character(0, Character::Clean(byte));
    }
    sim.inject(ms(503), Fault::PowerOff { station: 0 });
    sim.inject(ms(504), Fault::PowerOn { station: 0 });
    sim.run_until(ms(550));
    assert!(sim.deliveries(0).is_empty());
    assert_eq!(sim.host_stats(0).unwrap().reads, 0);
    assert_eq!(sim.host_stats(0).unwrap().last_completion, None);
    sim.assert_no_violations();
}

/// A delayed adapter event must not use a new core's reused TransmitId.
#[test]
fn old_start_cannot_admit_a_new_incarnations_frame() {
    let mut sim = Sim::new(SimConfig::new(38_400));
    sim.add_master(
        MasterSpec::new(0, 1, 38_400)
            .host(HostSpec::default())
            .adapter(AdapterSpec {
                host: Some(HostSpec::default()),
                start_delay: Duration::from_millis(100),
                ..AdapterSpec::default()
            }),
    );
    sim.inject(ms(550), Fault::PowerOff { station: 0 });
    sim.inject(ms(551), Fault::PowerOn { station: 0 });
    let token = ScriptSend::frame(Duration::ZERO, FrameType::TOKEN, 0, 1, &[]);
    raw_read(&mut sim, ms(560), &token.octets);
    sim.run_until(ms(600)); // old incarnation's pending TransmitId(0)
    assert!(sim.frames().is_empty());
    assert!(sim.nodes[0].driving.is_none());
    sim.run_until(ms(670));
    assert_eq!(sim.frames().len(), 1);
    assert!(sim.frames()[0].start >= ms(660));
    sim.assert_no_violations();
}

/// The MAC answers empty and maximum-size Test_Request data through host
/// chunks too, with no application delivery and no leaked local echo.
#[test]
fn host_test_requests_keep_empty_and_maximum_payloads() {
    for baud in [9_600, 38_400, 115_200] {
        for size in [0, 1, 501] {
            let mut sim = Sim::new(SimConfig::new(baud));
            let mut spec = MasterSpec::new(0, 1, baud).host(HostSpec {
                max_chunk: 3,
                random_chunks: true,
                echo_grace: Some(Duration::from_millis(40)),
                ..HostSpec::default()
            });
            spec.adapter.echo = true;
            sim.add_master(spec);
            sim.add_scripted(1, |_: &crate::SeenFrame, _: &mut Vec<ScriptSend>| {});
            let payload = vec![0xA9; size];
            sim.run_until(ms(10));
            sim.script_send(
                1,
                ScriptSend::frame(Duration::ZERO, FrameType::TEST_REQUEST, 0, 1, &payload),
            );
            sim.run_until(ms(2_000));
            let response = sim
                .frames()
                .iter()
                .find(|frame| frame.is(FrameType::TEST_RESPONSE, 0, 1))
                .unwrap();
            assert_eq!(response.data, payload);
            assert!(sim.deliveries(0).is_empty());
            assert_eq!(sim.core(0).unwrap().counters().own_address_frames, 0);
            sim.assert_no_violations();
        }
    }
}

/// An overload is diagnosed while storage stays capped, even in collect-
/// violations mode. No growing backlog is hidden behind a successful read.
#[test]
fn overload_reports_failure_without_growing_buffers() {
    let mut config = SimConfig::new(38_400);
    config.panic_on_violation = false;
    let mut sim = Sim::new(config);
    sim.add_master(MasterSpec::new(0, 1, 38_400).host(HostSpec::default()));
    for _ in 0..4_112 {
        sim.host_character(0, Character::Clean(0x55));
    }
    for _ in 0..530 {
        sim.host_expect_frame(0, 0, true);
    }
    let host = sim.master_mut(0).unwrap().host.as_ref().unwrap();
    assert_eq!(host.input.len(), 4096);
    assert_eq!(host.expected.len(), 513);
    assert_eq!(host.stats.max_buffered, 4096);
    assert_eq!(sim.report().violations.len(), 33);
    assert!(sim
        .report()
        .violations
        .iter()
        .all(|v| v.kind == crate::ViolationKind::EventGrowth));
}

/// A previous application's delayed decision cannot answer a new core's
/// reused request identifier after reboot.
#[test]
fn old_application_decision_cannot_answer_a_new_request() {
    let mut sim = Sim::new(SimConfig::new(38_400));
    sim.add_master(
        MasterSpec::new(0, 1, 38_400)
            .host(HostSpec::default())
            .application(crate::ReplyPolicy::Answer {
                delay: Duration::from_millis(100),
                payload: vec![1, 0, 0xAD],
            }),
    );
    let request = ScriptSend::frame(
        Duration::ZERO,
        FrameType::BACNET_DATA_EXPECTING_REPLY,
        0,
        1,
        &[1, 4, 9],
    );
    raw_read(&mut sim, ms(0), &request.octets);
    sim.inject(ms(50), Fault::PowerOff { station: 0 });
    sim.inject(ms(60), Fault::PowerOn { station: 0 });
    raw_read(&mut sim, ms(70), &request.octets);
    sim.run_until(ms(100));
    assert!(sim.frames().is_empty());
    sim.run_until(ms(180));
    assert_eq!(sim.frames()[0].start, ms(170));
    sim.assert_no_violations();
}
