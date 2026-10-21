//! Lighting Output carrying out FADE_TO, RAMP_TO, the steps and STOP, and
//! halting a command in progress (#1384; Clause 12.54, Tables 12-67 and
//! 12-68, Clause 12.54.6.1), on a hand-set monotonic clock.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::*;
use bacnet_types::enums::{LightingInProgress, LightingOperation as Op};

pub(super) const PV: PropertyIdentifier = PropertyIdentifier::PRESENT_VALUE;
const TV: PropertyIdentifier = PropertyIdentifier::TRACKING_VALUE;
const PA: PropertyIdentifier = PropertyIdentifier::PRIORITY_ARRAY;

const IDLE: LightingInProgress = LightingInProgress::IDLE;
const FADE_ACTIVE: LightingInProgress = LightingInProgress::FADE_ACTIVE;
const RAMP_ACTIVE: LightingInProgress = LightingInProgress::RAMP_ACTIVE;

fn ms(milliseconds: u64) -> Duration {
    Duration::from_millis(milliseconds)
}

/// A Lighting Output on a monotonic clock the test sets by hand.
pub(super) struct Fixture {
    pub(super) lo: LightingOutputObject,
    now: Arc<Mutex<Duration>>,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let mut lo = LightingOutputObject::new(1, "LO-1").unwrap();
        let now = Arc::new(Mutex::new(Duration::ZERO));
        let source = Arc::clone(&now);
        lo.bind_monotonic_clock_internal(Some(Arc::new(move || *source.lock().unwrap())));
        Self { lo, now }
    }

    /// Set the clock to `milliseconds`.
    pub(super) fn at(&mut self, milliseconds: u64) {
        *self.now.lock().unwrap() = ms(milliseconds);
    }

    /// Write Lighting_Command.
    pub(super) fn command(&mut self, command: BACnetLightingCommand) {
        self.lo.set_lighting_command(command).unwrap();
    }

    /// Write Present_Value at `priority`.
    pub(super) fn present(&mut self, value: PropertyValue, priority: u8) {
        self.lo
            .write_property_from(
                PV,
                None,
                value,
                Some(priority),
                &crate::command_source::test_origin(),
            )
            .unwrap();
    }

    pub(super) fn write(&mut self, property: PropertyIdentifier, value: PropertyValue) {
        self.lo.write_property(property, None, value, None).unwrap();
    }

    fn real(&self, property: PropertyIdentifier) -> f32 {
        match self.lo.read_property(property, None).unwrap() {
            PropertyValue::Real(value) => value,
            other => panic!("{property:?} read {other:?}"),
        }
    }

    pub(super) fn pv(&self) -> f32 {
        self.real(PV)
    }

    pub(super) fn tv(&self) -> f32 {
        self.real(TV)
    }

    pub(super) fn in_progress(&self) -> LightingInProgress {
        match self
            .lo
            .read_property(PropertyIdentifier::IN_PROGRESS, None)
            .unwrap()
        {
            PropertyValue::Enumerated(value) => LightingInProgress::from_raw(value),
            other => panic!("In_Progress read {other:?}"),
        }
    }

    pub(super) fn slot(&self, priority: u8) -> Option<f32> {
        match self
            .lo
            .read_property(PA, Some(u32::from(priority)))
            .unwrap()
        {
            PropertyValue::Real(value) => Some(value),
            PropertyValue::Null => None,
            other => panic!("slot {priority} read {other:?}"),
        }
    }

    pub(super) fn egress_active(&self) -> bool {
        self.lo
            .read_property(PropertyIdentifier::EGRESS_ACTIVE, None)
            .unwrap()
            == PropertyValue::Boolean(true)
    }

    /// Advance the object to the clock, as the server's task does.
    pub(super) fn advance(&mut self) -> bool {
        let now = *self.now.lock().unwrap();
        self.lo.advance_monotonic_time_internal(now)
    }

    pub(super) fn deadline(&self) -> Option<Duration> {
        self.lo.next_monotonic_deadline_internal()
    }
}

