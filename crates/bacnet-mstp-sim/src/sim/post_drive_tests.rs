//! Independent edge arithmetic for physical driver ownership.

use bacnet_mstp_core::{Duration, Instant};

use super::Sim;
use crate::{AdapterSpec, Driver, Fault, HostSpec, MasterSpec, ScriptSend, SimConfig};

fn us(value: u64) -> Instant {
    Instant::from_micros(value)
}

fn one_octet(hold: Duration) -> Sim {
    let mut sim = Sim::new(SimConfig::new(10_000));
    sim.add_scripted(1, |_: &crate::SeenFrame, _: &mut Vec<ScriptSend>| {});
    sim.script_send(
        0,
        ScriptSend {
            after: Duration::ZERO,
            inter_octet_gap: Duration::ZERO,
            post_drive: hold,
            octets: vec![0x55],
            malformed: true,
        },
    );
    sim
}

/// At 10 kbaud the stop bit begins at 900 us and ends at 1000 us.
/// A 1400 us hold therefore owns the line through, but excluding, 2400 us.
#[test]
fn driver_remains_owned_after_data_end_until_release() {
    let mut sim = one_octet(Duration::from_micros(1_400));
    sim.run_until(us(1_000));
    assert_eq!(sim.frames.len(), 1);
    assert_eq!(sim.frames[0].end, us(1_000));
    assert!(
        sim.nodes[0].driving.is_some(),
        "data end is not driver release"
    );
    assert_eq!(sim.nodes[0].listening_since, None);
    sim.run_until(us(2_399));
    assert!(sim.nodes[0].driving.is_some());
    sim.run_until(us(2_400));
    assert_eq!(sim.nodes[0].driving, None);
    assert_eq!(sim.nodes[0].listening_since, Some(us(2_400)));
    assert_eq!(sim.line.last_activity(), Some(us(1_000)));
    assert!(
        !sim.trace
            .entries()
            .any(|entry| matches!(entry.kind, crate::TraceKind::Garbled))
    );
    assert_eq!(sim.frames.len(), 1);
    assert_eq!(sim.report.driver_releases.len(), 1);
    let release = sim.report.driver_releases[0];
    assert_eq!(release.data_end, Some(us(1_000)));
    assert_eq!(release.scheduled, us(2_400));
    assert_eq!(release.at, us(2_400));
    assert_eq!(release.cause, crate::DriverReleaseCause::Completed);
    assert!(sim.meta.is_empty());
    assert_eq!(sim.line.live_count(), 0);
}

/// Zero hold retains the exact old end, receiver and activity timestamps.
#[test]
fn default_zero_hold_releases_at_the_final_stop() {
    assert_eq!(AdapterSpec::default().post_drive, Duration::ZERO);
    let mut sim = one_octet(Duration::ZERO);
    sim.run_until(us(1_000));
    assert_eq!(sim.nodes[0].driving, None);
    assert_eq!(sim.nodes[0].listening_since, Some(us(1_000)));
    assert_eq!(sim.report.driver_releases[0].at, us(1_000));
    assert_eq!(sim.frames[0].end, us(1_000));
}

