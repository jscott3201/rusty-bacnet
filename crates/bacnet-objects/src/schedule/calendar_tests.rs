//! Calendar object tests: the Date_List wire form (#996), its value checks
//! and Present_Value evaluated from the clock's local date (#1029).

use super::*;
use crate::clock::{ClockFrame, ClockReader};
use bacnet_types::calendar::SpecificDate;
use bacnet_types::constructed::{BACnetCalendarEntry, BACnetDateRange, BACnetWeekNDay};
use bacnet_types::error::RemoteError;
use std::sync::{Arc, Mutex};

const DATE_LIST: PropertyIdentifier = PropertyIdentifier::DATE_LIST;

fn app(bytes: &[u8]) -> PropertyValue {
    PropertyValue::ApplicationData(bytes.to_vec())
}

fn monday_14_sep_2026() -> Date {
    Date {
        year: 126,
        month: 9,
        day: 14,
        day_of_week: 1,
    }
}

/// One entry per CHOICE alternative and its golden encoding, worked out from
/// the Clause 20.2.1 tag rules: date `[0]` holding four octets is 0x0C;
/// date-range `[1]` is opening tag 0x1E, two application Dates (0xA4 each)
/// and closing tag 0x1F; weekNDay `[2]` holding three octets is 0x2B.
fn entries_and_wire() -> Vec<(BACnetCalendarEntry, &'static [u8])> {
    let date = monday_14_sep_2026();
    vec![
        (BACnetCalendarEntry::Date(date), &[0x0C, 126, 9, 14, 1]),
        (
            BACnetCalendarEntry::DateRange(BACnetDateRange {
                start_date: date,
                end_date: date,
            }),
            &[0x1E, 0xA4, 126, 9, 14, 1, 0xA4, 126, 9, 14, 1, 0x1F],
        ),
        // Every Monday: month and week-of-month unspecified.
        (
            BACnetCalendarEntry::WeekNDay(BACnetWeekNDay {
                month: BACnetWeekNDay::ANY,
                week_of_month: BACnetWeekNDay::ANY,
                day_of_week: 1,
            }),
            &[0x2B, 0xFF, 0xFF, 1],
        ),
    ]
}

fn wire_list() -> PropertyValue {
    PropertyValue::List(entries_and_wire().iter().map(|(_, w)| app(w)).collect())
}

fn configured() -> CalendarObject {
    let mut cal = CalendarObject::new(1, "CAL-1").unwrap();
    for (entry, _) in entries_and_wire() {
        cal.add_date_entry(entry).unwrap();
    }
    cal
}

/// A clock whose local date a test sets; `None` is a clock with no time.
struct DateClock(Mutex<Option<Date>>);

impl DateClock {
    fn new(date: Option<Date>) -> Arc<Self> {
        Arc::new(Self(Mutex::new(date)))
    }

    fn set(&self, date: Option<Date>) {
        *self.0.lock().unwrap() = date;
    }
}

impl ClockReader for DateClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        let local_date = (*self.0.lock().unwrap())?;
        Some(ClockFrame {
            local_date,
            local_time: Time {
                hour: 12,
                minute: 0,
                second: 0,
                hundredths: 0,
            },
            utc_offset: 0,
            daylight_savings_status: false,
        })
    }
}

fn ymd(year: u16, month: u8, day: u8) -> Date {
    SpecificDate::new(year, month, day).unwrap().to_date()
}

fn present_value(cal: &CalendarObject) -> PropertyValue {
    cal.read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap()
}

fn assert_error(result: Result<(), Error>, class: ErrorClass, code: ErrorCode, what: &str) {
    match result {
        Err(Error::Remote(RemoteError::Error { class: c, code: e })) => {
            assert_eq!(c, class, "{what}");
            assert_eq!(e, code, "{what}: expected {code:?}");
        }
        other => panic!("{what}: expected {class:?}/{code:?}, got {other:?}"),
    }
}

