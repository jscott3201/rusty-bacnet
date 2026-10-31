//! The Clause 12.57 state machine: what each write does in each state, the
//! countdown, and expiry.
//!
//! A refused write returns before anything changes. Each transition sets
//! Last_State_Change and Update_Time from the Device clock (unspecified
//! without one), and the state machine's other side effects: Initial_Timeout
//! on a start, Expiration_Time on entering IDLE or EXPIRED. The countdown
//! runs on the database's monotonic clock and stops while Out_Of_Service is
//! TRUE, keeping the time left (Clause 12.57, Out_Of_Service).

use bacnet_types::calendar::{SpecificDate, days_in_month};
use bacnet_types::enums::{TimerState, TimerTransition};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, Time};
use core::time::Duration;

use super::TimerObject;
use crate::clock::{UNSPECIFIED_DATETIME, current_datetime, stamp_datetime};
use crate::common;

const MILLIS_PER_DAY: u64 = 86_400_000;

/// Where a running timer's countdown stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Countdown {
    /// Not running.
    Stopped,
    /// Running in service: expires at this monotonic instant.
    Counting { deadline: Duration },
    /// Running out of service: held with this many milliseconds left.
    Held { remaining: u64 },
}

impl TimerObject {
    fn monotonic_now(&self) -> Duration {
        self.monotonic_clock
            .as_ref()
            .map_or(self.logical_now, |clock| clock())
    }

    /// Present_Value: the milliseconds left while running, rounded up so a
    /// running timer never reads zero before it expires; zero otherwise.
    pub(super) fn remaining(&self) -> u64 {
        match self.countdown {
            Countdown::Stopped => 0,
            Countdown::Held { remaining } => remaining,
            Countdown::Counting { deadline } => {
                let left = deadline.saturating_sub(self.monotonic_now());
                let millis = u64::try_from(left.as_millis()).unwrap_or(u64::MAX);
                millis.saturating_add(u64::from(left.subsec_nanos() % 1_000_000 != 0))
            }
        }
    }

    /// Whether the countdown has reached zero but no expiry pass has run
    /// yet: the server's task, or the next write, applies it. Reads serve
    /// the timer as already expired, so none shows RUNNING with nothing
    /// left.
    pub(super) fn is_due(&self) -> bool {
        self.due_deadline().is_some()
    }

    /// The deadline of a countdown that has reached zero unexpired.
    fn due_deadline(&self) -> Option<Duration> {
        match self.countdown {
            Countdown::Counting { deadline } if self.monotonic_now() >= deadline => Some(deadline),
            _ => None,
        }
    }

    /// The Device clock's date and time at the monotonic instant
    /// `deadline`, which has passed: now, less the time since. A natural
    /// expiry happens when the countdown reaches zero, so it carries this
    /// stamp whenever the pass, a read or a write gets to it. Unspecified
    /// without a Device clock, as every stamp is.
    fn deadline_datetime(&self, deadline: Duration) -> (Date, Time) {
        let Some(now) = current_datetime(self.clock.as_deref()) else {
            return UNSPECIFIED_DATETIME;
        };
        let since = self.monotonic_now().saturating_sub(deadline);
        let since = u64::try_from(since.as_millis()).unwrap_or(u64::MAX);
        sub_millis(now, since).unwrap_or(now)
    }

    /// Timer_State as served, a due expiry applied.
    pub(super) fn served_state(&self) -> TimerState {
        if self.is_due() {
            TimerState::EXPIRED
        } else {
            self.timer_state
        }
    }

    /// Last_State_Change as served, a due expiry applied.
    pub(super) fn served_last_state_change(&self) -> TimerTransition {
        if self.is_due() {
            TimerTransition::RUNNING_TO_EXPIRED
        } else {
            self.last_state_change
        }
    }

    /// Update_Time as served: a due expiry is stamped at its deadline, as
    /// the pass will store it.
    pub(super) fn served_update_time(&self) -> (Date, Time) {
        match self.due_deadline() {
            Some(deadline) => self.deadline_datetime(deadline),
            None => self.update_time,
        }
    }

