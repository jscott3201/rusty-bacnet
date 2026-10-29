//! Dates, times, date-times and timestamps in the formatter's forms.

use bacnet_types::primitives::{BACnetTimeStamp, Date, PropertyValue, Time};

use super::{parse_date, parse_date_time, parse_time, parse_timestamp, parse_value};

const ANY: u8 = 0xFF;

fn date(year: u8, month: u8, day: u8, day_of_week: u8) -> Date {
    Date {
        year,
        month,
        day,
        day_of_week,
    }
}

fn time(hour: u8, minute: u8, second: u8, hundredths: u8) -> Time {
    Time {
        hour,
        minute,
        second,
        hundredths,
    }
}

#[test]
fn dates_take_wildcards_special_values_and_a_weekday() {
    for (text, expected) in [
        ("2026-10-19", date(126, 10, 19, ANY)),
        ("2026-10-19 Mon", date(126, 10, 19, 1)),
        ("2026-10-25 sunday", date(126, 10, 25, 7)),
        ("*-*-* Fri", date(ANY, ANY, ANY, 5)),
        ("*-even-months-last-day", date(ANY, 14, 32, ANY)),
        ("2026-odd-months-odd-days", date(126, 13, 33, ANY)),
        ("*-12-even-days", date(ANY, 12, 34, ANY)),
        ("1900-01-01", date(0, 1, 1, ANY)),
        ("any date", date(ANY, ANY, ANY, ANY)),
    ] {
        assert_eq!(parse_date(text), Ok(expected), "{text}");
    }
}

#[test]
fn a_bad_date_is_refused_with_the_field() {
    for (text, error) in [
        ("1899-01-01", "invalid year 1899"),
        ("2155-01-01", "invalid year 2155"),
        ("2026-15-01", "invalid month '15'"),
        ("2026-10-35", "invalid day '35'"),
        ("2026-10", "invalid date"),
        ("2026-10-19 07:00", "has a time"),
        ("2026-10-19 Mon 07:00 extra", "unexpected 'extra'"),
    ] {
        let refused = parse_date(text).unwrap_err();
        assert!(refused.contains(error), "{text}: {refused}");
    }
}

#[test]
fn times_leave_off_zero_seconds_and_hundredths() {
    for (text, expected) in [
        ("07:00", time(7, 0, 0, 0)),
        ("14:30:05", time(14, 30, 5, 0)),
        ("14:30:05.25", time(14, 30, 5, 25)),
        ("*:30", time(ANY, 30, 0, 0)),
        ("07:00:*", time(7, 0, ANY, 0)),
        ("07:00:00.*", time(7, 0, 0, ANY)),
        ("any time", time(ANY, ANY, ANY, ANY)),
    ] {
        assert_eq!(parse_time(text), Ok(expected), "{text}");
    }
    for (text, error) in [
        ("24:00", "invalid hour '24'"),
        ("07:60", "invalid minute '60'"),
        ("07:00.5", "hundredths need seconds"),
        ("14:30:05.5", "two digits"),
        ("14:30:05.500", "two digits"),
        ("07", "expected HH:MM"),
    ] {
        let refused = parse_time(text).unwrap_err();
        assert!(refused.contains(error), "{text}: {refused}");
    }
}

#[test]
fn a_date_time_has_an_optional_time() {
    assert_eq!(
        parse_date_time("2026-10-25 Sun 14:30:05"),
        Ok((date(126, 10, 25, 7), Some(time(14, 30, 5, 0))))
    );
    assert_eq!(
        parse_date_time("any date 07:00"),
        Ok((date(ANY, ANY, ANY, ANY), Some(time(7, 0, 0, 0))))
    );
    assert_eq!(
        parse_date_time("2026-10-25 any time"),
        Ok((date(126, 10, 25, ANY), Some(time(ANY, ANY, ANY, ANY))))
    );
    assert_eq!(
        parse_date_time("unspecified"),
        Ok((date(ANY, ANY, ANY, ANY), Some(time(ANY, ANY, ANY, ANY))))
    );
    assert_eq!(
        parse_date_time("2026-10-25"),
        Ok((date(126, 10, 25, ANY), None))
    );
}

#[test]
fn timestamps_read_the_three_printed_forms() {
    assert_eq!(
        parse_timestamp("seq 42"),
        Ok(BACnetTimeStamp::SequenceNumber(42))
    );
    assert_eq!(
        parse_timestamp("14:30:05"),
        Ok(BACnetTimeStamp::Time(time(14, 30, 5, 0)))
    );
    assert_eq!(
        parse_timestamp("2026-10-25 Sun 14:30:05"),
        Ok(BACnetTimeStamp::DateTime {
            date: date(126, 10, 25, 7),
            time: time(14, 30, 5, 0),
        })
    );
    // A date alone leaves the time unspecified, as the formatter prints it.
    assert_eq!(
        parse_timestamp("2026-10-25"),
        Ok(BACnetTimeStamp::DateTime {
            date: date(126, 10, 25, ANY),
            time: time(ANY, ANY, ANY, ANY),
        })
    );
    assert!(
        parse_timestamp("seq 70000")
            .unwrap_err()
            .contains("0-65535")
    );
    assert!(parse_timestamp("soon").unwrap_err().contains("seq N"));
}

#[test]
fn the_untyped_grammar_reads_bare_dates_times_and_bits() {
    assert_eq!(
        parse_value("2026-10-19"),
        Ok(PropertyValue::Date(date(126, 10, 19, ANY)))
    );
    assert_eq!(
        parse_value("07:30"),
        Ok(PropertyValue::Time(time(7, 30, 0, 0)))
    );
    assert_eq!(
        parse_value("2026-10-19 Mon 07:30"),
        Ok(PropertyValue::List(vec![
            PropertyValue::Date(date(126, 10, 19, 1)),
            PropertyValue::Time(time(7, 30, 0, 0)),
        ]))
    );
    assert_eq!(
        parse_value("date:*-even-months-last-day"),
        Ok(PropertyValue::Date(date(ANY, 14, 32, ANY)))
    );
    assert_eq!(
        parse_value("time:14:30"),
        Ok(PropertyValue::Time(time(14, 30, 0, 0)))
    );
    assert_eq!(
        parse_value("bits(0101)"),
        Ok(PropertyValue::BitString {
            unused_bits: 4,
            data: vec![0b0101_0000],
        })
    );
    assert_eq!(
        parse_value("bits()"),
        Ok(PropertyValue::BitString {
            unused_bits: 0,
            data: vec![],
        })
    );
    assert!(parse_value("bits(012)").unwrap_err().contains("0/1"));
    // Numbers stay numbers.
    assert_eq!(parse_value("-5"), Ok(PropertyValue::Signed(-5)));
    assert_eq!(parse_value("2026"), Ok(PropertyValue::Unsigned(2026)));
}
