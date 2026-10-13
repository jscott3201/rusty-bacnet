//! The colour objects carrying out their commands (#1474), on a hand-set
//! monotonic clock: fades and ramps arrive on time, STOP and a Present_Value
//! write halt them, steps clamp, Transition shapes a direct write, and the
//! COV samples fall where the sample step puts them.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::*;
use crate::traits::BACnetObject;
use bacnet_types::constructed::BACnetColorCommand;
use bacnet_types::enums::{ColorOperation as Op, PropertyIdentifier as P};

/// A clock the test sets, in milliseconds.
#[derive(Clone)]
struct Clock(Arc<Mutex<Duration>>);

impl Clock {
    fn bind(object: &mut dyn BACnetObject) -> Self {
        let clock = Self(Arc::new(Mutex::new(Duration::ZERO)));
        let source = Arc::clone(&clock.0);
        object.bind_monotonic_clock_internal(Some(Arc::new(move || *source.lock().unwrap())));
        clock
    }

    fn set(&self, milliseconds: u64) -> Duration {
        let now = Duration::from_millis(milliseconds);
        *self.0.lock().unwrap() = now;
        now
    }
}

fn ms(milliseconds: u64) -> Duration {
    Duration::from_millis(milliseconds)
}

fn xy(x: f32, y: f32) -> PropertyValue {
    PropertyValue::List(vec![PropertyValue::Real(x), PropertyValue::Real(y)])
}

/// Assert `p` reads as the colour (`x`, `y`), to within float rounding.
fn assert_xy(object: &dyn BACnetObject, p: P, x: f32, y: f32) {
    let PropertyValue::List(items) = read(object, p) else {
        panic!("{p:?} isn't an xy colour");
    };
    let [PropertyValue::Real(at_x), PropertyValue::Real(at_y)] = items.as_slice() else {
        panic!("{p:?} isn't an xy colour");
    };
    assert!(
        (at_x - x).abs() < 1e-6 && (at_y - y).abs() < 1e-6,
        "{p:?} read ({at_x}, {at_y}), not ({x}, {y})"
    );
}

fn read(object: &dyn BACnetObject, p: P) -> PropertyValue {
    object.read_property(p, None).unwrap()
}

fn kelvin(object: &dyn BACnetObject, p: P) -> u64 {
    match read(object, p) {
        PropertyValue::Unsigned(kelvin) => kelvin,
        other => panic!("{p:?} read {other:?}"),
    }
}

fn in_progress(object: &dyn BACnetObject) -> u32 {
    match read(object, P::IN_PROGRESS) {
        PropertyValue::Enumerated(raw) => raw,
        other => panic!("In_Progress read {other:?}"),
    }
}

fn fade_to(x: f32, y: f32, fade_time: Option<u32>) -> BACnetColorCommand {
    BACnetColorCommand {
        target_color: Some(BACnetXyColor::new(x, y)),
        fade_time,
        ..BACnetColorCommand::new(Op::FADE_TO_COLOR)
    }
}

fn cct(operation: Op, target: Option<u32>) -> BACnetColorCommand {
    BACnetColorCommand {
        target_color_temperature: target,
        ..BACnetColorCommand::new(operation)
    }
}

/// A Color object at (0.1, 0.1) on a clock at 0.
fn color() -> (ColorObject, Clock) {
    let mut color = ColorObject::new(1, "CLR-1").unwrap();
    let clock = Clock::bind(&mut color);
    color
        .set_present_value(BACnetXyColor::new(0.1, 0.1))
        .unwrap();
    (color, clock)
}

/// A Color Temperature object at 3000 K on a clock at 0.
fn temperature() -> (ColorTemperatureObject, Clock) {
    let mut ct = ColorTemperatureObject::new(1, "CT-1").unwrap();
    let clock = Clock::bind(&mut ct);
    ct.set_present_value(3_000).unwrap();
    (ct, clock)
}

