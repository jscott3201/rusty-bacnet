//! The straight-line fade and ramp arithmetic and its COV sample schedule
//! (#1384), on hand-set instants.

use super::*;

use bacnet_types::constructed::BACnetXyColor;

fn ms(milliseconds: u64) -> Duration {
    Duration::from_millis(milliseconds)
}

#[test]
fn a_fade_moves_in_a_straight_line_over_its_time() {
    let fade = Transition::fade(20.0f32, 70.0, ms(1_000), ms(2_000)).unwrap();
    assert_eq!(fade.kind(), TransitionKind::Fade);
    for (at, value) in [
        (0, 20.0),
        (1_000, 20.0),
        (1_500, 32.5),
        (2_000, 45.0),
        (2_999, 69.975),
        (3_000, 70.0),
        (9_000, 70.0),
    ] {
        let read = fade.value_at(ms(at));
        assert!((read - value).abs() < 1e-4, "at {at} ms: {read} != {value}");
    }
    assert!(!fade.is_finished(ms(2_999)));
    assert!(fade.is_finished(ms(3_000)));
    // Downward too.
    let down = Transition::fade(80.0f32, 0.0, ms(0), ms(4_000)).unwrap();
    assert_eq!(down.value_at(ms(1_000)), 60.0);
}

#[test]
fn a_ramp_takes_the_distance_over_its_rate() {
    // 50 percent at 20 percent a second is 2.5 s.
    let ramp = Transition::ramp(10.0f32, 60.0, ms(500), 20.0).unwrap();
    assert_eq!(ramp.kind(), TransitionKind::Ramp);
    assert_eq!(ramp.value_at(ms(1_500)), 30.0);
    assert!(!ramp.is_finished(ms(2_999)));
    assert!(ramp.is_finished(ms(3_000)));
    assert_eq!(ramp.value_at(ms(3_000)), 60.0);
    // The slowest rate the standard allows over the whole range.
    let slow = Transition::ramp(0.0f32, 100.0, ms(0), 0.1).unwrap();
    assert!(!slow.is_finished(Duration::from_secs(999)));
    assert!(slow.is_finished(Duration::from_secs(1_000)));
}

#[test]
fn nothing_to_move_is_no_transition() {
    assert_eq!(Transition::fade(40.0f32, 40.0, ms(0), ms(100)), None);
    assert_eq!(Transition::fade(10.0f32, 40.0, ms(0), Duration::ZERO), None);
    assert_eq!(Transition::ramp(40.0f32, 40.0, ms(0), 1.0), None);
    for rate in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert_eq!(Transition::ramp(10.0f32, 40.0, ms(0), rate), None, "{rate}");
    }
}

#[test]
fn samples_follow_the_step_on_the_grid_and_stop_at_the_end() {
    // 0 to 100 over 10 s moves 10 percent a second.
    let fade = Transition::fade(0.0f32, 100.0, ms(50), ms(10_000)).unwrap();
    // A 5 percent step is due 0.5 s on, rounded up onto the 100 ms grid.
    assert_eq!(fade.next_sample(ms(50), 5.0), ms(600));
    assert_eq!(fade.next_sample(ms(600), 5.0), ms(1_100));
    // A step finer than the grid waits for the next grid point.
    assert_eq!(fade.next_sample(ms(600), 0.01), ms(700));
    assert_eq!(fade.next_sample(ms(650), 0.0), ms(700));
    assert_eq!(fade.next_sample(ms(650), f64::NAN), ms(700));
    // A step past the remaining distance samples at the end.
    assert_eq!(fade.next_sample(ms(9_000), 50.0), ms(10_050));
    assert_eq!(fade.next_sample(ms(10_040), 0.0), ms(10_050));
}

#[test]
fn a_run_samples_when_due_and_finishes_at_the_end() {
    let fade = Transition::fade(0.0f32, 100.0, ms(0), ms(1_000)).unwrap();
    let mut run = Run::start(fade, 25.0);
    assert_eq!(run.transition(), &fade);
    assert_eq!(run.deadline(), ms(300));
    assert_eq!(run.advance(ms(299), 25.0), Progress::Pending);
    assert_eq!(run.advance(ms(300), 25.0), Progress::Sampled);
    assert_eq!(run.deadline(), ms(600));
    // An advance a grid cell or more late schedules from when it ran.
    assert_eq!(run.advance(ms(750), 25.0), Progress::Sampled);
    assert_eq!(run.deadline(), ms(1_000));
    assert_eq!(run.advance(ms(1_000), 25.0), Progress::Finished);
}

