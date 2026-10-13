//! The Access_Event_Time an Access Point stamps on an event it records
//! itself: an Out_Of_Service edge, or a reported event that brings no time
//! of its own (#1248, #1132).
//!
//! Access_Event_Time moves with every update of Access_Event (Clause
//! 12.31.29) and is the point's Table 13-1 trigger, so two events in a row
//! must never share a time, or the second sends no COV report. Several events
//! of one access transaction share a tag (Clause 12.31.27.1), so the time
//! can't be derived from the tag. The point stamps each event strictly after
//! the time it served before:
//!
//! - With a usable Device clock, the clock's date and time, unless that
//!   isn't later than the time served, as when two events land in the same
//!   hundredth or the clock was set back. The stamp is then one hundredth of
//!   a second past the time served, carried into the seconds, minutes, hours
//!   and the next day as needed.
//! - Without one, the sequence-number form (Clause 21.6): one past the
//!   sequence number served, counting from 1 when the time served is in
//!   another form, and wrapping from 65535 back to 1. It never takes 0, the
//!   value of an update time with no update yet.
//!
//! A time the application passes with an event is served as given.

use bacnet_types::calendar::SpecificDate;
use bacnet_types::primitives::{BACnetTimeStamp, Date, Time};

use super::next_sequence;
use crate::clock::{current_datetime, ClockReader};

/// A date and time with every field given, ordered from the year down to
/// the hundredths.
type Instant = (SpecificDate, [u8; 4]);

/// The Access_Event_Time for an event recorded now, after `previous`.
pub(super) fn next_event_time(
    clock: Option<&dyn ClockReader>,
    previous: &BACnetTimeStamp,
) -> BACnetTimeStamp {
    let Some((date, time)) = current_datetime(clock) else {
        let last = match previous {
            BACnetTimeStamp::SequenceNumber(number) => *number,
            _ => 0,
        };
        return BACnetTimeStamp::SequenceNumber(next_sequence(last));
    };
    let now = BACnetTimeStamp::DateTime { date, time };
    let BACnetTimeStamp::DateTime {
        date: last_date,
        time: last_time,
    } = previous
    else {
        return now;
    };
    match (instant(&date, &time), instant(last_date, last_time)) {
        (Some(current), Some(last)) if current <= last => one_hundredth_after(last).unwrap_or(now),
        _ => now,
    }
}

/// `date` and `time` as an [`Instant`], or `None` unless both are fully
/// specified and the date names a real day.
fn instant(date: &Date, time: &Time) -> Option<Instant> {
    let day = SpecificDate::from_date(date)?;
    time.is_specific()
        .then_some((day, [time.hour, time.minute, time.second, time.hundredths]))
}

/// The date and time one hundredth of a second after `instant`, or `None`
/// past the last day a BACnet Date holds.
fn one_hundredth_after(
    (day, [hour, minute, second, hundredths]): Instant,
) -> Option<BACnetTimeStamp> {
    let mut time = [hour, minute, second, hundredths.saturating_add(1)];
    // Carry each field that reaches its limit into the one above it.
    for (field, limit) in [(3, 100), (2, 60), (1, 60)] {
        if time[field] >= limit {
            time[field] = 0;
            time[field - 1] = time[field - 1].saturating_add(1);
        }
    }
    let day = if time[0] >= 24 {
        time[0] = 0;
        next_day(day)?
    } else {
        day
    };
    let [hour, minute, second, hundredths] = time;
    Some(BACnetTimeStamp::DateTime {
        date: day.to_date(),
        time: Time {
            hour,
            minute,
            second,
            hundredths,
        },
    })
}

/// The day after `day`.
fn next_day(day: SpecificDate) -> Option<SpecificDate> {
    SpecificDate::new(day.year(), day.month(), day.day() + 1)
        .or_else(|| SpecificDate::new(day.year(), day.month() + 1, 1))
        .or_else(|| SpecificDate::new(day.year() + 1, 1, 1))
}