/// A Date_List refusal that names the entry at `position` (from 1) among the
/// entries the write carried (#1048).
fn assert_entry_error(
    result: Result<(), Error>,
    class: ErrorClass,
    code: ErrorCode,
    position: u32,
    what: &str,
) {
    crate::common::assert_list_element_refused(result, class, code, position, what);
}

#[test]
fn calendar_read_present_value_default() {
    let cal = CalendarObject::new(1, "CAL-1").unwrap();
    let val = cal
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Boolean(false));
}

#[test]
fn calendar_present_value_follows_the_clock_date() {
    // #1029: Present_Value used to stay whatever the application last set.
    let mut cal = CalendarObject::new(1, "CAL-1").unwrap();
    cal.add_date_entry(BACnetCalendarEntry::Date(ymd(2026, 12, 25)))
        .unwrap();
    // No clock: no local date, so no entry can match.
    assert_eq!(present_value(&cal), PropertyValue::Boolean(false));
    let clock = DateClock::new(Some(ymd(2026, 12, 24)));
    cal.bind_clock_internal(Some(clock.clone()));
    assert_eq!(present_value(&cal), PropertyValue::Boolean(false));
    // The date changes: the next read sees it, with no tick in between.
    clock.set(Some(ymd(2026, 12, 25)));
    assert_eq!(present_value(&cal), PropertyValue::Boolean(true));
    clock.set(Some(ymd(2026, 12, 26)));
    assert_eq!(present_value(&cal), PropertyValue::Boolean(false));
    // A clock without a time, or with a date that names no day, is FALSE.
    for date in [
        None,
        Some(Date {
            year: Date::UNSPECIFIED,
            ..ymd(2026, 12, 25)
        }),
        Some(Date {
            day: 32,
            ..ymd(2026, 12, 25)
        }),
    ] {
        clock.set(date);
        assert_eq!(
            present_value(&cal),
            PropertyValue::Boolean(false),
            "{date:?}"
        );
    }
    // Unbinding the clock leaves the calendar without a date.
    clock.set(Some(ymd(2026, 12, 25)));
    cal.bind_clock_internal(None);
    assert_eq!(present_value(&cal), PropertyValue::Boolean(false));
}

#[test]
fn calendar_present_value_follows_date_list_writes() {
    let clock = DateClock::new(Some(ymd(2026, 9, 14)));
    let mut cal = CalendarObject::new(1, "CAL-1").unwrap();
    cal.bind_clock_internal(Some(clock));
    assert_eq!(present_value(&cal), PropertyValue::Boolean(false));
    // Each choice in turn makes Monday 14 September 2026 a calendar day.
    for (entry, wire) in entries_and_wire() {
        cal.write_property(DATE_LIST, None, app(wire), None)
            .unwrap();
        assert_eq!(
            present_value(&cal),
            PropertyValue::Boolean(true),
            "{entry:?}"
        );
        cal.write_property(DATE_LIST, None, PropertyValue::List(vec![]), None)
            .unwrap();
        assert_eq!(present_value(&cal), PropertyValue::Boolean(false));
    }
    // Any matching entry is enough: a non-matching one beside it changes
    // nothing.
    cal.write_property(
        DATE_LIST,
        None,
        PropertyValue::List(vec![
            app(&[0x2B, 0xFF, 0xFF, 2]),
            app(&[0x2B, 0xFF, 0xFF, 1]),
        ]),
        None,
    )
    .unwrap();
    assert_eq!(present_value(&cal), PropertyValue::Boolean(true));
}

#[test]
fn calendar_state_hook_answers_for_any_day() {
    let cal = configured();
    // The configured entries: 14 September 2026, the same day as a range,
    // and every Monday.
    for (day, expected) in [
        (SpecificDate::new(2026, 9, 14).unwrap(), true),
        (SpecificDate::new(2026, 9, 21).unwrap(), true),
        (SpecificDate::new(2026, 9, 15).unwrap(), false),
    ] {
        assert_eq!(cal.is_active_on(day), expected, "{day:?}");
        assert_eq!(cal.calendar_state_internal(day), Some(expected), "{day:?}");
    }
}

