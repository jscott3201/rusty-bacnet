//! Schedule evaluation (#1028): the Clause 12.24.4 precedence of exceptions,
//! the weekly schedule and Schedule_Default; NULL hand-over; the special-event
//! period forms; Effective_Period; and what a tick writes.

use super::*;
use bacnet_types::constructed::{BACnetCalendarEntry, BACnetWeekNDay, SpecialEventPeriod};

const ANY: u8 = 0xFF;

fn day(year: u16, month: u8, d: u8) -> SpecificDate {
    SpecificDate::new(year, month, d).unwrap()
}

/// Monday 14 September 2026.
fn monday() -> SpecificDate {
    day(2026, 9, 14)
}

fn at(hour: u8, minute: u8) -> Time {
    Time {
        hour,
        minute,
        second: 0,
        hundredths: 0,
    }
}

fn tv(time: Time, value: PropertyValue) -> BACnetTimeValue {
    BACnetTimeValue { time, value }
}

fn real(value: f32) -> PropertyValue {
    PropertyValue::Real(value)
}

fn no_calendars(_: ObjectIdentifier) -> bool {
    false
}

fn every_day() -> SpecialEventPeriod {
    SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::WeekNDay(BACnetWeekNDay {
        month: ANY,
        week_of_month: ANY,
        day_of_week: ANY,
    }))
}

fn event(
    period: SpecialEventPeriod,
    list_of_time_values: Vec<BACnetTimeValue>,
    event_priority: u64,
) -> BACnetSpecialEvent {
    BACnetSpecialEvent {
        period,
        list_of_time_values,
        event_priority,
    }
}

fn schedule(default: PropertyValue) -> ScheduleObject {
    ScheduleObject::new(1, "SCHED-1", default).unwrap()
}

/// Occupied 08:00 to 17:00 on Mondays, 21.0 then 16.0.
fn monday_occupancy() -> ScheduleObject {
    let mut sched = schedule(real(10.0));
    sched
        .set_weekly_schedule(0, vec![tv(at(8, 0), real(21.0)), tv(at(17, 0), real(16.0))])
        .unwrap();
    sched
}

fn eval(sched: &ScheduleObject, today: SpecificDate, time: Time) -> Option<PropertyValue> {
    sched.evaluate(today, time, &no_calendars)
}

fn assert_code(result: Result<(), Error>, code: ErrorCode, what: &str) {
    match result {
        Err(Error::Protocol { code: c, .. }) => {
            assert_eq!(c, code, "{what}: expected {code:?}");
        }
        other => panic!("{what}: expected {code:?}, got {other:?}"),
    }
}

// --- Precedence -------------------------------------------------------------

#[test]
fn evaluate_falls_back_to_schedule_default() {
    let sched = schedule(real(10.0));
    assert_eq!(eval(&sched, monday(), at(12, 0)), Some(real(10.0)));
}

#[test]
fn evaluate_weekly_uses_the_latest_entry_at_or_before_now() {
    let sched = monday_occupancy();
    for (time, expected) in [
        (at(0, 0), real(10.0)), // before the first entry: Schedule_Default
        (at(7, 59), real(10.0)),
        (at(8, 0), real(21.0)),
        (at(12, 0), real(21.0)),
        (at(17, 0), real(16.0)),
        (at(23, 59), real(16.0)),
    ] {
        assert_eq!(eval(&sched, monday(), time), Some(expected), "{time:?}");
    }
    // Tuesday has no entries.
    assert_eq!(eval(&sched, day(2026, 9, 15), at(12, 0)), Some(real(10.0)));
    // Seconds and hundredths count, and entry order does not.
    let mut sched = schedule(real(10.0));
    let half_past = Time {
        second: 30,
        hundredths: 50,
        ..at(8, 0)
    };
    sched
        .set_weekly_schedule(
            0,
            vec![tv(at(17, 0), real(16.0)), tv(half_past, real(21.0))],
        )
        .unwrap();
    assert_eq!(
        eval(
            &sched,
            monday(),
            Time {
                hundredths: 49,
                ..half_past
            }
        ),
        Some(real(10.0))
    );
    assert_eq!(eval(&sched, monday(), half_past), Some(real(21.0)));
    assert_eq!(eval(&sched, monday(), at(17, 30)), Some(real(16.0)));
}

