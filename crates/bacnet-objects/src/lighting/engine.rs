//! Lighting Output's command engine (#1384): carrying out a lighting command
//! written to Lighting_Command, or a blink-warn value written to
//! Present_Value, against the priority array (Clause 12.54, Tables 12-65 and
//! 12-67).
//!
//! The engine is pure: each entry point takes the instant it acts at, from
//! the monotonic clock the server binds or a hand-set one. Nothing ticks.
//! Tracking_Value and In_Progress are worked out from the operation in
//! progress and the instant they are read at, and the object reports the
//! instants it needs advancing at (a COV sample, the end of an egress) as its
//! monotonic deadline for the server's task to wake for.
//!
//! What each operation does:
//!
//! - FADE_TO and RAMP_TO put the target level in the slot. When that slot is
//!   then the highest one in use, Tracking_Value moves from where it stood to
//!   the level, over the fade time or at the ramp rate, and In_Progress shows
//!   FADE_ACTIVE or RAMP_ACTIVE until it arrives. Otherwise only the slot
//!   changes.
//! - The step operations put Tracking_Value plus or minus the step increment
//!   in the slot, kept within 1.0 to 100.0. STEP_UP and STEP_DOWN do nothing
//!   from off; STEP_ON turns off into 1.0 and STEP_OFF turns 1.0 into off.
//! - WARN, WARN_RELINQUISH and WARN_OFF (and Present_Value -1.0, -2.0 and
//!   -3.0, Table 12-65) blink where the table's conditions allow. WARN does
//!   nothing more; the other two then hold the level for Egress_Time seconds
//!   before relinquishing the slot or writing 0.0 to it (Clause 12.54.6.2).
//!   Where the conditions rule the blink out they act at once: WARN changes
//!   nothing, WARN_RELINQUISH relinquishes and WARN_OFF writes 0.0. With
//!   Blink_Warn_Enable FALSE they always act at once (Clause 12.54.13).
//! - STOP ends a fade or ramp at its priority, leaving Tracking_Value in the
//!   slot, or cancels an egress timer there, leaving the slot as it is.
//!
//! Only one operation is in progress at a time, and it always sits at the
//! highest priority in use. Clause 12.54.6.1 halts it when a command other
//! than STOP, or a Present_Value write, arrives at its priority or a higher
//! one: a halted fade or ramp leaves its slot as it is, and a halted egress
//! takes effect at once. A step the table says to ignore halts nothing, and
//! neither does a proprietary operation, which this object stores but
//! defines no action for.
//!
//! The trims (#1528, see `trim`) sit on top of all this: Tracking_Value as
//! reported is the engine's value held between them, and a trim change runs
//! alongside whatever operation is in progress.

use std::time::Duration;

use bacnet_types::constructed::BACnetLightingCommand;
use bacnet_types::enums::{LightingInProgress, LightingOperation as Op};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::{normalized_level, LightingOutputObject};
use crate::common;
use crate::transition::{Progress, Run, Transition, TransitionKind};

/// How far Tracking_Value moves between COV samples while COV_Increment is
/// 0.0: one percent of the range.
const DEFAULT_SAMPLE_STEP: f64 = 1.0;

/// The command in progress.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Operation {
    /// A FADE_TO or RAMP_TO moving Tracking_Value to its slot's level.
    Moving { priority: u8, run: Run<f32> },
    /// A WARN_RELINQUISH or WARN_OFF holding the level until `deadline`.
    Egress {
        priority: u8,
        then: AfterEgress,
        deadline: Duration,
    },
}

impl Operation {
    fn priority(&self) -> u8 {
        match *self {
            Self::Moving { priority, .. } | Self::Egress { priority, .. } => priority,
        }
    }
}

/// What a slot gets when its egress time runs out or is cut short.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AfterEgress {
    /// WARN_RELINQUISH: the slot is relinquished.
    Relinquish,
    /// WARN_OFF: the slot is set to 0.0.
    Off,
}

impl AfterEgress {
    fn slot(self) -> Option<f32> {
        match self {
            Self::Relinquish => None,
            Self::Off => Some(0.0),
        }
    }
}

/// A Present_Value write once its special values are told apart.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum PresentValueWrite {
    /// A level for the slot (normalized), or NULL to relinquish it.
    Level(Option<f32>),
    /// -1.0, -2.0 or -3.0: the warn command of Table 12-65.
    Warn(Op),
}

