//! Lighting Output's trims (#1528, Addendum 135-2020ca part 5) on a hand-set
//! monotonic clock: the rows, Tracking_Value held between the trims with
//! In_Progress TRIM_ACTIVE, a trim change following Trim_Fade_Time, and
//! commands, fades, steps, STOP and an egress staying inside the trims.

use super::engine_tests::{op, Fixture};
use super::*;
use crate::property_metadata::PropertyPresenceCondition;
use bacnet_types::enums::{ErrorClass, ErrorCode, LightingInProgress, LightingOperation as Op};

const HIGH: PropertyIdentifier = PropertyIdentifier::HIGH_END_TRIM;
const LOW: PropertyIdentifier = PropertyIdentifier::LOW_END_TRIM;
const FADE_TIME: PropertyIdentifier = PropertyIdentifier::TRIM_FADE_TIME;

const IDLE: LightingInProgress = LightingInProgress::IDLE;
const FADE_ACTIVE: LightingInProgress = LightingInProgress::FADE_ACTIVE;
const TRIM_ACTIVE: LightingInProgress = LightingInProgress::TRIM_ACTIVE;

fn assert_error(result: Result<impl std::fmt::Debug, Error>, expected: ErrorCode) {
    let error = result.unwrap_err();
    assert!(
        matches!(error, Error::Protocol { class, code }
            if class == ErrorClass::PROPERTY.to_raw() as u32
                && code == expected.to_raw() as u32),
        "expected {expected:?}, got {error:?}"
    );
}

fn fade_to(level: f32, fade_time: u32, priority: u8) -> BACnetLightingCommand {
    BACnetLightingCommand {
        target_level: Some(level),
        fade_time: Some(fade_time),
        ..op(Op::FADE_TO, Some(priority))
    }
}

impl Fixture {
    /// A Lighting Output lit to `level` at priority 8.
    fn lit(level: f32) -> Self {
        let mut f = Self::new();
        f.present(PropertyValue::Real(level), 8);
        f
    }

    fn view(&self) -> (f32, f32, LightingInProgress) {
        (self.pv(), self.tv(), self.in_progress())
    }
}