/// A command for `operation` at `priority`, or the default priority.
pub(super) fn op(operation: Op, priority: Option<u8>) -> BACnetLightingCommand {
    BACnetLightingCommand {
        priority,
        ..BACnetLightingCommand::new(operation)
    }
}

fn fade_to(level: f32, fade_time: Option<u32>, priority: Option<u8>) -> BACnetLightingCommand {
    BACnetLightingCommand {
        target_level: Some(level),
        fade_time,
        ..op(Op::FADE_TO, priority)
    }
}

fn ramp_to(level: f32, ramp_rate: Option<f32>, priority: Option<u8>) -> BACnetLightingCommand {
    BACnetLightingCommand {
        target_level: Some(level),
        ramp_rate,
        ..op(Op::RAMP_TO, priority)
    }
}

fn step(operation: Op, step_increment: Option<f32>, priority: u8) -> BACnetLightingCommand {
    BACnetLightingCommand {
        step_increment,
        ..op(operation, Some(priority))
    }
}

fn assert_near(read: f32, expected: f32) {
    assert!((read - expected).abs() < 1e-3, "{read} != {expected}");
}

#[test]
fn fade_to_moves_tracking_value_over_its_fade_time_then_goes_idle() {
    let mut f = Fixture::new();
    f.present(PropertyValue::Real(20.0), 16);
    f.at(1_000);
    // No priority: Lighting_Command_Default_Priority, 16.
    f.command(fade_to(70.0, Some(2_000), None));
    // The target is in the slot and Present_Value at once.
    assert_eq!([f.slot(16), Some(f.pv())], [Some(70.0); 2]);
    assert_eq!((f.tv(), f.in_progress()), (20.0, FADE_ACTIVE));
    f.at(2_000);
    assert_eq!((f.tv(), f.in_progress()), (45.0, FADE_ACTIVE));
    f.at(2_999);
    assert!(f.tv() < 70.0);
    assert_eq!(f.in_progress(), FADE_ACTIVE);
    // At the fade time it reads as arrived, before the task has run.
    f.at(3_000);
    assert_eq!((f.tv(), f.in_progress()), (70.0, IDLE));
    assert!(f.advance());
    assert_eq!(f.deadline(), None);
    assert_eq!((f.tv(), f.pv(), f.in_progress()), (70.0, 70.0, IDLE));
}

#[test]
fn fade_to_without_a_fade_time_takes_default_fade_time() {
    let mut f = Fixture::new();
    f.lo.set_default_fade_time(4_000).unwrap();
    f.command(fade_to(80.0, None, Some(8)));
    assert_eq!(f.slot(8), Some(80.0));
    f.at(1_000);
    assert_eq!((f.tv(), f.in_progress()), (20.0, FADE_ACTIVE));
    f.at(4_000);
    assert_eq!((f.tv(), f.in_progress()), (80.0, IDLE));
}

#[test]
fn ramp_to_moves_at_its_ramp_rate_or_default_ramp_rate() {
    let mut f = Fixture::new();
    f.present(PropertyValue::Real(10.0), 16);
    // 50 percent at 20 percent a second: 2.5 s.
    f.command(ramp_to(60.0, Some(20.0), None));
    assert_eq!((f.pv(), f.tv(), f.in_progress()), (60.0, 10.0, RAMP_ACTIVE));
    f.at(1_000);
    assert_near(f.tv(), 30.0);
    f.at(2_499);
    assert_eq!(f.in_progress(), RAMP_ACTIVE);
    f.at(2_500);
    assert_eq!((f.tv(), f.in_progress()), (60.0, IDLE));
    // Down at Default_Ramp_Rate, 10 percent a second: 2 s.
    f.lo.set_default_ramp_rate(10.0).unwrap();
    f.command(ramp_to(40.0, None, None));
    f.at(3_500);
    assert_near(f.tv(), 50.0);
    assert_eq!(f.in_progress(), RAMP_ACTIVE);
    f.at(4_500);
    assert_eq!((f.tv(), f.in_progress()), (40.0, IDLE));
}

