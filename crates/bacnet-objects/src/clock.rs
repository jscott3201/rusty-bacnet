//! Dependency-neutral clock data exposed to BACnet objects.

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
    /// Return the BACnetDaysOfWeek bit for this frame, or `None` for an
    /// unavailable/invalid day-of-week value.
    pub fn day_of_week_bit(self) -> Option<u8> {
        (1..=7)
            .contains(&self.local_date.day_of_week)
            .then(|| 1 << (self.local_date.day_of_week - 1))
    }

    /// Whether this frame is a fully specified, internally consistent Device
    /// DateTime suitable for timestamping notifications.
    pub fn is_valid_actual_datetime(self) -> bool {
        let Some(year) = self.local_date.actual_year() else {
            return false;
        };
        if !(1..=12).contains(&self.local_date.month)
            || !(1..=7).contains(&self.local_date.day_of_week)
        {
            return false;
        }

        let max_day = days_in_month(year, self.local_date.month);
        if self.local_date.day == 0 || self.local_date.day > max_day {
            return false;
        }

        let days = days_from_civil(
            i64::from(year),
            i64::from(self.local_date.month),
            i64::from(self.local_date.day),
        );
        let expected_day_of_week = (days + 3).rem_euclid(7) as u8 + 1;
        if self.local_date.day_of_week != expected_day_of_week {
            return false;
        }

        self.local_time.hour <= 23
            && self.local_time.minute <= 59
            && self.local_time.second <= 59
            && self.local_time.hundredths <= 99
    }
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: u16) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_prime = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Synchronous read port for a coherent Device clock sample.
///
/// `None` means that no wall-clock frame is available. Implementations must
/// return all four fields from the same sample.
pub trait ClockReader: Send + Sync {
    /// Read one coherent frame, or report that no wall clock is available.
    fn read_clock(&self) -> Option<ClockFrame>;
}

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
