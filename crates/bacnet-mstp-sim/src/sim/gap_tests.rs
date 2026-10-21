//! Independent gap timing and octet-watchdog lifecycle evidence.

use bacnet_mstp_codec::FrameType;
use bacnet_mstp_core::{Duration, Instant};

use super::Sim;
use crate::{
    AdapterSpec, Driver, DriverReleaseCause, Fault, MasterSpec, ScriptSend, SimConfig, TraceKind,
    WireTiming,
};

fn us(value: u64) -> Instant {
    Instant::from_micros(value)
}

/// Give one actual core the token and an opaque maximum-size payload. Its
/// scripted peer stays quiet after the token, isolating transmit budgeting.
pub(super) fn sending(spec: MasterSpec, payload_len: usize) -> Sim {
    let mut sim = Sim::new(SimConfig::new(spec.config.baud_rate));
    sim.add_master(spec);
    sim.add_scripted(1, |_: &crate::SeenFrame, _: &mut Vec<ScriptSend>| {});
    sim.enqueue(us(0), 0, 1, &vec![0x11; payload_len]);
    sim.run_until(us(10_000));
    sim.script_send(
        1,
        ScriptSend::frame(Duration::ZERO, FrameType::TOKEN, 0, 1, &[]),
    );
    sim.run_until(us(40_000));
    assert!(sim.nodes[0].driving.is_some(), "{}", sim.dump(100));
    sim
}

pub(super) fn outgoing(sim: &Sim) -> (crate::line::TxId, Instant, Instant) {
    let tx = sim.nodes[0].driving.expect("data is on the wire");
    let owner = sim.line.owner(tx).unwrap();
    (tx, owner.start, owner.data_end)
}

fn gap_spec(baud: u32, extended: bool, budget: Duration) -> MasterSpec {
    let timing = WireTiming::new(baud);
    let mut spec = MasterSpec::new(0, 1, baud);
    if extended {
        spec = spec.extended_frames();
    }
    spec.adapter.inter_octet_gap = timing.max_inter_octet_gap();
    spec.adapter.post_drive = timing.max_post_drive();
    spec.config.transmit_watchdog = budget;
    spec.config
        .validate()
        .expect("an explicitly sufficient local budget");
    spec
}

/// Ten-bit characters and a 19-bit extra gap at 10 kbaud, with neither a
/// leading nor trailing gap. Each start comes from the transmission origin.
#[test]
fn gaps_follow_the_captured_character_schedule() {
    let mut sim = Sim::new(SimConfig::new(10_000));
    sim.add_scripted(1, |_: &crate::SeenFrame, _: &mut Vec<ScriptSend>| {});
    sim.script_send(
        0,
        ScriptSend {
            after: Duration::ZERO,
            inter_octet_gap: Duration::from_micros(1_900),
            post_drive: Duration::from_micros(1_400),
            octets: vec![0x55, 0xFF, 0xAA],
            malformed: true,
        },
    );
    for (end, quiet_until) in [(1_000, 2_899), (3_900, 5_799), (6_800, 8_199)] {
        sim.run_until(us(end));
        assert_eq!(sim.line.last_activity(), Some(us(end)));
        sim.run_until(us(quiet_until));
        assert_eq!(sim.line.last_activity(), Some(us(end)));
        assert!(sim.nodes[0].driving.is_some());
    }
    sim.run_until(us(8_200));
    let starts: Vec<_> = sim
        .trace
        .entries()
        .filter(|entry| matches!(entry.kind, TraceKind::Octet { .. }))
        .map(|entry| entry.at)
        .collect();
    assert_eq!(starts, [us(0), us(2_900), us(5_800)]);
    assert_eq!(sim.frames[0].end, us(6_800));
    assert_eq!(sim.report.driver_releases[0].at, us(8_200));
    assert_eq!(sim.line.last_activity(), Some(us(6_800)));
    assert!(!sim
        .trace
        .entries()
        .any(|entry| matches!(entry.kind, TraceKind::Garbled)));
}