#[test]
fn fade_to_below_one_percent_puts_one_percent_in_the_slot_and_keeps_the_command() {
    let mut f = Fixture::new();
    f.present(PropertyValue::Real(50.0), 16);
    let command = fade_to(0.5, Some(1_000), None);
    f.command(command);
    // Clause 12.54.4 takes the level as 1.0; Lighting_Command reports the
    // command as it was written.
    assert_eq!([f.slot(16), Some(f.pv())], [Some(1.0); 2]);
    assert_eq!(f.lo.lighting_command(), command);
    f.at(500);
    assert_eq!(f.tv(), 25.5);
    f.at(1_000);
    assert_eq!((f.tv(), f.in_progress()), (1.0, IDLE));
}

#[test]
fn a_fade_below_the_highest_priority_only_writes_its_slot() {
    let mut f = Fixture::new();
    f.present(PropertyValue::Real(80.0), 4);
    f.command(fade_to(30.0, Some(1_000), Some(8)));
    assert_eq!(f.slot(8), Some(30.0));
    assert_eq!((f.pv(), f.tv(), f.in_progress()), (80.0, 80.0, IDLE));
    assert_eq!(f.deadline(), None);
    // Relinquishing priority 4 shows the slot at once: nothing fades.
    f.present(PropertyValue::Null, 4);
    assert_eq!((f.pv(), f.tv(), f.in_progress()), (30.0, 30.0, IDLE));
}

#[test]
fn a_fade_or_ramp_to_the_level_it_stands_at_is_idle() {
    let mut f = Fixture::new();
    f.present(PropertyValue::Real(40.0), 16);
    f.command(fade_to(40.0, Some(5_000), None));
    assert_eq!((f.in_progress(), f.deadline()), (IDLE, None));
    f.command(ramp_to(40.0, Some(1.0), None));
    assert_eq!((f.in_progress(), f.deadline()), (IDLE, None));
}

#[test]
fn steps_follow_tracking_value_with_their_clamps_and_zero_level_rules() {
    let cases = [
        (50.0, Op::STEP_UP, 60.0),
        (95.0, Op::STEP_UP, 100.0),
        (0.0, Op::STEP_UP, 0.0),
        (50.0, Op::STEP_DOWN, 40.0),
        (5.0, Op::STEP_DOWN, 1.0),
        (0.0, Op::STEP_DOWN, 0.0),
        (0.0, Op::STEP_ON, 1.0),
        (50.0, Op::STEP_ON, 60.0),
        (1.0, Op::STEP_OFF, 0.0),
        (50.0, Op::STEP_OFF, 40.0),
        (5.0, Op::STEP_OFF, 1.0),
        (0.0, Op::STEP_OFF, 0.0),
    ];
    for (start, operation, expected) in cases {
        let mut f = Fixture::new();
        f.present(PropertyValue::Real(start), 16);
        f.command(step(operation, Some(10.0), 16));
        let case = format!("{operation:?} from {start}");
        assert_eq!(f.slot(16), Some(expected), "{case}");
        assert_eq!(
            (f.pv(), f.tv(), f.in_progress()),
            (expected, expected, IDLE)
        );
    }
    // Default_Step_Increment, and a step at another priority starts from
    // Tracking_Value all the same.
    let mut f = Fixture::new();
    f.lo.set_default_step_increment(2.5).unwrap();
    f.present(PropertyValue::Real(50.0), 16);
    f.command(step(Op::STEP_UP, None, 8));
    assert_eq!((f.slot(8), f.slot(16)), (Some(52.5), Some(50.0)));
    assert_eq!(f.pv(), 52.5);
}

