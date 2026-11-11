//! Dependency-neutral clock data exposed to BACnet objects.

use bacnet_types::bitstring::DaysOfWeek;
use bacnet_types::calendar::SpecificDate;
use bacnet_types::primitives::{Date, Time};

/// One coherent sample of the Device clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockFrame {
    /// Device local date.
    pub local_date: Date,
    /// Device local time.
    pub local_time: Time,
    /// Signed minutes west of UTC.
    pub utc_offset: i16,
    /// Whether daylight-saving time is currently applied.
    pub daylight_savings_status: bool,
}

impl ClockFrame {
    /// Return this frame's day of the week as a single [`DaysOfWeek`] flag,
    /// or `None` for an unavailable/invalid day-of-week value.
    pub fn day_of_week(self) -> Option<DaysOfWeek> {
        (1..=7)
            .contains(&self.local_date.day_of_week)
            .then(|| DaysOfWeek::from_bits_truncate(1 << (self.local_date.day_of_week - 1)))
    }

    /// Whether this frame is a fully specified, internally consistent Device
    /// DateTime suitable for timestamping notifications.
    pub fn is_valid_actual_datetime(self) -> bool {
        SpecificDate::from_date(&self.local_date)
            .is_some_and(|day| day.weekday() == self.local_date.day_of_week)
            && self.local_time.is_specific()
    }
}

/// Synchronous read port for a coherent Device clock sample.
///
/// `None` means that no wall-clock frame is available. Implementations must
/// return all four fields from the same sample.
pub trait ClockReader: Send + Sync {
    /// Read one coherent frame, or report that no wall clock is available.
    fn read_clock(&self) -> Option<ClockFrame>;
}

/// The local date and time to stamp on a BACnetDateTime property when an
/// object changes it: the Device clock's current frame, or a date and time
/// with every field unspecified when there is no clock or its frame is not a
/// valid actual date and time.
pub(crate) fn stamp_datetime(clock: Option<&dyn ClockReader>) -> (Date, Time) {
    current_datetime(clock).unwrap_or(UNSPECIFIED_DATETIME)
}

/// The Device clock's current local date and time, or `None` when there is
/// no clock or its frame is not a valid actual date and time.
pub(crate) fn current_datetime(clock: Option<&dyn ClockReader>) -> Option<(Date, Time)> {
    clock
        .and_then(ClockReader::read_clock)
        .filter(|frame| frame.is_valid_actual_datetime())
        .map(|frame| (frame.local_date, frame.local_time))
}

/// Hundredths of a second in a day.
const HUNDREDTHS_PER_DAY: i64 = 24 * 60 * 60 * 100;

/// The local moment `value` names, as hundredths of a second since
/// 1970-01-01 00:00 local time, or `None` when it names none.
///
/// A written BACnetDateTime names a moment when its date is a real day and
/// its hour and minute are set. Workstations often leave the seconds or the
/// hundredths unspecified, so each of those counts as zero. The weekday octet
/// may be unspecified or disagree with the date, but must be in range: it
/// never changes the day named. Anything else partly unspecified, and the
/// wholly unspecified value, name no moment.
pub(crate) fn local_moment((date, time): (Date, Time)) -> Option<i64> {
    let weekday = date.day_of_week == Date::UNSPECIFIED || (1..=7).contains(&date.day_of_week);
    let day = SpecificDate::from_date(&date).filter(|_| weekday)?;
    let or_zero = |field: u8| if field == Time::UNSPECIFIED { 0 } else { field };
    let time = Time {
        second: or_zero(time.second),
        hundredths: or_zero(time.hundredths),
        ..time
    };
    if !time.is_specific() {
        return None;
    }
    let of_day =
        ((i64::from(time.hour) * 60 + i64::from(time.minute)) * 60 + i64::from(time.second)) * 100
            + i64::from(time.hundredths);
    Some(day.days_since_1970() * HUNDREDTHS_PER_DAY + of_day)
}

/// A BACnetDateTime with every field unspecified, the value of a timestamp
/// that has never been set.
pub(crate) const UNSPECIFIED_DATETIME: (Date, Time) = (
    Date {
        year: Date::UNSPECIFIED,
        month: Date::UNSPECIFIED,
        day: Date::UNSPECIFIED,
        day_of_week: Date::UNSPECIFIED,
    },
    Time {
        hour: Time::UNSPECIFIED,
        minute: Time::UNSPECIFIED,
        second: Time::UNSPECIFIED,
        hundredths: Time::UNSPECIFIED,
    },
);

#[cfg(test)]
mod tests {
    use super::*;

    fn leap_day_frame() -> ClockFrame {
        ClockFrame {
            local_date: Date {
                year: 124,
                month: 2,
                day: 29,
                day_of_week: 4,
            },
            local_time: Time {
                hour: 23,
                minute: 59,
                second: 59,
                hundredths: 99,
            },
            utc_offset: 0,
            daylight_savings_status: false,
        }
    }

    #[test]
    fn actual_datetime_validation_checks_calendar_weekday_and_time() {
        let valid = leap_day_frame();
        assert!(valid.is_valid_actual_datetime());

        for invalid in [
            ClockFrame {
                local_date: Date {
                    day: 30,
                    ..valid.local_date
                },
                ..valid
            },
            ClockFrame {
                local_date: Date {
                    day_of_week: 5,
                    ..valid.local_date
                },
                ..valid
            },
            ClockFrame {
                local_time: Time {
                    hour: Time::UNSPECIFIED,
                    ..valid.local_time
                },
                ..valid
            },
        ] {
            assert!(!invalid.is_valid_actual_datetime());
        }
    }
}
