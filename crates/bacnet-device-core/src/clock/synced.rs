//! [`SyncedClock`]: the Device's wall clock as plain data on the host's
//! monotonic timeline, and the calendar arithmetic it runs on.
//!
//! Every division here is 32-bit: the elapsed time comes apart into seconds
//! through `bacnet-time`'s long division, and the day numbers and
//! hundredths fit an `i32` across the whole BACnet date range.

use bacnet_objects::clock::ClockFrame;
use bacnet_time::{Duration, Instant};
use bacnet_types::bitstring::DaysOfWeek;
use bacnet_types::calendar::days_in_month;
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, Time};

/// Hundredths of a second in a day.
const DAY: i32 = 8_640_000;
/// Hundredths in a minute and an hour.
const MINUTE: i32 = 6_000;
const HOUR: i32 = 360_000;
/// 1990-01-01, a Monday, as days since 1970-01-01: the date a clock that
/// has never been set starts from (Clause 12.11.24).
const RESTART_DAY: i32 = 7_305;

/// The Device's time zone: its UTC_Offset and Daylight_Savings_Status, as
/// the server's `ClockConfig` holds them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Zone {
    utc_offset: i16,
    daylight_savings: bool,
}

impl Zone {
    /// `utc_offset` minutes west of UTC (east is negative), and whether
    /// daylight-saving time is in effect, which moves local time an hour
    /// ahead of standard time. The offset must be a multiple of 15 minutes
    /// within -1440 to 1440 (Clause 12.11.25).
    pub fn new(utc_offset: i16, daylight_savings: bool) -> Result<Self, Error> {
        if !(-1440..=1440).contains(&utc_offset) || utc_offset % 15 != 0 {
            return Err(Error::range(
                "UTC offset must be a multiple of 15 minutes within -1440 to 1440",
            ));
        }
        Ok(Self {
            utc_offset,
            daylight_savings,
        })
    }

    /// The Device's UTC_Offset: minutes west of UTC.
    pub fn utc_offset(self) -> i16 {
        self.utc_offset
    }

    /// The Device's Daylight_Savings_Status.
    pub fn daylight_savings(self) -> bool {
        self.daylight_savings
    }

    /// Hundredths to add to UTC for local time: less the offset, plus an
    /// hour in daylight-saving time.
    fn to_local(self) -> i32 {
        let summer = if self.daylight_savings { HOUR } else { 0 };
        summer - i32::from(self.utc_offset) * MINUTE
    }
}

/// What a date and time handed to [`SyncedClock::set`] stand for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Basis {
    /// Local time, as a TimeSynchronization carries it: it becomes
    /// Local_Date and Local_Time as given, whatever the zone.
    Local,
    /// UTC, as a UTCTimeSynchronization carries it: local time is it less
    /// UTC_Offset, plus an hour in daylight-saving time.
    Utc,
}

/// The Device's wall clock: an instant on the host's monotonic timeline,
/// the UTC date and time it stood for, and the [`Zone`]. Plain data, with no
/// allocation; every call that reads it takes `now` on the same timeline.
///
/// It starts unsynchronized, at local midnight on 1990-01-01, and counts
/// from there, as Clauses 12.11.23 and 12.11.24 have a device with no time
/// source do after a restart. The first [`set`](Self::set) synchronizes it,
/// for good: from then on [`window`](Self::window) answers, and a change of
/// zone ([`set_zone`](Self::set_zone)) keeps it synchronized.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncedClock {
    zone: Zone,
    /// The instant the anchor stood for.
    at: Instant,
    /// UTC days since 1970-01-01 at `at`.
    day: i32,
    /// UTC hundredths since midnight at `at`, short of a day.
    hundredths: u32,
    synced: bool,
}

impl SyncedClock {
    /// An unsynchronized clock in `zone` reading local midnight, 1990-01-01,
    /// at `now`.
    pub fn new(zone: Zone, now: Instant) -> Self {
        Self::anchored(zone, now, RESTART_DAY, -zone.to_local(), false)
    }