#[test]
fn evaluate_typed_values_keep_their_own_datatype() {
    // #1028: these used to come back as Octet Strings of their encoding.
    for value in [
        PropertyValue::Boolean(true),
        PropertyValue::Unsigned(3),
        PropertyValue::Signed(-4),
        real(21.5),
        PropertyValue::Double(0.25),
        PropertyValue::Enumerated(1),
        PropertyValue::CharacterString("occupied".into()),
        PropertyValue::OctetString(vec![1, 2]),
        PropertyValue::Date(monday().to_date()),
        PropertyValue::Time(at(9, 0)),
        PropertyValue::ObjectIdentifier(ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap()),
    ] {
        let mut sched = schedule(PropertyValue::Null);
        sched
            .set_weekly_schedule(0, vec![tv(at(8, 0), value.clone())])
            .unwrap();
        assert_eq!(eval(&sched, monday(), at(9, 0)), Some(value));
    }
}

#[test]
fn evaluate_exception_takes_precedence_over_the_weekly_schedule() {
    let mut sched = monday_occupancy();
    sched
        .add_exception(event(every_day(), vec![tv(at(0, 0), real(5.0))], 10))
        .unwrap();
    assert_eq!(eval(&sched, monday(), at(12, 0)), Some(real(5.0)));
}

#[test]
fn evaluate_null_hands_control_to_the_next_source() {
    let mut sched = schedule(real(10.0));
    sched
        .set_weekly_schedule(
            0,
            vec![
                tv(at(8, 0), real(21.0)),
                tv(at(12, 0), PropertyValue::Null),
                tv(at(13, 0), real(22.0)),
            ],
        )
        .unwrap();
    // A NULL weekly value falls through to Schedule_Default.
    assert_eq!(eval(&sched, monday(), at(12, 30)), Some(real(10.0)));
    assert_eq!(eval(&sched, monday(), at(13, 0)), Some(real(22.0)));
    // An exception that turns NULL at 10:00 hands back to the weekly value.
    sched
        .add_exception(event(
            every_day(),
            vec![tv(at(0, 0), real(5.0)), tv(at(10, 0), PropertyValue::Null)],
            1,
        ))
        .unwrap();
    assert_eq!(eval(&sched, monday(), at(9, 0)), Some(real(5.0)));
    assert_eq!(eval(&sched, monday(), at(11, 0)), Some(real(21.0)));
    assert_eq!(eval(&sched, monday(), at(12, 30)), Some(real(10.0)));
    // A NULL Schedule_Default is the result when nothing else applies.
    let sched = schedule(PropertyValue::Null);
    assert_eq!(eval(&sched, monday(), at(12, 0)), Some(PropertyValue::Null));
}

#[test]
fn evaluate_best_priority_with_a_current_value_wins_and_the_lower_index_breaks_ties() {
    let mut sched = monday_occupancy();
    for (value, priority) in [(1.0, 15), (2.0, 5), (3.0, 7)] {
        sched
            .add_exception(event(
                every_day(),
                vec![tv(at(0, 0), real(value))],
                priority,
            ))
            .unwrap();
    }
    // Priority 5 beats 7 and 15 (1 is the best).
    assert_eq!(eval(&sched, monday(), at(12, 0)), Some(real(2.0)));
    // Equal priority: the earlier array element wins.
    sched
        .add_exception(event(every_day(), vec![tv(at(0, 0), real(4.0))], 5))
        .unwrap();
    assert_eq!(eval(&sched, monday(), at(12, 0)), Some(real(2.0)));
    // A better event whose value is NULL, or that has not started yet, is
    // passed over for the best one with a value.
    let mut sched = monday_occupancy();
    sched
        .add_exception(event(
            every_day(),
            vec![tv(at(0, 0), PropertyValue::Null)],
            1,
        ))
        .unwrap();
    sched
        .add_exception(event(every_day(), vec![tv(at(18, 0), real(8.0))], 2))
        .unwrap();
    sched
        .add_exception(event(every_day(), vec![tv(at(0, 0), real(9.0))], 16))
        .unwrap();
    assert_eq!(eval(&sched, monday(), at(12, 0)), Some(real(9.0)));
    assert_eq!(eval(&sched, monday(), at(18, 0)), Some(real(8.0)));
}

// --- Special-event periods ----------------------------------------------------

