//! Constructed values typed as JSON: each shape encodes as the element
//! the codecs write, reads back through the formatter, and refuses a
//! field it doesn't know.

use bacnet_encoding::constructed::{
    encode_calendar_entry_list, encode_daily_schedule, encode_destination_list,
    encode_special_event, encode_weekly_schedule,
};
use bacnet_types::MacAddr;
use bacnet_types::bitstring::{DaysOfWeek, EventTransitionBits};
use bacnet_types::constructed::{
    BACnetAddress, BACnetCalendarEntry, BACnetDateRange, BACnetDestination, BACnetRecipient,
    BACnetSpecialEvent, BACnetTimeValue, BACnetWeekNDay, SpecialEventPeriod,
};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, Time};
use bytes::BytesMut;

use super::parse_value_for;
use crate::format::{Floats, StateTexts, ValueFormat};

type O = ObjectType;
type P = PropertyIdentifier;

const ANY: u8 = 0xFF;

fn hm(hour: u8, minute: u8) -> Time {
    Time {
        hour,
        minute,
        second: 0,
        hundredths: 0,
    }
}

fn date(year: u8, month: u8, day: u8) -> Date {
    Date {
        year,
        month,
        day,
        day_of_week: ANY,
    }
}

fn entry(time: Time, value: PropertyValue) -> BACnetTimeValue {
    BACnetTimeValue { time, value }
}

fn octets(value: PropertyValue) -> Vec<u8> {
    match value {
        PropertyValue::ApplicationData(bytes) => bytes,
        other => panic!("expected encoded octets, got {other:?}"),
    }
}

fn typed(
    object_type: ObjectType,
    property: PropertyIdentifier,
    index: Option<u32>,
    json: &str,
) -> Vec<u8> {
    octets(parse_value_for(object_type, property, index, json).unwrap())
}

fn refused(
    object_type: ObjectType,
    property: PropertyIdentifier,
    index: Option<u32>,
    json: &str,
) -> String {
    parse_value_for(object_type, property, index, json).unwrap_err()
}

fn shown(
    object_type: ObjectType,
    property: PropertyIdentifier,
    index: Option<u32>,
    data: &[u8],
) -> String {
    let texts = StateTexts::default();
    ValueFormat::new(object_type, &texts, Floats::Exact)
        .whole_lists()
        .value_at(property, index, data)
}

#[test]
fn a_day_takes_pairs_or_objects_and_a_week_is_seven_days() {
    let day = vec![
        entry(hm(7, 0), PropertyValue::Real(72.5)),
        entry(hm(18, 0), PropertyValue::Null),
    ];
    let mut expected = BytesMut::new();
    encode_daily_schedule(&mut expected, &day).unwrap();
    for json in [
        r#"[["07:00", 72.5], ["18:00", null]]"#,
        r#"[{"time": "07:00", "value": 72.5}, {"value": null, "time": "18:00"}]"#,
    ] {
        let data = typed(O::SCHEDULE, P::WEEKLY_SCHEDULE, Some(1), json);
        assert_eq!(data, expected.to_vec(), "{json}");
        assert_eq!(
            shown(O::SCHEDULE, P::WEEKLY_SCHEDULE, Some(1), &data),
            "Mon 07:00 = 72.5, 18:00 = null"
        );
    }
    let week: [Vec<BACnetTimeValue>; 7] = std::array::from_fn(|day| match day {
        0 => vec![entry(hm(7, 0), PropertyValue::Unsigned(1))],
        6 => vec![entry(hm(9, 30), PropertyValue::Enumerated(1))],
        _ => Vec::new(),
    });
    let mut expected = BytesMut::new();
    encode_weekly_schedule(&mut expected, &week).unwrap();
    let data = typed(
        O::SCHEDULE,
        P::WEEKLY_SCHEDULE,
        None,
        r#"[[["07:00", 1]], [], [], [], [], [], [["09:30", "active"]]]"#,
    );
    assert_eq!(data, expected.to_vec());
    let short = refused(O::SCHEDULE, P::WEEKLY_SCHEDULE, None, "[[], []]");
    assert!(short.contains("7 days"), "{short}");
}

