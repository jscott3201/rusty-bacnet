use super::*;

#[test]
fn reference_times_take_minutes_seconds_and_hundredths() {
    let (date, time) = parse_reference_time("2026-10-05T09:30").unwrap();
    assert_eq!(
        date,
        Date {
            year: 126,
            month: 10,
            day: 5,
            day_of_week: 1,
        }
    );
    assert_eq!(
        time,
        Time {
            hour: 9,
            minute: 30,
            second: 0,
            hundredths: 0,
        }
    );
    let (_, time) = parse_reference_time("2026-10-05 09:30:15.25").unwrap();
    assert_eq!((time.second, time.hundredths), (15, 25));
    for bad in [
        "2026-10-05",
        "2026-02-30T09:00",
        "2026-10-05T24:00",
        "2026-10-05T09:00:00.5",
        "2026/10/05T09:00",
        "1899-12-31T23:59",
    ] {
        assert!(parse_reference_time(bad).is_err(), "{bad}");
    }
}

#[test]
fn a_start_picks_the_range_and_count_defaults() {
    let by_position = RangeOptions {
        position: Some(3),
        ..RangeOptions::default()
    };
    assert_eq!(
        by_position.spec().unwrap(),
        Some(RangeSpec::ByPosition {
            reference_index: 3,
            count: 100
        })
    );
    let backward = RangeOptions {
        sequence: Some(40),
        count: Some(-5),
        ..RangeOptions::default()
    };
    assert_eq!(
        backward.spec().unwrap(),
        Some(RangeSpec::BySequenceNumber {
            reference_seq: 40,
            count: -5
        })
    );
    assert_eq!(RangeOptions::default().spec().unwrap(), None);
    let count_alone = RangeOptions {
        count: Some(5),
        ..RangeOptions::default()
    };
    assert!(count_alone.spec().is_err());
    let zero = RangeOptions {
        position: Some(1),
        count: Some(0),
        ..RangeOptions::default()
    };
    assert!(zero.spec().is_err());
}

#[test]
fn all_pages_forward_from_the_start_given() {
    let all = RangeOptions {
        all: true,
        ..RangeOptions::default()
    };
    assert_eq!(all.pages().unwrap(), (LogCursor::Oldest, 100));
    let from = RangeOptions {
        all: true,
        sequence: Some(9),
        count: Some(25),
        ..RangeOptions::default()
    };
    assert_eq!(from.pages().unwrap(), (LogCursor::Sequence(9), 25));
    let backward = RangeOptions {
        all: true,
        count: Some(-25),
        ..RangeOptions::default()
    };
    assert!(backward.pages().is_err());
}