#[test]
fn a_step_during_a_fade_starts_from_tracking_value_and_halts_the_fade() {
    let mut f = Fixture::new();
    f.command(fade_to(100.0, Some(10_000), None));
    f.at(2_500);
    assert_eq!(f.tv(), 25.0);
    f.command(step(Op::STEP_UP, Some(10.0), 16));
    assert_eq!(f.slot(16), Some(35.0));
    assert_eq!((f.pv(), f.tv(), f.in_progress()), (35.0, 35.0, IDLE));
    assert_eq!(f.deadline(), None);
}

#[test]
fn stop_ends_a_fade_with_tracking_value_in_its_slot() {
    let mut f = Fixture::new();
    f.command(fade_to(100.0, Some(10_000), Some(8)));
    f.at(4_000);
    // A STOP at another priority, and one with no priority (16), find
    // nothing there to stop.
    f.command(op(Op::STOP, Some(3)));
    f.command(op(Op::STOP, None));
    assert_eq!((f.slot(8), f.in_progress()), (Some(100.0), FADE_ACTIVE));
    f.command(op(Op::STOP, Some(8)));
    assert_eq!(f.slot(8), Some(40.0));
    assert_eq!((f.pv(), f.tv(), f.in_progress()), (40.0, 40.0, IDLE));
    assert_eq!(f.deadline(), None);
    // With nothing in progress STOP is ignored.
    f.at(5_000);
    f.command(op(Op::STOP, Some(8)));
    assert_eq!((f.slot(8), f.pv()), (Some(40.0), 40.0));

    // A Tracking_Value between off and 1.0 goes into the slot as 1.0.
    let mut f = Fixture::new();
    f.command(fade_to(50.0, Some(10_000), None));
    f.at(100);
    assert_eq!(f.tv(), 0.5);
    f.command(op(Op::STOP, None));
    assert_eq!((f.slot(16), f.tv()), (Some(1.0), 1.0));
}

#[test]
fn a_higher_priority_present_value_write_halts_a_fade_where_its_slot_is() {
    let mut f = Fixture::new();
    f.command(fade_to(100.0, Some(10_000), Some(8)));
    f.at(2_000);
    f.present(PropertyValue::Real(30.0), 4);
    assert_eq!((f.pv(), f.tv(), f.in_progress()), (30.0, 30.0, IDLE));
    assert_eq!((f.slot(8), f.deadline()), (Some(100.0), None));
    // The halted fade doesn't come back: its slot shows at once.
    f.present(PropertyValue::Null, 4);
    assert_eq!((f.pv(), f.tv(), f.in_progress()), (100.0, 100.0, IDLE));
}

#[test]
fn a_higher_or_same_priority_command_replaces_a_fade_from_where_it_stands() {
    // Higher: a new fade from Tracking_Value at the moment of the write.
    let mut f = Fixture::new();
    f.command(fade_to(100.0, Some(10_000), Some(8)));
    f.at(5_000);
    f.command(fade_to(0.0, Some(1_000), Some(4)));
    assert_eq!((f.slot(4), f.slot(8)), (Some(0.0), Some(100.0)));
    f.at(5_500);
    assert_eq!((f.tv(), f.in_progress()), (25.0, FADE_ACTIVE));

    // Same priority: a ramp takes over from 30.
    let mut f = Fixture::new();
    f.command(fade_to(100.0, Some(10_000), Some(8)));
    f.at(3_000);
    f.command(ramp_to(50.0, Some(10.0), Some(8)));
    f.at(4_000);
    assert_near(f.tv(), 40.0);
    assert_eq!(f.in_progress(), RAMP_ACTIVE);
}

#[test]
fn a_same_priority_present_value_write_halts_a_fade() {
    let mut f = Fixture::new();
    f.command(fade_to(100.0, Some(10_000), Some(8)));
    f.at(3_000);
    f.present(PropertyValue::Real(60.0), 8);
    assert_eq!((f.pv(), f.tv(), f.in_progress()), (60.0, 60.0, IDLE));
    assert_eq!(f.deadline(), None);
}

