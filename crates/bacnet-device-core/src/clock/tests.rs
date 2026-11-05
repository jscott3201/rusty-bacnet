//! The clock on hand-set time: where it starts, how each basis sets it, how
//! it advances on `bacnet-time`, and the recipient windows it opens.
use super::*;
use bacnet_time::Duration;
use bacnet_types::constructed::BACnetRecipient;
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;

#[path = "responder_tests.rs"]
mod responder;

pub(super) fn date(year: u16, month: u8, day: u8, day_of_week: u8) -> Date {
    Date {
        year: (year - 1900) as u8,
        month,
        day,
        day_of_week,
    }
}

pub(super) fn time(hour: u8, minute: u8, second: u8, hundredths: u8) -> Time {
    Time {
        hour,
        minute,
        second,
        hundredths,
    }
}

/// An arbitrary instant on the host's timeline, far from its epoch.
const T0: Instant = Instant::from_secs(5_000);

fn zone(utc_offset: i16, daylight_savings: bool) -> Zone {
    Zone::new(utc_offset, daylight_savings).unwrap()
}

fn after(seconds: u64, nanos: u64) -> Instant {
    T0 + Duration::from_secs(seconds) + Duration::from_nanos(nanos)
}

/// The local date and time `clock` reads at `now`.
fn local(clock: &SyncedClock, now: Instant) -> (Date, Time) {
    let frame = clock.frame(now).unwrap();
    (frame.local_date, frame.local_time)
}

fn set(clock_zone: Zone, basis: Basis, date: Date, time: Time) -> SyncedClock {
    let mut clock = SyncedClock::new(clock_zone, T0);
    clock.set(basis, date, time, T0).unwrap();
    clock
}

/// A destination on `days`, `from` to `to`, for every transition.
fn destination(days: DaysOfWeek, from: Time, to: Time) -> BACnetDestination {
    BACnetDestination {
        valid_days: days,
        from_time: from,
        to_time: to,
        recipient: BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap()),
        process_identifier: 1,
        issue_confirmed_notifications: false,
        transitions: bacnet_types::bitstring::EventTransitionBits::all(),
    }
}

fn always() -> BACnetDestination {
    destination(DaysOfWeek::all(), time(0, 0, 0, 0), time(23, 59, 59, 99))
}

#[test]
fn zones_take_quarter_hours_within_a_day() {
    assert!(Zone::new(-1440, false).is_ok());
    assert!(Zone::new(1440, true).is_ok());
    assert!(Zone::new(-345, false).is_ok());
    for refused in [14, -14, 1441, -1455, i16::MAX] {
        assert!(Zone::new(refused, false).is_err(), "{refused}");
    }
    let zone = zone(300, true);
    assert_eq!((zone.utc_offset(), zone.daylight_savings()), (300, true));
}

/// Unsynchronized, the clock reads local midnight on Monday 1990-01-01 at
/// the instant it was built, in any zone, and counts from there; it opens
/// no window.
#[test]
fn an_unsynced_clock_starts_at_the_restart_value_and_counts() {
    for (offset, summer) in [(0, false), (300, true), (-570, false), (1440, true)] {
        let clock = SyncedClock::new(zone(offset, summer), T0);
        assert!(!clock.is_synced());
        assert_eq!(local(&clock, T0), (date(1990, 1, 1, 1), time(0, 0, 0, 0)));
        // A day, an hour, a minute, a second and a half later.
        assert_eq!(
            local(&clock, after(90_061, 500_000_000)),
            (date(1990, 1, 2, 2), time(1, 1, 1, 50))
        );
        let frame = clock.frame(T0).unwrap();
        assert_eq!(frame.utc_offset, offset);
        assert_eq!(frame.daylight_savings_status, summer);
        assert!(frame.is_valid_actual_datetime());
        assert_eq!(clock.window(T0), None);
        assert_eq!(clock.window(after(86_400, 0)), None);
    }
}

/// A local TimeSynchronization reads back as given whatever the zone; a
/// UTC one reads back less UTC_Offset, plus an hour in daylight-saving
/// time. The pairs here are the server's own (`clock.rs` tests).
#[test]
fn each_basis_sets_local_time_as_the_server_does() {
    let eastern_summer = zone(300, true);
    let local_set = set(
        eastern_summer,
        Basis::Local,
        date(2024, 7, 4, 4),
        time(9, 15, 0, 0),
    );
    assert!(local_set.is_synced());
    assert_eq!(
        local(&local_set, T0),
        (date(2024, 7, 4, 4), time(9, 15, 0, 0))
    );

    let utc_set = set(
        eastern_summer,
        Basis::Utc,
        date(2024, 7, 4, 4),
        time(13, 15, 0, 0),
    );
    assert_eq!(
        local(&utc_set, T0),
        (date(2024, 7, 4, 4), time(9, 15, 0, 0))
    );
    // The same instant either way.
    assert_eq!(utc_set, local_set);

    // UTC 2024-03-01 (a Friday) 00:30:00.25: an hour west is the evening
    // before, a leap day; an hour east is 01:30; west with daylight saving
    // back at 00:30.
    let utc = (date(2024, 3, 1, 5), time(0, 30, 0, 25));
    let cases = [
        ((60, false), (date(2024, 2, 29, 4), time(23, 30, 0, 25))),
        ((-60, false), (date(2024, 3, 1, 5), time(1, 30, 0, 25))),
        ((60, true), (date(2024, 3, 1, 5), time(0, 30, 0, 25))),
    ];
    for ((offset, summer), expected) in cases {
        let clock = set(zone(offset, summer), Basis::Utc, utc.0, utc.1);
        assert_eq!(
            local(&clock, T0),
            expected,
            "offset {offset}, summer {summer}"
        );
        let (today, now) = clock.window(T0).unwrap();
        assert_eq!(
            today,
            DaysOfWeek::from_bits_truncate(1 << (expected.0.day_of_week - 1))
        );
        assert_eq!(now, expected.1);
    }
}

