//! Capture, FIFO, incarnation, capacity and new-write lifecycle boundaries.

use bacnet_mstp_codec::FrameType;
use bacnet_mstp_core::{Duration, Instant};

use super::delayed_echo_tests::{peer, request};
use super::local_echo::RETURN_BOUND;
use crate::{Fault, HostSpec, MasterSpec, ScriptSend, Sim, SimConfig, TraceKind, ViolationKind};

fn ms(value: u64) -> Instant {
    Instant::from_micros(value * 1000)
}

fn immediate_profile(sim: &mut Sim) {
    let mut adapter = sim.master_mut(0).unwrap().spec.adapter;
    let host = adapter.host.as_mut().unwrap();
    host.echo_return_delay = Duration::ZERO;
    host.drain_duration = Duration::ZERO;
    sim.apply_fault(Fault::SetAdapter {
        station: 0,
        adapter,
    });
}

#[test]
fn transmit_capture_survives_profile_change_completion_and_physical_release() {
    let mut sim = request(16, 110, 5);
    let tx = sim.nodes[0].driving.unwrap();
    sim.run_until(ms(34));
    immediate_profile(&mut sim);
    sim.run_until(ms(105));
    assert_eq!(sim.nodes[0].driving, None);
    assert!(sim.line.get(tx).is_none());
    assert!(sim.line.owner(tx).is_none());
    assert!(sim
        .master_mut(0)
        .unwrap()
        .host
        .as_ref()
        .unwrap()
        .write
        .is_none());
    let queue = &sim.master_mut(0).unwrap().host.as_ref().unwrap().local_echo;
    assert_eq!(queue.len(), 13);
    for (index, echo) in queue.iter().enumerate() {
        assert_eq!(echo.tx, tx);
        assert_eq!(echo.incarnation, 1);
        assert_eq!(echo.wire_end, ms(32 + index as u64));
        assert_eq!(echo.requested_at, ms(142 + index as u64));
        assert_eq!(echo.available_at, echo.requested_at);
    }
    assert_eq!(sim.report.local_echo.pending, 13);
    assert_eq!(sim.report.local_echo.max_pending, 13);
    sim.assert_no_violations();
}

#[test]
fn old_returns_survive_a_later_write_and_new_profiles_cannot_overtake_them() {
    let mut sim = request(16, 110, 5);
    let old = sim.nodes[0].driving.unwrap();
    sim.run_until(ms(34));
    immediate_profile(&mut sim);
    peer(&mut sim, ms(120), &[1, 0, 0xA1]);
    sim.run_until(ms(143));
    let new = sim.nodes[0]
        .driving
        .expect("new write starts without draining old returns");
    assert_ne!(old, new);
    assert_eq!(sim.line.owner(new).unwrap().start, ms(141));
    assert_eq!(sim.report.local_echo.released, 2);
    let queue = &sim.master_mut(0).unwrap().host.as_ref().unwrap().local_echo;
    assert!(queue.iter().any(|echo| echo.tx == old));
    assert!(queue.iter().any(|echo| echo.tx == new));
    assert!(queue
        .iter()
        .filter(|echo| echo.tx == new)
        .all(|echo| echo.requested_at < echo.available_at && echo.available_at == ms(154)));
    sim.run_until(ms(158));
    let returned: Vec<_> = sim
        .trace
        .entries()
        .filter_map(|entry| match entry.kind {
            TraceKind::LocalEcho {
                transmission,
                requested_at,
                ..
            } => Some((transmission, requested_at, entry.at)),
            _ => None,
        })
        .collect();
    assert_eq!(returned.len(), 21);
    assert!(returned[..13].iter().all(|(tx, _, _)| *tx == old));
    assert!(returned[13..]
        .iter()
        .all(|(tx, requested, actual)| *tx == new && *actual == ms(154) && requested < actual));
    assert_eq!(sim.report.local_echo.pending, 0);
    assert_eq!(sim.report.local_echo.cancelled, 0);
    // Same-incarnation old bytes reach the real content heuristic. A new
    // confirmed start is not authority to discard those unread bytes.
    assert_eq!(sim.core(0).unwrap().counters().own_address_frames, 1);
    assert!(sim
        .report
        .violations
        .iter()
        .any(|v| v.kind == ViolationKind::CoreCounter));
}

