//! Network writes of Weekly_Schedule, Exception_Schedule and Effective_Period
//! (#1057), given the raw wire bytes the server passes on or the shape a read
//! returns.

use super::*;
use bacnet_types::constructed::SpecialEventPeriod;

type P = PropertyIdentifier;

/// The documented cap on Exception_Schedule events.
const EXCEPTION_CAP: usize = 1024;

/// A refused write: array index, value, then the error class and code it
/// draws, and what the case is.
type Refusal<V> = (Option<u32>, V, ErrorClass, ErrorCode, &'static str);

fn schedule() -> ScheduleObject {
    ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(10.0)).unwrap()
}

fn app(bytes: &[u8]) -> PropertyValue {
    PropertyValue::ApplicationData(bytes.to_vec())
}

fn read(sched: &ScheduleObject, property: PropertyIdentifier) -> PropertyValue {
    sched.read_property(property, None).unwrap()
}

fn write(
    sched: &mut ScheduleObject,
    property: PropertyIdentifier,
    index: Option<u32>,
    value: PropertyValue,
) -> Result<(), Error> {
    sched.write_property(property, index, value, None)
}

fn assert_code(result: Result<(), Error>, class: ErrorClass, code: ErrorCode, what: &str) {
    match result {
        Err(Error::Protocol { class: c, code: e }) => {
            assert_eq!((c, e), (class, code), "{what}: expected {class:?}/{code:?}");
        }
        other => panic!("{what}: expected {code:?}, got {other:?}"),
    }
}

fn assert_property_code(result: Result<(), Error>, code: ErrorCode, what: &str) {
    assert_code(result, ErrorClass::PROPERTY, code, what);
}

/// A daily schedule: the `[0]` frame around `time_values`.
fn day(time_values: &[&[u8]]) -> Vec<u8> {
    [&[0x0E][..], &time_values.concat(), &[0x0F]].concat()
}

/// An application Time hh:mm:00.00 (0xB4) followed by `value`.
fn tv(hour: u8, minute: u8, value: &[u8]) -> Vec<u8> {
    [&[0xB4, hour, minute, 0, 0][..], value].concat()
}

/// Application Real 21.5 and 16.0, Boolean TRUE.
const REAL_21_5: &[u8] = &[0x44, 0x41, 0xAC, 0x00, 0x00];
const REAL_16: &[u8] = &[0x44, 0x41, 0x80, 0x00, 0x00];
const TRUE: &[u8] = &[0x11];
const EMPTY_DAY: &[u8] = &[0x0E, 0x0F];

/// Occupied Monday: 21.5 at 08:00, 16.0 at 17:00.
fn occupied_day() -> Vec<u8> {
    day(&[&tv(8, 0, REAL_21_5), &tv(17, 0, REAL_16)])
}

fn at(hour: u8, minute: u8) -> Time {
    Time {
        hour,
        minute,
        second: 0,
        hundredths: 0,
    }
}

/// Monday 14 September 2026.
fn monday() -> SpecificDate {
    SpecificDate::new(2026, 9, 14).unwrap()
}

// --- Weekly_Schedule ---------------------------------------------------------

#[test]
fn weekly_schedule_whole_write_takes_the_wire_bytes_and_the_read_shape() {
    let mut sched = schedule();
    let mut wire = occupied_day();
    for _ in 0..6 {
        wire.extend_from_slice(EMPTY_DAY);
    }
    write(&mut sched, P::WEEKLY_SCHEDULE, None, app(&wire)).unwrap();
    let mut expected = vec![app(&occupied_day())];
    expected.extend(std::iter::repeat_n(app(EMPTY_DAY), 6));
    assert_eq!(
        read(&sched, P::WEEKLY_SCHEDULE),
        PropertyValue::List(expected)
    );
    // The written days drive the calculation.
    let no_calendars = |_| false;
    assert_eq!(
        sched.evaluate(monday(), at(9, 0), &no_calendars),
        Some(PropertyValue::Real(21.5))
    );

    // What a read returns writes back unchanged; so does a list of chunks
    // that each hold several days.
    let mut other = schedule();
    let as_read = read(&sched, P::WEEKLY_SCHEDULE);
    write(&mut other, P::WEEKLY_SCHEDULE, None, as_read.clone()).unwrap();
    assert_eq!(read(&other, P::WEEKLY_SCHEDULE), as_read);
    let mut split = schedule();
    let (head, tail) = wire.split_at(occupied_day().len() + EMPTY_DAY.len());
    write(
        &mut split,
        P::WEEKLY_SCHEDULE,
        None,
        PropertyValue::List(vec![app(head), app(tail)]),
    )
    .unwrap();
    assert_eq!(read(&split, P::WEEKLY_SCHEDULE), as_read);
}

