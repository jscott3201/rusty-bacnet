//! Real physical self-observation reaches the existing host adapter later.

use bacnet_mstp_core::{Duration, Instant};

use bacnet_mstp_codec::FrameType;

use crate::{
    Corruption, Fault, HostSpec, MasterSpec, ScriptSend, Sim, SimConfig, TraceKind, ViolationKind,
};

fn ms(value: u64) -> Instant {
    Instant::from_micros(value * 1000)
}

fn host(delay: Duration, grace: Duration) -> Sim {
    let mut sim = Sim::new(SimConfig::new(38_400));
    let mut spec = MasterSpec::new(0, 1, 38_400).host(HostSpec {
        echo_return_delay: delay,
        echo_grace: Some(grace),
        ..HostSpec::default()
    });
    spec.adapter.echo = true;
    sim.add_master(spec);
    sim
}

/// A real token causes a 13-octet data-expecting-reply write. With 16 ms
/// receive latency its S=31 ms, F=44 ms and a 69 ms drain fixes C=100 ms.
/// The quiet peer leaves the core waiting for a reply beyond echo handling.
pub(super) fn request(latency_ms: u64, delay_ms: u64, grace_ms: u64) -> Sim {
    let mut config = SimConfig::new(10_000);
    config.panic_on_violation = false; // Some callers retain expired-copy diagnostics.
    let mut sim = Sim::new(config);
    let mut spec = MasterSpec::new(0, 1, 10_000).host(HostSpec {
        receive_latency: Duration::from_millis(latency_ms),
        echo_return_delay: Duration::from_millis(delay_ms),
        echo_grace: Some(Duration::from_millis(grace_ms)),
        drain_duration: Duration::from_millis(if latency_ms == 16 { 69 } else { 78 }),
        ..HostSpec::default()
    });
    spec.adapter.echo = true;
    sim.add_master(spec);
    sim.add_scripted(1, |_: &crate::SeenFrame, _: &mut Vec<ScriptSend>| {});
    sim.enqueue(ms(0), 0, 1, &[1, 4, 0xC1]);
    sim.run_until(ms(10));
    sim.script_send(
        1,
        ScriptSend::frame(Duration::ZERO, FrameType::TOKEN, 0, 1, &[]),
    );
    sim.run_until(ms(if latency_ms == 16 { 31 } else { 22 }));
    assert!(sim.nodes[0].driving.is_some(), "{}", sim.dump(30));
    sim
}

pub(super) fn peer(sim: &mut Sim, at: Instant, payload: &[u8]) {
    sim.run_until(at);
    sim.script_send(
        1,
        ScriptSend::frame(
            Duration::ZERO,
            FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
            0,
            1,
            payload,
        ),
    );
}

#[test]
fn local_echo_waits_before_entering_the_host_read_buffer() {
    let mut sim = host(Duration::from_millis(30), Duration::from_millis(5));
    sim.run_until(ms(501));
    assert!(sim.line.last_activity().is_some());
    assert_eq!(
        sim.master_mut(0)
            .unwrap()
            .host
            .as_ref()
            .unwrap()
            .input
            .len(),
        0
    );
    assert_eq!(sim.host_stats(0).unwrap().characters, 0);
    assert_eq!(sim.report.local_echo.pending, 3);
    let first_return = ms(530) + sim.timing.octet();
    sim.run_until(first_return);
    let wire_end = sim
        .master_mut(0)
        .unwrap()
        .host
        .as_ref()
        .unwrap()
        .input
        .front()
        .unwrap()
        .wire_end;
    assert_eq!(wire_end, ms(500) + sim.timing.octet());
    assert_eq!(sim.host_stats(0).unwrap().characters, 0);
    assert_eq!(
        sim.master_mut(0).unwrap().host.as_ref().unwrap().ready,
        Some(first_return + Duration::from_millis(1))
    );
}

