//! Lighting Output's trims (#1528, Addendum 135-2020ca part 5).
//!
//! High_End_Trim and Low_End_Trim bound an operating range inside the
//! normalized 1.0 to 100.0 percent range, and Tracking_Value is held in it:
//! a level above the high trim tracks at the high trim, an on level below
//! the low trim at the low trim, and off stays off. Present_Value keeps the
//! level as commanded. In_Progress reads TRIM_ACTIVE, whatever fade or ramp
//! is running, while Present_Value lies outside the trims (Table 12-68 as
//! the addendum changes it), and while moving bounds (below) hold
//! Tracking_Value back; that second case is this implementation's, so that
//! IDLE always means Tracking_Value equals Present_Value. The trims stand
//! aside while Present_Value comes from slot 1 or 2 (Clauses 12.54.Y1 and
//! 12.54.Y2).
//!
//! A trim change doesn't move Tracking_Value at once. The bounds themselves
//! move, in a straight line from where they stood to the new trims over
//! Trim_Fade_Time (Clause 12.54.Y3), on the shared `transition` module, and
//! Tracking_Value follows them. The clause leaves the manner to the
//! implementation; moving the bounds means a trim change during a fade or
//! ramp needs no special case, as the fade runs on and is held by bounds
//! that are themselves moving.
//!
//! A command works from Tracking_Value as the object reports it, so a step
//! or a fade starts from the held level, and STOP leaves the held level in
//! the slot. A fade to a level past a trim runs its course behind the trim:
//! Tracking_Value reaches the trim early and stays there.
//!
//! Two choices go past the addendum, which leaves fades under a trim to the
//! implementation (Clauses 12.54.Y1 and 12.54.Y2):
//!
//! - The levels between off and the low trim are outside the operating
//!   range, so a fade or ramp doesn't crawl through them. One up from off
//!   starts at the low trim and runs from there to its target over its whole
//!   time; one down to off runs to the low trim over its whole time and goes
//!   off as it ends. Either reads FADE_ACTIVE or RAMP_ACTIVE.
//! - The dimmest on level a step knows is the low trim while one applies, not
//!   1.0, so STEP_OFF turns the light off from the low trim. STEP_DOWN still
//!   stops at 1.0, as Table 12-67 has it, which puts Present_Value below the
//!   trim while Tracking_Value stays at it.
//!
//! Steps follow the table otherwise, from the held level. So a STEP_UP while
//! Present_Value is above the high trim writes the trim plus the increment,
//! which lowers Present_Value.
//!
//! A high trim below the low one is a configuration error: Reliability says
//! so (Clauses 12.54.Y1 and 12.54.Y2), and the trims hold nothing until it's
//! put right. Putting it right gives back the Reliability the trims replaced.

use std::ops::RangeInclusive;
use std::time::Duration;

use bacnet_types::enums::Reliability;
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::LightingOutputObject;
use crate::common;
use crate::transition::{Progress, Run, Transition, TransitionLevel};

/// Trim_Fade_Time's range in milliseconds, none to a day (Clause 12.54.Y3).
const TRIM_FADE_TIME_MS: RangeInclusive<u32> = 0..=86_400_000;

/// The bounds of the operating range, in percent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Bounds {
    low: f32,
    high: f32,
}

impl Bounds {
    /// No bounds at all. The low one is 0.0 rather than the dimmest on
    /// level, 1.0, so that with no low trim a fade up from off still passes
    /// through the levels below 1.0 as it always has.
    const FULL: Self = Self {
        low: 0.0,
        high: 100.0,
    };

    /// `level` held within the bounds; off isn't an on level, so it stays.
    fn hold(self, level: f32) -> f32 {
        if level > 0.0 {
            level.clamp(self.low, self.high)
        } else {
            level
        }
    }
}

/// Moving bounds: each moves on its own straight line, and they are as far
/// apart as the bound that moves further, which spaces the COV samples of a
/// Tracking_Value that follows it.
impl TransitionLevel for Bounds {
    fn interpolate(from: Self, to: Self, fraction: f64) -> Self {
        Self {
            low: f32::interpolate(from.low, to.low, fraction),
            high: f32::interpolate(from.high, to.high, fraction),
        }
    }

    fn distance(from: Self, to: Self) -> f64 {
        f32::distance(from.low, to.low).max(f32::distance(from.high, to.high))
    }
}

/// The trims as configured, and a change of them under way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Trims {
    /// High_End_Trim, present once set.
    high_end: Option<f32>,
    /// Low_End_Trim, present once set.
    low_end: Option<f32>,
    /// Trim_Fade_Time in milliseconds, present with either trim.
    fade_time: u32,
    /// The bounds moving to the configured ones after a change.
    fade: Option<Run<Bounds>>,
    /// The Reliability a configuration error of the trims replaced, while it
    /// stands.
    replaced: Option<Reliability>,
}