#[test]
fn a_colour_fade_moves_tracking_value_and_completes_on_time() {
    let (mut color, clock) = color();
    color
        .set_color_command(fade_to(0.5, 0.4, Some(2_000)))
        .unwrap();
    // The target goes into Present_Value at once.
    assert_xy(&color, P::PRESENT_VALUE, 0.5, 0.4);
    assert_xy(&color, P::TRACKING_VALUE, 0.1, 0.1);
    assert_eq!(in_progress(&color), 1);
    clock.set(500);
    assert_xy(&color, P::TRACKING_VALUE, 0.2, 0.175);
    clock.set(1_999);
    assert_eq!(in_progress(&color), 1);
    let now = clock.set(2_000);
    assert_xy(&color, P::TRACKING_VALUE, 0.5, 0.4);
    assert_eq!(in_progress(&color), 0);
    // The server's advance ends the run, and nothing stays due.
    assert!(color.advance_monotonic_time_internal(now));
    assert_eq!(color.next_monotonic_deadline_internal(), None);
    assert_xy(&color, P::TRACKING_VALUE, 0.5, 0.4);
}

#[test]
fn a_colour_fade_without_a_time_takes_default_fade_time() {
    let (mut color, clock) = color();
    color
        .write_property(
            P::DEFAULT_FADE_TIME,
            None,
            PropertyValue::Unsigned(1_000),
            None,
        )
        .unwrap();
    color.set_color_command(fade_to(0.3, 0.3, None)).unwrap();
    clock.set(999);
    assert_eq!(in_progress(&color), 1);
    clock.set(1_000);
    assert_eq!(in_progress(&color), 0);
}

#[test]
fn stop_halts_a_colour_fade_where_it_stands() {
    let (mut color, clock) = color();
    color
        .set_color_command(fade_to(0.5, 0.5, Some(4_000)))
        .unwrap();
    clock.set(1_000);
    color
        .set_color_command(BACnetColorCommand::new(Op::STOP))
        .unwrap();
    // Present_Value takes the colour reached, a quarter of the way.
    assert_xy(&color, P::PRESENT_VALUE, 0.2, 0.2);
    assert_xy(&color, P::TRACKING_VALUE, 0.2, 0.2);
    assert_eq!(in_progress(&color), 0);
    assert_eq!(color.next_monotonic_deadline_internal(), None);
    clock.set(4_000);
    assert_xy(&color, P::TRACKING_VALUE, 0.2, 0.2);
    // With nothing running, STOP changes nothing.
    color
        .set_color_command(BACnetColorCommand::new(Op::STOP))
        .unwrap();
    assert_xy(&color, P::PRESENT_VALUE, 0.2, 0.2);
}

#[test]
fn a_present_value_write_halts_a_colour_fade() {
    let (mut color, clock) = color();
    color
        .set_color_command(fade_to(0.5, 0.5, Some(4_000)))
        .unwrap();
    clock.set(1_000);
    color
        .write_property(P::PRESENT_VALUE, None, xy(0.7, 0.2), None)
        .unwrap();
    assert_xy(&color, P::PRESENT_VALUE, 0.7, 0.2);
    assert_xy(&color, P::TRACKING_VALUE, 0.7, 0.2);
    assert_eq!(in_progress(&color), 0);
    assert_eq!(color.next_monotonic_deadline_internal(), None);
}

#[test]
fn a_new_fade_starts_from_where_the_last_one_stood() {
    let (mut color, clock) = color();
    color
        .set_color_command(fade_to(0.5, 0.5, Some(4_000)))
        .unwrap();
    clock.set(2_000);
    color
        .set_color_command(fade_to(0.1, 0.1, Some(1_000)))
        .unwrap();
    assert_xy(&color, P::TRACKING_VALUE, 0.3, 0.3);
    clock.set(2_500);
    assert_xy(&color, P::TRACKING_VALUE, 0.2, 0.2);
}

#[test]
fn transition_fade_makes_a_colour_write_fade() {
    let (mut color, clock) = color();
    for (p, value) in [
        (P::TRANSITION, PropertyValue::Enumerated(1)),
        (P::DEFAULT_FADE_TIME, PropertyValue::Unsigned(1_000)),
    ] {
        color.write_property(p, None, value, None).unwrap();
    }
    clock.set(100);
    color
        .write_property(P::PRESENT_VALUE, None, xy(0.3, 0.5), None)
        .unwrap();
    assert_xy(&color, P::PRESENT_VALUE, 0.3, 0.5);
    assert_xy(&color, P::TRACKING_VALUE, 0.1, 0.1);
    assert_eq!(in_progress(&color), 1);
    clock.set(600);
    assert_xy(&color, P::TRACKING_VALUE, 0.2, 0.3);
    clock.set(1_100);
    assert_eq!(in_progress(&color), 0);
    // FADE_TO_COLOR ignores Transition: its own time rules.
    color
        .set_color_command(fade_to(0.1, 0.1, Some(200)))
        .unwrap();
    clock.set(1_300);
    assert_eq!(in_progress(&color), 0);
}