/// A held idle driver garbles a new character, even if it releases part way
/// through that character. Exact release wins the tie with new activity.
#[test]
fn hold_overlap_is_visible_without_rewriting_the_completed_frame() {
    for start_ns in [1_000_000, 1_000_001, 2_399_999, 2_400_000, 2_400_001] {
        let mut sim = one_octet(Duration::from_micros(1_400));
        let start = Instant::from_nanos(start_ns);
        sim.inject(start, Fault::Noise { octets: vec![0xFF] });
        sim.run_until(start + Duration::from_micros(1_000));
        assert_eq!(sim.frames.len(), 2);
        assert_eq!(sim.frames[0].end, us(1_000));
        assert!(!sim.frames[0].collided);
        assert_eq!(sim.frames[1].collided, start_ns < 2_400_000);
        assert_eq!(
            sim.report.collisions.len(),
            usize::from(start_ns < 2_400_000)
        );
        if let Some(collision) = sim.report.collisions.first() {
            assert_eq!(collision.other, Driver::Station(1));
            let hold = collision.post_drive.expect("physical hold evidence");
            assert_eq!(hold.data_end, us(1_000));
            assert_eq!(hold.release, us(2_400));
        }
        // Holding idle does not reopen a receiver window after the competing
        // byte ends, and creates neither duplicate receipts nor later errors.
        let activity = sim.line.last_activity();
        sim.run_until(us(5_000));
        assert_eq!(sim.line.last_activity(), activity);
        assert_eq!(sim.frames.len(), 2);
        assert_eq!(sim.report.driver_releases.len(), 2);
        sim.assert_no_violations();
    }
}

/// A receiver cannot hear peer characters while its own idle driver is held;
/// re-enabling midway through a character does not turn a fragment clean.
#[test]
fn receiver_stays_off_until_release_and_then_receives_again() {
    let mut sim = one_octet(Duration::from_micros(1_400));
    sim.inject(us(1_100), Fault::Noise { octets: vec![0xAA] });
    sim.run_until(us(2_100));
    let super::NodeKind::Scripted(node) = &sim.nodes[0].kind else {
        unreachable!()
    };
    assert_eq!(node.last_character, None);
    sim.inject(us(2_300), Fault::Noise { octets: vec![0xAA] });
    sim.run_until(us(3_300));
    let super::NodeKind::Scripted(node) = &sim.nodes[0].kind else {
        unreachable!()
    };
    assert_eq!(node.last_character, Some(us(3_300)));
    assert!(sim.frames.last().unwrap().collided);
    sim.inject(us(3_400), Fault::Noise { octets: vec![0xAA] });
    sim.run_until(us(4_400));
    assert!(!sim.frames.last().unwrap().collided);
    sim.assert_no_violations();
}

/// Power loss during hold releases immediately without truncating data that
/// already completed; its stale release cannot clear a restarted driver.
#[test]
fn power_loss_during_hold_fences_the_old_release() {
    let mut sim = one_octet(Duration::from_micros(1_400));
    sim.run_until(us(1_200));
    sim.power_off(0);
    assert_eq!(sim.frames.len(), 1);
    assert!(!sim.frames[0].truncated);
    assert_eq!(
        sim.report.driver_releases[0].cause,
        crate::DriverReleaseCause::PowerLoss
    );
    assert_eq!(sim.report.driver_releases[0].at, us(1_200));
    assert_eq!(sim.report.driver_releases[0].data_end, Some(us(1_000)));
    sim.power_on(0);
    sim.script_send(
        0,
        ScriptSend {
            after: Duration::ZERO,
            inter_octet_gap: Duration::ZERO,
            post_drive: Duration::from_micros(1_400),
            octets: vec![0xFF],
            malformed: true,
        },
    );
    let second = sim.nodes[0].driving;
    sim.run_until(us(2_400));
    assert_eq!(sim.nodes[0].driving, second);
    assert!(second.is_some());
    assert_eq!(sim.nodes[0].listening_since, None);
    assert_eq!(sim.frames.len(), 2);
    assert_eq!(sim.report.driver_releases.len(), 1);
    sim.run_until(us(3_600));
    assert_eq!(sim.nodes[0].driving, None);
    assert_eq!(sim.report.driver_releases.len(), 2);
    assert!(sim.meta.is_empty());
    assert_eq!(sim.line.live_count(), 0);
}

