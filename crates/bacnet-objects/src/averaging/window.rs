//! The sliding sample window behind the Averaging statistics (Clause 12.5).
//!
//! The window keeps the most recent Window_Samples attempts in arrival order.
//! Each slot holds either the REAL that was sampled or a marker for an attempt
//! that produced no value. Minimum_Value, Maximum_Value and
//! Average_Value cover the valid slots only; Attempted_Samples counts every
//! slot and Valid_Samples the valid ones, so subtracting the second from the
//! first tells a reader how many attempts in the window failed (Clauses
//! 12.5.11 and 12.5.12).

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use core::time::Duration;

use bacnet_types::error::Error;

use super::schedule::{MIN_SAMPLE_PERIOD, SampleSchedule};
use crate::common;
use crate::traits::MonotonicClock;

/// Largest Window_Samples an Averaging object accepts.
///
/// Clause 12.5.15 sets a floor of 15 samples per window but no ceiling, while
/// the window keeps one slot per sample, so the buffer needs a bound. 1,440
/// slots hold one sample a minute for a whole day, the longest window building
/// automation commonly averages over, at about 11 KiB per object (eight bytes
/// a slot). A write or setter above it fails with PROPERTY /
/// VALUE_OUT_OF_RANGE.
pub const MAX_WINDOW_SAMPLES: u32 = 1_440;

/// Window_Samples of a new Averaging object: the 15-sample floor every
/// implementation has to support (Clause 12.5.15).
pub const DEFAULT_WINDOW_SAMPLES: u32 = 15;

/// Window_Interval of a new Averaging object, in seconds. Fifteen minutes
/// spreads the default 15 samples one minute apart.
pub const DEFAULT_WINDOW_INTERVAL: u32 = 900;

// The default meets the 15-sample floor, and the bound admits it.
const _: () = assert!(DEFAULT_WINDOW_SAMPLES >= 15 && MAX_WINDOW_SAMPLES >= DEFAULT_WINDOW_SAMPLES);

/// Statistics over the valid slots, recomputed on every change.
#[derive(Debug, Clone, Copy)]
struct Statistics {
    minimum: f32,
    maximum: f32,
    average: f32,
    valid: u32,
}

impl Statistics {
    /// The values with no valid sample in the window: Clause 12.5 gives
    /// positive infinity for Minimum_Value, NaN for Average_Value and negative
    /// infinity for Maximum_Value. `f32::NAN` is one fixed bit pattern, so a
    /// COV comparison of two empty windows sees no change.
    const EMPTY: Self = Self {
        minimum: f32::INFINITY,
        maximum: f32::NEG_INFINITY,
        average: f32::NAN,
        valid: 0,
    };
}

/// A bounded buffer of the most recent sample attempts, and the schedule the
/// server fills it on.
#[derive(Debug, Clone)]
pub(super) struct SampleWindow {
    /// Window_Interval, in seconds.
    interval: u32,
    /// Window_Samples: the most slots the buffer holds.
    capacity: u32,
    /// Oldest first; `None` marks an attempt that yielded no value.
    slots: VecDeque<Option<f32>>,
    statistics: Statistics,
    /// Restarted by every [`reset`](Self::reset), which every route that
    /// empties the window goes through.
    schedule: SampleSchedule,
}

impl SampleWindow {
    pub(super) fn new() -> Self {
        Self {
            interval: DEFAULT_WINDOW_INTERVAL,
            capacity: DEFAULT_WINDOW_SAMPLES,
            slots: VecDeque::new(),
            statistics: Statistics::EMPTY,
            schedule: SampleSchedule::default(),
        }
    }

    pub(super) fn interval(&self) -> u32 {
        self.interval
    }

    pub(super) fn capacity(&self) -> u32 {
        self.capacity
    }

    pub(super) fn minimum(&self) -> f32 {
        self.statistics.minimum
    }