#[test]
fn evaluate_exception_applies_only_on_days_its_calendar_entry_matches() {
    let christmas = day(2026, 12, 25); // a Friday
    let week_before = day(2026, 12, 18);
    let entry = |e| SpecialEventPeriod::CalendarEntry(e);
    let date = |year, month, d, weekday| Date {
        year,
        month,
        day: d,
        day_of_week: weekday,
    };
    let wnd = |month, week_of_month, day_of_week| {
        BACnetCalendarEntry::WeekNDay(BACnetWeekNDay {
            month,
            week_of_month,
            day_of_week,
        })
    };
    for (what, period, on_christmas, on_week_before) in [
        (
            "exact date",
            entry(BACnetCalendarEntry::Date(christmas.to_date())),
            true,
            false,
        ),
        (
            "every year's 25 December",
            entry(BACnetCalendarEntry::Date(date(ANY, 12, 25, ANY))),
            true,
            false,
        ),
        (
            "every Friday",
            entry(BACnetCalendarEntry::Date(date(ANY, ANY, ANY, 5))),
            true,
            true,
        ),
        (
            "odd months",
            entry(BACnetCalendarEntry::Date(date(ANY, 13, ANY, ANY))),
            false,
            false,
        ),
        (
            "date range over Christmas",
            entry(BACnetCalendarEntry::DateRange(BACnetDateRange {
                start_date: day(2026, 12, 24).to_date(),
                end_date: day(2026, 12, 26).to_date(),
            })),
            true,
            false,
        ),
        (
            "range open at the start",
            entry(BACnetCalendarEntry::DateRange(BACnetDateRange {
                start_date: unspecified_date(),
                end_date: day(2026, 12, 20).to_date(),
            })),
            false,
            true,
        ),
        ("last Friday of December", entry(wnd(12, 6, 5)), true, false),
        (
            "Friday in the week before the last",
            entry(wnd(12, 7, 5)),
            false,
            true,
        ),
        (
            "third Friday (days 15-21)",
            entry(wnd(ANY, 3, 5)),
            false,
            true,
        ),
        ("any December day", entry(wnd(12, ANY, ANY)), true, true),
        ("any November day", entry(wnd(11, ANY, ANY)), false, false),
    ] {
        let mut sched = schedule(real(10.0));
        sched
            .add_exception(event(period, vec![tv(at(0, 0), real(1.0))], 8))
            .unwrap();
        let expect = |hit| Some(if hit { real(1.0) } else { real(10.0) });
        assert_eq!(
            eval(&sched, christmas, at(9, 0)),
            expect(on_christmas),
            "{what}"
        );
        assert_eq!(
            eval(&sched, week_before, at(9, 0)),
            expect(on_week_before),
            "{what}"
        );
    }
}

#[test]
fn evaluate_calendar_reference_follows_the_referenced_calendar() {
    let holidays = ObjectIdentifier::new(ObjectType::CALENDAR, 3).unwrap();
    let other = ObjectIdentifier::new(ObjectType::CALENDAR, 4).unwrap();
    let mut sched = monday_occupancy();
    sched
        .add_exception(event(
            SpecialEventPeriod::CalendarReference(holidays),
            vec![tv(at(0, 0), real(12.0))],
            4,
        ))
        .unwrap();
    let only = |active: ObjectIdentifier| move |oid: ObjectIdentifier| oid == active;
    // Calendar 3 FALSE: the weekly value; TRUE: the exception's.
    assert_eq!(
        sched.evaluate(monday(), at(9, 0), &only(other)),
        Some(real(21.0))
    );
    assert_eq!(
        sched.evaluate(monday(), at(9, 0), &only(holidays)),
        Some(real(12.0))
    );
}

// --- Effective_Period ---------------------------------------------------------

#[test]
fn evaluate_only_within_effective_period_with_both_ends_included() {
    let mut sched = monday_occupancy();
    sched
        .set_effective_period(BACnetDateRange {
            start_date: day(2026, 9, 1).to_date(),
            end_date: day(2026, 9, 30).to_date(),
        })
        .unwrap();
    for (d, inside) in [
        (day(2026, 8, 31), false),
        (day(2026, 9, 1), true),
        (day(2026, 9, 30), true),
        (day(2026, 10, 1), false),
        (day(2027, 9, 15), false),
    ] {
        assert_eq!(eval(&sched, d, at(12, 0)).is_some(), inside, "{d:?}");
    }
    // An unspecified end leaves that side open.
    sched
        .set_effective_period(BACnetDateRange {
            start_date: day(2026, 9, 1).to_date(),
            end_date: unspecified_date(),
        })
        .unwrap();
    assert!(eval(&sched, day(2154, 12, 31), at(12, 0)).is_some());
    assert!(eval(&sched, day(2026, 8, 31), at(12, 0)).is_none());
}