#[test]
fn lower_priority_writes_leave_a_fade_running() {
    let mut f = Fixture::new();
    f.command(fade_to(100.0, Some(10_000), Some(8)));
    f.at(2_000);
    f.present(PropertyValue::Real(20.0), 10);
    f.command(fade_to(70.0, Some(1_000), Some(12)));
    f.command(step(Op::STEP_DOWN, Some(5.0), 14));
    assert_eq!((f.slot(10), f.slot(12)), (Some(20.0), Some(70.0)));
    // The step worked from the fade's Tracking_Value, 20.
    assert_eq!(f.slot(14), Some(15.0));
    assert_eq!((f.tv(), f.in_progress()), (20.0, FADE_ACTIVE));
    f.at(10_000);
    assert_eq!((f.pv(), f.tv(), f.in_progress()), (100.0, 100.0, IDLE));
}

#[test]
fn cov_samples_follow_cov_increment_on_the_grid_until_the_end() {
    // COV_Increment 0.0 samples each percent: 100 ms at 10 percent a second.
    let mut f = Fixture::new();
    f.command(fade_to(100.0, Some(10_000), None));
    assert_eq!(f.deadline(), Some(ms(100)));
    f.at(50);
    assert!(!f.advance());
    f.at(100);
    assert!(f.advance());
    assert_eq!(f.deadline(), Some(ms(200)));

    // COV_Increment 25: a quarter of a 1 s fade, rounded onto the 100 ms
    // grid, and the end.
    let mut f = Fixture::new();
    f.write(PropertyIdentifier::COV_INCREMENT, PropertyValue::Real(25.0));
    f.command(fade_to(100.0, Some(1_000), None));
    let mut samples = Vec::new();
    for at in (0..=1_200).step_by(50) {
        f.at(at);
        if f.advance() {
            samples.push((at, f.tv()));
        }
    }
    assert_eq!(
        samples,
        [(300, 30.0), (600, 60.0), (900, 90.0), (1_000, 100.0)]
    );
    assert_eq!(f.deadline(), None);
}

#[test]
fn a_cov_snapshot_reads_as_the_object_did_when_it_was_taken() {
    let mut f = Fixture::new();
    f.command(fade_to(100.0, Some(10_000), None));
    f.at(2_000);
    let snapshot = f.lo.cov_snapshot_internal().unwrap();
    f.at(5_000);
    assert_eq!(
        snapshot.read_property(TV, None).unwrap(),
        PropertyValue::Real(20.0)
    );
    assert_eq!(f.tv(), 50.0);
    f.at(10_000);
    assert_eq!(
        snapshot
            .read_property(PropertyIdentifier::IN_PROGRESS, None)
            .unwrap(),
        PropertyValue::Enumerated(FADE_ACTIVE.to_raw())
    );
}

#[test]
fn starting_a_fade_wakes_the_server_task_and_nothing_else_does() {
    let mut f = Fixture::new();
    let wakes = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&wakes);
    f.lo.bind_deadline_waker_internal(Some(Arc::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    })));
    f.present(PropertyValue::Real(40.0), 16);
    f.command(fade_to(40.0, Some(1_000), None));
    f.command(step(Op::STEP_UP, None, 16));
    assert_eq!(wakes.load(Ordering::SeqCst), 0);
    f.command(fade_to(80.0, Some(1_000), None));
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
}

#[test]
fn with_no_clock_bound_logical_time_drives_a_fade() {
    let mut lo = LightingOutputObject::new(1, "LO-1").unwrap();
    lo.set_lighting_command(fade_to(100.0, Some(1_000), None))
        .unwrap();
    assert!(lo.advance_time_internal(ms(500)));
    assert_eq!(
        lo.read_property(TV, None).unwrap(),
        PropertyValue::Real(50.0)
    );
    assert!(lo.advance_time_internal(ms(500)));
    assert_eq!(
        lo.read_property(PropertyIdentifier::IN_PROGRESS, None)
            .unwrap(),
        PropertyValue::Enumerated(IDLE.to_raw())
    );
    assert!(!lo.advance_time_internal(ms(500)));
}