    /// Expiration_Time: when a running timer will expire, worked out now
    /// from the Device clock; the stored value in IDLE and EXPIRED, and the
    /// deadline's time for a due expiry.
    ///
    /// While Out_Of_Service holds a running countdown this is still now plus
    /// the time left, so it moves on with the Device clock. Clause 12.57 has
    /// a RUNNING timer's Expiration_Time worked out when read, and lets an
    /// out-of-service timer behave as specified apart from not counting
    /// down; it names no other value for the held case.
    pub(super) fn expiration_time_now(&self) -> (Date, Time) {
        if let Some(deadline) = self.due_deadline() {
            return self.deadline_datetime(deadline);
        }
        if self.timer_state != TimerState::RUNNING {
            return self.expiration_time;
        }
        current_datetime(self.clock.as_deref())
            .and_then(|now| add_millis(now, self.remaining()))
            .unwrap_or(UNSPECIFIED_DATETIME)
    }

    /// Apply an expiry whose deadline has passed before the task's pass
    /// got to it, so a write acts on the state the timer is really in.
    pub(super) fn apply_due_expiry(&mut self) {
        self.expire_if_due(self.monotonic_now());
    }

    /// Out_Of_Service changed: hold a running countdown, or resume it.
    pub(super) fn service_changed(&mut self) {
        // A countdown that already ran out expires rather than being held
        // with nothing left.
        self.apply_due_expiry();
        match (self.out_of_service, self.countdown) {
            (true, Countdown::Counting { .. }) => {
                self.countdown = Countdown::Held {
                    remaining: self.remaining(),
                };
            }
            (false, Countdown::Held { remaining }) => self.count_down(remaining),
            _ => {}
        }
    }

    fn count_down(&mut self, remaining: u64) {
        let deadline = self
            .monotonic_now()
            .saturating_add(Duration::from_millis(remaining));
        self.countdown = Countdown::Counting { deadline };
        if let Some(waker) = &self.deadline_waker {
            waker();
        }
    }

    /// Record a transition a write made, stamped now.
    fn transition(&mut self, transition: TimerTransition) {
        self.transition_at(transition, stamp_datetime(self.clock.as_deref()));
    }

    /// Record a transition into the current state, made at `stamp`.
    fn transition_at(&mut self, transition: TimerTransition, stamp: (Date, Time)) {
        self.last_state_change = transition;
        self.update_time = stamp;
    }

    /// The start request, with `timeout` milliseconds.
    fn start(&mut self, timeout: u64) {
        let transition = match self.timer_state {
            TimerState::IDLE => TimerTransition::IDLE_TO_RUNNING,
            TimerState::RUNNING => TimerTransition::RUNNING_TO_RUNNING,
            _ => TimerTransition::EXPIRED_TO_RUNNING,
        };
        self.initial_timeout = timeout;
        if self.out_of_service {
            self.countdown = Countdown::Held { remaining: timeout };
        } else {
            self.count_down(timeout);
        }
        self.timer_state = TimerState::RUNNING;
        self.transition(transition);
    }

    /// Leave RUNNING for EXPIRED, by `transition`, at `stamp`.
    fn expire(&mut self, transition: TimerTransition, stamp: (Date, Time)) {
        self.countdown = Countdown::Stopped;
        self.timer_state = TimerState::EXPIRED;
        self.transition_at(transition, stamp);
        self.expiration_time = stamp;
    }

    /// The expire request a write makes: forced, and stamped now.
    fn force_expiry(&mut self) {
        self.expire(
            TimerTransition::FORCED_TO_EXPIRED,
            stamp_datetime(self.clock.as_deref()),
        );
    }

    /// The clear request from RUNNING or EXPIRED.
    fn clear(&mut self, transition: TimerTransition) {
        self.countdown = Countdown::Stopped;
        self.timer_state = TimerState::IDLE;
        self.expiration_time = UNSPECIFIED_DATETIME;
        self.transition(transition);
    }