/// A stuck octet adapter is not the host await policy. Remove the UART's
/// completion events to isolate whether the existing core watchdog runs.
#[test]
fn octet_watchdog_is_serviced_while_a_transmission_is_outstanding() {
    let mut sim = Sim::new(SimConfig::new(38_400));
    let spec = MasterSpec::new(0, 1, 38_400);
    let budget = spec.config.transmit_watchdog;
    sim.add_master(spec);
    sim.run_until(us(500_000));
    assert!(sim.master_mut(0).unwrap().transmitting.is_some());
    sim.dynamic = crate::event::Queue::new();
    sim.run_until(us(500_000) + budget);
    assert_eq!(sim.core(0).unwrap().counters().transmit_timeouts, 1);
    assert_eq!(sim.nodes[0].driving, None);
    assert!(sim.core(0).is_some());
    assert_eq!(
        sim.report.driver_releases[0].cause,
        DriverReleaseCause::Watchdog
    );
    assert_eq!(sim.report.driver_releases[0].data_end, None);
    sim.assert_no_violations();
}

/// Actual 511- and 1516-wire-octet frames, not maximum storage estimates,
/// complete at independently calculated F with an explicit adequate budget.
#[test]
fn maximum_frames_complete_with_an_explicit_schedule_budget() {
    for (baud, standard_ns, extended_ns) in [
        (9_600, 1_541_666_660, 4_577_603_490),
        (38_400, 385_416_410, 1_144_400_365),
        (115_200, 128_472_300, 381_466_950),
    ] {
        for (length, wire_len, duration_ns) in [(501, 511, standard_ns), (1497, 1516, extended_ns)]
        {
            let duration = Duration::from_nanos(duration_ns);
            let spec = gap_spec(baud, length > 501, duration + Duration::from_millis(10));
            let mut sim = sending(spec, length);
            let (tx, start, end) = outgoing(&sim);
            assert_eq!(sim.line.get(tx).unwrap().octets.len(), wire_len);
            assert_eq!(end, start + duration);
            sim.run_until(end);
            let frame = sim.frames.last().unwrap();
            assert_eq!(frame.octets.len(), wire_len);
            assert_eq!(frame.start, start);
            assert_eq!(frame.end, end);
            assert!(frame.is_clean());
            assert_eq!(frame.data, vec![0x11; length]);
            assert_eq!(sim.nodes[0].driving, Some(tx));
            assert_eq!(sim.core(0).unwrap().counters().transmit_timeouts, 0);
            let release = end + sim.timing.max_post_drive();
            sim.run_until(release);
            let record = sim.report.driver_releases.last().unwrap();
            assert_eq!(record.at, release);
            assert_eq!(record.data_end, Some(end));
            assert_eq!(record.cause, DriverReleaseCause::Completed);
            sim.assert_no_violations();
        }
    }
}

/// The valid default standard-frame budget at 9600 is too small for maximum
/// gaps. Its timeout stops this operation once and leaves the powered core
/// able to accept a later token and transmit a fresh short frame.
#[test]
fn valid_but_insufficient_budget_aborts_once_and_recovers_without_reboot() {
    let mut spec = MasterSpec::new(0, 1, 9_600);
    spec.adapter.inter_octet_gap = WireTiming::new(9_600).max_inter_octet_gap();
    let budget = spec.config.transmit_watchdog;
    spec.config.validate().unwrap();
    let mut sim = sending(spec, 501);
    let (tx, start, end) = outgoing(&sim);
    let incarnation = sim.master_mut(0).unwrap().incarnation;
    let timeout = start + budget;
    assert!(timeout < end);
    sim.run_until(timeout);
    assert_eq!(sim.core(0).unwrap().counters().transmit_timeouts, 1);
    // Failure counters are disjoint by reason, not a cumulative total.
    assert_eq!(sim.core(0).unwrap().counters().transmit_aborts, 0);
    assert_eq!(sim.nodes[0].driving, None);
    assert_eq!(sim.nodes[0].listening_since, Some(timeout));
    assert!(sim.line.get(tx).is_none());
    assert!(sim.line.owner(tx).is_none());
    assert!(!sim.meta.contains_key(&tx));
    let record = sim.report.driver_releases.last().unwrap();
    assert_eq!(record.cause, DriverReleaseCause::Watchdog);
    assert_eq!(record.at, timeout);
    assert_eq!(record.data_end, None);
    assert_eq!(sim.frames.last().unwrap().end, timeout);
    assert!(sim.frames.last().unwrap().truncated);

    // Consume stale physical work before the core can generate its own next
    // token, then deliver a fresh token after changing only the gap profile.
    sim.run_until(timeout + Duration::from_millis(10));
    sim.apply_fault(Fault::SetAdapter {
        station: 0,
        adapter: AdapterSpec::default(),
    });
    sim.enqueue(sim.now, 0, 1, &[1, 0, 0x42]);
    sim.script_send(
        1,
        ScriptSend::frame(Duration::ZERO, FrameType::TOKEN, 0, 1, &[]),
    );
    sim.run_for(Duration::from_millis(60));
    assert_eq!(sim.master_mut(0).unwrap().incarnation, incarnation);
    assert_eq!(sim.core(0).unwrap().counters().transmit_timeouts, 1);
    assert_eq!(sim.frames.iter().filter(|frame| frame.truncated).count(), 1);
    assert!(sim
        .frames
        .iter()
        .any(|frame| frame.is_clean() && frame.data == [1, 0, 0x42]));
    assert_eq!(
        sim.report
            .driver_releases
            .iter()
            .filter(|r| r.cause == DriverReleaseCause::Watchdog)
            .count(),
        1
    );
    sim.assert_no_violations();
}