#[test]
fn a_cct_fade_and_ramp_complete_at_the_clamped_target() {
    let (mut ct, clock) = temperature();
    ct.set_min_max(2_000, 6_500).unwrap();
    // FADE_TO_CCT 1000 K clamps to 2000 K, over 1 s.
    let fade = BACnetColorCommand {
        fade_time: Some(1_000),
        ..cct(Op::FADE_TO_CCT, Some(1_000))
    };
    ct.set_color_command(fade).unwrap();
    assert_eq!(kelvin(&ct, P::PRESENT_VALUE), 2_000);
    assert_eq!(in_progress(&ct), 1);
    clock.set(250);
    assert_eq!(kelvin(&ct, P::TRACKING_VALUE), 2_750);
    let now = clock.set(1_000);
    assert!(ct.advance_monotonic_time_internal(now));
    assert_eq!(kelvin(&ct, P::TRACKING_VALUE), 2_000);
    assert_eq!(in_progress(&ct), 0);
    // RAMP_TO_CCT 30000 K clamps to 6500 K; 4500 K at the default 100 K/s is
    // 45 s.
    ct.set_color_command(cct(Op::RAMP_TO_CCT, Some(30_000)))
        .unwrap();
    assert_eq!(kelvin(&ct, P::PRESENT_VALUE), 6_500);
    assert_eq!(in_progress(&ct), 2);
    clock.set(11_000);
    assert_eq!(kelvin(&ct, P::TRACKING_VALUE), 3_000);
    clock.set(45_999);
    assert_eq!(in_progress(&ct), 2);
    clock.set(46_000);
    assert_eq!(in_progress(&ct), 0);
    assert_eq!(kelvin(&ct, P::TRACKING_VALUE), 6_500);
}

#[test]
fn steps_change_the_temperature_at_once_and_clamp() {
    let (mut ct, clock) = temperature();
    ct.set_min_max(2_700, 3_120).unwrap();
    let step = |operation, increment| BACnetColorCommand {
        step_increment: increment,
        ..BACnetColorCommand::new(operation)
    };
    // The default increment is 50 K.
    ct.set_color_command(step(Op::STEP_UP_CCT, None)).unwrap();
    assert_eq!(kelvin(&ct, P::PRESENT_VALUE), 3_050);
    assert_eq!(kelvin(&ct, P::TRACKING_VALUE), 3_050);
    assert_eq!(in_progress(&ct), 0);
    ct.set_color_command(step(Op::STEP_UP_CCT, Some(100)))
        .unwrap();
    assert_eq!(kelvin(&ct, P::PRESENT_VALUE), 3_120);
    ct.set_color_command(step(Op::STEP_DOWN_CCT, Some(30_000)))
        .unwrap();
    assert_eq!(kelvin(&ct, P::PRESENT_VALUE), 2_700);
    // A step works from Tracking_Value and halts the ramp under way.
    ct.set_color_command(cct(Op::RAMP_TO_CCT, Some(3_100)))
        .unwrap();
    clock.set(2_000);
    ct.set_color_command(step(Op::STEP_UP_CCT, Some(10)))
        .unwrap();
    assert_eq!(kelvin(&ct, P::PRESENT_VALUE), 2_910);
    assert_eq!(kelvin(&ct, P::TRACKING_VALUE), 2_910);
    assert_eq!(in_progress(&ct), 0);
    assert_eq!(ct.next_monotonic_deadline_internal(), None);
}