#[test]
fn late_local_echo_changes_notification_without_moving_physical_completion() {
    let mut sim = host(Duration::from_millis(30), Duration::from_millis(5));
    let end = sim.timing.end_of(ms(500), 8);
    sim.run_until(end + Duration::from_millis(5));
    assert_eq!(sim.host_stats(0).unwrap().echo_missing, 1);
    assert_eq!(sim.frames[0].end, end);
    assert_eq!(sim.report.driver_releases[0].at, end);
    assert_eq!(sim.report.host_transmits[0].completion, end);
    assert_eq!(
        sim.report.host_transmits[0].notification,
        end + Duration::from_millis(5)
    );
    assert_eq!(sim.line.last_activity(), Some(end));
    sim.assert_no_violations();
}

/// Independent C=100/g=5 oracle: the ordinary read arrives at 109 or 110.
/// At 109 a complete late copy is still hunted. At 110 the receive path
/// expires Probe before inspecting its bytes, retaining the negative control.
#[test]
fn real_delayed_copy_inside_probe_is_dropped_but_expiry_equality_is_not() {
    for (delay, read, own) in [(61, 109, 0), (62, 110, 1)] {
        let mut sim = request(16, delay, 5);
        sim.run_until(ms(105));
        let timing = sim.report.host_transmits[0];
        assert_eq!(timing.start, ms(31));
        assert_eq!(timing.data_end, ms(44));
        assert_eq!(timing.completion, ms(100));
        assert_eq!(timing.notification, ms(105));
        assert_eq!(sim.host_stats(0).unwrap().echo_missing, 1);
        assert!(sim.report.local_echo.pending > 0 || sim.report.local_echo.released > 0);
        sim.run_until(ms(read));
        assert_eq!(sim.core(0).unwrap().counters().own_address_frames, own);
        assert_eq!(sim.report.local_echo.released, 13);
        assert_eq!(sim.report.local_echo.pending, 0);
        assert_eq!(sim.host_stats(0).unwrap().last_completion, Some(ms(100)));
        if own == 0 {
            sim.assert_no_violations();
        } else {
            assert!(sim
                .report
                .violations
                .iter()
                .any(|v| v.kind == ViolationKind::CoreCounter));
        }
    }
}

/// A complete chunk ready at Grace equality is inspected before the timer.
/// Availability itself does not bypass the ordinary receive-latency clock.
#[test]
fn ready_whole_echo_wins_grace_equality_but_later_read_counts_missing() {
    for (delay, notification, missing) in [(57, 105, 0), (58, 105, 1)] {
        let mut sim = request(16, delay, 5);
        sim.run_until(ms(106));
        assert_eq!(sim.report.host_transmits[0].notification, ms(notification));
        assert_eq!(sim.report.host_transmits[0].completion, ms(100));
        assert_eq!(sim.host_stats(0).unwrap().echo_missing, missing);
        assert_eq!(sim.core(0).unwrap().counters().own_address_frames, 0);
        sim.assert_no_violations();
    }
}

/// The first two bytes arrive in Grace, the rest through Residual after N
/// and beyond C+2g. Residual is not a second universal grace deadline.
#[test]
fn a_real_prefix_spanning_grace_keeps_its_residual_tail() {
    let mut sim = request(1, 80, 5);
    sim.run_until(ms(104));
    assert_eq!(
        sim.master_mut(0)
            .unwrap()
            .host
            .as_ref()
            .unwrap()
            .held_at
            .len(),
        2
    );
    sim.run_until(ms(105));
    assert_eq!(sim.report.host_transmits[0].completion, ms(100));
    assert_eq!(sim.report.host_transmits[0].notification, ms(105));
    assert_eq!(sim.host_stats(0).unwrap().echo_missing, 0);
    assert!(sim.report.local_echo.pending > 0);
    sim.run_until(ms(116));
    assert_eq!(sim.report.local_echo.released, 13);
    assert_eq!(
        sim.master_mut(0)
            .unwrap()
            .host
            .as_ref()
            .unwrap()
            .held_at
            .len(),
        0
    );
    assert_eq!(sim.core(0).unwrap().counters().own_address_frames, 0);
    assert_eq!(sim.host_stats(0).unwrap().echo_fragments_dropped, 0);
    sim.assert_no_violations();
}