// --- Ticks --------------------------------------------------------------------

fn references() -> Vec<BACnetObjectPropertyReference> {
    vec![
        BACnetObjectPropertyReference::new(
            ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 5).unwrap(),
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ),
        BACnetObjectPropertyReference::new_indexed(
            ObjectIdentifier::new(ObjectType::MULTI_STATE_OUTPUT, 7).unwrap(),
            PropertyIdentifier::STATE_TEXT.to_raw(),
            2,
        ),
    ]
}

fn with_references(mut sched: ScheduleObject) -> ScheduleObject {
    for reference in references() {
        sched.add_object_property_reference(reference).unwrap();
    }
    sched
}

fn tick(sched: &mut ScheduleObject, today: SpecificDate, time: Time) -> Option<ScheduleWrite> {
    sched.tick_schedule(today, time, &no_calendars)
}

#[test]
fn tick_schedule_writes_the_typed_value_at_priority_for_writing() {
    let mut sched = with_references(monday_occupancy());
    sched.set_priority_for_writing(9).unwrap();
    // Start-up enters the Effective_Period: the default is written.
    assert_eq!(
        tick(&mut sched, monday(), at(7, 0)),
        Some(ScheduleWrite {
            value: real(10.0),
            priority: 9,
            references: references(),
            retry: false,
        })
    );
    assert_eq!(tick(&mut sched, monday(), at(7, 30)), None);
    let write = tick(&mut sched, monday(), at(8, 0)).unwrap();
    assert_eq!(write.value, real(21.0));
    assert_eq!(write.priority, 9);
    assert_eq!(write.references, references());
    assert_eq!(
        sched
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        real(21.0)
    );
    assert_eq!(
        sched
            .read_property(PropertyIdentifier::PRIORITY_FOR_WRITING, None)
            .unwrap(),
        PropertyValue::Unsigned(9)
    );
    // Unchanged on the next tick: nothing to write.
    assert_eq!(tick(&mut sched, monday(), at(9, 0)), None);
}

#[test]
fn tick_schedule_writes_null_to_relinquish() {
    let mut sched = with_references(schedule(PropertyValue::Null));
    sched
        .set_weekly_schedule(
            0,
            vec![tv(at(8, 0), real(21.0)), tv(at(17, 0), PropertyValue::Null)],
        )
        .unwrap();
    assert_eq!(
        tick(&mut sched, monday(), at(9, 0)).unwrap().value,
        real(21.0)
    );
    let write = tick(&mut sched, monday(), at(17, 0)).unwrap();
    assert_eq!(write.value, PropertyValue::Null);
    assert_eq!(write.priority, 16);
    assert_eq!(*sched.present_value(), PropertyValue::Null);
}

#[test]
fn tick_schedule_writes_on_entering_effective_period_even_when_unchanged() {
    let mut sched = with_references(schedule(real(10.0)));
    sched
        .set_effective_period(BACnetDateRange {
            start_date: day(2026, 9, 1).to_date(),
            end_date: day(2026, 9, 30).to_date(),
        })
        .unwrap();
    // Before the period: inactive, nothing written, Present_Value kept.
    assert_eq!(tick(&mut sched, day(2026, 8, 31), at(12, 0)), None);
    // Entering it writes the value, though it equals Present_Value.
    assert_eq!(
        tick(&mut sched, day(2026, 9, 1), at(0, 0)).map(|w| w.value),
        Some(real(10.0))
    );
    assert_eq!(tick(&mut sched, day(2026, 9, 1), at(0, 1)), None);
    assert_eq!(tick(&mut sched, day(2026, 9, 30), at(23, 59)), None);
    // Leaving it writes nothing; a value that would have changed is not
    // calculated, so Present_Value keeps its last value.
    sched
        .set_weekly_schedule(3, vec![tv(at(0, 0), real(30.0))])
        .unwrap();
    assert_eq!(tick(&mut sched, day(2026, 10, 1), at(12, 0)), None);
    assert_eq!(*sched.present_value(), real(10.0));
    // Coming back into it (the clock set back) writes again.
    assert_eq!(
        tick(&mut sched, day(2026, 9, 15), at(12, 0)).map(|w| w.value),
        Some(real(10.0))
    );
}

