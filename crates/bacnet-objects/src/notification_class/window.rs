//! When a Recipient_List destination's valid days and time window are open:
//! the one test the server's distribution and `bacnet-device-core`'s clock
//! both judge by, so the two agree on any day and time.

use bacnet_types::bitstring::DaysOfWeek;
use bacnet_types::constructed::BACnetDestination;
use bacnet_types::primitives::Time;

/// The last hundredth of a day, 23:59:59.99, in hundredths since midnight.
const LAST_HUNDREDTH: u32 = 24 * 360_000 - 1;

/// Convert a `Time` to centiseconds (hundredths of a second since midnight).
fn time_to_centiseconds(t: &Time) -> u32 {
    let field = |value: u8| {
        if value == Time::UNSPECIFIED {
            0
        } else {
            u32::from(value)
        }
    };
    field(t.hour) * 360_000 + field(t.minute) * 6_000 + field(t.second) * 100 + field(t.hundredths)
}

/// Check if `current` falls within the `[from, to]` time window.
///
/// If either bound has an unspecified hour (0xFF), the window is treated as
/// "all day". A window whose `to` is earlier than its `from` (e.g.
/// 22:00–02:00) crosses midnight: it is active from `from` up to midnight and
/// again from midnight up to `to`, i.e. when `current >= from || current <= to`.
/// This matches the ASHRAE 135-2020 reading that `To_Time` is the end of the
/// active period; a wrap-around pair denotes an overnight schedule.
pub(super) fn time_in_window(current: &Time, from: &Time, to: &Time) -> bool {
    if from.hour == Time::UNSPECIFIED || to.hour == Time::UNSPECIFIED {
        return true;
    }
    let cur = time_to_centiseconds(current);
    let from_cs = time_to_centiseconds(from);
    let to_cs = time_to_centiseconds(to);
    if to_cs < from_cs {
        // Overnight window crossing midnight.
        cur >= from_cs || cur <= to_cs
    } else {
        cur >= from_cs && cur <= to_cs
    }
}

/// Whether `destination` takes notifications on `today` (the local day, a
/// single [`DaysOfWeek`] flag) at local time `now`: `today` is one of its
/// valid days and `now` falls in its window, both ends included, as
/// [`lookup_notification_recipients`](super::lookup_notification_recipients)
/// selects. Its transitions aren't looked at.
pub fn in_window(destination: &BACnetDestination, today: DaysOfWeek, now: &Time) -> bool {
    destination.valid_days.intersects(today)
        && time_in_window(now, &destination.from_time, &destination.to_time)
}

/// Whether `destination` is open whatever the day and time: every day is
/// valid and its window covers the whole day by the test [`in_window`]
/// applies (an unspecified hour, 00:00:00.00 to 23:59:59.99, or a window
/// across midnight that leaves no hundredth out). A device that can't tell
/// the day and time sends only to such destinations.
pub fn always_in_window(destination: &BACnetDestination) -> bool {
    if !destination.valid_days.contains(DaysOfWeek::all()) {
        return false;
    }
    let (from, to) = (&destination.from_time, &destination.to_time);
    if from.hour == Time::UNSPECIFIED || to.hour == Time::UNSPECIFIED {
        return true;
    }
    let (from, to) = (time_to_centiseconds(from), time_to_centiseconds(to));
    if to < from {
        // Across midnight: open from `from` on, and up to `to`.
        to + 1 >= from
    } else {
        from == 0 && to >= LAST_HUNDREDTH
    }
}