#[test]
fn a_run_woken_slightly_late_keeps_its_cadence() {
    // 10 percent a second with a 1 percent step: a sample every 100 ms.
    let fade = Transition::fade(0.0f32, 100.0, ms(0), ms(10_000)).unwrap();
    let mut run = Run::start(fade, 1.0);
    assert_eq!(run.deadline(), ms(100));
    // Each wake comes 1 ms after its sample, as a real timer's might; the
    // next sample stays a grid cell after the planned one.
    for planned in (100..=900).step_by(100) {
        assert_eq!(run.advance(ms(planned + 1), 1.0), Progress::Sampled);
        assert_eq!(run.deadline(), ms(planned + 100), "woken at {planned} + 1");
    }
    // 99 ms late still counts as on time.
    assert_eq!(run.advance(ms(1_099), 1.0), Progress::Sampled);
    assert_eq!(run.deadline(), ms(1_100));
}

#[test]
fn a_finer_step_brings_the_pending_sample_forward() {
    // 0 to 100 over 10 s moves 10 percent a second.
    let fade = Transition::fade(0.0f32, 100.0, ms(0), ms(10_000)).unwrap();
    let mut run = Run::start(fade, 5.0);
    assert_eq!(run.deadline(), ms(500));
    // A subscriber's 1 percent step is due 100 ms after the start (#1510).
    assert_eq!(run.advance(ms(50), 1.0), Progress::Pending);
    assert_eq!(run.deadline(), ms(100));
    assert_eq!(run.advance(ms(100), 1.0), Progress::Sampled);
    assert_eq!(run.deadline(), ms(200));
    // Back to the coarse step, the pending sample stays where it is, and
    // the one after it is a coarse step on.
    assert_eq!(run.advance(ms(150), 5.0), Progress::Pending);
    assert_eq!(run.deadline(), ms(200));
    assert_eq!(run.advance(ms(200), 5.0), Progress::Sampled);
    assert_eq!(run.deadline(), ms(700));
    // A finer step whose sample would already have fallen due samples at
    // once, and the next is planned from now.
    assert_eq!(run.advance(ms(450), 1.0), Progress::Sampled);
    assert_eq!(run.deadline(), ms(600));
}

#[test]
fn an_xy_colour_fades_both_coordinates_along_one_line() {
    let from = BACnetXyColor::new(0.1, 0.2);
    let to = BACnetXyColor::new(0.4, 0.6);
    // A 3-4-5 triangle: the distance is 0.5.
    assert!((BACnetXyColor::distance(from, to) - 0.5).abs() < 1e-6);
    let fade = Transition::fade(from, to, ms(0), ms(1_000)).unwrap();
    let half = fade.value_at(ms(500));
    assert!((half.x - 0.25).abs() < 1e-6 && (half.y - 0.4).abs() < 1e-6);
    assert_eq!(fade.value_at(ms(1_000)), to);
    // The same colour is nothing to fade.
    assert_eq!(Transition::fade(to, to, ms(0), ms(1_000)), None);
}

#[test]
fn kelvin_rounds_to_the_nearest_degree_and_ramps_by_its_distance() {
    assert_eq!(u32::interpolate(2_700, 2_701, 0.49), 2_700);
    assert_eq!(u32::interpolate(2_700, 2_701, 0.5), 2_701);
    // Downward as well as upward.
    assert_eq!(u32::interpolate(6_500, 2_700, 0.25), 5_550);
    assert_eq!(u32::distance(6_500, 2_700), 3_800.0);
    // 3,800 K at 100 K/s is 38 s.
    let ramp = Transition::ramp(6_500u32, 2_700, ms(0), 100.0).unwrap();
    assert!(!ramp.is_finished(ms(37_999)));
    assert!(ramp.is_finished(ms(38_000)));
    assert_eq!(ramp.value_at(ms(19_000)), 4_600);
}