#[test]
fn calendar_write_present_value_denied() {
    let mut cal = CalendarObject::new(1, "CAL-1").unwrap();
    let result = cal.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Boolean(true),
        None,
    );
    assert!(result.is_err());
}

#[test]
fn calendar_property_list_contains_date_list() {
    let cal = CalendarObject::new(1, "CAL-1").unwrap();
    assert!(cal.property_list().contains(&DATE_LIST));
    assert!(!cal.is_array_property(DATE_LIST));
}

#[test]
fn calendar_date_list_empty_by_default() {
    let cal = CalendarObject::new(1, "CAL-1").unwrap();
    assert_eq!(
        cal.read_property(DATE_LIST, None).unwrap(),
        PropertyValue::List(vec![])
    );
    assert!(cal.date_list().is_empty());
}

#[test]
fn calendar_date_list_reads_each_entry_under_its_choice_tag() {
    // #996: entries used to read as an application Date or Octet String.
    let mut cal = configured();
    // Direct reads ignore the index; the service handlers reject list indexing.
    for index in [None, Some(0), Some(1), Some(u32::MAX)] {
        assert_eq!(cal.read_property(DATE_LIST, index).unwrap(), wire_list());
    }
    let entries: Vec<_> = entries_and_wire().into_iter().map(|(e, _)| e).collect();
    assert_eq!(cal.date_list(), entries.as_slice());
    cal.clear_date_list();
    assert_eq!(
        cal.read_property(DATE_LIST, None).unwrap(),
        PropertyValue::List(vec![])
    );
}

#[test]
fn calendar_date_list_write_accepts_every_choice_in_every_value_shape() {
    let all: Vec<u8> = entries_and_wire()
        .iter()
        .flat_map(|(_, w)| w.iter().copied())
        .collect();
    let entries: Vec<_> = entries_and_wire().into_iter().map(|(e, _)| e).collect();
    for (what, value, expected) in [
        // The service decoders hand over one element per entry ...
        ("one element per entry", wire_list(), entries.clone()),
        // ... or a single element when the payload holds one entry.
        (
            "one entry",
            app(&[0x2B, 0xFF, 0xFF, 1]),
            vec![entries[2].clone()],
        ),
        ("a pre-encoded list", app(&all), entries.clone()),
        ("an empty list", PropertyValue::List(vec![]), vec![]),
    ] {
        let mut cal = configured();
        cal.write_property(DATE_LIST, None, value, None)
            .unwrap_or_else(|e| panic!("{what}: {e:?}"));
        assert_eq!(cal.date_list(), expected.as_slice(), "{what}");
    }
    // A read value writes back unchanged.
    let mut cal = configured();
    let before = cal.read_property(DATE_LIST, None).unwrap();
    cal.write_property(DATE_LIST, None, before.clone(), None)
        .unwrap();
    assert_eq!(cal.read_property(DATE_LIST, None).unwrap(), before);
}

#[test]
fn calendar_date_list_write_refuses_other_datatypes() {
    let date = monday_14_sep_2026();
    // A value that holds no entries names none; a foreign element or entry
    // is named by its position among the entries (#1048).
    for (what, value, entry) in [
        // The projection Date_List used to read as (#996).
        ("Date", PropertyValue::Date(date), None),
        (
            "Octet String",
            PropertyValue::OctetString(vec![255, 255, 1]),
            None,
        ),
        ("Null", PropertyValue::Null, None),
        ("Unsigned", PropertyValue::Unsigned(1), None),
        (
            "list holding a Date",
            PropertyValue::List(vec![app(&[0x0C, 126, 9, 14, 1]), PropertyValue::Date(date)]),
            Some(2),
        ),
        // Raw bytes whose leading tag is no calendar-entry choice.
        (
            "application-tagged Date bytes",
            app(&[0xA4, 126, 9, 14, 1]),
            Some(1),
        ),
        (
            "unknown alternative [3]",
            app(&[0x3B, 0xFF, 0xFF, 1]),
            Some(1),
        ),
    ] {
        let mut cal = configured();
        let result = cal.write_property(DATE_LIST, None, value, None);
        let (class, code) = (ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE);
        match entry {
            Some(entry) => assert_entry_error(result, class, code, entry, what),
            None => assert_error(result, class, code, what),
        }
        assert_eq!(cal.read_property(DATE_LIST, None).unwrap(), wire_list());
    }
}