    /// A Present_Value write: zero is the expire request, a timeout within
    /// Min_Pres_Value..=Max_Pres_Value the start request, and anything else
    /// is refused.
    pub(super) fn write_present_value(&mut self, timeout: u64) -> Result<(), Error> {
        if timeout == 0 {
            if self.timer_state == TimerState::RUNNING {
                self.force_expiry();
            }
            return Ok(());
        }
        if !self.in_range(timeout) {
            return Err(common::value_out_of_range_error());
        }
        self.start(timeout);
        Ok(())
    }

    /// A Timer_Running write: TRUE starts with Default_Timeout in any state,
    /// FALSE expires a running timer and is ignored otherwise.
    pub(super) fn write_timer_running(&mut self, running: bool) {
        if running {
            self.start(self.default_timeout);
        } else if self.timer_state == TimerState::RUNNING {
            self.force_expiry();
        }
    }

    /// A Timer_State write: IDLE is the clear request, any other state is
    /// refused.
    pub(super) fn write_timer_state(&mut self, state: TimerState) -> Result<(), Error> {
        if state != TimerState::IDLE {
            return Err(common::value_out_of_range_error());
        }
        match self.timer_state {
            TimerState::RUNNING => self.clear(TimerTransition::RUNNING_TO_IDLE),
            TimerState::EXPIRED => self.clear(TimerTransition::EXPIRED_TO_IDLE),
            _ => {}
        }
        Ok(())
    }

    /// Expire a countdown whose deadline has come; `true` when one did.
    pub(super) fn expire_if_due(&mut self, now: Duration) -> bool {
        match self.countdown {
            Countdown::Counting { deadline } if now >= deadline => {
                let stamp = self.deadline_datetime(deadline);
                self.expire(TimerTransition::RUNNING_TO_EXPIRED, stamp);
                true
            }
            _ => false,
        }
    }
}

/// Split a date and time with no unspecified field into its day and the
/// milliseconds since that midnight.
fn split((date, time): (Date, Time)) -> Option<(SpecificDate, u64)> {
    if !time.is_specific() {
        return None;
    }
    let of_day = u64::from(time.hour) * 3_600_000
        + u64::from(time.minute) * 60_000
        + u64::from(time.second) * 1_000
        + u64::from(time.hundredths) * 10;
    Some((SpecificDate::from_date(&date)?, of_day))
}

/// The time of day `millis` milliseconds after midnight, to the hundredth.
fn time_of_day(millis: u64) -> Time {
    Time {
        hour: (millis / 3_600_000) as u8,
        minute: (millis / 60_000 % 60) as u8,
        second: (millis / 1_000 % 60) as u8,
        hundredths: (millis % 1_000 / 10) as u8,
    }
}

/// A date and time `millis` milliseconds after `start`, or `None` past the
/// last day a BACnet Date can carry.
fn add_millis(start: (Date, Time), millis: u64) -> Option<(Date, Time)> {
    let (day, of_day) = split(start)?;
    let total = of_day.checked_add(millis)?;
    let day = add_days(day, total / MILLIS_PER_DAY)?;
    Some((day.to_date(), time_of_day(total % MILLIS_PER_DAY)))
}

/// A date and time `millis` milliseconds before `start`, or `None` before
/// the first day a BACnet Date can carry.
fn sub_millis(start: (Date, Time), millis: u64) -> Option<(Date, Time)> {
    let (day, of_day) = split(start)?;
    if millis <= of_day {
        return Some((day.to_date(), time_of_day(of_day - millis)));
    }
    let short = millis - of_day;
    let days = short.div_ceil(MILLIS_PER_DAY);
    let day = sub_days(day, days)?;
    Some((day.to_date(), time_of_day(days * MILLIS_PER_DAY - short)))
}

fn sub_days(day: SpecificDate, days: u64) -> Option<SpecificDate> {
    let (mut year, mut month) = (day.year(), day.month());
    let mut day_of_month = i128::from(day.day()) - i128::from(days);
    // At most one step per month back to the first year a Date can carry.
    while day_of_month < 1 {
        month -= 1;
        if month == 0 {
            month = 12;
            year -= 1;
            if year < 1900 {
                return None;
            }
        }
        day_of_month += i128::from(days_in_month(year, month));
    }
    SpecificDate::new(year, month, day_of_month as u8)
}

