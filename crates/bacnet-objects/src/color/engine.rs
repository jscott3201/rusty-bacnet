//! The command engine the Color and Color Temperature objects share (#1474):
//! the fade or ramp under way, and the clock it runs on.
//!
//! It is Lighting Output's engine (#1384) cut down to what the colour objects
//! need. They have no priority array, no warn and no egress, so at most one
//! transition runs, and Present_Value holds its target from the moment it
//! starts. Tracking_Value is worked out from the transition and the instant
//! it is read at, and In_Progress shows FADE_ACTIVE or RAMP_ACTIVE until it
//! arrives (Clauses 12.X.5, 12.X.7, 12.Y.5 and 12.Y.7 of Addendum
//! 135-2020ca). Nothing ticks: the object reports the instant of the next
//! COV sample, or of the arrival, as its monotonic deadline, and the server's
//! task advances it then.

use std::sync::Arc;
use std::time::Duration;

use bacnet_types::enums::ColorOperationInProgress;

use crate::traits::{DeadlineWaker, MonotonicClock};
use crate::transition::{Progress, Run, Transition, TransitionKind, TransitionLevel};

/// The transition a colour object is running, and its clock.
#[derive(Clone)]
pub(super) struct Engine<V> {
    /// The fade or ramp under way, if any.
    run: Option<Run<V>>,
    /// How far the value moves between COV samples, in its own units.
    sample_step: f64,
    /// The finest COV increment a Tracking_Value subscriber asks for, as the
    /// server last passed it (#1510).
    finest_increment: Option<f64>,
    monotonic_clock: Option<Arc<MonotonicClock>>,
    deadline_waker: Option<Arc<DeadlineWaker>>,
    /// The time an object with no clock bound has been advanced to.
    logical_now: Duration,
}

impl<V: TransitionLevel> Engine<V> {
    /// An idle engine that samples a running value each `sample_step`.
    pub(super) fn new(sample_step: f64) -> Self {
        Self {
            run: None,
            sample_step,
            finest_increment: None,
            monotonic_clock: None,
            deadline_waker: None,
            logical_now: Duration::ZERO,
        }
    }

    /// The instant on the bound monotonic clock, or the logical time
    /// [`advance_by`](Self::advance_by) keeps without one.
    pub(super) fn now(&self) -> Duration {
        self.monotonic_clock
            .as_ref()
            .map_or(self.logical_now, |clock| clock())
    }

    /// Tracking_Value at `now`: the transition's value while one runs,
    /// otherwise `present`, the object's Present_Value.
    pub(super) fn tracking(&self, present: V, now: Duration) -> V {
        self.run
            .map_or(present, |run| run.transition().value_at(now))
    }

    /// In_Progress at `now`: FADE_ACTIVE or RAMP_ACTIVE while a transition is
    /// still moving, otherwise IDLE.
    pub(super) fn in_progress(&self, now: Duration) -> ColorOperationInProgress {
        match self.run {
            Some(run) if !run.transition().is_finished(now) => match run.transition().kind() {
                TransitionKind::Fade => ColorOperationInProgress::FADE_ACTIVE,
                TransitionKind::Ramp => ColorOperationInProgress::RAMP_ACTIVE,
            },
            _ => ColorOperationInProgress::IDLE,
        }
    }

    /// Run `transition`, replacing any in progress, and wake the server's
    /// task for its first sample. `None`, nothing to move, leaves the engine
    /// idle, so the value is at its target at once.
    pub(super) fn start(&mut self, transition: Option<Transition<V>>) {
        self.run = transition.map(|transition| Run::start(transition, self.step()));
        if self.run.is_some() {
            if let Some(waker) = &self.deadline_waker {
                waker();
            }
        }
    }

    /// End the transition in progress, if any, and return where it stood at
    /// `now`.
    pub(super) fn halt(&mut self, now: Duration) -> Option<V> {
        self.run.take().map(|run| run.transition().value_at(now))
    }

    /// Advance to `now`: end a transition that has arrived, and take a COV
    /// sample that is due. `true` when Tracking_Value or In_Progress changed
    /// for a COV report to carry.
    pub(super) fn advance_to(&mut self, now: Duration) -> bool {
        let step = self.step();
        let Some(run) = &mut self.run else {
            return false;
        };
        match run.advance(now, step) {
            Progress::Finished => {
                self.run = None;
                true
            }
            Progress::Sampled => true,
            Progress::Pending => false,
        }
    }

    /// Advance the logical clock an object with no monotonic clock keeps.
    pub(super) fn advance_by(&mut self, elapsed: Duration) -> bool {
        self.logical_now = self.logical_now.saturating_add(elapsed);
        self.advance_to(self.logical_now)
    }

    /// The next instant the engine needs advancing at, if any.
    pub(super) fn deadline(&self) -> Option<Duration> {
        self.run.map(|run| run.deadline())
    }

    /// The sample step: the object's own, or a Tracking_Value
    /// subscriber's increment where that is finer.
    fn step(&self) -> f64 {
        self.finest_increment
            .map_or(self.sample_step, |finest| self.sample_step.min(finest))
    }

    /// Take the finest increment a Tracking_Value subscriber asks for.
    pub(super) fn set_finest_increment(&mut self, finest: Option<f64>) {
        self.finest_increment = finest;
    }

    pub(super) fn bind_clock(&mut self, clock: Option<Arc<MonotonicClock>>) {
        self.monotonic_clock = clock;
    }

    pub(super) fn bind_waker(&mut self, waker: Option<Arc<DeadlineWaker>>) {
        self.deadline_waker = waker;
    }

    /// The engine as a COV snapshot holds it: its clock stopped at this
    /// instant, and no waker.
    pub(super) fn frozen(&self) -> Self {
        Self {
            run: self.run,
            sample_step: self.sample_step,
            finest_increment: self.finest_increment,
            monotonic_clock: None,
            deadline_waker: None,
            logical_now: self.now(),
        }
    }
}