impl Trims {
    pub(super) const NONE: Self = Self {
        high_end: None,
        low_end: None,
        fade_time: 0,
        fade: None,
        replaced: None,
    };

    pub(super) fn high_end(&self) -> Option<f32> {
        self.high_end
    }

    pub(super) fn low_end(&self) -> Option<f32> {
        self.low_end
    }

    /// Whether Trim_Fade_Time is present: with either trim (the footnote to
    /// Table 12-64 ties it to them).
    pub(super) fn has_fade_time(&self) -> bool {
        self.high_end.is_some() || self.low_end.is_some()
    }

    /// A high trim below the low one.
    fn misconfigured(&self) -> bool {
        matches!((self.high_end, self.low_end), (Some(high), Some(low)) if high < low)
    }

    /// The bounds the trims set, or the full range when they're
    /// misconfigured.
    fn configured(&self) -> Bounds {
        if self.misconfigured() {
            return Bounds::FULL;
        }
        Bounds {
            low: self.low_end.unwrap_or(Bounds::FULL.low),
            high: self.high_end.unwrap_or(Bounds::FULL.high),
        }
    }

    /// Whether the bounds are still moving after a trim change at `now`.
    fn moving(&self, now: Duration) -> bool {
        self.fade
            .is_some_and(|run| !run.transition().is_finished(now))
    }

    /// The bounds in effect at `now`.
    fn at(&self, now: Duration) -> Bounds {
        self.fade
            .map_or_else(|| self.configured(), |run| run.transition().value_at(now))
    }

    /// Advance a trim change to `now`. `true` when Tracking_Value is due a
    /// COV sample, or the change has arrived.
    pub(super) fn advance(&mut self, now: Duration, step: f64) -> bool {
        let Some(run) = &mut self.fade else {
            return false;
        };
        match run.advance(now, step) {
            Progress::Finished => {
                self.fade = None;
                true
            }
            Progress::Sampled => true,
            Progress::Pending => false,
        }
    }

    /// When a trim change next needs advancing, if one is under way.
    pub(super) fn deadline(&self) -> Option<Duration> {
        self.fade.map(|run| run.deadline())
    }
}

/// Check a trim: 1.0 to 100.0 percent. Anything else, NaN included, is
/// VALUE_OUT_OF_RANGE.
fn trim_level(value: f32) -> Result<f32, Error> {
    if (1.0..=100.0).contains(&value) {
        Ok(value)
    } else {
        Err(common::value_out_of_range_error())
    }
}

impl LightingOutputObject {
    /// Set High_End_Trim, the top of the range Tracking_Value is held in, as
    /// a WriteProperty of it would; `None` takes the property away.
    ///
    /// A trim is 1.0 to 100.0 percent; anything else is refused with
    /// VALUE_OUT_OF_RANGE and changes nothing. Setting either trim makes
    /// Trim_Fade_Time present too. Tracking_Value follows the change over
    /// Trim_Fade_Time, and a high trim below the low one sets Reliability to
    /// CONFIGURATION_ERROR and holds nothing until it's put right.
    pub fn set_high_end_trim(&mut self, trim: Option<f32>) -> Result<(), Error> {
        let trim = trim.map(trim_level).transpose()?;
        self.change_trims(|trims| trims.high_end = trim);
        Ok(())
    }

    /// Set Low_End_Trim, the bottom of the range Tracking_Value is held in
    /// while the light is on; `None` takes the property away. Checked and
    /// applied as [`set_high_end_trim`](Self::set_high_end_trim) is.
    pub fn set_low_end_trim(&mut self, trim: Option<f32>) -> Result<(), Error> {
        let trim = trim.map(trim_level).transpose()?;
        self.change_trims(|trims| trims.low_end = trim);
        Ok(())
    }

    /// Set Trim_Fade_Time, the milliseconds Tracking_Value takes to follow a
    /// trim change: 0 (at once, the initial value) to 86,400,000 (a day).
    /// Anything else is refused with VALUE_OUT_OF_RANGE (Clause 12.54.Y3).
    /// The property is present only with a trim, but the setting is kept
    /// either way. A change already under way keeps its own time.
    pub fn set_trim_fade_time(&mut self, milliseconds: u32) -> Result<(), Error> {
        if !TRIM_FADE_TIME_MS.contains(&milliseconds) {
            return Err(common::value_out_of_range_error());
        }
        self.trims.fade_time = milliseconds;
        Ok(())
    }