    /// The clock reading `day` and `utc` (hundredths, which may run past
    /// either end of the day) at `at`.
    fn anchored(zone: Zone, at: Instant, day: i32, utc: i32, synced: bool) -> Self {
        Self {
            zone,
            at,
            day: day + utc.div_euclid(DAY),
            hundredths: utc.rem_euclid(DAY).unsigned_abs(),
            synced,
        }
    }

    /// Set the clock to `date` and `time` at `now`, read on `basis`, and
    /// count it synchronized. Both must be specific: a real date of 1900 to
    /// 2154 whose weekday matches it, and a time with every field in range.
    /// A value whose local date would fall outside that range is refused
    /// too. A refused value leaves the clock as it was.
    pub fn set(&mut self, basis: Basis, date: Date, time: Time, now: Instant) -> Result<(), Error> {
        let (day, hundredths) = specific(date, time)?;
        let utc = match basis {
            Basis::Local => hundredths - self.zone.to_local(),
            Basis::Utc => hundredths,
        };
        let set = Self::anchored(self.zone, now, day, utc, true);
        set.frame(now)
            .ok_or(Error::range("the local date falls outside 1900 to 2154"))?;
        *self = set;
        Ok(())
    }

    /// Move the clock to `zone` at `now`, as a host does when daylight
    /// saving starts or ends, or to correct its UTC_Offset.
    ///
    /// A synchronized clock keeps the UTC instant it stands for and stays
    /// synchronized, so Local_Date and Local_Time move by the difference
    /// between the zones: an hour for daylight saving alone. A clock that
    /// was never set has no such instant: it keeps its count of local time
    /// from 1990-01-01, as one built in `zone` would read (Clauses 12.11.23,
    /// 12.11.24). Refused, leaving the clock as it was, when the local date
    /// at `now` would fall outside 1900 to 2154.
    pub fn set_zone(&mut self, zone: Zone, now: Instant) -> Result<(), Error> {
        let moved = if self.synced {
            Self { zone, ..*self }
        } else {
            let local = self.hundredths as i32 + self.zone.to_local();
            Self::anchored(zone, self.at, self.day, local - zone.to_local(), false)
        };
        moved
            .frame(now)
            .ok_or(Error::range("the local date falls outside 1900 to 2154"))?;
        *self = moved;
        Ok(())
    }

    /// Whether a [`set`](Self::set) has synchronized the clock.
    pub fn is_synced(&self) -> bool {
        self.synced
    }

    /// The zone the clock keeps local time in.
    pub fn zone(&self) -> Zone {
        self.zone
    }

    /// The Device's Local_Date, Local_Time, UTC_Offset and
    /// Daylight_Savings_Status at `now`, synchronized or not; `None` once
    /// the local date passes 2154. An instant before the anchor reads as the
    /// anchor.
    pub fn frame(&self, now: Instant) -> Option<ClockFrame> {
        let (day, utc) = self.utc_at(now);
        let local = utc + self.zone.to_local();
        let day = day + local.div_euclid(DAY);
        let within = local.rem_euclid(DAY);
        let (year, month, day_of_month) = civil(day);
        let year = u8::try_from(year - 1900)
            .ok()
            .filter(|&year| year != 0xFF)?;
        let second = within / 100;
        Some(ClockFrame {
            local_date: Date {
                year,
                month,
                day: day_of_month,
                day_of_week: weekday(day),
            },
            // Each field is below its unit's count, so the casts keep it.
            local_time: Time {
                hour: (second / 3_600) as u8,
                minute: (second / 60 % 60) as u8,
                second: (second % 60) as u8,
                hundredths: (within % 100) as u8,
            },
            utc_offset: self.zone.utc_offset,
            daylight_savings_status: self.zone.daylight_savings,
        })
    }

