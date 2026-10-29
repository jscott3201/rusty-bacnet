//! Lifecycle and reference-receipt qualification under real contention.
use super::contention_tests::{bus, capture, frame, seeded_bus, send};
use crate::sim::master::Input;
use crate::{ContentionPolicy, Fault, TraceKind, ViolationKind, Who};
use bacnet_mstp_core::{Duration, Instant};

fn us(value: u64) -> Instant {
    Instant::from_micros(value)
}

#[test]
fn off_reboot_and_mid_character_enable_do_not_inherit_a_winning_stream() {
    for (off, on) in [(0, 500), (4_000, 4_500), (14_000, 15_000)] {
        let mut sim = bus(10_000, true, false);
        capture(&mut sim, 10);
        let node = sim.index_of(10);
        sim.master_mut(node)
            .unwrap()
            .spec
            .adapter
            .host
            .as_mut()
            .unwrap()
            .receive_latency = Duration::from_millis(16);
        if off == 0 {
            sim.power_off(node);
        } else {
            sim.inject(us(off), Fault::PowerOff { station: 10 });
        }
        sim.inject(us(on), Fault::PowerOn { station: 10 });
        send(&mut sim, 1, Instant::ZERO, frame(1, false, &[1, 0, 0xA1]));
        send(&mut sim, 2, Instant::ZERO, frame(2, false, &[1, 0, 0xB2]));
        send(&mut sim, 3, us(40_000), frame(3, false, &[1, 0, 0xC3]));
        sim.run_until(us(80_000));
        assert_eq!(
            sim.deliveries(10)
                .iter()
                .map(|d| d.data.clone())
                .collect::<Vec<_>>(),
            [vec![1, 0, 0xC3]],
            "off {off}"
        );
        assert_eq!(sim.contention_stats(10).pending, 0);
        assert_eq!(sim.contention_stats(10).intact, u64::from(off == 14_000));
        assert_eq!(sim.contention_stats(10).cancelled, u64::from(off == 14_000));
        assert_eq!(sim.contention_stats(10).checked, 0);
        if on == 500 {
            let first = sim
                .trace()
                .entries()
                .find(|e| {
                    e.who == Who::Station(10) && matches!(e.kind, TraceKind::Contention { .. })
                })
                .unwrap();
            assert!(matches!(
                first.kind,
                TraceKind::Contention { byte: None, .. }
            ));
            assert_eq!(first.at, us(1_000));
        }
        sim.assert_no_violations();
    }
}

#[test]
fn terminal_counter_check_detects_core_loss_before_later_valid_input() {
    let mut sim = bus(10_000, false, false);
    capture(&mut sim, 10);
    send(&mut sim, 1, Instant::ZERO, frame(1, false, &[1, 0, 0xA1]));
    send(&mut sim, 2, Instant::ZERO, frame(2, false, &[1, 0, 0xB2]));
    sim.run_until(us(12_000));
    // Fault the real parser alone after all but the final CRC byte. The
    // independent forwarded-stream decoder remains intact and catches it.
    let node = sim.index_of(10);
    sim.master_input(node, Input::Garbled).unwrap();
    send(&mut sim, 3, us(20_000), frame(3, false, &[1, 0, 0xC3]));
    sim.run_until(us(40_000));
    assert_eq!(sim.core(10).unwrap().counters().frames_received, 1);
    assert_eq!(
        sim.deliveries(10)
            .iter()
            .map(|d| d.data.clone())
            .collect::<Vec<_>>(),
        [vec![1, 0, 0xC3]]
    );
    assert_eq!(sim.contention_stats(10).checked, 1);
    assert_eq!(sim.contention_stats(10).matched, 0);
    let missed: Vec<_> = sim
        .report()
        .violations
        .iter()
        .filter(|v| v.kind == ViolationKind::ListenerMissedFrame)
        .collect();
    assert_eq!(missed.len(), 1);
    assert!(
        missed[0]
            .detail
            .contains("terminal input counted 0, expected 1")
    );
    assert_eq!(missed[0].at, us(13_000));
}