    /// Change the trims at the current instant: the bounds move from where
    /// they stand to the new ones over Trim_Fade_Time.
    fn change_trims(&mut self, change: impl FnOnce(&mut Trims)) {
        let now = self.now();
        self.advance_to(now);
        let from = self.trims.at(now);
        change(&mut self.trims);
        // The trims set CONFIGURATION_ERROR, and clear only what they set.
        match (self.trims.misconfigured(), self.trims.replaced) {
            (true, None) => {
                self.trims.replaced = Some(self.reliability);
                self.reliability = Reliability::CONFIGURATION_ERROR;
            }
            (false, Some(replaced)) => {
                self.trims.replaced = None;
                if self.reliability == Reliability::CONFIGURATION_ERROR {
                    self.reliability = replaced;
                }
            }
            _ => {}
        }
        let fade_time = Duration::from_millis(u64::from(self.trims.fade_time));
        let fade = Transition::fade(from, self.trims.configured(), now, fade_time);
        self.trims.fade = fade.map(|fade| Run::start(fade, self.sample_step()));
        if self.trims.fade.is_some() {
            self.wake();
        }
    }

    /// Whether the trims stand aside: Present_Value comes from slot 1 or 2
    /// (Clauses 12.54.Y1 and 12.54.Y2).
    fn trims_bypassed(&self) -> bool {
        matches!(self.highest_priority(), Some(1 | 2))
    }

    /// `level` as Tracking_Value reports it at `now`: held in the operating
    /// range there.
    pub(super) fn trimmed(&self, level: f32, now: Duration) -> f32 {
        if self.trims_bypassed() {
            level
        } else {
            self.trims.at(now).hold(level)
        }
    }

    /// Whether In_Progress reads TRIM_ACTIVE at `now`, where `untrimmed` is
    /// Tracking_Value before the trims hold it: Present_Value lies outside
    /// the trims, or the bounds of a trim change still moving hold
    /// Tracking_Value back.
    pub(super) fn trim_active(&self, untrimmed: f32, now: Duration) -> bool {
        !self.trims_bypassed()
            && (self.trims.configured().hold(self.present_value) != self.present_value
                || (self.trims.moving(now) && self.trims.at(now).hold(untrimmed) != untrimmed))
    }

    /// The low bound in effect at `now`, or 0.0 when no low trim holds
    /// anything.
    fn low_bound(&self, now: Duration) -> f32 {
        if self.trims_bypassed() {
            0.0
        } else {
            self.trims.at(now).low
        }
    }

    /// The dimmest on level a step knows at `now`: the low trim while one
    /// applies, otherwise 1.0. STEP_OFF turns the light off from it.
    pub(super) fn step_floor(&self, now: Duration) -> f32 {
        self.low_bound(now).max(1.0)
    }

    /// The line a fade or ramp from `from` to `to` runs at `now`: one up from
    /// off starts at the low trim, and one down to off ends its line there,
    /// going off as it finishes. Unchanged with no low trim in effect, or
    /// from the low trim itself, which holds there until the end.
    pub(super) fn trimmed_line(&self, from: f32, to: f32, now: Duration) -> (f32, f32) {
        let low = self.low_bound(now);
        if low <= 0.0 {
            (from, to)
        } else if from == 0.0 && to > 0.0 {
            (low, to)
        } else if to == 0.0 && from > low {
            (from, low)
        } else {
            (from, to)
        }
    }

    /// Read High_End_Trim, Low_End_Trim or Trim_Fade_Time, if `property` is
    /// one of them and present.
    pub(super) fn read_trim(
        &self,
        property: bacnet_types::enums::PropertyIdentifier,
    ) -> Option<Result<PropertyValue, Error>> {
        use bacnet_types::enums::PropertyIdentifier as P;
        let value = match property {
            P::HIGH_END_TRIM => self.trims.high_end.map(PropertyValue::Real),
            P::LOW_END_TRIM => self.trims.low_end.map(PropertyValue::Real),
            P::TRIM_FADE_TIME => self
                .trims
                .has_fade_time()
                .then(|| PropertyValue::Unsigned(u64::from(self.trims.fade_time))),
            _ => return None,
        };
        Some(value.ok_or_else(common::unknown_property_error))
    }

    /// Write High_End_Trim, Low_End_Trim or Trim_Fade_Time, if `property` is
    /// one of them and present.
    pub(super) fn write_trim(
        &mut self,
        property: bacnet_types::enums::PropertyIdentifier,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        use bacnet_types::enums::PropertyIdentifier as P;
        if let Err(error) = self.read_trim(property)? {
            return Some(Err(error));
        }
        Some(match (property, value) {
            (P::HIGH_END_TRIM, PropertyValue::Real(trim)) => self.set_high_end_trim(Some(*trim)),
            (P::LOW_END_TRIM, PropertyValue::Real(trim)) => self.set_low_end_trim(Some(*trim)),
            (P::TRIM_FADE_TIME, PropertyValue::Unsigned(milliseconds)) => {
                common::u64_to_u32(*milliseconds).and_then(|ms| self.set_trim_fade_time(ms))
            }
            _ => Err(common::invalid_data_type_error()),
        })
    }
}