#[test]
fn calendar_date_list_write_refuses_malformed_entries() {
    for (what, bytes) in [
        ("date [0] with three octets", &[0x0B, 126, 9, 14][..]),
        ("truncated date [0]", &[0x0C, 126, 9]),
        ("weekNDay [2] with four octets", &[0x2C, 0xFF, 0xFF, 1, 0]),
        (
            "date-range [1] never closed",
            &[0x1E, 0xA4, 126, 9, 14, 1, 0xA4, 126, 9, 14, 1],
        ),
        (
            "date-range [1] holding context-tagged dates",
            &[0x1E, 0x0C, 126, 9, 14, 1, 0x1C, 126, 9, 14, 1, 0x1F],
        ),
        (
            "date-range [1] as a primitive",
            &[0x1D, 8, 126, 9, 14, 1, 126, 9, 14, 1],
        ),
        (
            "a good entry, then a truncated one",
            &[0x2B, 0xFF, 0xFF, 1, 0x2B, 0xFF],
        ),
    ] {
        let mut cal = configured();
        // The good entry decodes; the one after it is entry 2.
        let entry = if what.starts_with("a good entry") {
            2
        } else {
            1
        };
        assert_entry_error(
            cal.write_property(DATE_LIST, None, app(bytes), None),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_ENCODING,
            entry,
            what,
        );
        assert_eq!(cal.read_property(DATE_LIST, None).unwrap(), wire_list());
    }
}

#[test]
fn calendar_date_list_write_caps_the_entry_count() {
    let entry = app(&[0x2B, 0xFF, 0xFF, 1]);
    let mut cal = configured();
    cal.write_property(
        DATE_LIST,
        None,
        PropertyValue::List(vec![entry.clone(); date_list::MAX_DATE_LIST_ENTRIES]),
        None,
    )
    .unwrap();
    assert_eq!(cal.date_list().len(), date_list::MAX_DATE_LIST_ENTRIES);

    let mut cal = configured();
    assert_entry_error(
        cal.write_property(
            DATE_LIST,
            None,
            PropertyValue::List(vec![entry; date_list::MAX_DATE_LIST_ENTRIES + 1]),
            None,
        ),
        ErrorClass::RESOURCES,
        ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
        date_list::MAX_DATE_LIST_ENTRIES as u32 + 1,
        "one entry over the cap",
    );
    assert_eq!(cal.read_property(DATE_LIST, None).unwrap(), wire_list());
}

#[test]
fn calendar_date_list_write_refuses_an_array_index() {
    let mut cal = configured();
    for index in [0, 1, u32::MAX] {
        assert_error(
            cal.write_property(DATE_LIST, Some(index), wire_list(), None),
            ErrorClass::PROPERTY,
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
            "indexed write",
        );
    }
    assert_eq!(cal.read_property(DATE_LIST, None).unwrap(), wire_list());
}