#[test]
fn stop_and_a_present_value_write_halt_a_ramp() {
    let (mut ct, clock) = temperature();
    ct.set_color_command(cct(Op::RAMP_TO_CCT, Some(5_000)))
        .unwrap();
    clock.set(4_000);
    ct.set_color_command(BACnetColorCommand::new(Op::STOP))
        .unwrap();
    assert_eq!(kelvin(&ct, P::PRESENT_VALUE), 3_400);
    assert_eq!(in_progress(&ct), 0);
    ct.set_color_command(cct(Op::RAMP_TO_CCT, Some(5_000)))
        .unwrap();
    clock.set(5_000);
    ct.write_property(P::PRESENT_VALUE, None, PropertyValue::Unsigned(2_700), None)
        .unwrap();
    assert_eq!(kelvin(&ct, P::PRESENT_VALUE), 2_700);
    assert_eq!(kelvin(&ct, P::TRACKING_VALUE), 2_700);
    assert_eq!(in_progress(&ct), 0);
}

#[test]
fn transition_ramp_makes_a_temperature_write_ramp() {
    let (mut ct, clock) = temperature();
    for (p, value) in [
        (P::TRANSITION, PropertyValue::Enumerated(2)),
        (P::DEFAULT_RAMP_RATE, PropertyValue::Unsigned(500)),
    ] {
        ct.write_property(p, None, value, None).unwrap();
    }
    ct.write_property(P::PRESENT_VALUE, None, PropertyValue::Unsigned(2_000), None)
        .unwrap();
    assert_eq!(in_progress(&ct), 2);
    clock.set(1_000);
    assert_eq!(kelvin(&ct, P::TRACKING_VALUE), 2_500);
    clock.set(2_000);
    assert_eq!(in_progress(&ct), 0);
    // Steps ignore Transition.
    ct.set_color_command(BACnetColorCommand::new(Op::STEP_UP_CCT))
        .unwrap();
    assert_eq!(kelvin(&ct, P::TRACKING_VALUE), 2_050);
}

#[test]
fn cov_samples_follow_the_sample_step_on_the_grid() {
    // 3000 K to 4000 K over 1 s moves 10 K each 10 ms, finer than the
    // 100 ms grid, so each grid point samples.
    let (mut ct, clock) = temperature();
    let fade = BACnetColorCommand {
        fade_time: Some(1_000),
        ..cct(Op::FADE_TO_CCT, Some(4_000))
    };
    ct.set_color_command(fade).unwrap();
    assert_eq!(ct.next_monotonic_deadline_internal(), Some(ms(100)));
    assert!(!ct.advance_monotonic_time_internal(clock.set(99)));
    assert!(ct.advance_monotonic_time_internal(clock.set(100)));
    assert_eq!(ct.next_monotonic_deadline_internal(), Some(ms(200)));
    // A colour fade of 0.4 along x over 20 s moves 0.001 each 50 ms, so the
    // first grid point samples.
    let (mut color, clock) = color();
    color
        .set_color_command(fade_to(0.5, 0.1, Some(20_000)))
        .unwrap();
    assert_eq!(color.next_monotonic_deadline_internal(), Some(ms(100)));
    // Over 380 s it moves 0.001 each 950 ms, rounded up onto the grid.
    color
        .set_color_command(fade_to(0.5, 0.1, Some(380_000)))
        .unwrap();
    assert_eq!(color.next_monotonic_deadline_internal(), Some(ms(1_000)));
    assert!(color.advance_monotonic_time_internal(clock.set(1_000)));
    assert_eq!(color.next_monotonic_deadline_internal(), Some(ms(2_000)));
}

#[test]
fn a_cov_snapshot_reads_as_the_object_did_when_taken() {
    let (mut ct, clock) = temperature();
    let fade = BACnetColorCommand {
        fade_time: Some(1_000),
        ..cct(Op::FADE_TO_CCT, Some(4_000))
    };
    ct.set_color_command(fade).unwrap();
    clock.set(500);
    let snapshot = ct.cov_snapshot_internal().unwrap();
    clock.set(1_000);
    assert_eq!(kelvin(snapshot.as_ref(), P::TRACKING_VALUE), 3_500);
    assert_eq!(in_progress(snapshot.as_ref()), 1);
    assert_eq!(kelvin(&ct, P::TRACKING_VALUE), 4_000);
}

