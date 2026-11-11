//! The text forms of the coming BACnet RDF encoding (#2473): an object
//! identifier as `type,instance`, a weekday digit or `*` after a date, and
//! a week-n-day as `month,week,day`. Each parses to the value its existing
//! form gives, and every value the formatter prints still parses back.

use bacnet_encoding::primitives::encode_property_value;
use bacnet_types::constructed::BACnetWeekNDay;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{BACnetTimeStamp, Date, ObjectIdentifier, PropertyValue, Time};
use bytes::BytesMut;
use proptest::prelude::*;

use super::{
    parse_date, parse_date_time, parse_object_specifier, parse_time, parse_timestamp, parse_value,
    parse_value_for, parse_week_n_day, parse_weekday_digit,
};
use crate::format::{Floats, StateTexts, ValueFormat, format_property_value};
use crate::label::kebab;

type O = ObjectType;
type P = PropertyIdentifier;

const ANY: u8 = 0xFF;

const NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

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

/// What the formatter prints for `value` of `property`, as a read shows it.
fn printed(object_type: ObjectType, property: PropertyIdentifier, value: &PropertyValue) -> String {
    let mut data = BytesMut::new();
    encode_property_value(&mut data, value).unwrap();
    ValueFormat::new(object_type, &StateTexts::default(), Floats::Exact)
        .whole_lists()
        .value_at(property, None, &data)
}

#[test]
fn an_object_identifier_takes_type_comma_instance() {
    for (rdf, colon) in [
        ("analog-input,3", "analog-input:3"),
        ("ANALOG_INPUT,3", "ANALOG_INPUT:3"),
        ("Analog-Input,3", "analog-input:3"),
        ("ai,3", "ai:3"),
        ("0,3", "0:3"),
        ("128,1", "128:1"),
        ("device,0", "device:0"),
        ("device,4194303", "device:4194303"),
        (" analog-input,3", "analog-input:3"),
    ] {
        assert_eq!(
            parse_object_specifier(rdf),
            parse_object_specifier(colon),
            "{rdf}"
        );
        assert!(parse_object_specifier(rdf).is_ok(), "{rdf}");
    }
    assert_eq!(
        parse_object_specifier("128,1"),
        Ok((ObjectType::from_raw(128), 1))
    );
    for (text, error) in [
        (
            "analog-input",
            "expected 'type:instance' or 'type,instance'",
        ),
        ("analog-input,", "invalid instance number: ''"),
        ("analog-input,x", "invalid instance number: 'x'"),
        ("analog-input, 3", "invalid instance number: ' 3'"),
        ("analog-input,*", "invalid instance number: '*'"),
        (",3", "unknown object type: ''"),
        ("no-such-type,3", "unknown object type"),
        // A colon splits first, so the comma is left in the instance.
        ("ai:1,2", "invalid instance number: '1,2'"),
        ("ai,1:2", "unknown object type: 'ai,1'"),
    ] {
        let refused = parse_object_specifier(text).unwrap_err();
        assert!(refused.contains(error), "{text}: {refused}");
    }
}

#[test]
fn the_instance_limit_is_the_one_every_form_has() {
    assert_eq!(
        parse_value("object:analog-input,4194303"),
        Ok(PropertyValue::ObjectIdentifier(
            ObjectIdentifier::new(O::ANALOG_INPUT, ObjectIdentifier::MAX_INSTANCE).unwrap()
        ))
    );
    let colon = parse_value("object:analog-input:4194304").unwrap_err();
    let comma = parse_value("object:analog-input,4194304").unwrap_err();
    assert_eq!(comma, colon);
    assert!(comma.starts_with("invalid object identifier"), "{comma}");
    assert!(
        comma.contains("instance number 4194304 exceeds max 4194303"),
        "{comma}"
    );
    // A type past the 10-bit field, by number.
    let colon = parse_value("object:1024:1").unwrap_err();
    assert_eq!(parse_value("object:1024,1").unwrap_err(), colon);
    assert_eq!(
        parse_value("object:analog-input,0"),
        parse_value("object:ai:0")
    );
}