#[test]
fn the_trim_rows_are_absent_until_set_then_present_and_writable() {
    let mut f = Fixture::new();
    for p in [HIGH, LOW, FADE_TIME] {
        assert!(!f.lo.property_list().contains(&p));
        assert_error(f.lo.read_property(p, None), ErrorCode::UNKNOWN_PROPERTY);
        assert_error(
            f.lo.write_property(p, None, PropertyValue::Real(50.0), None),
            ErrorCode::UNKNOWN_PROPERTY,
        );
    }
    // A high trim brings Trim_Fade_Time with it, which is then required.
    f.lo.set_high_end_trim(Some(80.0)).unwrap();
    let list = f.lo.property_list();
    assert!(list.contains(&HIGH) && list.contains(&FADE_TIME) && !list.contains(&LOW));
    assert!(f.lo.required_properties().contains(&FADE_TIME));
    assert!(!f.lo.required_properties().contains(&HIGH));
    let metadata = f.lo.property_metadata();
    let row = |p| {
        *metadata
            .iter()
            .find(|row| row.property_identifier == p)
            .unwrap()
    };
    assert_eq!(
        row(FADE_TIME).presence_condition,
        Some(PropertyPresenceCondition::LightingTrims)
    );
    assert!(f.lo.is_writable_property(HIGH) && f.lo.is_writable_property(FADE_TIME));
    assert_eq!(
        metadata.last().unwrap().property_identifier,
        PropertyIdentifier::PROPERTY_LIST
    );
    drop(metadata);
    assert_eq!(
        f.lo.read_property(HIGH, None).unwrap(),
        PropertyValue::Real(80.0)
    );
    assert_eq!(
        f.lo.read_property(FADE_TIME, None).unwrap(),
        PropertyValue::Unsigned(0)
    );
    assert_error(
        f.lo.write_property(LOW, None, PropertyValue::Real(5.0), None),
        ErrorCode::UNKNOWN_PROPERTY,
    );
    assert_error(f.lo.read_property(LOW, None), ErrorCode::UNKNOWN_PROPERTY);
    f.lo.set_low_end_trim(Some(10.0)).unwrap();
    f.write(LOW, PropertyValue::Real(5.0));
    f.write(HIGH, PropertyValue::Real(100.0));
    f.write(FADE_TIME, PropertyValue::Unsigned(86_400_000));
    assert_eq!(
        f.lo.read_property(LOW, None).unwrap(),
        PropertyValue::Real(5.0)
    );
    assert_eq!(
        f.lo.read_property(HIGH, None).unwrap(),
        PropertyValue::Real(100.0)
    );
    // Trims are 1.0 to 100.0 percent, and Trim_Fade_Time at most a day.
    for (p, value) in [
        (HIGH, PropertyValue::Real(0.5)),
        (HIGH, PropertyValue::Real(100.5)),
        (LOW, PropertyValue::Real(f32::NAN)),
        (LOW, PropertyValue::Real(0.0)),
        (FADE_TIME, PropertyValue::Unsigned(86_400_001)),
        (FADE_TIME, PropertyValue::Unsigned(u64::MAX)),
    ] {
        assert_error(
            f.lo.write_property(p, None, value, None),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    for (p, value) in [
        (HIGH, PropertyValue::Double(50.0)),
        (LOW, PropertyValue::Unsigned(50)),
        (FADE_TIME, PropertyValue::Real(100.0)),
    ] {
        assert_error(
            f.lo.write_property(p, None, value, None),
            ErrorCode::INVALID_DATA_TYPE,
        );
    }
    assert_eq!(
        f.lo.read_property(LOW, None).unwrap(),
        PropertyValue::Real(5.0)
    );
    // Taking both trims away takes Trim_Fade_Time with them.
    f.lo.set_high_end_trim(None).unwrap();
    f.lo.set_low_end_trim(None).unwrap();
    assert!(matches!(f.lo.property_metadata(), Cow::Borrowed(_)));
    assert_error(
        f.lo.read_property(FADE_TIME, None),
        ErrorCode::UNKNOWN_PROPERTY,
    );
}

#[test]
fn tracking_value_is_held_between_the_trims_and_off_stays_off() {
    let mut f = Fixture::new();
    f.lo.set_high_end_trim(Some(80.0)).unwrap();
    f.lo.set_low_end_trim(Some(20.0)).unwrap();
    for (level, tracking, in_progress) in [
        (90.0, 80.0, TRIM_ACTIVE),
        (80.0, 80.0, IDLE),
        (50.0, 50.0, IDLE),
        (10.0, 20.0, TRIM_ACTIVE),
        (1.0, 20.0, TRIM_ACTIVE),
        // Off isn't an on level below the low trim.
        (0.0, 0.0, IDLE),
    ] {
        f.present(PropertyValue::Real(level), 8);
        // Present_Value keeps the level as commanded.
        assert_eq!(f.view(), (level, tracking, in_progress), "{level}");
    }
}

#[test]
fn a_level_commanded_at_priority_1_or_2_is_not_held() {
    let mut f = Fixture::lit(90.0);
    f.lo.set_high_end_trim(Some(80.0)).unwrap();
    assert_eq!(f.view(), (90.0, 80.0, TRIM_ACTIVE));
    for priority in [1, 2] {
        f.present(PropertyValue::Real(95.0), priority);
        assert_eq!(f.view(), (95.0, 95.0, IDLE), "priority {priority}");
        f.present(PropertyValue::Null, priority);
    }
    f.present(PropertyValue::Real(95.0), 3);
    assert_eq!(f.view(), (95.0, 80.0, TRIM_ACTIVE));
}

#[test]
fn a_trim_change_moves_tracking_value_over_trim_fade_time() {
    let mut f = Fixture::lit(90.0);
    f.lo.set_trim_fade_time(2_000).unwrap();
    // The high bound moves from 100 down to 70 over 2 s; Tracking_Value
    // meets it at 90 after 2/3 s and follows it down.
    f.lo.set_high_end_trim(Some(70.0)).unwrap();
    assert_eq!(f.view(), (90.0, 90.0, TRIM_ACTIVE));
    assert!(f.deadline().is_some());
    f.at(1_000);
    assert_eq!(f.view(), (90.0, 85.0, TRIM_ACTIVE));
    f.at(1_500);
    assert_eq!(f.tv(), 77.5);
    f.at(2_000);
    assert_eq!(f.view(), (90.0, 70.0, TRIM_ACTIVE));
    assert!(f.advance());
    assert_eq!(f.deadline(), None);
    // Back up to 100: TRIM_ACTIVE while the bound still holds Tracking_Value
    // under Present_Value, then IDLE once it passes 90.
    f.lo.set_high_end_trim(Some(100.0)).unwrap();
    f.at(3_000);
    assert_eq!(f.view(), (90.0, 85.0, TRIM_ACTIVE));
    f.at(3_500);
    assert_eq!(f.view(), (90.0, 90.0, IDLE));
    f.at(4_000);
    assert!(f.advance());
    assert_eq!(f.deadline(), None);
    // With no Trim_Fade_Time a change shows at once.
    f.lo.set_trim_fade_time(0).unwrap();
    f.lo.set_high_end_trim(Some(60.0)).unwrap();
    assert_eq!(f.view(), (90.0, 60.0, TRIM_ACTIVE));
    assert_eq!(f.deadline(), None);
}

#[test]
fn a_trim_change_samples_tracking_value_by_its_step() {
    let mut f = Fixture::lit(90.0);
    f.lo.set_trim_fade_time(10_000).unwrap();
    // COV_Increment 0.0 samples each percent: 30 percent over 10 s is a
    // sample every 1/3 s, rounded up onto the 100 ms grid.
    f.lo.set_high_end_trim(Some(70.0)).unwrap();
    assert_eq!(f.deadline(), Some(Duration::from_millis(400)));
    f.at(400);
    assert!(f.advance());
    assert_eq!(f.deadline(), Some(Duration::from_millis(800)));
}

#[test]
fn a_fade_runs_behind_a_trim_and_steps_and_stop_work_from_the_held_level() {
    let mut f = Fixture::new();
    f.lo.set_high_end_trim(Some(80.0)).unwrap();
    // FADE_TO 90 over 1 s at 9 percent a tenth: past the trim, so
    // TRIM_ACTIVE from the start, and Tracking_Value stops at 80.
    f.command(fade_to(90.0, 1_000, 8));
    assert_eq!(f.view(), (90.0, 0.0, TRIM_ACTIVE));
    f.at(500);
    assert_eq!(f.view(), (90.0, 45.0, TRIM_ACTIVE));
    f.at(900);
    assert_eq!(f.view(), (90.0, 80.0, TRIM_ACTIVE));
    f.at(1_000);
    assert!(f.advance());
    assert_eq!(f.view(), (90.0, 80.0, TRIM_ACTIVE));
    // A step works from the held level: 80 + 1 goes in the slot, which the
    // trim still holds at 80.
    f.command(BACnetLightingCommand {
        step_increment: Some(1.0),
        ..op(Op::STEP_UP, Some(8))
    });
    assert_eq!((f.slot(8), f.tv()), (Some(81.0), 80.0));
    // STOP during a fade leaves the held level in the slot.
    f.command(fade_to(100.0, 1_000, 8));
    f.command(fade_to(40.0, 1_000, 8));
    f.at(1_500);
    assert_eq!(f.view(), (40.0, 60.0, FADE_ACTIVE));
    f.command(op(Op::STOP, Some(8)));
    assert_eq!(f.view(), (60.0, 60.0, IDLE));
    assert_eq!(f.slot(8), Some(60.0));
}

#[test]
fn a_trim_change_during_a_fade_holds_the_fade_as_it_runs() {
    let mut f = Fixture::new();
    f.command(fade_to(90.0, 2_000, 8));
    f.at(1_000);
    assert_eq!(f.view(), (90.0, 45.0, FADE_ACTIVE));
    f.lo.set_high_end_trim(Some(40.0)).unwrap();
    assert_eq!(f.view(), (90.0, 40.0, TRIM_ACTIVE));
    // The fade runs on behind the trim.
    f.at(1_500);
    assert_eq!(f.tv(), 40.0);
    f.lo.set_high_end_trim(Some(100.0)).unwrap();
    assert_eq!(f.view(), (90.0, 67.5, FADE_ACTIVE));
    f.at(2_000);
    assert_eq!(f.view(), (90.0, 90.0, IDLE));
}

#[test]
fn a_high_trim_below_the_low_one_is_a_configuration_error_and_holds_nothing() {
    let mut f = Fixture::lit(90.0);
    f.lo.set_low_end_trim(Some(40.0)).unwrap();
    f.lo.set_high_end_trim(Some(30.0)).unwrap();
    let reliability = |f: &Fixture| f.lo.read_property(PropertyIdentifier::RELIABILITY, None);
    assert_eq!(
        reliability(&f).unwrap(),
        PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw())
    );
    // Status_Flags shows the fault.
    assert_eq!(
        f.lo.read_property(PropertyIdentifier::STATUS_FLAGS, None)
            .unwrap(),
        PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0x40],
        }
    );
    assert_eq!(f.view(), (90.0, 90.0, IDLE));
    f.lo.set_high_end_trim(Some(60.0)).unwrap();
    assert_eq!(
        reliability(&f).unwrap(),
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
    );
    assert_eq!(f.view(), (90.0, 60.0, TRIM_ACTIVE));
}