#[test]
fn delayed_echo_and_real_shared_prefix_peer_keep_payload_and_order() {
    let mut sim = request(16, 48, 5);
    peer(&mut sim, ms(48), &[1, 0, 0xA1]);
    peer(&mut sim, ms(65), &[1, 0, 0xA2]);
    // Both real peers finish before local echo becomes available at 80 ms.
    // Drain holds their common ready read until C=100 ms, so their shared
    // 55 FF prefixes face the expected echo before the full copy follows.
    sim.run_until(ms(100));
    assert_eq!(
        sim.deliveries(0)
            .iter()
            .map(|d| d.data.clone())
            .collect::<Vec<_>>(),
        vec![vec![1, 0, 0xA1], vec![1, 0, 0xA2]]
    );
    assert_eq!(sim.report.host_transmits[0].completion, ms(100));
    assert_eq!(sim.report.host_transmits[0].notification, ms(100));
    assert_eq!(sim.host_stats(0).unwrap().echo_mismatches, 1);
    assert_eq!(sim.core(0).unwrap().counters().own_address_frames, 0);
    assert!(sim.frames.iter().all(|frame| frame.is_clean()));
    sim.assert_no_violations();
}

/// A delayed local stream can interleave inside a real peer frame. There is
/// no provenance-based perfect suppression: the missing peer receipt remains
/// a diagnostic even though the physical frames themselves are clean.
#[test]
fn interleaved_local_echo_does_not_hide_a_missed_peer_receipt() {
    let mut sim = request(16, 61, 5);
    peer(&mut sim, ms(85), &[1, 0, 0xA1]);
    // A new poll starts at 106 ms and its 69 ms drain blocks the deferred
    // receipt check until 175 ms. Waiting only until 120 ms is premature.
    sim.run_until(ms(180));
    assert!(sim.deliveries(0).is_empty());
    assert!(sim.frames.iter().all(|frame| frame.is_clean()));
    assert!(sim
        .report
        .violations
        .iter()
        .any(|v| v.kind == ViolationKind::ListenerMissedFrame));
    assert!(sim.core(0).unwrap().counters().parser_errors > 0);
}

#[test]
fn local_returns_retain_actual_corruption_and_framing_errors() {
    for corruption in [Corruption::Xor(0x80), Corruption::FramingError] {
        let mut sim = host(Duration::from_millis(4), Duration::from_millis(10));
        sim.inject(
            ms(0),
            Fault::CorruptNext {
                station: 0,
                frame_type: None,
                offset: 2,
                corruption,
            },
        );
        sim.run_until(ms(515));
        let returns: Vec<_> = sim
            .trace
            .entries()
            .filter_map(|entry| match entry.kind {
                TraceKind::LocalEcho {
                    byte,
                    wire_end,
                    transmission: 0,
                    ..
                } => Some((byte, wire_end)),
                _ => None,
            })
            .collect();
        assert_eq!(returns.len(), 8);
        assert_eq!(
            returns[2].0,
            match corruption {
                Corruption::Xor(mask) => Some(sim.frames[0].octets[2] ^ mask),
                Corruption::FramingError => None,
            }
        );
        assert_eq!(returns[2].1, ms(500) + sim.timing.boundary(3));
        assert!(sim.frames[0].corrupted);
        sim.assert_no_violations();
    }
}

#[test]
fn zero_delay_preserves_completion_and_missing_echo_is_independent() {
    assert_eq!(HostSpec::default().echo_return_delay, Duration::ZERO);
    for echo in [false, true] {
        let mut sim = host(Duration::ZERO, Duration::from_millis(5));
        sim.master_mut(0).unwrap().spec.adapter.echo = echo;
        let end = sim.timing.end_of(ms(500), 8);
        sim.run_until(end + Duration::from_millis(5));
        assert_eq!(sim.frames[0].start, ms(500));
        assert_eq!(sim.frames[0].end, end);
        assert_eq!(sim.report.driver_releases[0].at, end);
        assert_eq!(sim.report.host_transmits[0].completion, end);
        assert_eq!(
            sim.report.host_transmits[0].notification,
            if echo {
                end
            } else {
                end + Duration::from_millis(5)
            }
        );
        assert_eq!(sim.host_stats(0).unwrap().echo_missing, u32::from(!echo));
        assert_eq!(sim.report.local_echo.captured, if echo { 8 } else { 0 });
        assert_eq!(sim.report.local_echo.pending, 0);
        sim.assert_no_violations();
    }
}