#[test]
fn tick_schedule_updates_present_value_without_references() {
    let mut sched = monday_occupancy();
    assert_eq!(tick(&mut sched, monday(), at(9, 0)), None);
    assert_eq!(*sched.present_value(), real(21.0));
}

#[test]
fn tick_schedule_does_nothing_while_out_of_service() {
    let mut sched = with_references(monday_occupancy());
    sched
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    assert_eq!(tick(&mut sched, monday(), at(9, 0)), None);
    assert_eq!(*sched.present_value(), real(10.0));
    // The calculation itself ignores Out_Of_Service.
    assert_eq!(eval(&sched, monday(), at(9, 0)), Some(real(21.0)));
}

// --- Value checks -------------------------------------------------------------

#[test]
fn schedule_setters_refuse_values_outside_their_datatype_or_range() {
    let mut sched = schedule(real(10.0));
    let list = PropertyValue::List(vec![real(1.0)]);
    for (what, entries, code) in [
        (
            "hour 24",
            vec![tv(at(24, 0), real(1.0))],
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            "unspecified minute",
            vec![tv(at(8, ANY), real(1.0))],
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            "a constructed value",
            vec![tv(at(8, 0), list.clone())],
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (
            "encoded bytes",
            vec![tv(at(8, 0), PropertyValue::ApplicationData(vec![0x21, 1]))],
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (
            "the same time twice",
            vec![tv(at(8, 0), real(1.0)), tv(at(8, 0), real(2.0))],
            ErrorCode::DUPLICATE_ENTRY,
        ),
    ] {
        assert_code(sched.set_weekly_schedule(0, entries.clone()), code, what);
        assert_code(
            sched.add_exception(event(every_day(), entries, 8)),
            code,
            what,
        );
    }
    for priority in [0, 17] {
        assert_code(
            sched.set_priority_for_writing(priority),
            ErrorCode::VALUE_OUT_OF_RANGE,
            "Priority_For_Writing",
        );
    }
    // The error a network write of these priorities gets too (#1087).
    for priority in [0, 17, 300, u64::MAX] {
        assert_code(
            sched.add_exception(event(every_day(), vec![], priority)),
            ErrorCode::VALUE_OUT_OF_RANGE,
            "event priority",
        );
    }
    let bad_entry =
        SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::WeekNDay(BACnetWeekNDay {
            month: 0,
            week_of_month: ANY,
            day_of_week: ANY,
        }));
    assert_code(
        sched.add_exception(event(bad_entry, vec![], 8)),
        ErrorCode::VALUE_OUT_OF_RANGE,
        "inline entry month 0",
    );
    assert_code(
        sched.set_effective_period(BACnetDateRange {
            start_date: Date {
                year: ANY,
                ..day(2026, 9, 1).to_date()
            },
            end_date: unspecified_date(),
        }),
        ErrorCode::VALUE_OUT_OF_RANGE,
        "Effective_Period start without a year",
    );
    assert_code(
        sched.write_property(
            PropertyIdentifier::SCHEDULE_DEFAULT,
            None,
            list.clone(),
            None,
        ),
        ErrorCode::INVALID_DATA_TYPE,
        "Schedule_Default list",
    );
    assert!(matches!(
        ScheduleObject::new(2, "SCHED-2", list),
        Err(Error::Protocol { code, .. }) if code == ErrorCode::INVALID_DATA_TYPE
    ));
    // Nothing was stored.
    assert!(sched.weekly_schedule.iter().all(Vec::is_empty));
    assert!(sched.exception_schedule.is_empty());
    assert_eq!(sched.priority_for_writing, 16);
    assert!(sched.effective_period.start_date.is_unspecified());
    assert_eq!(sched.schedule_default, real(10.0));
    // The boundary priorities are accepted.
    sched.set_priority_for_writing(1).unwrap();
    sched.add_exception(event(every_day(), vec![], 16)).unwrap();
}