#[test]
fn an_egress_holds_the_trimmed_level_and_ends_off() {
    let mut f = Fixture::lit(90.0);
    f.lo.set_high_end_trim(Some(80.0)).unwrap();
    f.write(
        PropertyIdentifier::BLINK_WARN_ENABLE,
        PropertyValue::Boolean(true),
    );
    f.write(PropertyIdentifier::EGRESS_TIME, PropertyValue::Unsigned(2));
    f.command(op(Op::WARN_OFF, Some(8)));
    assert_eq!(f.view(), (90.0, 80.0, TRIM_ACTIVE));
    f.at(2_000);
    assert!(f.advance());
    assert_eq!(f.view(), (0.0, 0.0, IDLE));
}

fn step(operation: Op, increment: f32, priority: u8) -> BACnetLightingCommand {
    BACnetLightingCommand {
        step_increment: Some(increment),
        ..op(operation, Some(priority))
    }
}

#[test]
fn step_off_turns_off_from_the_low_trim_and_step_down_stops_at_one() {
    let mut f = Fixture::lit(50.0);
    f.lo.set_low_end_trim(Some(20.0)).unwrap();
    // STEP_DOWN keeps the table's floor of 1.0: Present_Value goes below the
    // trim, which holds Tracking_Value at it.
    f.command(step(Op::STEP_DOWN, 40.0, 8));
    assert_eq!(f.view(), (10.0, 20.0, TRIM_ACTIVE));
    f.command(step(Op::STEP_DOWN, 1.0, 8));
    assert_eq!(f.view(), (19.0, 20.0, TRIM_ACTIVE));
    // Above the trim STEP_OFF steps down; at it, it turns the light off.
    f.present(PropertyValue::Real(25.0), 8);
    f.command(step(Op::STEP_OFF, 1.0, 8));
    assert_eq!(f.view(), (24.0, 24.0, IDLE));
    f.present(PropertyValue::Real(20.0), 8);
    f.command(step(Op::STEP_OFF, 1.0, 8));
    assert_eq!(f.view(), (0.0, 0.0, IDLE));
    // From a held level below the trim too.
    f.present(PropertyValue::Real(5.0), 8);
    f.command(step(Op::STEP_OFF, 1.0, 8));
    assert_eq!(f.view(), (0.0, 0.0, IDLE));
    // With the trims standing aside at priority 2, the floor is 1.0 again.
    f.present(PropertyValue::Real(20.0), 2);
    f.command(step(Op::STEP_OFF, 1.0, 2));
    assert_eq!(f.view(), (19.0, 19.0, IDLE));
    f.present(PropertyValue::Real(1.0), 2);
    f.command(step(Op::STEP_OFF, 1.0, 2));
    assert_eq!(f.slot(2), Some(0.0));
    // A high trim alone leaves the floor at 1.0.
    let mut f = Fixture::lit(1.0);
    f.lo.set_high_end_trim(Some(80.0)).unwrap();
    f.command(step(Op::STEP_OFF, 1.0, 8));
    assert_eq!(f.view(), (0.0, 0.0, IDLE));
}

