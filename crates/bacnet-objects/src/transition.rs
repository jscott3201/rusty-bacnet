//! Timed transitions of an output value: the fade and ramp arithmetic a
//! lighting or colour command runs (#1384).
//!
//! Lighting Output's FADE_TO and RAMP_TO (Clause 12.54, Table 12-67) move
//! Tracking_Value in a straight line from where it stands to the commanded
//! level: a fade over a set time, a ramp at a set rate. The colour objects of
//! Addendum 135-2020ca fade an xy colour and fade or ramp a colour
//! temperature the same way, so nothing here knows about lighting: a value
//! type says how to interpolate between two of its values and how far apart
//! they are ([`TransitionLevel`]), and a [`Transition`] holds the line in
//! time.
//!
//! Everything is pure. Every instant is a `Duration` on the caller's
//! monotonic clock (the server's, or a hand-set one in a test), passed in, so
//! the value at any instant is computed from the line rather than stepped by
//! a timer. A [`Run`] adds the state an object keeps while a transition is
//! under way: when the value was last sampled for COV and when it is next
//! due, which a finer sample step can bring forward (#1510).

use std::time::Duration;

use bacnet_types::constructed::BACnetXyColor;

/// How often, at most, a running transition's value is sampled for COV.
///
/// Samples fall on multiples of this interval on the monotonic clock, so
/// objects fading at the same time share the server's wakeups instead of
/// each adding its own.
pub(crate) const SAMPLE_GRID: Duration = Duration::from_millis(100);

/// A value a transition can move along a straight line.
pub(crate) trait TransitionLevel: Copy + PartialEq {
    /// The value `fraction` of the way from `from` to `to`, where `fraction`
    /// runs from 0.0 (at `from`) to 1.0 (at `to`).
    fn interpolate(from: Self, to: Self, fraction: f64) -> Self;

    /// How far apart two values are, in the units a ramp rate is given in
    /// per second and a COV sample step is given in.
    fn distance(from: Self, to: Self) -> f64;
}

/// A lighting level in percent.
impl TransitionLevel for f32 {
    fn interpolate(from: f32, to: f32, fraction: f64) -> f32 {
        let (from, to) = (f64::from(from), f64::from(to));
        (from + (to - from) * fraction) as f32
    }

    fn distance(from: f32, to: f32) -> f64 {
        (f64::from(to) - f64::from(from)).abs()
    }
}

/// A CIE 1931 xy colour (a Color object's FADE_TO_COLOR, #1474). Each
/// coordinate moves on its own straight line, so the colour crosses the
/// diagram in a straight line too; the addendum leaves the path to the
/// implementation. The distance is the length of that line, which only
/// spaces the COV samples, as a colour fade has no rate.
impl TransitionLevel for BACnetXyColor {
    fn interpolate(from: Self, to: Self, fraction: f64) -> Self {
        Self::new(
            f32::interpolate(from.x, to.x, fraction),
            f32::interpolate(from.y, to.y, fraction),
        )
    }

    fn distance(from: Self, to: Self) -> f64 {
        f32::distance(from.x, to.x).hypot(f32::distance(from.y, to.y))
    }
}

/// A colour temperature in kelvin (a Color Temperature object's fades,
/// ramps and steps, #1474). Values in between round to the nearest kelvin,
/// as Present_Value and Tracking_Value are Unsigned; the distance is in
/// kelvin, the unit a ramp rate is given in per second.
impl TransitionLevel for u32 {
    fn interpolate(from: u32, to: u32, fraction: f64) -> u32 {
        let (from, to) = (f64::from(from), f64::from(to));
        (from + (to - from) * fraction).round() as u32
    }

    fn distance(from: u32, to: u32) -> f64 {
        f64::from(from.abs_diff(to))
    }
}

/// Whether a transition was asked for by its duration or by its rate; the
/// object reports the two as different In_Progress values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransitionKind {
    /// Over a set time: FADE_ACTIVE.
    Fade,
    /// At a set rate: RAMP_ACTIVE.
    Ramp,
}

/// A value moving in a straight line from `from` at `start` to `to` at
/// `end`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Transition<V> {
    kind: TransitionKind,
    from: V,
    to: V,
    start: Duration,
    end: Duration,
}

impl<V: TransitionLevel> Transition<V> {
    /// A fade from `from` to `to` that starts at `start` and takes
    /// `duration`.
    ///
    /// `None` when there is nothing to move: the two values are equal, or
    /// the fade takes no time. The caller then sets the value at once.
    pub(crate) fn fade(from: V, to: V, start: Duration, duration: Duration) -> Option<Self> {
        if from == to || duration.is_zero() {
            return None;
        }
        Some(Self {
            kind: TransitionKind::Fade,
            from,
            to,
            start,
            end: start.saturating_add(duration),
        })
    }