#[test]
fn calendar_entries_take_their_three_kinds() {
    let entries = vec![
        BACnetCalendarEntry::Date(date(126, 12, 25)),
        BACnetCalendarEntry::DateRange(BACnetDateRange {
            start_date: date(126, 7, 1),
            end_date: date(126, 7, 4),
        }),
        BACnetCalendarEntry::WeekNDay(BACnetWeekNDay {
            month: ANY,
            week_of_month: 1,
            day_of_week: 1,
        }),
    ];
    let mut expected = BytesMut::new();
    encode_calendar_entry_list(&mut expected, &entries);
    let data = typed(
        O::CALENDAR,
        P::DATE_LIST,
        None,
        r#"[
            {"kind": "date", "date": "2026-12-25"},
            {"kind": "date_range", "start_date": "2026-07-01", "end_date": "2026-07-04"},
            {"kind": "week_n_day", "month": null, "week_of_month": 1, "day_of_week": 1}
        ]"#,
    );
    assert_eq!(data, expected.to_vec());
    assert!(
        shown(O::CALENDAR, P::DATE_LIST, None, &data).starts_with("[2026-12-25, "),
        "{}",
        shown(O::CALENDAR, P::DATE_LIST, None, &data)
    );
}

#[test]
fn special_events_take_an_entry_or_a_calendar() {
    let calendar = ObjectIdentifier::new(O::CALENDAR, 1).unwrap();
    for (period_json, period) in [
        (
            r#"{"kind": "date", "date": "2026-12-25"}"#,
            SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::Date(date(126, 12, 25))),
        ),
        (
            r#""calendar:1""#,
            SpecialEventPeriod::CalendarReference(calendar),
        ),
    ] {
        let event = BACnetSpecialEvent {
            period,
            list_of_time_values: vec![entry(hm(8, 0), PropertyValue::Real(65.0))],
            event_priority: 10,
        };
        let mut expected = BytesMut::new();
        encode_special_event(&mut expected, &event).unwrap();
        let json = format!(
            r#"{{"period": {period_json}, "time_values": [["08:00", 65.0]], "priority": 10}}"#
        );
        assert_eq!(
            typed(O::SCHEDULE, P::EXCEPTION_SCHEDULE, Some(1), &json),
            expected.to_vec()
        );
        // The whole list is the elements one after another.
        assert_eq!(
            typed(
                O::SCHEDULE,
                P::EXCEPTION_SCHEDULE,
                None,
                &format!("[{json}, {json}]")
            ),
            [expected.to_vec(), expected.to_vec()].concat()
        );
    }
    let not_a_calendar = refused(
        O::SCHEDULE,
        P::EXCEPTION_SCHEDULE,
        Some(1),
        r#"{"period": "av:1", "time_values": [], "priority": 10}"#,
    );
    assert!(not_a_calendar.contains("Calendar"), "{not_a_calendar}");
}

#[test]
fn destinations_default_like_python_and_take_names() {
    let device = ObjectIdentifier::new(O::DEVICE, 99).unwrap();
    let full_day = BACnetDestination {
        valid_days: DaysOfWeek::all(),
        from_time: hm(0, 0),
        to_time: Time {
            hour: 23,
            minute: 59,
            second: 59,
            hundredths: 99,
        },
        recipient: BACnetRecipient::Device(device),
        process_identifier: 7,
        issue_confirmed_notifications: false,
        transitions: EventTransitionBits::all(),
    };
    let weekdays = BACnetDestination {
        valid_days: DaysOfWeek::from_bits_truncate(0b001_1111),
        from_time: hm(8, 0),
        to_time: hm(17, 0),
        recipient: BACnetRecipient::Address(BACnetAddress {
            network_number: 5,
            mac_address: MacAddr::from_slice(&[10, 0, 0, 100, 0xBA, 0xC0]),
        }),
        process_identifier: 8,
        issue_confirmed_notifications: true,
        transitions: EventTransitionBits::TO_OFFNORMAL | EventTransitionBits::TO_NORMAL,
    };
    let mut expected = BytesMut::new();
    encode_destination_list(&mut expected, &[full_day, weekdays]).unwrap();
    let data = typed(
        O::NOTIFICATION_CLASS,
        P::RECIPIENT_LIST,
        None,
        r#"[
            {"recipient": "device:99", "process_identifier": 7},
            {"recipient": {"kind": "address", "network_number": 5, "mac_address": "0a:00:00:64:ba:c0"},
             "process_identifier": 8, "valid_days": ["mon", "tue", "wednesday", "thu", "fri"],
             "from_time": "08:00", "to_time": "17:00", "issue_confirmed_notifications": true,
             "transitions": ["to-offnormal", "TO_NORMAL"]}
        ]"#,
    );
    assert_eq!(data, expected.to_vec());
    // Bits as a number, as Python takes them.
    let numbered = typed(
        O::NOTIFICATION_CLASS,
        P::RECIPIENT_LIST,
        Some(1),
        r#"{"recipient": {"kind": "device", "object_identifier": "device:99"},
            "process_identifier": 7, "valid_days": 127, "transitions": 7}"#,
    );
    assert_eq!(
        numbered,
        typed(
            O::NOTIFICATION_CLASS,
            P::RECIPIENT_LIST,
            Some(1),
            r#"{"recipient": "device:99", "process_identifier": 7}"#
        )
    );
}