/// At 38400, character 200 ends at 151302200 ns, character 201 starts
/// at 151796991 ns, and the final character spans 385156410..385416410 ns.
#[test]
fn watchdog_aborts_in_a_gap_or_final_character_and_wins_an_octet_start_tie() {
    for (budget_ns, started, fragment) in [
        (151_500_000, 201, false),
        (151_796_991, 201, false),
        (385_300_000, 511, true),
    ] {
        let spec = gap_spec(38_400, false, Duration::from_nanos(budget_ns));
        let mut sim = sending(spec, 501);
        let (tx, start, end) = outgoing(&sim);
        let timeout = start + Duration::from_nanos(budget_ns);
        sim.run_until(timeout);
        let octets = sim
            .trace
            .entries()
            .filter(|entry| {
                entry.who == crate::Who::Station(0) && matches!(entry.kind, TraceKind::Octet { .. })
            })
            .count();
        assert_eq!(octets, started);
        assert_eq!(sim.nodes[0].driving, None);
        assert!(sim.core(0).is_some());
        assert!(sim.line.owner(tx).is_none());
        assert_eq!(
            sim.report.driver_releases.last().unwrap().cause,
            DriverReleaseCause::Watchdog
        );
        sim.run_until(end + sim.timing.max_post_drive());
        assert_eq!(sim.core(0).unwrap().counters().transmit_timeouts, 1);
        let own: Vec<_> = sim
            .frames
            .iter()
            .filter(|frame| frame.driver == Driver::Station(0))
            .collect();
        assert_eq!(own.len(), 1);
        assert!(own[0].truncated);
        assert_eq!(own[0].end, timeout);
        let garbled = sim
            .trace
            .entries()
            .filter(|entry| matches!(entry.kind, TraceKind::Garbled))
            .count();
        assert_eq!(garbled, usize::from(fragment));
        assert_eq!(
            sim.trace
                .entries()
                .filter(|entry| entry.who == crate::Who::Station(0)
                    && matches!(entry.kind, TraceKind::Octet { .. }))
                .count(),
            started
        );
        assert_eq!(
            sim.line.last_activity(),
            Some(if fragment {
                end
            } else {
                start + Duration::from_nanos(151_302_200)
            })
        );
        sim.assert_no_violations();
    }
}