#[test]
fn own_source_after_filtering_is_still_a_receipt_and_a_diagnostic() {
    let mut sim = bus(10_000, false, false);
    capture(&mut sim, 10);
    send(&mut sim, 1, Instant::ZERO, frame(10, false, &[1, 0, 0xA1]));
    send(&mut sim, 2, Instant::ZERO, frame(2, false, &[1, 0, 0xB2]));
    sim.run_until(us(20_000));
    assert_eq!(sim.core(10).unwrap().counters().frames_received, 1);
    assert_eq!(sim.core(10).unwrap().counters().own_address_frames, 1);
    assert_eq!(sim.contention_stats(10).matched, 1);
    assert!(
        sim.report()
            .violations
            .iter()
            .any(|v| v.kind == ViolationKind::CoreCounter)
    );
    assert!(
        !sim.report()
            .violations
            .iter()
            .any(|v| v.kind == ViolationKind::ListenerMissedFrame)
    );
}

fn replay(
    seed: u64,
    extra_draws: bool,
    other_receiver: bool,
) -> (Vec<(Instant, TraceKind)>, crate::ContentionStats) {
    let mut sim = seeded_bus(115_200, false, false, seed);
    sim.set_contention(10, ContentionPolicy::Seeded);
    if other_receiver {
        sim.set_contention(11, ContentionPolicy::Seeded);
    }
    if extra_draws {
        sim.rng.bytes(100);
        sim.jitter.bytes(100);
        sim.chunks.bytes(100);
    }
    let first = frame(1, false, &[0x77; 200]);
    send(&mut sim, 1, Instant::ZERO, first);
    send(&mut sim, 2, Instant::ZERO, frame(2, false, &[0x88; 200]));
    sim.run_until(us(30_000));
    let observations = sim
        .trace()
        .entries()
        .filter(|e| e.who == Who::Station(10) && matches!(e.kind, TraceKind::Contention { .. }))
        .map(|e| (e.at, e.kind.clone()))
        .collect();
    assert_eq!(sim.contention_stats(10).windows, 210);
    assert!(
        sim.contention_stats(10).captured > 0
            && sim.contention_stats(10).altered > 0
            && sim.contention_stats(10).framing_errors > 0
    );
    assert!(sim.report().max_pending_events <= 12);
    assert_eq!(sim.contention_stats(10).max_pending, 0);
    assert!(sim.deliveries(10).is_empty());
    assert_eq!(sim.core(10).unwrap().counters().frames_received, 0);
    assert_eq!(sim.contention_stats(10).decoded, 0);
    sim.assert_no_violations();
    (observations, *sim.contention_stats(10))
}

#[test]
fn seeded_observations_replay_independently_of_other_random_streams_and_receivers() {
    for seed in 0..8 {
        let expected = replay(seed, false, false);
        assert_eq!(replay(seed, false, false), expected);
        assert_eq!(replay(seed, true, true), expected);
    }
    assert_ne!(replay(1, false, false).0, replay(2, false, false).0);
}

