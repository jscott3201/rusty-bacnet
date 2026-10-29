//! When the server samples an Averaging object (Clauses 12.5.14 and 12.5.15,
//! #1144).
//!
//! Samples are spaced Window_Interval / Window_Samples apart on the
//! database's monotonic clock. The schedule restarts whenever the window is
//! emptied, so a window filled by the server covers Window_Interval from the
//! moment it was last reset: Attempted_Samples reaches Window_Samples only once
//! that much time has passed (Clause 12.5.11).

use alloc::sync::Arc;
use core::fmt;
use core::time::Duration;

use crate::traits::MonotonicClock;

/// Shortest spacing at which the server samples one Averaging object.
///
/// Window_Interval / Window_Samples can be well under a millisecond (one
/// second over 1,440 samples), and every sample takes the database's write
/// lock. A configured spacing below this floor is stretched to it, so such a
/// window spans more than Window_Interval. Clause 12.5.14 leaves the shortest
/// acceptable Window_Interval to the implementation.
pub const MIN_SAMPLE_PERIOD: Duration = Duration::from_millis(100);

/// The monotonic instant the next scheduled sample is due.
#[derive(Clone, Default)]
pub(super) struct SampleSchedule {
    clock: Option<Arc<MonotonicClock>>,
    /// `None` until a clock is bound.
    next_due: Option<Duration>,
}

impl fmt::Debug for SampleSchedule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SampleSchedule")
            .field("clock_bound", &self.clock.is_some())
            .field("next_due", &self.next_due)
            .finish()
    }
}

impl SampleSchedule {
    /// Bind or remove the clock and restart the schedule from its present
    /// reading. Removing it stops scheduled sampling.
    pub(super) fn bind(&mut self, clock: Option<Arc<MonotonicClock>>, period: Duration) {
        self.clock = clock;
        self.restart(period);
    }

    /// Start counting periods from now. The first sample falls one period
    /// later, so the window holds Window_Samples attempts exactly when
    /// Window_Interval has passed since the restart.
    pub(super) fn restart(&mut self, period: Duration) {
        self.next_due = self
            .clock
            .as_ref()
            .map(|clock| clock().saturating_add(period));
    }

    pub(super) fn next_due(&self) -> Option<Duration> {
        self.next_due
    }

    /// Whether a sample is due at `now`; when it is, move on to the next one.
    ///
    /// Due times keep a fixed cadence from the restart, so a pass that runs a
    /// little late doesn't shift later samples. One that falls a whole period
    /// or more behind takes a single sample and restarts the cadence from
    /// `now` instead of catching up with a burst of samples.
    pub(super) fn take_due(&mut self, now: Duration, period: Duration) -> bool {
        match self.next_due {
            Some(due) if due <= now => {
                let next = due.saturating_add(period);
                self.next_due = Some(if next > now {
                    next
                } else {
                    now.saturating_add(period)
                });
                true
            }
            _ => false,
        }
    }
}