#[test]
fn power_loss_during_data_records_one_truncation_and_one_release() {
    let mut sim = one_octet(Duration::from_micros(1_400));
    sim.run_until(us(500));
    sim.power_off(0);
    sim.run_until(us(5_000));
    assert_eq!(sim.frames.len(), 1);
    assert!(sim.frames[0].truncated);
    assert_eq!(sim.report.driver_releases.len(), 1);
    assert_eq!(sim.report.driver_releases[0].data_end, None);
    assert_eq!(sim.report.driver_releases[0].at, us(500));
    assert_eq!(sim.nodes[0].listening_since, None);
    assert!(sim.meta.is_empty());
    assert_eq!(sim.line.live_count(), 0);
}

/// Octet completion at F starts the core usage timer before physical R.
#[test]
fn core_timer_origin_remains_the_final_data_stop() {
    let mut sim = Sim::new(SimConfig::new(38_400));
    let mut spec = MasterSpec::new(0, 1, 38_400);
    spec.adapter.post_drive = sim.timing.max_post_drive();
    let usage = spec.config.t_usage_timeout;
    sim.add_master(spec);
    let end = us(502_084); // 500 ms generation plus ceil(80 / 38400) seconds.
    sim.run_until(end);
    assert_eq!(sim.frames[0].end, end);
    assert!(sim.nodes[0].driving.is_some());
    assert_eq!(sim.core(0).unwrap().next_deadline(), Some(end + usage));
    sim.run_until(end + sim.timing.max_post_drive());
    assert_eq!(sim.core(0).unwrap().next_deadline(), Some(end + usage));
    sim.assert_no_violations();
}

/// A live configuration change affects subsequent transmissions, never the
/// release already captured for a frame on the wire.
#[test]
fn adapter_change_does_not_retime_an_existing_release() {
    let mut sim = Sim::new(SimConfig::new(38_400));
    let hold = sim.timing.max_post_drive();
    sim.add_master(MasterSpec::new(0, 1, 38_400).adapter(AdapterSpec {
        post_drive: hold,
        ..AdapterSpec::default()
    }));
    sim.run_until(us(501_000));
    sim.apply_fault(Fault::SetAdapter {
        station: 0,
        adapter: AdapterSpec::default(),
    });
    sim.run_until(us(550_000));
    assert_eq!(sim.report.driver_releases[0].at, sim.frames[0].end + hold);
    assert!(sim.report.driver_releases.len() >= 2);
    assert_eq!(sim.report.driver_releases[1].at, sim.frames[1].end);
    sim.assert_no_violations();
}

/// The queued release of a completed frame cannot move the receiver-on edge
/// or touch the fresh core after a master reboots during the hold.
#[test]
fn master_reboot_during_hold_preserves_the_new_incarnation() {
    let mut sim = Sim::new(SimConfig::new(38_400));
    sim.add_master(MasterSpec::new(0, 1, 38_400).adapter(AdapterSpec {
        post_drive: sim.timing.max_post_drive(),
        ..AdapterSpec::default()
    }));
    sim.run_until(us(502_100));
    let incarnation = sim.master_mut(0).unwrap().incarnation;
    sim.power_off(0);
    sim.power_on(0);
    let boot = sim.now;
    sim.run_until(us(503_000));
    assert_eq!(sim.master_mut(0).unwrap().incarnation, incarnation + 1);
    assert_eq!(sim.nodes[0].listening_since, Some(boot));
    assert_eq!(sim.report.driver_releases.len(), 1);
    assert!(!sim.frames[0].truncated);
    sim.assert_no_violations();
}