#[test]
fn local_availability_precedes_a_same_time_new_transmit_start() {
    let mut sim = request(16, 109, 5);
    let old = sim.nodes[0].driving.unwrap();
    sim.run_until(ms(34));
    immediate_profile(&mut sim);
    peer(&mut sim, ms(120), &[1, 0, 0xA1]);
    sim.run_until(ms(141));
    assert_ne!(sim.nodes[0].driving.unwrap(), old);
    assert_eq!(
        sim.master_mut(0)
            .unwrap()
            .host
            .as_ref()
            .unwrap()
            .input
            .len(),
        1
    );
    let at: Vec<_> = sim
        .trace
        .entries()
        .filter(|entry| entry.at == ms(141))
        .collect();
    let returned = at
        .iter()
        .position(|entry| matches!(entry.kind, TraceKind::LocalEcho { .. }))
        .unwrap();
    let started = at
        .iter()
        .position(|entry| matches!(entry.kind, TraceKind::FrameStart(_)))
        .unwrap();
    assert!(returned < started);
    sim.assert_no_violations();
}

#[test]
fn power_loss_wins_a_return_tie_and_reboot_cannot_observe_old_data() {
    let mut sim = request(16, 61, 5);
    sim.inject(ms(93), Fault::PowerOff { station: 0 });
    sim.inject(ms(93), Fault::PowerOn { station: 0 });
    sim.run_until(ms(109));
    assert_eq!(sim.master_mut(0).unwrap().incarnation, 2);
    assert_eq!(sim.report.local_echo.cancelled, 13);
    assert_eq!(sim.report.local_echo.released, 0);
    assert_eq!(sim.report.local_echo.pending, 0);
    assert_eq!(sim.host_stats(0).unwrap().reads, 0);
    assert_eq!(sim.host_stats(0).unwrap().last_completion, None);
    immediate_profile(&mut sim);
    sim.enqueue(ms(110), 0, 1, &[1, 4, 0xC2]);
    sim.run_until(ms(110));
    sim.script_send(
        1,
        ScriptSend::frame(Duration::ZERO, FrameType::TOKEN, 0, 1, &[]),
    );
    sim.run_until(ms(149));
    assert_eq!(sim.report.local_echo.released, 13);
    assert_eq!(sim.report.local_echo.cancelled, 13);
    assert_eq!(sim.report.local_echo.pending, 0);
    assert!(sim
        .trace
        .entries()
        .filter_map(|entry| match entry.kind {
            TraceKind::LocalEcho { incarnation, .. } => Some(incarnation),
            _ => None,
        })
        .all(|incarnation| incarnation == 2));
    assert_eq!(sim.report.host_transmits.last().unwrap().incarnation, 2);
    assert_eq!(sim.core(0).unwrap().counters().own_address_frames, 0);
    sim.assert_no_violations();
}

/// An overloaded positive wire workload reports local capacity failure once
/// per episode, counts every refusal, and can reboot into an ordinary adapter.
#[test]
fn local_return_capacity_is_bounded_and_power_cycle_recovers() {
    let mut config = SimConfig::new(38_400);
    config.panic_on_violation = false;
    let mut sim = Sim::new(config);
    let mut spec = MasterSpec::new(0, 1, 38_400)
        .extended_frames()
        .host(HostSpec {
            echo_return_delay: Duration::from_secs(10),
            echo_grace: Some(Duration::from_millis(5)),
            ..HostSpec::default()
        });
    spec.adapter.echo = true;
    sim.add_master(spec);
    sim.add_master(MasterSpec::new(1, 1, 38_400).extended_frames());
    for index in 0..5 {
        let mut payload = vec![0x11; 1497];
        payload[0] = 1;
        payload[1] = 0;
        payload[2] = index;
        sim.enqueue(ms(800), 0, 1, &payload);
    }
    sim.run_until(ms(4000));
    assert_eq!(sim.deliveries(1).len(), 5);
    assert_eq!(sim.report.local_echo.pending, RETURN_BOUND);
    assert_eq!(sim.report.local_echo.max_pending, RETURN_BOUND);
    assert!(sim.report.local_echo.overflows > 0);
    assert_eq!(sim.report.violations.len(), 1, "{}", sim.dump(60));
    assert_eq!(sim.report.violations[0].kind, ViolationKind::EventGrowth);
    assert!(sim.report.max_pending_events <= 12);
    sim.power_off(0);
    assert_eq!(sim.report.local_echo.pending, 0);
    assert_eq!(sim.report.local_echo.cancelled, RETURN_BOUND as u64);
    immediate_profile(&mut sim);
    sim.power_on(0);
    sim.enqueue(ms(4500), 0, 1, &[1, 0, 0xF1]);
    sim.run_until(ms(12000));
    assert!(sim
        .deliveries(1)
        .iter()
        .any(|delivery| delivery.data == [1, 0, 0xF1]));
    assert_eq!(sim.report.violations.len(), 1, "{}", sim.dump(60));
    assert_eq!(sim.core(0).unwrap().counters().own_address_frames, 0);
    assert_eq!(sim.report.local_echo.max_pending, RETURN_BOUND);
    assert_eq!(
        sim.report.local_echo.captured,
        sim.report.local_echo.released
            + sim.report.local_echo.cancelled
            + sim.report.local_echo.pending as u64
    );
}