#[test]
fn weekly_schedule_indexed_write_replaces_one_day() {
    let mut sched = schedule();
    write(
        &mut sched,
        P::WEEKLY_SCHEDULE,
        Some(3),
        app(&occupied_day()),
    )
    .unwrap();
    let PropertyValue::List(days) = read(&sched, P::WEEKLY_SCHEDULE) else {
        panic!("Weekly_Schedule reads as a list");
    };
    for (index, element) in days.iter().enumerate() {
        let expected = if index == 2 {
            occupied_day()
        } else {
            EMPTY_DAY.to_vec()
        };
        assert_eq!(*element, app(&expected), "day {}", index + 1);
    }
    // Writing an empty day clears it again.
    write(&mut sched, P::WEEKLY_SCHEDULE, Some(3), app(EMPTY_DAY)).unwrap();
    assert_eq!(
        read(&sched, P::WEEKLY_SCHEDULE),
        PropertyValue::List(vec![app(EMPTY_DAY); 7])
    );
}

#[test]
fn weekly_schedule_refused_writes_leave_it_unchanged() {
    let duplicate = day(&[&tv(8, 0, REAL_21_5), &tv(8, 0, REAL_16)]);
    // 0xFF in the minute: not a specific time.
    let unspecified = day(&[&tv(8, 0xFF, REAL_21_5)]);
    // A context-tagged value inside a time-value.
    let context_value = day(&[&[0xB4, 8, 0, 0, 0, 0x19, 1][..]]);
    let six_days = [occupied_day(), EMPTY_DAY.repeat(5)].concat();
    let eight_days = [occupied_day(), EMPTY_DAY.repeat(7)].concat();
    let with_bad_day = [occupied_day(), EMPTY_DAY.repeat(5), duplicate.clone()].concat();
    let cases: &[Refusal<PropertyValue>] = &[
        (
            Some(1),
            app(&duplicate),
            ErrorClass::PROPERTY,
            ErrorCode::DUPLICATE_ENTRY,
            "time twice in a day",
        ),
        (
            None,
            app(&with_bad_day),
            ErrorClass::PROPERTY,
            ErrorCode::DUPLICATE_ENTRY,
            "time twice on Sunday of a whole write",
        ),
        (
            Some(1),
            app(&unspecified),
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
            "unspecified minute",
        ),
        (
            None,
            app(&six_days),
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
            "six days",
        ),
        (
            None,
            app(&eight_days),
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
            "eight days",
        ),
        (
            Some(1),
            app(REAL_21_5),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_TYPE,
            "a Real where a day belongs",
        ),
        (
            Some(1),
            PropertyValue::Real(21.5),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_TYPE,
            "a primitive value",
        ),
        (
            Some(1),
            app(&context_value),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_ENCODING,
            "context-tagged value",
        ),
        (
            Some(1),
            app(&occupied_day()[..6]),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_ENCODING,
            "truncated day",
        ),
        (
            Some(1),
            app(&[occupied_day(), EMPTY_DAY.to_vec()].concat()),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_ENCODING,
            "two days at one index",
        ),
        (
            Some(0),
            PropertyValue::Unsigned(7),
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
            "the fixed size",
        ),
        (
            Some(8),
            app(EMPTY_DAY),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_ARRAY_INDEX,
            "index past Sunday",
        ),
    ];
    for (index, value, class, code, what) in cases {
        let mut sched = schedule();
        write(
            &mut sched,
            P::WEEKLY_SCHEDULE,
            Some(2),
            app(&occupied_day()),
        )
        .unwrap();
        let before = read(&sched, P::WEEKLY_SCHEDULE);
        assert_code(
            write(&mut sched, P::WEEKLY_SCHEDULE, *index, value.clone()),
            *class,
            *code,
            what,
        );
        assert_eq!(read(&sched, P::WEEKLY_SCHEDULE), before, "{what}");
    }
}