/// C=max(drain,F) remains independent of R, with echo delivery optionally
/// delaying the notification while preserving its completion timestamp.
#[test]
fn host_drain_and_echo_do_not_replace_completion_with_release() {
    for drain_us in [0, 10_000] {
        for echo in [false, true] {
            let mut sim = Sim::new(SimConfig::new(38_400));
            let mut spec = MasterSpec::new(0, 1, 38_400).host(HostSpec {
                drain_duration: Duration::from_micros(drain_us),
                echo_grace: echo.then_some(Duration::from_millis(5)),
                ..HostSpec::default()
            });
            spec.adapter.echo = echo;
            spec.adapter.post_drive = sim.timing.max_post_drive();
            sim.add_master(spec);
            let end = us(502_084);
            let completion = us(500_000 + drain_us).max(end);
            sim.run_until(end);
            assert!(sim.nodes[0].driving.is_some());
            sim.run_until(completion + Duration::from_millis(2));
            assert_eq!(sim.frames[0].end, end);
            assert_eq!(
                sim.report.driver_releases[0].at,
                end + sim.timing.max_post_drive()
            );
            let stats = sim.host_stats(0).unwrap();
            assert_eq!(stats.last_completion, Some(completion));
            assert_eq!(stats.echo_missing, 0);
            assert_eq!(stats.echo_mismatches, 0);
            assert_eq!(sim.core(0).unwrap().counters().own_address_frames, 0);
            sim.assert_no_violations();
        }
    }
}

/// Isolate admission with a synthetic completed driver: normal 14-bit holds
/// expire before the 40-bit turnaround, but admission must enforce ownership
/// itself even when the core already has another pending frame.
#[test]
fn own_driver_release_precedes_new_start_confirmation() {
    let mut sim = Sim::new(SimConfig::new(38_400));
    let spec = MasterSpec::new(0, 1, 38_400).adapter(AdapterSpec {
        start_delay: Duration::from_millis(5),
        ..AdapterSpec::default()
    });
    sim.add_master(spec);
    sim.run_until(us(500_000));
    let id = sim.master_mut(0).unwrap().pending.as_ref().unwrap().id;
    let old = sim.line.begin(
        sim.timing,
        Driver::Station(0),
        vec![0x55],
        us(499_739),
        None,
        sim.timing.max_post_drive(),
        Duration::ZERO,
    );
    let release = sim.line.owner(old).unwrap().release;
    assert!(release > sim.now);
    sim.line.finish(old);
    sim.nodes[0].driving = Some(old);
    sim.nodes[0].listening_since = None;
    sim.push(
        release,
        crate::event::Class::DriverRelease,
        0u8,
        super::Event::DriverRelease { tx: old },
    );
    sim.adapter_start(0, id);
    assert!(sim.master_mut(0).unwrap().pending.is_some());
    assert_eq!(sim.master_mut(0).unwrap().transmitting, None);
    sim.run_until(release - Duration::from_nanos(1));
    assert_eq!(sim.nodes[0].driving, Some(old));
    sim.run_until(release);
    assert!(sim.master_mut(0).unwrap().pending.is_none());
    assert_eq!(sim.master_mut(0).unwrap().transmitting, Some(id));
    assert_ne!(sim.nodes[0].driving, Some(old));
    assert!(sim.nodes[0].driving.is_some());
    sim.assert_no_violations();
}

#[test]
#[should_panic(expected = "post-drive hold exceeds")]
fn unpowered_master_configuration_rejects_an_invalid_hold() {
    let mut sim = Sim::new(SimConfig::new(115_200));
    let mut spec = MasterSpec::new(0, 1, 115_200).unpowered();
    spec.adapter.post_drive = Duration::from_nanos(121_528);
    sim.add_master(spec);
}

#[test]
#[should_panic(expected = "post-drive hold exceeds")]
fn adapter_reconfiguration_rejects_an_invalid_hold() {
    let mut sim = Sim::new(SimConfig::new(115_200));
    sim.add_master(MasterSpec::new(0, 1, 115_200));
    sim.apply_fault(Fault::SetAdapter {
        station: 0,
        adapter: AdapterSpec {
            post_drive: Duration::from_nanos(121_528),
            ..AdapterSpec::default()
        },
    });
}

#[test]
#[should_panic(expected = "post-drive hold exceeds")]
fn scripted_driver_rejects_an_invalid_hold() {
    one_octet(Duration::from_nanos(1_400_001));
}