/// The final stop and normal completion run before a same-instant watchdog,
/// while the independently scheduled post-drive hold still survives.
#[test]
fn final_data_completion_wins_the_watchdog_tie() {
    let spec = gap_spec(38_400, false, Duration::from_nanos(385_416_410));
    let mut sim = sending(spec, 501);
    let (tx, _, end) = outgoing(&sim);
    sim.run_until(end);
    assert_eq!(sim.core(0).unwrap().counters().transmit_timeouts, 0);
    assert_eq!(sim.nodes[0].driving, Some(tx));
    assert!(sim.frames.last().unwrap().is_clean());
    sim.run_until(end + sim.timing.max_post_drive());
    assert_eq!(
        sim.report.driver_releases.last().unwrap().cause,
        DriverReleaseCause::Completed
    );
    sim.assert_no_violations();
}

/// Normal gap validation cannot be bypassed to exercise receive timeout.
/// Feed the existing receive seam at exact indication spacings instead.
#[test]
fn receive_abort_is_strictly_after_the_indication_timeout() {
    for (spacing_us, aborted) in [(6_000, 0), (6_001, 1)] {
        let mut sim = Sim::new(SimConfig::new(10_000));
        sim.add_master(MasterSpec::new(0, 1, 10_000));
        sim.advance(us(1_000));
        sim.master_input(0, super::master::Input::Octet(0x55))
            .unwrap();
        sim.advance(us(2_000));
        sim.master_input(0, super::master::Input::Octet(0xFF))
            .unwrap();
        sim.advance(us(2_000 + spacing_us));
        sim.master_input(0, super::master::Input::Octet(0x01))
            .unwrap();
        assert_eq!(sim.core(0).unwrap().counters().frame_aborts, aborted);
        sim.assert_no_violations();
    }
}

#[test]
fn zero_gap_keeps_contiguous_boundaries_and_no_trailing_delay() {
    assert_eq!(AdapterSpec::default().inter_octet_gap, Duration::ZERO);
    let mut sim = Sim::new(SimConfig::new(10_000));
    sim.add_scripted(1, |_: &crate::SeenFrame, _: &mut Vec<ScriptSend>| {});
    let mut send = ScriptSend::frame(Duration::ZERO, FrameType::TOKEN, 0, 1, &[]);
    assert_eq!(send.inter_octet_gap, Duration::ZERO);
    send.octets = vec![0x55, 0xFF, 0xAA];
    send.malformed = true;
    sim.script_send(0, send);
    sim.run_until(us(3_000));
    let starts: Vec<_> = sim
        .trace
        .entries()
        .filter(|entry| matches!(entry.kind, TraceKind::Octet { .. }))
        .map(|entry| entry.at)
        .collect();
    assert_eq!(starts, [us(0), us(1_000), us(2_000)]);
    assert_eq!(sim.frames[0].end, us(3_000));
    assert_eq!(sim.report.driver_releases[0].at, us(3_000));
}

#[test]
fn adapter_replacement_cannot_retime_captured_gaps_or_post_drive() {
    let mut sim = sending(gap_spec(38_400, false, Duration::from_secs(1)), 501);
    let (_, start, end) = outgoing(&sim);
    sim.apply_fault(Fault::SetAdapter {
        station: 0,
        adapter: AdapterSpec::default(),
    });
    sim.run_until(end + sim.timing.max_post_drive());
    let frame = sim
        .frames
        .iter()
        .find(|frame| frame.driver == Driver::Station(0))
        .unwrap();
    assert_eq!(frame.end, start + Duration::from_nanos(385_416_410));
    let release = sim.report.driver_releases.last().unwrap();
    assert_eq!(release.at, end + sim.timing.max_post_drive());
    assert_eq!(release.cause, DriverReleaseCause::Completed);
    sim.assert_no_violations();
}

fn raw_gap() -> Sim {
    let mut sim = Sim::new(SimConfig::new(10_000));
    sim.add_scripted(1, |_: &crate::SeenFrame, _: &mut Vec<ScriptSend>| {});
    sim.script_send(
        0,
        ScriptSend {
            after: Duration::ZERO,
            inter_octet_gap: Duration::from_micros(1_900),
            post_drive: Duration::from_micros(1_400),
            octets: vec![0x55, 0xFF, 0xAA],
            malformed: true,
        },
    );
    sim
}