// --- Exception_Schedule ------------------------------------------------------

/// Christmas Day 2026 (a Friday), 16.0 all day, priority 3: the calendar-entry
/// `[0]` frame around date `[0]` (0x0C), the `[2]` time-values, priority `[3]`.
fn christmas() -> Vec<u8> {
    [
        &[0x0E, 0x0C, 126, 12, 25, 5, 0x0F, 0x2E][..],
        &tv(0, 0, REAL_16),
        &[0x2F, 0x39, 3],
    ]
    .concat()
}

/// Calendar 7 (0x01800007) is TRUE: 21.5 from 08:00, priority 1.
fn holiday_reference() -> Vec<u8> {
    [
        &[0x1C, 0x01, 0x80, 0x00, 0x07, 0x2E][..],
        &tv(8, 0, REAL_21_5),
        &[0x2F, 0x39, 1],
    ]
    .concat()
}

fn exceptions(sched: &ScheduleObject) -> PropertyValue {
    read(sched, P::EXCEPTION_SCHEDULE)
}

#[test]
fn exception_schedule_whole_write_replaces_the_array() {
    let mut sched = schedule();
    let wire = [christmas(), holiday_reference()].concat();
    write(&mut sched, P::EXCEPTION_SCHEDULE, None, app(&wire)).unwrap();
    assert_eq!(
        exceptions(&sched),
        PropertyValue::List(vec![app(&christmas()), app(&holiday_reference())])
    );
    // The referenced Calendar puts the written event in effect.
    let calendar = ObjectIdentifier::new(ObjectType::CALENDAR, 7).unwrap();
    assert_eq!(
        sched.evaluate(monday(), at(9, 0), &|oid| oid == calendar),
        Some(PropertyValue::Real(21.5))
    );
    // The read shape writes back; an empty write empties it.
    let mut other = schedule();
    write(&mut other, P::EXCEPTION_SCHEDULE, None, exceptions(&sched)).unwrap();
    assert_eq!(exceptions(&other), exceptions(&sched));
    write(&mut sched, P::EXCEPTION_SCHEDULE, None, app(&[])).unwrap();
    assert_eq!(exceptions(&sched), PropertyValue::List(vec![]));
}

#[test]
fn exception_schedule_indexed_write_replaces_one_event() {
    let mut sched = schedule();
    let wire = [christmas(), christmas()].concat();
    write(&mut sched, P::EXCEPTION_SCHEDULE, None, app(&wire)).unwrap();
    write(
        &mut sched,
        P::EXCEPTION_SCHEDULE,
        Some(2),
        app(&holiday_reference()),
    )
    .unwrap();
    assert_eq!(
        exceptions(&sched),
        PropertyValue::List(vec![app(&christmas()), app(&holiday_reference())])
    );
    assert_property_code(
        write(
            &mut sched,
            P::EXCEPTION_SCHEDULE,
            Some(3),
            app(&christmas()),
        ),
        ErrorCode::INVALID_ARRAY_INDEX,
        "past the end",
    );
}