/// The local return belongs to the transmitting adapter, but its bytes are
/// the selected receiver observation, even when another contributor won.
#[test]
fn delayed_own_echo_carries_the_actual_receiver_specific_stream() {
    let mut sim = super::delayed_echo_tests::request(16, 61, 5);
    sim.config.tolerance.scripted_collisions = true;
    sim.add_scripted(2, |_: &crate::SeenFrame, _: &mut Vec<crate::ScriptSend>| {});
    sim.set_contention(0, ContentionPolicy::Capture(crate::Driver::Station(2)));
    let response = crate::ScriptSend::frame(
        Duration::ZERO,
        bacnet_mstp_codec::FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
        0,
        1,
        &[1, 0, 0xD2],
    );
    let actual = response.octets.clone();
    let node = sim.index_of(2);
    sim.script_send(node, response);
    sim.run_until(us(92_000));
    assert_eq!(sim.report().local_echo.captured, 13);
    assert_eq!(sim.report().local_echo.released, 0);
    assert_eq!(sim.report().local_echo.pending, 13);
    sim.run_until(us(109_000));
    let returns: Vec<_> = sim
        .trace()
        .entries()
        .filter_map(|e| match e.kind {
            TraceKind::LocalEcho { wire_end, byte, .. } if e.who == Who::Station(0) => {
                Some((e.at, wire_end, byte))
            }
            _ => None,
        })
        .collect();
    assert_eq!(returns.len(), 13);
    for (index, ((at, wire_end, byte), actual)) in returns.into_iter().zip(actual).enumerate() {
        assert_eq!(wire_end, us(32_000 + index as u64 * 1_000));
        assert_eq!(at, wire_end + Duration::from_millis(61));
        assert_eq!(byte, Some(actual));
    }
    assert_eq!(sim.host_stats(0).unwrap().echo_missing, 1);
    assert_eq!(sim.core(0).unwrap().counters().own_address_frames, 0);
    assert_eq!(
        sim.deliveries(0)
            .iter()
            .map(|d| d.data.clone())
            .collect::<Vec<_>>(),
        [vec![1, 0, 0xD2]]
    );
    assert_eq!(sim.contention_stats(0).intact, 0); // Echo eligibility is not a listening receipt.
    assert_eq!(sim.report().local_echo.pending, 0);
    let timing = &sim.report().host_transmits[0];
    assert_eq!(
        (
            timing.start,
            timing.data_end,
            timing.completion,
            timing.notification
        ),
        (us(31_000), us(44_000), us(100_000), us(105_000))
    );
    assert_eq!(sim.frames().iter().filter(|f| f.collided).count(), 2);
    sim.assert_no_violations();
}

#[test]
fn a_complete_filter_match_is_an_explicit_disposition_not_a_missing_receipt() {
    let mut sim = bus(10_000, true, false);
    capture(&mut sim, 10);
    let first = frame(1, false, &[1, 0, 0xA1]);
    let node = sim.index_of(10);
    // Isolate the real filter's complete-match disposition. Provenance must
    // neither force these bytes through nor report its suppression as loss.
    sim.master_mut(node)
        .unwrap()
        .host
        .as_mut()
        .unwrap()
        .filter
        .start(&first.octets, Instant::ZERO, Duration::from_millis(50));
    send(&mut sim, 1, Instant::ZERO, first);
    send(&mut sim, 2, Instant::ZERO, frame(2, false, &[1, 0, 0xB2]));
    sim.run_until(us(20_000));
    assert!(sim.deliveries(10).is_empty());
    assert_eq!(sim.contention_stats(10).intact, 1);
    assert_eq!(sim.contention_stats(10).cancelled, 1);
    assert_eq!(sim.contention_stats(10).pending, 0);
    assert_eq!(sim.contention_stats(10).checked, 0);
    sim.assert_no_violations();
}