#[test]
fn unknown_and_missing_fields_are_refused_with_their_place() {
    for (property, index, json, error) in [
        (
            P::RECIPIENT_LIST,
            None,
            r#"[{"recipient": "device:99", "process_identifier": 7, "colour": "red"}]"#,
            "destination[0]: unknown field 'colour'; the fields are from_time, \
             issue_confirmed_notifications, process_identifier, recipient, to_time, \
             transitions, valid_days",
        ),
        (
            P::RECIPIENT_LIST,
            Some(1),
            r#"{"recipient": "device:99"}"#,
            "destination: missing field 'process_identifier'",
        ),
        (
            P::RECIPIENT_LIST,
            Some(1),
            r#"{"recipient": "device:99", "process_identifier": 7, "valid_days": ["someday"]}"#,
            "destination.valid_days: unknown name 'someday'; expected one of: monday,",
        ),
        (
            P::RECIPIENT_LIST,
            Some(1),
            r#"{"recipient": "device:99", "process_identifier": 7, "from_time": "25:00"}"#,
            "destination.from_time: invalid hour '25'",
        ),
        (P::RECIPIENT_LIST, Some(1), "{not json", "invalid JSON"),
    ] {
        let refused = refused(O::NOTIFICATION_CLASS, property, index, json);
        assert!(refused.contains(error), "{json}: {refused}");
    }
    for (json, error) in [
        (r#"[{"kind": "weekly"}]"#, "date-list"),
        (
            r#"[{"kind": "date", "date": "2026-12-25", "year": 1}]"#,
            "unknown field 'year'",
        ),
        (
            r#"[{"kind": "date", "date": "2026-15-25"}]"#,
            "calendar-entry[0].date: invalid month",
        ),
    ] {
        let refused = refused(O::CALENDAR, P::DATE_LIST, None, json);
        let error = error.replace("date-list", "calendar-entry[0].kind");
        assert!(refused.contains(&error), "{json}: {refused}");
    }
    let refused = refused(
        O::SCHEDULE,
        P::WEEKLY_SCHEDULE,
        Some(1),
        r#"[["07:00", [1]]]"#,
    );
    assert!(refused.contains("day[0].value: expected null"), "{refused}");
    // Text that isn't JSON reads the untyped grammar.
    assert_eq!(
        parse_value_for(O::SCHEDULE, P::WEEKLY_SCHEDULE, Some(0), "7"),
        Ok(PropertyValue::Unsigned(7))
    );
}

#[test]
fn numbers_and_priorities_out_of_range_are_refused() {
    for (value, error) in [
        ("1e300", "out of a REAL's range"),
        ("-3000000000", "out of a Signed's range"),
    ] {
        let json = format!(r#"[["07:00", {value}]]"#);
        let refused = refused(O::SCHEDULE, P::WEEKLY_SCHEDULE, Some(1), &json);
        assert!(refused.contains(error), "{value}: {refused}");
    }
    // An exponent is a REAL, whole or not, and so is a whole number past
    // an Unsigned's or a Signed's range.
    for value in ["1e20", "1.5e3", "18446744073709551616"] {
        let json = format!(r#"[["07:00", {value}]]"#);
        let day = vec![entry(hm(7, 0), PropertyValue::Real(value.parse().unwrap()))];
        let mut expected = BytesMut::new();
        encode_daily_schedule(&mut expected, &day).unwrap();
        assert_eq!(
            typed(O::SCHEDULE, P::WEEKLY_SCHEDULE, Some(1), &json),
            expected.to_vec(),
            "{value}"
        );
    }
    for priority in ["0", "17", "-1", "1.5"] {
        let json =
            format!(r#"{{"period": "calendar:1", "time_values": [], "priority": {priority}}}"#);
        let refused = refused(O::SCHEDULE, P::EXCEPTION_SCHEDULE, Some(1), &json);
        assert!(
            refused.contains("priority from 1 to 16"),
            "{priority}: {refused}"
        );
    }
}