    /// The local day and time recipient windows are judged at `now`, or
    /// `None` until the clock is synchronized: a Notification Class
    /// destination is then open only if it is open at every time
    /// ([`destination_open`](super::destination_open)).
    pub fn window(&self, now: Instant) -> Option<(DaysOfWeek, Time)> {
        if !self.synced {
            return None;
        }
        let frame = self.frame(now)?;
        Some((frame.day_of_week()?, frame.local_time))
    }

    /// The UTC day and hundredths since midnight at `now`.
    fn utc_at(&self, now: Instant) -> (i32, i32) {
        let elapsed = now.saturating_duration_since(self.at);
        let seconds = elapsed.as_secs();
        // Below a second: under 10^9 nanoseconds.
        let nanos = (elapsed - Duration::from_secs(seconds)).as_nanos() as u32;
        // u32 seconds last 136 years, past the end of the BACnet dates.
        let seconds = u32::try_from(seconds).unwrap_or(u32::MAX);
        let days = (seconds / 86_400) as i32;
        let hundredths = (seconds % 86_400 * 100 + nanos / 10_000_000) as i32;
        let total = self.hundredths as i32 + hundredths;
        (self.day.saturating_add(days) + total / DAY, total % DAY)
    }
}

/// `date` and `time` as UTC-free day number and hundredths, if both are
/// specific and consistent.
fn specific(date: Date, time: Time) -> Result<(i32, i32), Error> {
    let year = date
        .actual_year()
        .ok_or(Error::range("the date's year is unspecified"))?;
    if date.day == 0 || date.day > days_in_month(year, date.month) {
        return Err(Error::range("the date names no day"));
    }
    let day = days_from_civil(i32::from(year), date.month, date.day);
    if date.day_of_week != weekday(day) {
        return Err(Error::range("the date's day of the week is wrong"));
    }
    if time.hour > 23 || time.minute > 59 || time.second > 59 || time.hundredths > 99 {
        return Err(Error::range("the time isn't a specific time of day"));
    }
    let hundredths = i32::from(time.hour) * HOUR
        + i32::from(time.minute) * MINUTE
        + i32::from(time.second) * 100
        + i32::from(time.hundredths);
    Ok((day, hundredths))
}

/// Day number `day`'s weekday, counted as BACnet counts it from Monday as 1
/// to Sunday as 7; 1970-01-01 was a Thursday.
fn weekday(day: i32) -> u8 {
    (day + 3).rem_euclid(7) as u8 + 1
}

/// Days since 1970-01-01 of `year`-`month`-`day` (Howard Hinnant's
/// days-from-civil, in 32 bits for years 1 to 5 million).
fn days_from_civil(year: i32, month: u8, day: u8) -> i32 {
    let year = year - i32::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month = i32::from(month);
    let shifted_month = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + i32::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The year, month and day of day number `day`: the inverse of
/// [`days_from_civil`].
fn civil(day: i32) -> (i32, u8, u8) {
    let shifted = day + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u8;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i32::from(month <= 2);
    (year, month as u8, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::calendar::SpecificDate;

    /// Every day a BACnet Date can carry comes back from its day number,
    /// which, with its weekday, matches the calendar type's 64-bit
    /// arithmetic.
    #[test]
    fn the_32_bit_calendar_matches_the_64_bit_one() {
        let mut next = days_from_civil(1900, 1, 1);
        for year in 1900..=2154 {
            for month in 1..=12 {
                for day in 1..=days_in_month(year, month) {
                    let expected = SpecificDate::new(year, month, day).unwrap();
                    let number = days_from_civil(i32::from(year), month, day);
                    assert_eq!(number, next);
                    assert_eq!(i64::from(number), expected.days_since_1970());
                    assert_eq!(civil(number), (i32::from(year), month, day));
                    assert_eq!(weekday(number), expected.weekday());
                    next += 1;
                }
            }
        }
        assert_eq!(days_from_civil(1990, 1, 1), RESTART_DAY);
        assert_eq!(weekday(RESTART_DAY), 1);
    }
}