impl PresentValueWrite {
    /// Decode a written Present_Value: NULL, a level from 0.0 to 100.0, or
    /// one of the three special values. Any other REAL is
    /// VALUE_OUT_OF_RANGE, and any other datatype INVALID_DATA_TYPE.
    pub(super) fn decode(value: PropertyValue) -> Result<Self, Error> {
        match value {
            PropertyValue::Null => Ok(Self::Level(None)),
            PropertyValue::Real(level) => Ok(match warn_operation(level) {
                Some(operation) => Self::Warn(operation),
                None => Self::Level(Some(normalized_level(level)?)),
            }),
            _ => Err(common::invalid_data_type_error()),
        }
    }
}

/// The warn command a special Present_Value stands for (Table 12-65).
fn warn_operation(value: f32) -> Option<Op> {
    if value == -1.0 {
        Some(Op::WARN)
    } else if value == -2.0 {
        Some(Op::WARN_RELINQUISH)
    } else if value == -3.0 {
        Some(Op::WARN_OFF)
    } else {
        None
    }
}

/// A level the engine works out, put in a slot the way a commanded level is
/// (#1385). Every such level already lies within 0.0 to 100.0; the clamp only
/// keeps rounding from carrying it out.
fn engine_level(value: f32) -> f32 {
    normalized_level(value.clamp(0.0, 100.0)).unwrap_or(0.0)
}

/// The level a step operation writes from `tracking`, or `None` when the
/// operation is to be ignored (Table 12-67). `floor` is the dimmest on level
/// in effect, 1.0 or the low trim (#1528), from which STEP_OFF turns off.
fn step_level(operation: Op, tracking: f32, increment: f32, floor: f32) -> Option<f32> {
    let up = (tracking + increment).min(100.0);
    let down = (tracking - increment).max(1.0);
    match operation {
        Op::STEP_UP => (tracking != 0.0).then_some(up),
        Op::STEP_DOWN => (tracking != 0.0).then_some(down),
        Op::STEP_ON => Some(if tracking == 0.0 { 1.0 } else { up }),
        Op::STEP_OFF if tracking == floor => Some(0.0),
        Op::STEP_OFF => (tracking != 0.0).then_some(down),
        _ => None,
    }
}

impl LightingOutputObject {
    /// The instant on the bound monotonic clock, or the logical time
    /// `advance_time_internal` keeps without one.
    pub(super) fn now(&self) -> Duration {
        self.monotonic_clock
            .as_ref()
            .map_or(self.logical_now, |clock| clock())
    }

    /// Tracking_Value at `now`: the fade or ramp's value while one runs,
    /// otherwise Present_Value (Clause 12.54.5), held within the trims
    /// (#1528).
    pub(super) fn tracking_value_at(&self, now: Duration) -> f32 {
        self.trimmed(self.untrimmed_at(now), now)
    }

    /// Tracking_Value at `now` before the trims hold it: Present_Value once
    /// a fade or ramp has arrived, as one down to off under a low trim ends
    /// its line at the trim (#1528).
    fn untrimmed_at(&self, now: Duration) -> f32 {
        match self.operation {
            Some(Operation::Moving { run, .. }) if !run.transition().is_finished(now) => {
                run.transition().value_at(now)
            }
            _ => self.present_value,
        }
    }

    /// In_Progress at `now`: TRIM_ACTIVE while the trims hold Tracking_Value
    /// apart from Present_Value, whatever else runs (#1528); otherwise
    /// FADE_ACTIVE or RAMP_ACTIVE while a fade or ramp is still moving, and
    /// IDLE.
    pub(super) fn in_progress_at(&self, now: Duration) -> LightingInProgress {
        if self.trim_active(self.untrimmed_at(now), now) {
            return LightingInProgress::TRIM_ACTIVE;
        }
        match self.operation {
            Some(Operation::Moving { run, .. }) if !run.transition().is_finished(now) => {
                match run.transition().kind() {
                    TransitionKind::Fade => LightingInProgress::FADE_ACTIVE,
                    TransitionKind::Ramp => LightingInProgress::RAMP_ACTIVE,
                }
            }
            _ => LightingInProgress::IDLE,
        }
    }