#[test]
fn delayed_return_changes_no_wire_peer_clock_or_completion_budget() {
    let mut immediate = request(16, 0, 5);
    let mut delayed = request(16, 61, 5);
    immediate.run_until(ms(110));
    delayed.run_until(ms(110));
    assert_eq!(immediate.frames, delayed.frames);
    assert_eq!(
        immediate.report.driver_releases,
        delayed.report.driver_releases
    );
    assert_eq!(immediate.report.collisions, delayed.report.collisions);
    assert_eq!(immediate.line.last_activity(), Some(ms(44)));
    assert_eq!(delayed.line.last_activity(), Some(ms(44)));
    for sim in [&immediate, &delayed] {
        let crate::sim::NodeKind::Scripted(peer) = &sim.nodes[1].kind else {
            unreachable!()
        };
        assert_eq!(peer.last_character, Some(ms(44)));
    }
    let first = immediate.report.host_transmits[0];
    let second = delayed.report.host_transmits[0];
    assert_eq!(
        (
            first.start,
            first.data_end,
            first.completion,
            first.watchdog
        ),
        (
            second.start,
            second.data_end,
            second.completion,
            second.watchdog
        )
    );
    assert_eq!(first.notification, ms(100));
    assert_eq!(second.notification, ms(105));
    immediate.assert_no_violations();
    delayed.assert_no_violations();
}

fn partial_return() -> Sim {
    let mut sim = request(1, 80, 5);
    sim.run_until(ms(25)); // Exactly three physical own characters observed.
    let mut adapter = sim.master_mut(0).unwrap().spec.adapter;
    adapter.echo = false;
    sim.apply_fault(Fault::SetAdapter {
        station: 0,
        adapter,
    });
    sim.run_until(ms(116));
    assert_eq!(sim.report.local_echo.released, 3);
    assert_eq!(
        sim.master_mut(0)
            .unwrap()
            .host
            .as_ref()
            .unwrap()
            .held_at
            .len(),
        3
    );
    sim
}

#[test]
fn residual_prefix_expires_only_after_strict_read_silence() {
    for (noise_at, dropped) in [(204, 0), (205, 1)] {
        let mut sim = partial_return();
        // The last prefix byte was read at 106 ms. Noise ends one ms after
        // its start and receives one further ms of host-read latency.
        sim.inject(ms(noise_at), Fault::Noise { octets: vec![0xAA] });
        sim.run_until(ms(noise_at + 2));
        assert_eq!(sim.host_stats(0).unwrap().echo_fragments_dropped, dropped);
        assert_eq!(sim.host_stats(0).unwrap().echo_missing, 0);
        sim.assert_no_violations();
    }
}

#[test]
fn confirmed_new_start_ends_an_unfinished_residual_return() {
    let mut sim = partial_return();
    sim.run_until(ms(399));
    assert_eq!(
        sim.master_mut(0)
            .unwrap()
            .host
            .as_ref()
            .unwrap()
            .held_at
            .len(),
        3
    );
    assert_eq!(sim.host_stats(0).unwrap().echo_fragments_dropped, 0);
    sim.run_until(ms(401));
    assert!(sim.nodes[0].driving.is_some());
    assert_eq!(
        sim.master_mut(0)
            .unwrap()
            .host
            .as_ref()
            .unwrap()
            .held_at
            .len(),
        0
    );
    assert_eq!(sim.host_stats(0).unwrap().echo_fragments_dropped, 1);
    assert_eq!(sim.report.local_echo.cancelled, 0);
    sim.assert_no_violations();
}