#[test]
fn exception_schedule_index_zero_resizes_with_empty_events() {
    let mut sched = schedule();
    write(&mut sched, P::EXCEPTION_SCHEDULE, None, app(&christmas())).unwrap();
    write(
        &mut sched,
        P::EXCEPTION_SCHEDULE,
        Some(0),
        PropertyValue::Unsigned(3),
    )
    .unwrap();
    // An appended event: a wholly unspecified date, no time-values,
    // priority 16.
    let empty: &[u8] = &[
        0x0E, 0x0C, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x2E, 0x2F, 0x39, 16,
    ];
    assert_eq!(
        exceptions(&sched),
        PropertyValue::List(vec![app(&christmas()), app(empty), app(empty)])
    );
    // Empty events never supply a value, so the result is unchanged.
    let no_calendars = |_| false;
    assert_eq!(
        sched.evaluate(monday(), at(9, 0), &no_calendars),
        Some(PropertyValue::Real(10.0))
    );
    write(
        &mut sched,
        P::EXCEPTION_SCHEDULE,
        Some(0),
        PropertyValue::Unsigned(1),
    )
    .unwrap();
    assert_eq!(
        exceptions(&sched),
        PropertyValue::List(vec![app(&christmas())])
    );
    for (value, class, code) in [
        (
            PropertyValue::Unsigned(EXCEPTION_CAP as u64 + 1),
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
        ),
        (
            PropertyValue::Unsigned(u64::MAX),
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
        ),
        (
            PropertyValue::Real(2.0),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_TYPE,
        ),
    ] {
        assert_code(
            write(&mut sched, P::EXCEPTION_SCHEDULE, Some(0), value.clone()),
            class,
            code,
            &format!("{value:?}"),
        );
        assert_eq!(
            exceptions(&sched),
            PropertyValue::List(vec![app(&christmas())])
        );
    }
}

#[test]
fn exception_schedule_refused_writes_leave_it_unchanged() {
    let event = |period: &[u8], time_values: &[u8], priority: u8| {
        [period, &[0x2E], time_values, &[0x2F, 0x39, priority]].concat()
    };
    let christmas_day: &[u8] = &[0x0E, 0x0C, 126, 12, 25, 5, 0x0F];
    let duplicate = event(
        christmas_day,
        &[tv(8, 0, REAL_21_5), tv(8, 0, REAL_16)].concat(),
        3,
    );
    // Month 15 is outside a calendar date's range.
    let month_15 = event(&[0x0E, 0x0C, 126, 15, 1, 0xFF, 0x0F], &[], 3);
    let too_many = christmas().repeat(EXCEPTION_CAP + 1);
    let cases: &[Refusal<Vec<u8>>] = &[
        (
            Some(1),
            duplicate.clone(),
            ErrorClass::PROPERTY,
            ErrorCode::DUPLICATE_ENTRY,
            "time twice in one event",
        ),
        (
            None,
            [christmas(), duplicate].concat(),
            ErrorClass::PROPERTY,
            ErrorCode::DUPLICATE_ENTRY,
            "time twice in the second event of a whole write",
        ),
        (
            Some(1),
            month_15,
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
            "month 15",
        ),
        (
            Some(1),
            event(christmas_day, &tv(0xFF, 0, REAL_16), 3),
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
            "unspecified hour",
        ),
        // A priority outside 1 to 16 decodes and is out of range, as from
        // add_exception (#1087).
        (
            Some(1),
            event(christmas_day, &[], 0),
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
            "priority 0",
        ),
        (
            Some(1),
            event(christmas_day, &[], 17),
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
            "priority 17",
        ),
        (
            None,
            [
                christmas(),
                christmas_day.to_vec(),
                vec![0x2E, 0x2F, 0x3A, 0x01, 0x2C],
            ]
            .concat(),
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
            "priority 300 in the second event of a whole write",
        ),
        (
            Some(1),
            TRUE.to_vec(),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_TYPE,
            "a Boolean where an event belongs",
        ),
        (
            Some(1),
            EMPTY_DAY.to_vec(),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_ENCODING,
            "a daily schedule",
        ),
        (
            None,
            too_many,
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
            "one event past the cap",
        ),
    ];
    for (index, wire, class, code, what) in cases {
        let mut sched = schedule();
        write(&mut sched, P::EXCEPTION_SCHEDULE, None, app(&christmas())).unwrap();
        let before = exceptions(&sched);
        assert_code(
            write(&mut sched, P::EXCEPTION_SCHEDULE, *index, app(wire)),
            *class,
            *code,
            what,
        );
        assert_eq!(exceptions(&sched), before, "{what}");
    }
}