    /// Egress_Active: an egress timer is running.
    pub(super) fn egress_active(&self) -> bool {
        matches!(self.operation, Some(Operation::Egress { .. }))
    }

    /// Carry out a written Lighting_Command at `now`, at its priority or
    /// Lighting_Command_Default_Priority.
    pub(super) fn execute(&mut self, command: &BACnetLightingCommand, now: Duration) {
        let priority = command
            .priority
            .unwrap_or(self.lighting_command_default_priority as u8);
        self.advance_to(now);
        self.operate(command, priority, now);
    }

    /// Carry out a Present_Value write at `priority` and `now`.
    pub(super) fn write_present_value(
        &mut self,
        priority: u8,
        write: PresentValueWrite,
        now: Duration,
    ) {
        self.advance_to(now);
        match write {
            PresentValueWrite::Level(level) => {
                self.halt_for(priority);
                self.set_slot(priority, level);
            }
            PresentValueWrite::Warn(operation) => {
                self.operate(&BACnetLightingCommand::new(operation), priority, now);
            }
        }
    }

    /// Advance to `now`: finish a fade or ramp that has arrived, run out an
    /// egress timer that is due, move a trim change on, and take a COV sample
    /// that is due. `true` when something a COV report could carry changed.
    pub(super) fn advance_to(&mut self, now: Duration) -> bool {
        let step = self.sample_step();
        let trims = self.trims.advance(now, step);
        let operation = match self.operation {
            Some(Operation::Egress {
                priority,
                then,
                deadline,
            }) if now >= deadline => {
                self.operation = None;
                self.set_slot(priority, then.slot());
                true
            }
            Some(Operation::Moving { priority, mut run }) => {
                let progress = run.advance(now, step);
                // Even with nothing due, a finer step may have brought the
                // next sample forward, so the run goes back either way.
                self.operation =
                    (progress != Progress::Finished).then_some(Operation::Moving { priority, run });
                progress != Progress::Pending
            }
            _ => false,
        };
        trims || operation
    }