/// The held idle driver owns every gap. Competing data wholly within that
/// gap is interference, but the gap alone does not synthesize characters.
#[test]
fn mid_gap_interference_sees_the_driver_without_phantom_windows() {
    let mut sim = raw_gap();
    sim.inject(us(1_500), Fault::Noise { octets: vec![0xAA] });
    sim.run_until(us(2_500));
    assert_eq!(sim.report.collisions.len(), 1);
    assert_eq!(sim.report.collisions[0].other, Driver::Station(1));
    assert_eq!(sim.report.collisions[0].post_drive, None);
    assert_eq!(sim.line.last_activity(), Some(us(2_500)));
    sim.run_until(us(2_899));
    assert_eq!(sim.line.last_activity(), Some(us(2_500)));
    sim.run_until(us(8_200));
    let held = sim
        .frames
        .iter()
        .find(|frame| frame.driver == Driver::Station(1))
        .unwrap();
    assert!(held.collided);
    assert_eq!(held.end, us(6_800));
    assert_eq!(sim.report.driver_releases.last().unwrap().at, us(8_200));
    assert_eq!(
        sim.trace
            .entries()
            .filter(|entry| matches!(entry.kind, TraceKind::Garbled))
            .count(),
        1
    );
    sim.assert_no_violations();
}

/// A queued next octet from an old transmission cannot enter a fresh one
/// after a power cut in the gap. The new frame has a new captured schedule.
#[test]
fn reboot_in_a_gap_fences_the_old_octet_and_end_events() {
    let mut sim = raw_gap();
    sim.run_until(us(1_500));
    sim.power_off(0);
    sim.power_on(0);
    sim.script_send(
        0,
        ScriptSend {
            after: Duration::ZERO,
            inter_octet_gap: Duration::ZERO,
            post_drive: Duration::ZERO,
            octets: vec![0xFF, 0xAA],
            malformed: true,
        },
    );
    sim.run_until(us(9_000));
    assert_eq!(sim.frames.len(), 2);
    assert!(sim.frames[0].truncated);
    assert_eq!(sim.frames[0].end, us(1_500));
    assert!(sim.frames[1].is_clean());
    assert_eq!(sim.frames[1].start, us(1_500));
    assert_eq!(sim.frames[1].end, us(3_500));
    assert_eq!(sim.report.driver_releases.len(), 2);
    assert_eq!(
        sim.report.driver_releases[0].cause,
        DriverReleaseCause::PowerLoss
    );
    assert_eq!(sim.report.driver_releases[0].data_end, None);
    assert_eq!(sim.line.last_activity(), Some(us(3_500)));
    assert!(sim.meta.is_empty());
    assert_eq!(sim.line.live_count(), 0);
    assert!(!sim
        .trace
        .entries()
        .any(|entry| matches!(entry.kind, TraceKind::Garbled)));
    sim.assert_no_violations();
}

#[test]
#[should_panic(expected = "inter-octet gap exceeds")]
fn unpowered_master_rejects_a_gap_one_nanosecond_over_the_fractional_limit() {
    let mut sim = Sim::new(SimConfig::new(115_200));
    let mut spec = MasterSpec::new(0, 1, 115_200).unpowered();
    spec.adapter.inter_octet_gap = Duration::from_nanos(164_931);
    sim.add_master(spec);
}

#[test]
#[should_panic(expected = "inter-octet gap exceeds")]
fn adapter_replacement_rejects_excessive_gap() {
    let mut sim = Sim::new(SimConfig::new(115_200));
    sim.add_master(MasterSpec::new(0, 1, 115_200));
    sim.apply_fault(Fault::SetAdapter {
        station: 0,
        adapter: AdapterSpec {
            inter_octet_gap: Duration::from_nanos(164_931),
            ..AdapterSpec::default()
        },
    });
}

#[test]
#[should_panic(expected = "inter-octet gap exceeds")]
fn scripted_transmission_rejects_excessive_gap() {
    let mut sim = Sim::new(SimConfig::new(10_000));
    sim.add_scripted(1, |_: &crate::SeenFrame, _: &mut Vec<ScriptSend>| {});
    sim.script_send(
        0,
        ScriptSend::frame(Duration::ZERO, FrameType::TOKEN, 0, 1, &[])
            .inter_octet_gap(Duration::from_nanos(1_900_001)),
    );
}