#[test]
fn with_no_clock_a_fade_waits_on_logical_time() {
    let mut color = ColorObject::new(1, "CLR-1").unwrap();
    color
        .set_color_command(fade_to(0.5, 0.5, Some(1_000)))
        .unwrap();
    assert_eq!(in_progress(&color), 1);
    assert!(color.advance_time_internal(ms(1_000)));
    assert_eq!(in_progress(&color), 0);
    assert_xy(&color, P::TRACKING_VALUE, 0.5, 0.5);
}

#[test]
fn cct_commands_without_their_field_take_the_written_defaults() {
    let (mut ct, clock) = temperature();
    for (p, value) in [
        (P::DEFAULT_FADE_TIME, 2_000),
        (P::DEFAULT_RAMP_RATE, 250),
        (P::DEFAULT_STEP_INCREMENT, 75),
    ] {
        ct.write_property(p, None, PropertyValue::Unsigned(value), None)
            .unwrap();
    }
    // FADE_TO_CCT 4000 K with no fade time takes the 2,000 ms default.
    ct.set_color_command(cct(Op::FADE_TO_CCT, Some(4_000)))
        .unwrap();
    clock.set(1_000);
    assert_eq!(kelvin(&ct, P::TRACKING_VALUE), 3_500);
    clock.set(1_999);
    assert_eq!(in_progress(&ct), 1);
    clock.set(2_000);
    assert_eq!(in_progress(&ct), 0);
    // RAMP_TO_CCT 3000 K with no rate moves at the 250 K/s default: 1000 K
    // in 4 s.
    ct.set_color_command(cct(Op::RAMP_TO_CCT, Some(3_000)))
        .unwrap();
    clock.set(4_000);
    assert_eq!(kelvin(&ct, P::TRACKING_VALUE), 3_500);
    clock.set(5_999);
    assert_eq!(in_progress(&ct), 2);
    clock.set(6_000);
    assert_eq!(in_progress(&ct), 0);
    // The steps with no increment move the 75 K default.
    ct.set_color_command(BACnetColorCommand::new(Op::STEP_UP_CCT))
        .unwrap();
    assert_eq!(kelvin(&ct, P::PRESENT_VALUE), 3_075);
    for _ in 0..2 {
        ct.set_color_command(BACnetColorCommand::new(Op::STEP_DOWN_CCT))
            .unwrap();
    }
    assert_eq!(kelvin(&ct, P::PRESENT_VALUE), 2_925);
}

#[test]
fn transition_fade_makes_a_temperature_write_fade() {
    let (mut ct, clock) = temperature();
    for (p, value) in [
        (P::TRANSITION, PropertyValue::Enumerated(1)),
        (P::DEFAULT_FADE_TIME, PropertyValue::Unsigned(1_000)),
    ] {
        ct.write_property(p, None, value, None).unwrap();
    }
    ct.write_property(P::PRESENT_VALUE, None, PropertyValue::Unsigned(2_000), None)
        .unwrap();
    assert_eq!(kelvin(&ct, P::PRESENT_VALUE), 2_000);
    assert_eq!(kelvin(&ct, P::TRACKING_VALUE), 3_000);
    assert_eq!(in_progress(&ct), 1);
    clock.set(500);
    assert_eq!(kelvin(&ct, P::TRACKING_VALUE), 2_500);
    clock.set(1_000);
    assert_eq!(in_progress(&ct), 0);
    assert_eq!(kelvin(&ct, P::TRACKING_VALUE), 2_000);
}

#[test]
fn limits_that_leave_out_a_fades_target_halt_it() {
    let (mut ct, clock) = temperature();
    let fade = BACnetColorCommand {
        fade_time: Some(10_000),
        ..cct(Op::FADE_TO_CCT, Some(5_000))
    };
    ct.set_color_command(fade).unwrap();
    clock.set(1_000);
    assert_eq!(kelvin(&ct, P::TRACKING_VALUE), 3_200);
    // 5000 K lies past the new maximum: Present_Value moves to 4000 K at
    // once, and the fade ends.
    ct.set_min_max(2_000, 4_000).unwrap();
    assert_eq!(kelvin(&ct, P::PRESENT_VALUE), 4_000);
    assert_eq!(kelvin(&ct, P::TRACKING_VALUE), 4_000);
    assert_eq!(in_progress(&ct), 0);
    assert_eq!(ct.next_monotonic_deadline_internal(), None);
}