    /// The next instant the engine needs advancing at, if any.
    pub(super) fn next_deadline(&self) -> Option<Duration> {
        let operation = self.operation.map(|operation| match operation {
            Operation::Moving { run, .. } => run.deadline(),
            Operation::Egress { deadline, .. } => deadline,
        });
        match (operation, self.trims.deadline()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Carry out `command`'s operation at `priority`.
    fn operate(&mut self, command: &BACnetLightingCommand, priority: u8, now: Duration) {
        let tracking = self.tracking_value_at(now);
        let operation = command.operation;
        match operation {
            Op::FADE_TO | Op::RAMP_TO => {
                let Some(target) = command.target_level else {
                    return;
                };
                self.halt_for(priority);
                let level = engine_level(target);
                self.set_slot(priority, Some(level));
                if self.highest_priority() != Some(priority) {
                    return;
                }
                let (from, to) = self.trimmed_line(tracking, level, now);
                let transition = if operation == Op::FADE_TO {
                    let fade_time = command.fade_time.unwrap_or(self.default_fade_time);
                    let fade_time = Duration::from_millis(u64::from(fade_time));
                    Transition::fade(from, to, now, fade_time)
                } else {
                    let rate = command.ramp_rate.unwrap_or(self.default_ramp_rate);
                    Transition::ramp(from, to, now, f64::from(rate))
                };
                if let Some(transition) = transition {
                    let run = Run::start(transition, self.sample_step());
                    self.operation = Some(Operation::Moving { priority, run });
                    self.wake();
                }
            }
            Op::STEP_UP | Op::STEP_DOWN | Op::STEP_ON | Op::STEP_OFF => {
                let increment = command
                    .step_increment
                    .unwrap_or(self.default_step_increment);
                // During a WARN_OFF or WARN_RELINQUISH egress at this
                // priority, Tracking_Value is the level being held, so the
                // step works from it: the halt below gives the slot its
                // final value and the step result then replaces it. A step
                // during the warning thus keeps the lights on at the stepped
                // level, which is the override Clause 12.54.6.2 gives
                // occupants the egress time for, rather than stepping from
                // the off or relinquished level the egress was heading to.
                let floor = self.step_floor(now);
                if let Some(level) = step_level(operation, tracking, increment, floor) {
                    self.halt_for(priority);
                    self.set_slot(priority, Some(engine_level(level)));
                }
            }
            Op::WARN => {
                self.halt_for(priority);
                if self.blink_warn_enable
                    && self.highest_priority() == Some(priority)
                    && self.slot(priority) != Some(0.0)
                {
                    self.request_blink();
                }
            }
            Op::WARN_RELINQUISH => {
                self.halt_for(priority);
                let eligible = self.blink_warn_enable
                    && self.highest_priority() == Some(priority)
                    && self.slot(priority) != Some(0.0)
                    && self.next_level_below(priority) <= 0.0;
                self.warn_then(priority, AfterEgress::Relinquish, eligible, now);
            }
            Op::WARN_OFF => {
                self.halt_for(priority);
                let eligible = self.blink_warn_enable
                    && self.highest_priority() == Some(priority)
                    && self.present_value != 0.0;
                self.warn_then(priority, AfterEgress::Off, eligible, now);
            }
            Op::STOP => self.stop(priority, tracking),
            // A proprietary operation: stored, with no action defined here.
            _ => {}
        }
    }

    /// WARN_RELINQUISH or WARN_OFF: blink and start the egress timer when
    /// `eligible`, otherwise give the slot its final value now.
    fn warn_then(&mut self, priority: u8, then: AfterEgress, eligible: bool, now: Duration) {
        if !eligible {
            self.set_slot(priority, then.slot());
            return;
        }
        self.request_blink();
        let egress = Duration::from_secs(u64::from(self.egress_time));
        if egress.is_zero() {
            self.set_slot(priority, then.slot());
        } else {
            self.operation = Some(Operation::Egress {
                priority,
                then,
                deadline: now.saturating_add(egress),
            });
            self.wake();
        }
    }

    /// STOP at `priority`: end a fade or ramp there with `tracking` in its
    /// slot, or cancel an egress timer there. Anything else is ignored.
    fn stop(&mut self, priority: u8, tracking: f32) {
        match self.operation {
            Some(Operation::Moving { priority: at, .. }) if at == priority => {
                self.operation = None;
                self.set_slot(priority, Some(engine_level(tracking)));
            }
            Some(Operation::Egress { priority: at, .. }) if at == priority => {
                self.operation = None;
            }
            _ => {}
        }
    }

    /// Stop the running operation if a write at `priority` reaches it
    /// (Clause 12.54.6.1): a fade or ramp stops where its slot is, and an
    /// egress gives its slot the final value at once.
    fn halt_for(&mut self, priority: u8) {
        let Some(operation) = self.operation else {
            return;
        };
        if priority > operation.priority() {
            return;
        }
        self.operation = None;
        if let Operation::Egress { priority, then, .. } = operation {
            self.set_slot(priority, then.slot());
        }
    }

    fn slot(&self, priority: u8) -> Option<f32> {
        self.priority_array[usize::from(priority - 1)]
    }

    fn set_slot(&mut self, priority: u8, level: Option<f32>) {
        self.priority_array[usize::from(priority - 1)] = level;
        self.recalculate_present_value();
    }

    /// The highest priority with a level in its slot.
    pub(super) fn highest_priority(&self) -> Option<u8> {
        self.priority_array
            .iter()
            .position(Option::is_some)
            .map(|index| index as u8 + 1)
    }

    /// The level the next slot below `priority` holds, or Relinquish_Default
    /// when none does.
    fn next_level_below(&self, priority: u8) -> f32 {
        self.priority_array[usize::from(priority)..]
            .iter()
            .flatten()
            .next()
            .copied()
            .unwrap_or(self.relinquish_default)
    }

    fn request_blink(&mut self) {
        self.blink_request_count = self.blink_request_count.saturating_add(1);
    }

    /// How far Tracking_Value moves between COV samples: COV_Increment, or
    /// one percent while that is 0.0, or a Tracking_Value subscriber's own
    /// increment where that is finer (#1510).
    pub(super) fn sample_step(&self) -> f64 {
        let own = if self.cov_increment > 0.0 {
            f64::from(self.cov_increment)
        } else {
            DEFAULT_SAMPLE_STEP
        };
        self.finest_tracking_increment
            .map_or(own, |finest| own.min(finest))
    }

    /// Tell the server's monotonic task a deadline was armed.
    pub(super) fn wake(&self) {
        if let Some(waker) = &self.deadline_waker {
            waker();
        }
    }
}