#[test]
fn json_references_take_the_comma_form() {
    let event =
        |period: &str| format!(r#"{{"period": "{period}", "time_values": [], "priority": 10}}"#);
    for (rdf, colon) in [("calendar,1", "calendar:1"), ("6,1", "6:1")] {
        assert_eq!(
            parse_value_for(O::SCHEDULE, P::EXCEPTION_SCHEDULE, Some(1), &event(rdf)),
            parse_value_for(O::SCHEDULE, P::EXCEPTION_SCHEDULE, Some(1), &event(colon)),
            "{rdf}"
        );
    }
    let not_a_calendar =
        parse_value_for(O::SCHEDULE, P::EXCEPTION_SCHEDULE, Some(1), &event("av,1")).unwrap_err();
    assert!(not_a_calendar.contains("Calendar"), "{not_a_calendar}");
    let destination =
        |recipient: &str| format!(r#"{{"recipient": "{recipient}", "process_identifier": 7}}"#);
    assert_eq!(
        parse_value_for(
            O::NOTIFICATION_CLASS,
            P::RECIPIENT_LIST,
            Some(1),
            &destination("device,99")
        ),
        parse_value_for(
            O::NOTIFICATION_CLASS,
            P::RECIPIENT_LIST,
            Some(1),
            &destination("device:99")
        )
    );
}

#[test]
fn a_date_takes_a_weekday_digit_or_star() {
    for (rdf, existing) in [
        ("2026-11-10 2", "2026-11-10 Tue"),
        ("2026-11-10 *", "2026-11-10"),
        ("2026-11-* *", "2026-11-*"),
        ("*-*-* *", "any date"),
        ("*-*-* 5", "*-*-* Fri"),
        ("*-11-10 7", "*-11-10 Sun"),
        ("2026-*-10 1", "2026-*-10 monday"),
        ("2026-14-32 3", "2026-even-months-last-day Wed"),
        ("any date 4", "*-*-* Thu"),
    ] {
        assert_eq!(parse_date(rdf), parse_date(existing), "{rdf}");
        assert!(parse_date(rdf).is_ok(), "{rdf}");
        assert_eq!(parse_value(rdf), parse_value(existing), "{rdf}");
        assert_eq!(
            parse_value(&format!("date:{rdf}")),
            parse_value(&format!("date:{existing}")),
            "{rdf}"
        );
    }
    assert_eq!(parse_date("2026-11-10 2"), Ok(date(126, 11, 10, 2)));
    assert_eq!(parse_date("*-*-* *"), Ok(date(ANY, ANY, ANY, ANY)));
    for (text, error) in [
        ("2026-11-10 0", "invalid weekday '0'"),
        ("2026-11-10 8", "invalid weekday '8'"),
        ("2026-11-10 9", "invalid weekday '9'"),
        ("2026-11-10 **", "invalid time '**'"),
        ("2026-11-10 2 3", "invalid time '3'"),
        ("2026-11-10 2 *", "invalid time '*'"),
    ] {
        let refused = parse_date_time(text).unwrap_err();
        assert!(refused.contains(error), "{text}: {refused}");
    }
}

/// A weekday that isn't the date's own is kept as typed, by digit as by
/// name: 10 November 2026 is a Tuesday, and Wednesday is kept.
#[test]
fn a_weekday_that_disagrees_with_the_date_is_kept_as_typed() {
    assert_eq!(parse_date("2026-11-10 Wed"), Ok(date(126, 11, 10, 3)));
    assert_eq!(parse_date("2026-11-10 3"), Ok(date(126, 11, 10, 3)));
}

#[test]
fn a_date_time_string_takes_the_weekday_digit() {
    let expected = (date(126, 11, 10, 2), Some(time(16, 35, 0, 0)));
    for text in [
        "2026-11-10 2 16:35:00.00",
        "2026-11-10 Tue 16:35",
        "2026-11-10 2 16:35",
    ] {
        assert_eq!(parse_date_time(text), Ok(expected), "{text}");
    }
    assert_eq!(
        parse_date_time("2026-11-10 * 16:35:00.00"),
        parse_date_time("2026-11-10 16:35")
    );
    assert_eq!(
        parse_timestamp("2026-11-10 2 16:35:00.00"),
        Ok(BACnetTimeStamp::DateTime {
            date: date(126, 11, 10, 2),
            time: time(16, 35, 0, 0),
        })
    );
    assert_eq!(
        parse_value("2026-11-10 2 16:35:00.00"),
        parse_value("2026-11-10 Tue 16:35")
    );
    // A property that holds a BACnetDateTime, and a timestamp element.
    assert_eq!(
        parse_value_for(
            O::TREND_LOG,
            P::START_TIME,
            None,
            "2026-11-10 2 16:35:00.00"
        ),
        parse_value_for(O::TREND_LOG, P::START_TIME, None, "2026-11-10 Tue 16:35")
    );
    assert_eq!(
        parse_value_for(O::DATETIME_VALUE, P::PRESENT_VALUE, None, "2026-11-10 *"),
        parse_value_for(O::DATETIME_VALUE, P::PRESENT_VALUE, None, "2026-11-10")
    );
    assert_eq!(
        parse_value_for(
            O::ANALOG_INPUT,
            P::EVENT_TIME_STAMPS,
            Some(1),
            "2026-11-10 2 16:35:00.00"
        ),
        parse_value_for(
            O::ANALOG_INPUT,
            P::EVENT_TIME_STAMPS,
            Some(1),
            "2026-11-10 Tue 16:35"
        )
    );
}

/// A time already takes `*` in each field and the hundredths form.
#[test]
fn a_time_takes_star_in_each_field() {
    for (rdf, existing) in [
        ("*:*:*.*", "any time"),
        ("16:35:00.00", "16:35"),
        ("*:35:00.00", "*:35"),
        ("16:*:00.00", "16:*"),
        ("16:35:*.00", "16:35:*"),
        ("16:35:00.*", "16:35:00.*"),
    ] {
        assert_eq!(parse_time(rdf), parse_time(existing), "{rdf}");
        assert!(parse_time(rdf).is_ok(), "{rdf}");
        assert_eq!(parse_value(rdf), parse_value(existing), "{rdf}");
    }
}

#[test]
fn a_week_n_day_takes_month_week_day() {
    for (text, expected) in [
        ("11,4,4", (11, 4, 4)),
        ("*,*,1", (ANY, ANY, 1)),
        ("*,*,*", (ANY, ANY, ANY)),
        ("13,6,7", (13, 6, 7)),
        ("14,9,*", (14, 9, ANY)),
        (" 11, 4, 4 ", (11, 4, 4)),
    ] {
        assert_eq!(
            parse_week_n_day(text),
            Ok(BACnetWeekNDay {
                month: expected.0,
                week_of_month: expected.1,
                day_of_week: expected.2,
            }),
            "{text}"
        );
    }
    for (text, error) in [
        ("11,4", "expected month,week,day"),
        ("11,4,4,4", "expected month,week,day"),
        ("", "expected month,week,day"),
        ("0,1,1", "invalid month '0'"),
        ("15,1,1", "invalid month '15'"),
        ("1,0,1", "invalid week of the month '0'"),
        ("1,10,1", "invalid week of the month '10'"),
        ("1,1,0", "invalid day of the week '0'"),
        ("1,1,8", "invalid day of the week '8'"),
        ("1,1,mon", "invalid day of the week 'mon'"),
        ("255,1,1", "invalid month '255'"),
    ] {
        let refused = parse_week_n_day(text).unwrap_err();
        assert!(refused.contains(error), "{text}: {refused}");
    }
}

#[test]
fn a_calendar_entry_takes_the_week_n_day_as_one_text() {
    let typed = |fields: &str| {
        parse_value_for(
            O::CALENDAR,
            P::DATE_LIST,
            None,
            &format!(r#"[{{"kind": "week_n_day"{fields}}}]"#),
        )
    };
    for (rdf, numbers) in [
        (
            r#", "week_n_day": "11,4,4""#,
            r#", "month": 11, "week_of_month": 4, "day_of_week": 4"#,
        ),
        (
            r#", "week_n_day": "*,*,1""#,
            r#", "month": null, "week_of_month": 255, "day_of_week": 1"#,
        ),
        // Every field left out is any.
        (r#", "week_n_day": "*,*,*""#, ""),
    ] {
        assert!(typed(numbers).is_ok(), "{numbers}");
        assert_eq!(typed(rdf), typed(numbers), "{rdf}");
    }
    // A special event's period takes the same entry.
    let event = |period: &str| {
        format!(r#"{{"period": {period}, "time_values": [["08:00", 65.0]], "priority": 10}}"#)
    };
    assert_eq!(
        parse_value_for(
            O::SCHEDULE,
            P::EXCEPTION_SCHEDULE,
            Some(1),
            &event(r#"{"kind": "week_n_day", "week_n_day": "11,4,4"}"#)
        ),
        parse_value_for(
            O::SCHEDULE,
            P::EXCEPTION_SCHEDULE,
            Some(1),
            &event(r#"{"kind": "week_n_day", "month": 11, "week_of_month": 4, "day_of_week": 4}"#)
        )
    );
    for (fields, error) in [
        (
            r#", "week_n_day": "11,4,4", "month": 11"#,
            "calendar-entry[0]: week_n_day stands for month, week_of_month and day_of_week; \
             drop month",
        ),
        (
            r#", "week_n_day": [11, 4, 4]"#,
            "calendar-entry[0].week_n_day: expected a string",
        ),
        (
            r#", "week_n_day": "11,4""#,
            "calendar-entry[0].week_n_day: invalid week-n-day '11,4'",
        ),
    ] {
        let refused = typed(fields).unwrap_err();
        assert!(refused.contains(error), "{fields}: {refused}");
    }
    // A date entry's date takes the weekday digit.
    assert_eq!(
        parse_value_for(
            O::CALENDAR,
            P::DATE_LIST,
            None,
            r#"[{"kind": "date", "date": "2026-12-25 5"}]"#
        ),
        parse_value_for(
            O::CALENDAR,
            P::DATE_LIST,
            None,
            r#"[{"kind": "date", "date": "2026-12-25 Fri"}]"#
        )
    );
}

#[test]
fn the_weekday_digit_is_one_to_seven_or_star() {
    for (word, day) in [("1", Some(1)), ("7", Some(7)), ("*", Some(ANY))] {
        assert_eq!(parse_weekday_digit(word), day, "{word}");
    }
    for word in ["0", "8", "Mon", "", "**", "12", " 1"] {
        assert_eq!(parse_weekday_digit(word), None, "{word}");
    }
}

/// A month, day or weekday field: a value in `range`, or unspecified.
fn field(range: std::ops::RangeInclusive<u8>) -> impl Strategy<Value = u8> {
    prop_oneof![9 => range, 1 => Just(ANY)]
}

fn any_date() -> impl Strategy<Value = Date> {
    (
        prop_oneof![9 => 0u8..=254, 1 => Just(ANY)],
        field(1..=14),
        field(1..=34),
        field(1..=7),
    )
        .prop_map(|(year, month, day, day_of_week)| date(year, month, day, day_of_week))
}

fn any_time() -> impl Strategy<Value = Time> {
    (field(0..=23), field(0..=59), field(0..=59), field(0..=99))
        .prop_map(|(hour, minute, second, hundredths)| time(hour, minute, second, hundredths))
}

/// A date as the wire view prints it, `YYYY-MM-DD` with `*` for any
/// field and no weekday.
fn ymd(date: &Date) -> String {
    format_property_value(&PropertyValue::Date(Date {
        day_of_week: ANY,
        ..*date
    }))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Every spelling we print an object in parses back, and the coming
    /// BACnet RDF encoding's `type,instance` (by name or by number) gives
    /// the same object.
    #[test]
    fn printed_object_identifiers_parse_back(
        raw in 0u32..=1023,
        instance in prop_oneof![0u32..=ObjectIdentifier::MAX_INSTANCE, Just(0), Just(ObjectIdentifier::MAX_INSTANCE)],
    ) {
        let object_type = ObjectType::from_raw(raw);
        let expected = Ok((object_type, instance));
        for text in [
            format!("{}:{instance}", kebab(object_type)),
            format!("{object_type}:{instance}"),
            format!("{},{instance}", kebab(object_type)),
            format!("{object_type},{instance}"),
            format!("{raw},{instance}"),
            format!("{raw}:{instance}"),
        ] {
            prop_assert_eq!(&parse_object_specifier(&text), &expected, "{}", text);
        }
    }

    /// A date the formatter prints parses back, and so does the wire
    /// view's form of it.
    #[test]
    fn printed_dates_parse_back(date in any_date()) {
        let text = printed(O::DATE_VALUE, P::PRESENT_VALUE, &PropertyValue::Date(date));
        prop_assert_eq!(parse_date(&text), Ok(date), "{}", text);
        let wire = ymd(&date);
        prop_assert_eq!(parse_date(&wire), Ok(Date { day_of_week: ANY, ..date }), "{}", wire);
    }

    /// A weekday digit gives what the weekday's name gives, `*` what no
    /// weekday gives, in a date and in a date-time.
    #[test]
    fn weekday_digits_read_as_their_names(date in any_date(), time in any_time()) {
        let ymd = ymd(&date);
        let at = format_property_value(&PropertyValue::Time(time));
        let (rdf, named) = match date.day_of_week {
            ANY => (format!("{ymd} *"), ymd.clone()),
            day => (
                format!("{ymd} {day}"),
                format!("{ymd} {}", NAMES[usize::from(day - 1)]),
            ),
        };
        prop_assert_eq!(parse_date(&rdf), Ok(date), "{}", rdf);
        prop_assert_eq!(parse_date(&rdf), parse_date(&named), "{}", rdf);
        prop_assert_eq!(
            parse_date_time(&format!("{rdf} {at}")),
            Ok((date, Some(time))),
            "{} {}", rdf, at
        );
        prop_assert_eq!(
            parse_date_time(&format!("{rdf} {at}")),
            parse_date_time(&format!("{named} {at}"))
        );
    }

    /// A time the formatter prints parses back, and so does the wire
    /// view's `HH:MM:SS.hh`, which is the RDF time form.
    #[test]
    fn printed_times_parse_back(time in any_time()) {
        let text = printed(O::TIME_VALUE, P::PRESENT_VALUE, &PropertyValue::Time(time));
        prop_assert_eq!(parse_time(&text), Ok(time), "{}", text);
        let wire = format_property_value(&PropertyValue::Time(time));
        prop_assert_eq!(parse_time(&wire), Ok(time), "{}", wire);
    }

    /// A date-time the formatter prints parses back for a property that
    /// holds one.
    #[test]
    fn printed_date_times_parse_back(date in any_date(), time in any_time()) {
        let value = PropertyValue::List(vec![PropertyValue::Date(date), PropertyValue::Time(time)]);
        let text = printed(O::TREND_LOG, P::START_TIME, &value);
        prop_assert_eq!(
            parse_value_for(O::TREND_LOG, P::START_TIME, None, &text),
            Ok(value),
            "{}", text
        );
    }

    /// `month,week,day` gives what the three numbers give.
    #[test]
    fn week_n_day_text_reads_as_its_numbers(
        month in field(1..=14),
        week in field(1..=9),
        day in field(1..=7),
    ) {
        let text = |value: u8| match value {
            ANY => "*".to_string(),
            value => value.to_string(),
        };
        let rdf = format!("{},{},{}", text(month), text(week), text(day));
        prop_assert_eq!(
            parse_week_n_day(&rdf),
            Ok(BACnetWeekNDay { month, week_of_month: week, day_of_week: day })
        );
        let numbers = format!(
            r#"[{{"kind": "week_n_day", "month": {month}, "week_of_month": {week}, "day_of_week": {day}}}]"#
        );
        let one_text = format!(r#"[{{"kind": "week_n_day", "week_n_day": "{rdf}"}}]"#);
        prop_assert_eq!(
            parse_value_for(O::CALENDAR, P::DATE_LIST, None, &one_text),
            parse_value_for(O::CALENDAR, P::DATE_LIST, None, &numbers)
        );
    }
}
