//! Clock-aligned acquisition for a POLLED Trend Log Multiple (Clauses
//! 12.30.14 and 12.30.15).
//!
//! Boundaries are counted on the Device clock's local time: boundary `k`
//! falls `k * interval + offset` hundredths after midnight on 1970-01-01,
//! local time. The interval divides a day, so every day's boundaries sit at
//! the same times of day and the count runs on across midnight. Each pass
//! looks at the Device clock again: a boundary is due once the clock has
//! reached one past the boundary last served, so the plan follows the Device
//! clock rather than a deadline on the monotonic clock. When the Device clock
//! has jumped since the last look (set by hand, a time synchronization, a
//! daylight-saving change), the plan starts again from where the clock now
//! stands: the boundaries a forward jump skipped are not made up off the
//! grid, and the ones a backward jump repeats are logged again as the clock
//! reaches them.

use core::time::Duration;

use bacnet_types::calendar::SpecificDate;

use crate::clock::ClockFrame;

/// Hundredths of a second in a day: an aligned interval divides it.
pub(super) const DAY: u32 = 8_640_000;

/// The most, in hundredths, the Device clock may stray from the monotonic
/// clock between two looks before the difference counts as a clock change
/// rather than drift or rounding: one second, against looks at most 100 ms
/// apart.
const JUMP: i64 = 100;

/// Where an aligned log's plan stands.
#[derive(Debug, Default)]
pub(super) struct Alignment {
    /// The boundary last served, or the one the plan starts after.
    served: Option<i64>,
    /// The Device clock at the last look, in hundredths since 1970-01-01
    /// local time, with the monotonic time of that look.
    seen: Option<(Duration, i64)>,
}

impl Alignment {
    /// Look at the Device clock `frame`, read at monotonic `now`, and return
    /// the boundary to acquire for, if one is due. The first look, and the
    /// first after a clock change, starts the plan there, taking a boundary
    /// only when the clock stands exactly on one.
    pub(super) fn due(
        &mut self,
        now: Duration,
        frame: &ClockFrame,
        interval: u32,
        offset: u32,
    ) -> Option<i64> {
        let local = local_hundredths(frame)?;
        let since = local - i64::from(offset);
        let boundary = since.div_euclid(i64::from(interval));
        let jumped = self.seen.is_some_and(|(then, before)| {
            let elapsed =
                i64::try_from(now.saturating_sub(then).as_millis() / 10).unwrap_or(i64::MAX);
            (local - before.saturating_add(elapsed)).abs() > JUMP
        });
        self.seen = Some((now, local));
        match self.served {
            Some(served) if !jumped => (boundary > served).then_some(boundary),
            _ => {
                let on_boundary = since.rem_euclid(i64::from(interval)) == 0;
                self.served = Some(boundary - i64::from(on_boundary));
                on_boundary.then_some(boundary)
            }
        }
    }

    /// Record that `boundary` has been served.
    pub(super) fn serve(&mut self, boundary: i64) {
        self.served = Some(boundary);
    }

    /// How long from monotonic `now` until the Device clock should reach the
    /// next boundary, judged from the last look; none before the first look,
    /// and at least a hundredth after it, as the clock reads in hundredths.
    pub(super) fn wait(&self, now: Duration, interval: u32, offset: u32) -> Duration {
        let (Some(served), Some((then, local))) = (self.served, self.seen) else {
            return Duration::ZERO;
        };
        let next = (served + 1) * i64::from(interval) + i64::from(offset);
        let left = Duration::from_millis(u64::try_from(next - local).unwrap_or(0) * 10)
            .saturating_sub(now.saturating_sub(then));
        left.max(Duration::from_millis(10))
    }
}

/// `frame`'s local date and time in hundredths since 1970-01-01, or `None`
/// for a frame that isn't an actual date and time.
fn local_hundredths(frame: &ClockFrame) -> Option<i64> {
    let day = SpecificDate::from_date(&frame.local_date)?;
    let t = frame.local_time;
    let time_of_day = ((i64::from(t.hour) * 60 + i64::from(t.minute)) * 60 + i64::from(t.second))
        * 100
        + i64::from(t.hundredths);
    Some(day.days_since_1970() * i64::from(DAY) + time_of_day)
}