#[test]
fn add_exception_refuses_an_event_past_the_cap() {
    let mut sched = schedule();
    let event = || BACnetSpecialEvent {
        period: SpecialEventPeriod::CalendarReference(
            ObjectIdentifier::new(ObjectType::CALENDAR, 1).unwrap(),
        ),
        list_of_time_values: vec![],
        event_priority: 8,
    };
    for _ in 0..EXCEPTION_CAP {
        sched.add_exception(event()).unwrap();
    }
    assert_code(
        sched.add_exception(event()),
        ErrorClass::RESOURCES,
        ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
        "event 1,025",
    );
}

// --- Effective_Period --------------------------------------------------------

const SEPTEMBER: &[u8] = &[0xA4, 126, 9, 1, 2, 0xA4, 126, 9, 30, 3];

#[test]
fn effective_period_write_takes_two_application_dates() {
    let mut sched = schedule();
    write(&mut sched, P::EFFECTIVE_PERIOD, None, app(SEPTEMBER)).unwrap();
    assert_eq!(read(&sched, P::EFFECTIVE_PERIOD), app(SEPTEMBER));
    let no_calendars = |_| false;
    assert_eq!(
        sched.evaluate(monday(), at(9, 0), &no_calendars),
        Some(PropertyValue::Real(10.0))
    );
    let october = SpecificDate::new(2026, 10, 1).unwrap();
    assert_eq!(sched.evaluate(october, at(9, 0), &no_calendars), None);
    // Open-ended: an unspecified end date.
    let open: &[u8] = &[0xA4, 126, 9, 1, 2, 0xA4, 0xFF, 0xFF, 0xFF, 0xFF];
    write(&mut sched, P::EFFECTIVE_PERIOD, None, app(open)).unwrap();
    assert_eq!(read(&sched, P::EFFECTIVE_PERIOD), app(open));
}

#[test]
fn effective_period_refused_writes_leave_it_unchanged() {
    let cases: &[(Option<u32>, PropertyValue, ErrorCode, &str)] = &[
        // A start date with only its year unspecified is neither specific
        // nor wholly unspecified.
        (
            None,
            app(&[0xA4, 0xFF, 9, 1, 2, 0xA4, 126, 9, 30, 3]),
            ErrorCode::VALUE_OUT_OF_RANGE,
            "partly unspecified start",
        ),
        (None, app(REAL_16), ErrorCode::INVALID_DATA_TYPE, "a Real"),
        (
            None,
            PropertyValue::Date(Date {
                year: 126,
                month: 9,
                day: 1,
                day_of_week: 2,
            }),
            ErrorCode::INVALID_DATA_TYPE,
            "a primitive Date",
        ),
        (
            None,
            app(&SEPTEMBER[..5]),
            ErrorCode::INVALID_DATA_ENCODING,
            "one date",
        ),
        (
            None,
            app(&[SEPTEMBER, &[0x00]].concat()),
            ErrorCode::INVALID_DATA_ENCODING,
            "trailing NULL",
        ),
        (
            Some(1),
            app(SEPTEMBER),
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
            "an array index",
        ),
    ];
    for (index, value, code, what) in cases {
        let mut sched = schedule();
        let before = read(&sched, P::EFFECTIVE_PERIOD);
        assert_property_code(
            write(&mut sched, P::EFFECTIVE_PERIOD, *index, value.clone()),
            *code,
            what,
        );
        assert_eq!(read(&sched, P::EFFECTIVE_PERIOD), before, "{what}");
    }
}