/// Entries whose encoding is well formed but whose octets are outside their
/// Clause 21 ranges, each with what it breaks.
fn out_of_range_entries() -> Vec<(&'static str, Vec<u8>)> {
    let range =
        |start: [u8; 4], end: [u8; 4]| [&[0x1E, 0xA4][..], &start, &[0xA4], &end, &[0x1F]].concat();
    let good = [126, 7, 1, 3];
    let open = [0xFF; 4];
    vec![
        ("date month 0", vec![0x0C, 126, 0, 1, 0xFF]),
        ("date month 15", vec![0x0C, 126, 15, 1, 0xFF]),
        ("date day 0", vec![0x0C, 126, 7, 0, 0xFF]),
        ("date day 35", vec![0x0C, 126, 7, 35, 0xFF]),
        ("date weekday 0", vec![0x0C, 0xFF, 0xFF, 0xFF, 0]),
        ("date weekday 8", vec![0x0C, 0xFF, 0xFF, 0xFF, 8]),
        (
            "range start with year unspecified",
            range([0xFF, 7, 1, 3], good),
        ),
        (
            "range end with day unspecified",
            range(good, [126, 8, 0xFF, 1]),
        ),
        (
            "range end on the last-day value",
            range(good, [126, 8, 32, 1]),
        ),
        ("range start in odd months", range([126, 13, 1, 3], open)),
        ("range end on 30 February", range(open, [126, 2, 30, 1])),
        ("range start weekday 9", range([126, 7, 1, 9], good)),
        ("weekNDay month 0", vec![0x2B, 0, 0xFF, 0xFF]),
        ("weekNDay month 15", vec![0x2B, 15, 0xFF, 0xFF]),
        ("weekNDay week 0", vec![0x2B, 0xFF, 0, 0xFF]),
        ("weekNDay week 10", vec![0x2B, 0xFF, 10, 0xFF]),
        ("weekNDay weekday 0", vec![0x2B, 0xFF, 0xFF, 0]),
        ("weekNDay weekday 8", vec![0x2B, 0xFF, 0xFF, 8]),
    ]
}

#[test]
fn calendar_date_list_write_refuses_out_of_range_entries() {
    // #1029: these decoded and were stored as written.
    for (what, bytes) in out_of_range_entries() {
        for (shape, value, entry) in [
            ("alone", app(&bytes), 1),
            // A good entry first does not save the list.
            (
                "after a good entry",
                PropertyValue::List(vec![app(&[0x2B, 0xFF, 0xFF, 1]), app(&bytes)]),
                2,
            ),
        ] {
            let mut cal = configured();
            assert_entry_error(
                cal.write_property(DATE_LIST, None, value, None),
                ErrorClass::PROPERTY,
                ErrorCode::VALUE_OUT_OF_RANGE,
                entry,
                &format!("{what}, {shape}"),
            );
            assert_eq!(cal.read_property(DATE_LIST, None).unwrap(), wire_list());
        }
    }
    // The boundary values themselves are accepted.
    let mut cal = configured();
    cal.write_property(
        DATE_LIST,
        None,
        PropertyValue::List(vec![
            app(&[0x0C, 0, 14, 34, 7]),
            app(&[0x0C, 254, 1, 1, 1]),
            app(&[0x2B, 14, 9, 7]),
            app(&[0x2B, 1, 1, 1]),
            app(&[
                0x1E, 0xA4, 0xFF, 0xFF, 0xFF, 0xFF, 0xA4, 126, 2, 28, 0xFF, 0x1F,
            ]),
        ]),
        None,
    )
    .unwrap();
    assert_eq!(cal.date_list().len(), 5);
}

#[test]
fn calendar_add_date_entry_checks_values_and_the_cap() {
    let mut cal = CalendarObject::new(1, "CAL-1").unwrap();
    for (what, bytes) in out_of_range_entries() {
        let (entry, _) = bacnet_encoding::constructed::decode_calendar_entry(&bytes, 0).unwrap();
        assert_error(
            cal.add_date_entry(entry),
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
            what,
        );
    }
    assert!(cal.date_list().is_empty());
    let monday = BACnetCalendarEntry::WeekNDay(BACnetWeekNDay {
        month: BACnetWeekNDay::ANY,
        week_of_month: BACnetWeekNDay::ANY,
        day_of_week: 1,
    });
    for _ in 0..date_list::MAX_DATE_LIST_ENTRIES {
        cal.add_date_entry(monday.clone()).unwrap();
    }
    assert_error(
        cal.add_date_entry(monday),
        ErrorClass::RESOURCES,
        ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
        "one entry over the cap",
    );
    assert_eq!(cal.date_list().len(), date_list::MAX_DATE_LIST_ENTRIES);
}
