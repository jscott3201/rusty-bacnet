//! A zone change at run time (#2395): what a synchronized clock keeps, what
//! one never set keeps, what is refused, and the recipient windows after it.
use super::{T0, after, date, destination, local, set, time, zone};
use crate::clock::{Basis, SyncedClock, destination_open};
use bacnet_types::bitstring::DaysOfWeek;

/// Thursday 2024-07-04, 13:15 UTC, five hours west: 08:15 local.
fn synced() -> SyncedClock {
    set(
        zone(300, false),
        Basis::Utc,
        date(2024, 7, 4, 4),
        time(13, 15, 0, 0),
    )
}

/// Daylight saving moves a synchronized clock's local time an hour, and an
/// offset change moves it by the difference. The clock stays synchronized
/// and keeps its UTC instant: it is the clock a UTC synchronization of the
/// same value sets in the new zone, and it runs on from there.
#[test]
fn a_zone_change_moves_local_time_and_keeps_the_sync() {
    let mut clock = synced();
    assert_eq!(local(&clock, T0), (date(2024, 7, 4, 4), time(8, 15, 0, 0)));
    let steps = [
        ((300, true), (date(2024, 7, 4, 4), time(9, 15, 0, 0))),
        ((360, true), (date(2024, 7, 4, 4), time(8, 15, 0, 0))),
        // 10:45 east is 24:00, midnight on Friday; daylight saving reads
        // 01:00, and a quarter-hour less east in standard time is Thursday
        // again.
        ((-645, false), (date(2024, 7, 5, 5), time(0, 0, 0, 0))),
        ((-645, true), (date(2024, 7, 5, 5), time(1, 0, 0, 0))),
        ((-630, false), (date(2024, 7, 4, 4), time(23, 45, 0, 0))),
    ];
    for ((offset, summer), expected) in steps {
        // The change is made a minute and a half after the synchronization.
        let now = after(90, 0);
        clock.set_zone(zone(offset, summer), now).unwrap();
        assert!(clock.is_synced(), "offset {offset}, summer {summer}");
        assert_eq!(clock.zone(), zone(offset, summer));
        assert_eq!(local(&clock, T0), expected, "{offset}, {summer}");
        let frame = clock.frame(now).unwrap();
        assert_eq!(
            (frame.utc_offset, frame.daylight_savings_status),
            (offset, summer)
        );
        let (today, at) = clock.window(T0).unwrap();
        assert_eq!(
            today,
            DaysOfWeek::from_bits_truncate(1 << (expected.0.day_of_week - 1))
        );
        assert_eq!(at, expected.1);
        let resynced = set(
            zone(offset, summer),
            Basis::Utc,
            date(2024, 7, 4, 4),
            time(13, 15, 0, 0),
        );
        assert_eq!(clock, resynced);
    }
    assert_eq!(
        local(&clock, after(61, 500_000_000)),
        (date(2024, 7, 4, 4), time(23, 46, 1, 50))
    );
}

/// A clock that was never set has no UTC instant to keep: it goes on
/// counting local time from 1990-01-01 in the new zone, as one built there
/// reads, stays unsynchronized and opens no window. The first
/// synchronization is then read in the new zone.
#[test]
fn an_unsynced_clock_keeps_its_count_of_local_time() {
    for (offset, summer) in [(300, true), (-645, false), (1440, true), (-1440, false)] {
        let mut clock = SyncedClock::new(zone(60, false), T0);
        clock
            .set_zone(zone(offset, summer), after(3_600, 0))
            .unwrap();
        assert!(!clock.is_synced());
        assert_eq!(clock, SyncedClock::new(zone(offset, summer), T0));
        assert_eq!(
            local(&clock, after(3_661, 0)),
            (date(1990, 1, 1, 1), time(1, 1, 1, 0))
        );
        let frame = clock.frame(T0).unwrap();
        assert_eq!(
            (frame.utc_offset, frame.daylight_savings_status),
            (offset, summer)
        );
        assert_eq!(clock.window(after(3_661, 0)), None);
    }
    let mut clock = SyncedClock::new(zone(0, false), T0);
    clock.set_zone(zone(300, true), T0).unwrap();
    clock
        .set(Basis::Utc, date(2024, 7, 4, 4), time(13, 15, 0, 0), T0)
        .unwrap();
    assert_eq!(local(&clock, T0), (date(2024, 7, 4, 4), time(9, 15, 0, 0)));
}

/// A zone that would put the local date outside 1900 to 2154 is refused,
/// and the clock is as it was: still synchronized, in its old zone.
#[test]
fn a_zone_out_of_range_is_refused_and_changes_nothing() {
    // 1900-01-01 was a Monday; an hour west of 00:30 UTC is 1899.
    let mut first = set(
        zone(0, false),
        Basis::Utc,
        date(1900, 1, 1, 1),
        time(0, 30, 0, 0),
    );
    let before = first;
    assert!(first.set_zone(zone(60, false), T0).is_err());
    assert_eq!(first, before);
    // In daylight saving the same offset is still 00:30.
    first.set_zone(zone(60, true), T0).unwrap();
    assert_eq!(local(&first, T0), (date(1900, 1, 1, 1), time(0, 30, 0, 0)));

    // 2154-12-31 was a Tuesday; daylight saving at 23:30 would be 2155.
    let mut last = set(
        zone(0, false),
        Basis::Utc,
        date(2154, 12, 31, 2),
        time(23, 30, 0, 0),
    );
    let before = last;
    assert!(last.set_zone(zone(0, true), T0).is_err());
    assert_eq!(last, before);
    // The range is judged at the instant of the change: fine now, refused
    // once the clock has run 20 minutes closer to midnight.
    assert!(last.set_zone(zone(-15, false), after(1_200, 0)).is_err());
    assert_eq!(last, before);
    last.set_zone(zone(-15, false), T0).unwrap();
    assert!(last.is_synced());
}

/// Recipient windows follow the new local time: a morning window open at
/// 08:15 closes when daylight saving makes it 09:15 and opens again an hour
/// further west; a Saturday destination closes when the change carries the
/// clock into Sunday.
#[test]
fn recipient_windows_follow_the_new_local_time() {
    let morning = destination(DaysOfWeek::all(), time(8, 0, 0, 0), time(8, 59, 59, 99));
    let mut clock = synced();
    assert!(destination_open(&morning, clock.window(T0)));
    clock.set_zone(zone(300, true), T0).unwrap();
    assert!(!destination_open(&morning, clock.window(T0)));
    clock.set_zone(zone(360, true), T0).unwrap();
    assert!(destination_open(&morning, clock.window(T0)));
    // It closes at 09:00 by the new local time: 45 minutes on.
    assert!(destination_open(&morning, clock.window(after(2_699, 0))));
    assert!(!destination_open(&morning, clock.window(after(2_700, 0))));

    // Saturday 2024-07-06, 23:30 UTC.
    let saturday = destination(DaysOfWeek::SATURDAY, time(0, 0, 0, 0), time(23, 59, 59, 99));
    let mut clock = set(
        zone(0, false),
        Basis::Utc,
        date(2024, 7, 6, 6),
        time(23, 30, 0, 0),
    );
    assert!(destination_open(&saturday, clock.window(T0)));
    clock.set_zone(zone(0, true), T0).unwrap();
    assert_eq!(
        clock.window(T0),
        Some((DaysOfWeek::SUNDAY, time(0, 30, 0, 0)))
    );
    assert!(!destination_open(&saturday, clock.window(T0)));
}