    /// A ramp from `from` to `to` that starts at `start` and moves
    /// `rate_per_second` units each second, so it takes the distance over
    /// the rate.
    ///
    /// `None` when the values are equal, the rate isn't a positive finite
    /// number, or the ramp would take no measurable time.
    pub(crate) fn ramp(from: V, to: V, start: Duration, rate_per_second: f64) -> Option<Self> {
        if from == to || !rate_per_second.is_finite() || rate_per_second <= 0.0 {
            return None;
        }
        let seconds = V::distance(from, to) / rate_per_second;
        let duration = Duration::try_from_secs_f64(seconds).ok()?;
        let mut ramp = Self::fade(from, to, start, duration)?;
        ramp.kind = TransitionKind::Ramp;
        Some(ramp)
    }

    /// Whether this is a fade or a ramp.
    pub(crate) fn kind(&self) -> TransitionKind {
        self.kind
    }

    /// Whether the transition has reached its target by `now`.
    pub(crate) fn is_finished(&self, now: Duration) -> bool {
        now >= self.end
    }

    /// The value at `now`: `from` until the start, `to` from the end on, and
    /// the straight line between.
    pub(crate) fn value_at(&self, now: Duration) -> V {
        if now <= self.start {
            return self.from;
        }
        if now >= self.end {
            return self.to;
        }
        let elapsed = (now - self.start).as_secs_f64();
        let span = (self.end - self.start).as_secs_f64();
        V::interpolate(self.from, self.to, elapsed / span)
    }

    /// The next instant after `after` to sample the value for COV: once it
    /// has moved `step` further, rounded up onto [`SAMPLE_GRID`], and never
    /// past the end.
    ///
    /// A `step` of zero (or one that isn't a finite number) samples at the
    /// next grid point.
    pub(crate) fn next_sample(&self, after: Duration, step: f64) -> Duration {
        let distance = V::distance(self.from, self.to);
        let moving = if step.is_finite() && step > 0.0 && distance > 0.0 {
            (self.end - self.start).mul_f64((step / distance).min(1.0))
        } else {
            Duration::ZERO
        };
        let due = after.saturating_add(moving);
        let sample = if due > after {
            grid_at_or_after(due)
        } else {
            grid_after(after)
        };
        sample.min(self.end)
    }
}

/// The first multiple of [`SAMPLE_GRID`] at or after `instant`.
fn grid_at_or_after(instant: Duration) -> Duration {
    let grid = SAMPLE_GRID.as_nanos();
    let cells = instant.as_nanos().div_ceil(grid);
    from_nanos(cells.saturating_mul(grid))
}

/// The first multiple of [`SAMPLE_GRID`] strictly after `instant`.
fn grid_after(instant: Duration) -> Duration {
    let grid = SAMPLE_GRID.as_nanos();
    let cells = instant.as_nanos() / grid + 1;
    from_nanos(cells.saturating_mul(grid))
}

fn from_nanos(nanos: u128) -> Duration {
    Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
}

/// A transition under way, with the instant its value is next due to be
/// sampled for COV.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Run<V> {
    transition: Transition<V>,
    /// The instant the next sample is planned from: the start, then each
    /// sample's planned instant.
    last_sample: Duration,
    next_sample: Duration,
}

/// What [`Run::advance`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Progress {
    /// Nothing is due yet.
    Pending,
    /// A COV sample fell due; the next one is scheduled.
    Sampled,
    /// The transition reached its target; the caller ends the run.
    Finished,
}

impl<V: TransitionLevel> Run<V> {
    /// Start running `transition`, sampling each time the value has moved
    /// `step` (see [`Transition::next_sample`]).
    pub(crate) fn start(transition: Transition<V>, step: f64) -> Self {
        Self {
            next_sample: transition.next_sample(transition.start, step),
            last_sample: transition.start,
            transition,
        }
    }

    /// The transition being run.
    pub(crate) fn transition(&self) -> &Transition<V> {
        &self.transition
    }

    /// When the run next needs its owner advanced: the next sample, which is
    /// never later than the end.
    pub(crate) fn deadline(&self) -> Duration {
        self.next_sample
    }

    /// Advance to `now`, scheduling the next sample when one fell due.
    ///
    /// `step` may be finer than the one the pending sample was planned with,
    /// as when a COV subscriber asks for a smaller increment than the
    /// object's (#1510): the pending sample then comes forward to where the
    /// finer step puts it, from the last sample, and is taken now if that
    /// has passed. A coarser step leaves the pending sample where it is.
    ///
    /// A wake a little late (less than one grid cell, as a real timer
    /// usually is) schedules from the sample it was meant for, so the
    /// cadence holds instead of slipping a cell each time. A wake later than
    /// that schedules from `now`. A planned sample short of the end is a
    /// grid point, so the next one from it is a cell or more on, past `now`.
    pub(crate) fn advance(&mut self, now: Duration, step: f64) -> Progress {
        if self.transition.is_finished(now) {
            return Progress::Finished;
        }
        let replanned = self.transition.next_sample(self.last_sample, step);
        self.next_sample = self.next_sample.min(replanned);
        if now < self.next_sample {
            return Progress::Pending;
        }
        let planned = self.next_sample;
        let from = if now - planned < SAMPLE_GRID {
            planned
        } else {
            now
        };
        self.last_sample = from;
        self.next_sample = self.transition.next_sample(from, step);
        Progress::Sampled
    }
}

#[cfg(test)]
#[path = "transition_tests.rs"]
mod tests;