    pub(super) fn maximum(&self) -> f32 {
        self.statistics.maximum
    }

    pub(super) fn average(&self) -> f32 {
        self.statistics.average
    }

    /// Attempts in the current window, which stops growing at Window_Samples.
    pub(super) fn attempted(&self) -> u32 {
        // The buffer never holds more than `capacity` (a u32) slots.
        self.slots.len() as u32
    }

    pub(super) fn valid(&self) -> u32 {
        self.statistics.valid
    }

    /// Set Window_Interval and discard the samples. Zero fails with PROPERTY /
    /// VALUE_OUT_OF_RANGE: the clause leaves the minimum to the implementation,
    /// and a window with no length has no sample spacing.
    pub(super) fn set_interval(&mut self, interval: u32) -> Result<(), Error> {
        if interval == 0 {
            return Err(common::value_out_of_range_error());
        }
        self.interval = interval;
        self.reset();
        Ok(())
    }

    /// Set Window_Samples and discard the samples. Zero, or more than
    /// [`MAX_WINDOW_SAMPLES`], fails with PROPERTY / VALUE_OUT_OF_RANGE.
    pub(super) fn set_capacity(&mut self, capacity: u32) -> Result<(), Error> {
        if !(1..=MAX_WINDOW_SAMPLES).contains(&capacity) {
            return Err(common::value_out_of_range_error());
        }
        self.capacity = capacity;
        self.reset();
        self.slots.shrink_to(capacity as usize);
        Ok(())
    }

    /// Record one attempt, dropping the oldest once the window is full.
    pub(super) fn push(&mut self, sample: Option<f32>) {
        if self.slots.len() >= self.capacity as usize {
            self.slots.pop_front();
        }
        self.slots.push_back(sample);
        self.recompute();
    }

    /// Discard every sample: the counts go to zero and the statistics to
    /// their empty-window values. Scheduled sampling starts over from now.
    pub(super) fn reset(&mut self) {
        self.slots.clear();
        self.statistics = Statistics::EMPTY;
        let period = self.period();
        self.schedule.restart(period);
    }

    /// Spacing of scheduled samples: Window_Interval / Window_Samples, but no
    /// shorter than [`MIN_SAMPLE_PERIOD`].
    pub(super) fn period(&self) -> Duration {
        // Window_Samples is never zero: construction and set_capacity keep it
        // at least one.
        (Duration::from_secs(self.interval.into()) / self.capacity).max(MIN_SAMPLE_PERIOD)
    }

    /// Bind or remove the monotonic clock and restart the schedule. The
    /// samples already taken stay.
    pub(super) fn bind_clock(&mut self, clock: Option<Arc<MonotonicClock>>) {
        let period = self.period();
        self.schedule.bind(clock, period);
    }

    /// When the next scheduled sample is due, once a clock is bound.
    pub(super) fn next_due(&self) -> Option<Duration> {
        self.schedule.next_due()
    }

    /// Whether a scheduled sample is due at `now`, stepping the schedule past
    /// it when it is. Recording the sample is the caller's job.
    pub(super) fn take_due(&mut self, now: Duration) -> bool {
        let period = self.period();
        self.schedule.take_due(now, period)
    }

    fn recompute(&mut self) {
        let mut statistics = Statistics::EMPTY;
        let mut sum = 0.0f64;
        for &value in self.slots.iter().flatten() {
            statistics.minimum = statistics.minimum.min(value);
            statistics.maximum = statistics.maximum.max(value);
            sum += f64::from(value);
            statistics.valid += 1;
        }
        if statistics.valid > 0 {
            // Summing the whole window each time in f64 avoids the drift an
            // incremental update builds up as samples leave the window. The
            // clamp keeps the rounded mean inside the sampled range.
            let average = (sum / f64::from(statistics.valid)) as f32;
            statistics.average = average.clamp(statistics.minimum, statistics.maximum);
        }
        self.statistics = statistics;
    }
}