fn add_days(day: SpecificDate, days: u64) -> Option<SpecificDate> {
    let (mut year, mut month) = (day.year(), day.month());
    let mut day_of_month = u64::from(day.day()).checked_add(days)?;
    // At most one step per month up to the last year a Date can carry.
    loop {
        let length = u64::from(days_in_month(year, month));
        if day_of_month <= length {
            break;
        }
        day_of_month -= length;
        month += 1;
        if month > 12 {
            month = 1;
            year += 1;
            if year > 2154 {
                return None;
            }
        }
    }
    SpecificDate::new(year, month, day_of_month as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(year: u16, month: u8, day: u8, hour: u8, minute: u8) -> (Date, Time) {
        (
            SpecificDate::new(year, month, day).unwrap().to_date(),
            Time {
                hour,
                minute,
                second: 0,
                hundredths: 0,
            },
        )
    }

    #[test]
    fn adding_milliseconds_rolls_over_days_months_and_leap_years() {
        assert_eq!(
            add_millis(at(2024, 2, 28, 23, 59), 120_000),
            Some(at(2024, 2, 29, 0, 1))
        );
        assert_eq!(
            add_millis(at(2023, 12, 31, 12, 0), MILLIS_PER_DAY),
            Some(at(2024, 1, 1, 12, 0))
        );
        assert_eq!(
            add_millis(at(2026, 1, 1, 0, 0), 365 * MILLIS_PER_DAY),
            Some(at(2027, 1, 1, 0, 0))
        );
        let (_, time) = add_millis(at(2026, 1, 1, 0, 0), 1_234).unwrap();
        assert_eq!((time.second, time.hundredths), (1, 23));
        assert_eq!(add_millis(at(2154, 12, 31, 23, 0), 3_600_000), None);
        assert_eq!(add_millis(at(2026, 1, 1, 0, 0), u64::MAX), None);
        assert_eq!(add_millis(UNSPECIFIED_DATETIME, 1), None);
    }

    #[test]
    fn subtracting_milliseconds_rolls_back_days_months_and_leap_years() {
        assert_eq!(
            sub_millis(at(2026, 10, 31, 12, 1), 60_000),
            Some(at(2026, 10, 31, 12, 0))
        );
        // Across midnight, a month end, a leap day and a year end.
        assert_eq!(
            sub_millis(at(2026, 11, 1, 0, 1), 120_000),
            Some(at(2026, 10, 31, 23, 59))
        );
        assert_eq!(
            sub_millis(at(2024, 3, 1, 0, 0), 1),
            Some((
                SpecificDate::new(2024, 2, 29).unwrap().to_date(),
                Time {
                    hour: 23,
                    minute: 59,
                    second: 59,
                    hundredths: 99,
                }
            ))
        );
        assert_eq!(
            sub_millis(at(2027, 1, 1, 12, 0), MILLIS_PER_DAY),
            Some(at(2026, 12, 31, 12, 0))
        );
        assert_eq!(
            sub_millis(at(2027, 1, 1, 0, 0), 365 * MILLIS_PER_DAY),
            Some(at(2026, 1, 1, 0, 0))
        );
        // Exactly midnight stays on its day.
        assert_eq!(
            sub_millis(at(2026, 11, 1, 0, 1), 60_000),
            Some(at(2026, 11, 1, 0, 0))
        );
        // Each undoes the other.
        let start = at(2025, 12, 31, 23, 30);
        for millis in [0, 10, 1_800_000, MILLIS_PER_DAY * 400 + 123_450] {
            assert_eq!(
                sub_millis(add_millis(start, millis).unwrap(), millis),
                Some(start)
            );
        }
        assert_eq!(sub_millis(at(1900, 1, 1, 0, 0), 1), None);
        assert_eq!(sub_millis(at(2026, 1, 1, 0, 0), u64::MAX), None);
        assert_eq!(sub_millis(UNSPECIFIED_DATETIME, 1), None);
    }
}