/// Elapsed time comes apart through `bacnet-time`'s long division and
/// 32-bit arithmetic: the hundredth turns at 10 ms, not before, and a long
/// run lands on the right day across a leap day and a year end.
#[test]
fn the_clock_advances_on_bacnet_time() {
    for (anchor, next) in [
        (time(0, 0, 0, 99), time(0, 0, 1, 0)),
        (time(0, 0, 59, 99), time(0, 1, 0, 0)),
        (time(23, 59, 59, 99), time(0, 0, 0, 0)),
    ] {
        let clock = set(Zone::default(), Basis::Utc, date(2024, 7, 4, 4), anchor);
        assert_eq!(local(&clock, after(0, 9_999_999)).1, anchor);
        assert_eq!(local(&clock, after(0, 10_000_000)).1, next);
    }
    // Sunday 2023-12-31 12:00, then 427 days, 3 hours, 4 minutes and
    // 5.999999999 seconds: Sunday 2025-03-02 15:04:05.99.
    let clock = set(
        Zone::default(),
        Basis::Local,
        date(2023, 12, 31, 7),
        time(12, 0, 0, 0),
    );
    let seconds = 427 * 86_400 + 3 * 3_600 + 4 * 60 + 5;
    assert_eq!(
        local(&clock, after(seconds, 999_999_999)),
        (date(2025, 3, 2, 7), time(15, 4, 5, 99))
    );
    // An instant before the anchor reads as the anchor.
    assert_eq!(
        local(&clock, T0 - Duration::from_secs(60)),
        local(&clock, T0)
    );
    // Past 2154 there is no frame.
    let late = set(
        Zone::default(),
        Basis::Utc,
        date(2154, 12, 31, 2),
        time(23, 59, 59, 99),
    );
    assert!(late.frame(after(0, 10_000_000)).is_none());
}

/// A refused value leaves the clock as it was, unsynchronized here.
#[test]
fn a_value_that_isnt_specific_changes_nothing() {
    let refused = [
        (date(2023, 2, 29, 3), time(12, 0, 0, 0)),
        (date(2024, 2, 29, 5), time(12, 0, 0, 0)),
        (date(2024, 13, 1, 1), time(12, 0, 0, 0)),
        (
            Date {
                year: Date::UNSPECIFIED,
                ..date(2024, 2, 29, 4)
            },
            time(12, 0, 0, 0),
        ),
        (
            Date {
                day_of_week: Date::UNSPECIFIED,
                ..date(2024, 2, 29, 4)
            },
            time(12, 0, 0, 0),
        ),
        (date(2024, 2, 29, 4), time(24, 0, 0, 0)),
        (date(2024, 2, 29, 4), time(Time::UNSPECIFIED, 0, 0, 0)),
        (date(2024, 2, 29, 4), time(12, 0, 0, Time::UNSPECIFIED)),
    ];
    let mut clock = SyncedClock::new(zone(60, false), T0);
    let before = clock;
    for (date, time) in refused {
        for basis in [Basis::Local, Basis::Utc] {
            assert!(
                clock.set(basis, date, time, after(1, 0)).is_err(),
                "{date:?} {time:?}"
            );
            assert_eq!(clock, before);
        }
    }
    // A UTC value whose local date would be 1899.
    assert!(
        clock
            .set(Basis::Utc, date(1900, 1, 1, 1), time(0, 30, 0, 0), T0)
            .is_err()
    );
    assert_eq!(clock, before);
}