#[test]
fn a_fade_or_ramp_up_from_off_starts_at_the_low_trim() {
    let mut f = Fixture::new();
    f.lo.set_low_end_trim(Some(20.0)).unwrap();
    // FADE_TO 50 over 1 s runs 20 to 50 over the whole second.
    f.command(fade_to(50.0, 1_000, 8));
    assert_eq!(f.view(), (50.0, 20.0, FADE_ACTIVE));
    f.at(500);
    assert_eq!(f.view(), (50.0, 35.0, FADE_ACTIVE));
    f.at(1_000);
    assert_eq!(f.view(), (50.0, 50.0, IDLE));
    assert!(f.advance());
    // RAMP_TO 50 at 10 percent a second from off: 20 to 50 is 3 s.
    f.present(PropertyValue::Real(0.0), 8);
    f.command(BACnetLightingCommand {
        target_level: Some(50.0),
        ramp_rate: Some(10.0),
        ..op(Op::RAMP_TO, Some(8))
    });
    assert_eq!(f.view(), (50.0, 20.0, LightingInProgress::RAMP_ACTIVE));
    f.at(2_500);
    assert_eq!(f.view(), (50.0, 35.0, LightingInProgress::RAMP_ACTIVE));
    f.at(4_000);
    assert_eq!(f.view(), (50.0, 50.0, IDLE));
}