/// Real bus traffic can exceed a stalled host's input/audit bounds. Both
/// failures stay explicit; a power cycle owns cancellation and recovery.
#[test]
fn stalled_host_bounds_receipts_and_reboot_cancels_them() {
    let mut config = crate::SimConfig::new(10_000);
    config.panic_on_violation = false;
    config.trace_capacity = 64;
    config.tolerance.scripted_collisions = true;
    let mut sim = crate::Sim::new(config);
    let mut spec = crate::MasterSpec::new(0, 2, 10_000).host(crate::HostSpec {
        drain_duration: Duration::from_secs(10),
        ..crate::HostSpec::default()
    });
    spec.config.transmit_watchdog = Duration::from_secs(11);
    sim.add_master(spec);
    for station in [1, 2] {
        sim.add_scripted(
            station,
            |_: &crate::SeenFrame, _: &mut Vec<crate::ScriptSend>| {},
        );
    }
    sim.set_contention(0, ContentionPolicy::Capture(crate::Driver::Station(1)));
    // The host's actual token-generation poll starts at 500 ms and releases
    // its physical driver at 508 ms; its serial drain returns at 10.5 s.
    sim.run_until(us(510_000));
    assert_eq!(sim.frames()[0].end, us(508_000));
    for index in 0..520 {
        let at = us(520_000 + index * 16_000);
        send(&mut sim, 1, at, frame(1, false, &[1, 0, 0xA1]));
        send(&mut sim, 2, at, frame(2, false, &[1, 0, 0xB2]));
    }
    sim.run_until(us(8_900_000));
    assert_eq!(sim.contention_stats(0).intact, 520);
    assert_eq!(sim.contention_stats(0).pending, 513);
    assert_eq!(sim.contention_stats(0).max_pending, 513);
    assert_eq!(sim.contention_stats(0).overflows, 7);
    assert_eq!(sim.host_stats(0).unwrap().max_buffered, 4096);
    assert_eq!(
        sim.report()
            .violations
            .iter()
            .filter(|v| v.detail == "receiver receipts exceeded 513 candidates")
            .count(),
        1
    );
    assert!(
        sim.report()
            .violations
            .iter()
            .all(|v| v.kind == ViolationKind::EventGrowth)
    );
    assert!(sim.report().max_pending_events <= 12);
    assert_eq!(sim.trace().entries().count(), 64);
    let failures = sim.report().violations.len();
    sim.power_off(0);
    assert_eq!(sim.contention_stats(0).pending, 0);
    assert_eq!(sim.contention_stats(0).cancelled, 513);
    sim.master_mut(0)
        .unwrap()
        .spec
        .adapter
        .host
        .as_mut()
        .unwrap()
        .drain_duration = Duration::ZERO;
    sim.power_on(0);
    send(&mut sim, 1, us(8_910_000), frame(1, false, &[1, 0, 0xC3]));
    sim.run_until(us(8_940_000));
    assert_eq!(
        sim.deliveries(0)
            .iter()
            .map(|d| d.data.clone())
            .collect::<Vec<_>>(),
        [vec![1, 0, 0xC3]]
    );
    assert_eq!(sim.report().violations.len(), failures);
}

#[test]
fn enabling_policy_mid_candidate_waits_for_a_known_parser_reset() {
    let mut sim = bus(10_000, false, false);
    let mut prefix = frame(3, false, &[0x66; 80]);
    prefix.octets.truncate(8);
    prefix.malformed = true;
    send(&mut sim, 3, Instant::ZERO, prefix);
    sim.run_until(us(8_000));
    capture(&mut sim, 10);
    send(&mut sim, 1, us(8_000), frame(1, false, &[1, 0, 0xA1]));
    send(&mut sim, 2, us(8_000), frame(2, false, &[1, 0, 0xB2]));
    sim.run_until(us(25_000));
    assert_eq!(sim.contention_stats(10).decoded, 1);
    assert_eq!(sim.contention_stats(10).unqualified, 1);
    assert_eq!(sim.contention_stats(10).checked, 0);
    assert!(sim.deliveries(10).is_empty());
    // A gap longer than the receiver's frame-abort interval synchronizes
    // both parsers; a later intact winner qualifies normally.
    send(&mut sim, 1, us(40_000), frame(1, false, &[1, 0, 0xD4]));
    send(&mut sim, 2, us(40_000), frame(2, false, &[1, 0, 0xE5]));
    sim.run_until(us(60_000));
    assert_eq!(sim.contention_stats(10).checked, 1);
    assert_eq!(sim.contention_stats(10).matched, 1);
    assert_eq!(
        sim.deliveries(10)
            .iter()
            .map(|d| d.data.clone())
            .collect::<Vec<_>>(),
        [vec![1, 0, 0xD4]]
    );
    sim.assert_no_violations();
}