/// Unsynchronized, only a destination open every day and all day takes
/// notifications; once synchronized, each destination's own days and
/// window decide.
#[test]
fn unsynced_fails_closed_and_a_sync_opens_the_windows() {
    let weekdays = DaysOfWeek::all() - DaysOfWeek::SATURDAY - DaysOfWeek::SUNDAY;
    let business = destination(weekdays, time(8, 0, 0, 0), time(17, 0, 0, 0));
    let unspecified = destination(
        DaysOfWeek::all(),
        time(Time::UNSPECIFIED, 0, 0, 0),
        time(Time::UNSPECIFIED, 0, 0, 0),
    );
    // Across midnight and leaving no hundredth out: open all day.
    let wrapped = destination(DaysOfWeek::all(), time(12, 0, 0, 0), time(11, 59, 59, 99));
    let gap = destination(DaysOfWeek::all(), time(12, 0, 0, 0), time(11, 59, 59, 98));
    let not_sunday = destination(
        DaysOfWeek::all() - DaysOfWeek::SUNDAY,
        time(0, 0, 0, 0),
        time(23, 59, 59, 99),
    );
    let short_day = destination(DaysOfWeek::all(), time(0, 0, 0, 1), time(23, 59, 59, 99));

    let mut clock = SyncedClock::new(zone(300, false), T0);
    let unsynced = clock.window(T0);
    assert_eq!(unsynced, None);
    for (dest, open) in [
        (&always(), true),
        (&unspecified, true),
        (&wrapped, true),
        (&business, false),
        (&gap, false),
        (&not_sunday, false),
        (&short_day, false),
    ] {
        assert_eq!(destination_open(dest, unsynced), open, "{dest:?}");
    }

    // Wednesday 2024-07-03, 09:00 local.
    clock
        .set(Basis::Local, date(2024, 7, 3, 3), time(9, 0, 0, 0), T0)
        .unwrap();
    let synced = clock.window(T0);
    assert_eq!(synced, Some((DaysOfWeek::WEDNESDAY, time(9, 0, 0, 0))));
    for dest in [&business, &always(), &gap, &not_sunday, &short_day] {
        assert!(destination_open(dest, synced), "{dest:?}");
    }
    // 17:00:00.00 is the window's last hundredth; 17:00:00.01 is past it.
    assert!(destination_open(
        &business,
        clock.window(after(8 * 3_600, 0))
    ));
    assert!(!destination_open(
        &business,
        clock.window(after(8 * 3_600, 10_000_000))
    ));
}

/// A Saturday-only destination open to 23:59:59.99 closes as the clock
/// turns to Sunday, when a Sunday one from 00:00 opens; an overnight window
/// on Saturday holds past midnight only on the days it names.
#[test]
fn windows_turn_at_midnight_and_the_day_of_the_week() {
    let saturday = destination(DaysOfWeek::SATURDAY, time(0, 0, 0, 0), time(23, 59, 59, 99));
    let sunday = destination(DaysOfWeek::SUNDAY, time(0, 0, 0, 0), time(0, 0, 0, 0));
    let overnight = destination(DaysOfWeek::SATURDAY, time(22, 0, 0, 0), time(2, 0, 0, 0));
    // UTC 2024-07-07 04:59:59.99 is Saturday 23:59:59.99 at UTC_Offset 300.
    let clock = set(
        zone(300, false),
        Basis::Utc,
        date(2024, 7, 7, 7),
        time(4, 59, 59, 99),
    );
    let before = clock.window(T0);
    let after_midnight = clock.window(after(0, 10_000_000));
    assert_eq!(before, Some((DaysOfWeek::SATURDAY, time(23, 59, 59, 99))));
    assert_eq!(after_midnight, Some((DaysOfWeek::SUNDAY, time(0, 0, 0, 0))));
    assert!(destination_open(&saturday, before));
    assert!(!destination_open(&saturday, after_midnight));
    assert!(!destination_open(&sunday, before));
    assert!(destination_open(&sunday, after_midnight));
    assert!(destination_open(&overnight, before));
    // Sunday 00:00 is inside 22:00-02:00, but Sunday isn't a valid day.
    assert!(!destination_open(&overnight, after_midnight));
}

/// Daylight-saving time moves local time, and so the windows, an hour.
#[test]
fn daylight_saving_moves_the_window_an_hour() {
    let morning = destination(DaysOfWeek::all(), time(8, 0, 0, 0), time(8, 59, 59, 99));
    let utc = (date(2024, 7, 4, 4), time(13, 30, 0, 0));
    let standard = set(zone(300, false), Basis::Utc, utc.0, utc.1);
    let summer = set(zone(300, true), Basis::Utc, utc.0, utc.1);
    assert!(destination_open(&morning, standard.window(T0)));
    assert!(!destination_open(&morning, summer.window(T0)));
}

/// The lent clock reads the host's timer each time the database asks.
#[test]
fn a_device_clock_reads_the_hosts_timer() {
    use std::sync::atomic::{AtomicU64, Ordering};
    let nanos = Arc::new(AtomicU64::new(T0.as_nanos()));
    let timer = Arc::clone(&nanos);
    let mut clock = DeviceClock::new(
        zone(0, false),
        Arc::new(move || Instant::from_nanos(timer.load(Ordering::Relaxed))),
    );
    assert_eq!(clock.read_clock().unwrap().local_date, date(1990, 1, 1, 1));
    clock
        .set(Basis::Utc, date(2026, 11, 4, 3), time(10, 0, 0, 0))
        .unwrap();
    nanos.fetch_add(61_000_000_000, Ordering::Relaxed);
    assert_eq!(clock.now(), after(61, 0));
    let frame = clock.read_clock().unwrap();
    assert_eq!(
        (frame.local_date, frame.local_time),
        (date(2026, 11, 4, 3), time(10, 1, 1, 0))
    );
    assert!(clock.clock().is_synced());
}