#[test]
fn a_fade_down_to_off_ends_at_the_low_trim_then_goes_off() {
    let mut f = Fixture::lit(60.0);
    f.lo.set_low_end_trim(Some(20.0)).unwrap();
    // FADE_TO 0 over 1 s runs 60 to 20 over the whole second, then off.
    f.command(fade_to(0.0, 1_000, 8));
    assert_eq!(f.view(), (0.0, 60.0, FADE_ACTIVE));
    f.at(500);
    assert_eq!(f.view(), (0.0, 40.0, FADE_ACTIVE));
    f.at(999);
    assert!(f.tv() > 20.0 && f.in_progress() == FADE_ACTIVE);
    // At the fade time it reads off, before the task has run.
    f.at(1_000);
    assert_eq!(f.view(), (0.0, 0.0, IDLE));
    assert!(f.advance());
    assert_eq!((f.view(), f.deadline()), ((0.0, 0.0, IDLE), None));
    // From the low trim itself it stays there for the fade, then goes off.
    f.present(PropertyValue::Real(20.0), 8);
    f.command(fade_to(0.0, 1_000, 8));
    f.at(1_500);
    assert_eq!(f.view(), (0.0, 20.0, FADE_ACTIVE));
    f.at(2_000);
    assert_eq!(f.view(), (0.0, 0.0, IDLE));
}
