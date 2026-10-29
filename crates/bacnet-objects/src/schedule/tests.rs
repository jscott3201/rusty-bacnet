use super::*;
use bacnet_types::constructed::{
    BACnetCalendarEntry, BACnetDateRange, BACnetSpecialEvent, BACnetTimeValue, BACnetWeekNDay,
    SpecialEventPeriod,
};
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::{Date, Time};

// --- Schedule ---

#[test]
fn schedule_read_present_value_default() {
    let sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    let val = sched
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Real(72.0));
}

#[test]
fn schedule_read_schedule_default() {
    let sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    let val = sched
        .read_property(PropertyIdentifier::SCHEDULE_DEFAULT, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Real(72.0));
}

#[test]
fn schedule_write_schedule_default() {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    sched
        .write_property(
            PropertyIdentifier::SCHEDULE_DEFAULT,
            None,
            PropertyValue::Real(68.0),
            None,
        )
        .unwrap();
    let val = sched
        .read_property(PropertyIdentifier::SCHEDULE_DEFAULT, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Real(68.0));
}

// --- Schedule weekly_schedule ---

fn make_time(hour: u8, minute: u8) -> Time {
    Time {
        hour,
        minute,
        second: 0,
        hundredths: 0,
    }
}

fn make_tv(hour: u8, minute: u8, value: PropertyValue) -> BACnetTimeValue {
    BACnetTimeValue {
        time: make_time(hour, minute),
        value,
    }
}

fn app(bytes: &[u8]) -> PropertyValue {
    PropertyValue::ApplicationData(bytes.to_vec())
}

/// An empty BACnetDailySchedule: opening and closing tag `[0]`.
const EMPTY_DAY: &[u8] = &[0x0E, 0x0F];

fn weekly(sched: &ScheduleObject, index: Option<u32>) -> Result<PropertyValue, Error> {
    sched.read_property(PropertyIdentifier::WEEKLY_SCHEDULE, index)
}

#[test]
fn schedule_weekly_schedule_empty_by_default() {
    let sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    assert_eq!(
        weekly(&sched, None).unwrap(),
        PropertyValue::List(vec![app(EMPTY_DAY); 7])
    );
}

/// Monday's BACnetDailySchedule: the `[0]` frame around two time-values,
/// each an application Time (0xB4) then the value under its own application
/// tag: Unsigned 1 (0x21 0x01), then Null (0x00).
const MONDAY: &[u8] = &[
    0x0E, 0xB4, 8, 0, 0, 0, 0x21, 1, 0xB4, 17, 0, 0, 0, 0x00, 0x0F,
];

fn monday_schedule() -> ScheduleObject {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    sched
        .set_weekly_schedule(
            0,
            vec![
                make_tv(8, 0, PropertyValue::Unsigned(1)),
                make_tv(17, 0, PropertyValue::Null),
            ],
        )
        .unwrap();
    sched
}

#[test]
fn schedule_weekly_schedule_set_monday_read_no_index() {
    // #996: each day used to read as a list of [Time, Octet String] pairs.
    let mut expected = vec![app(MONDAY)];
    expected.extend(std::iter::repeat_n(app(EMPTY_DAY), 6));
    assert_eq!(
        weekly(&monday_schedule(), None).unwrap(),
        PropertyValue::List(expected)
    );
}

#[test]
fn schedule_weekly_schedule_index_0_returns_count() {
    let sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    assert_eq!(weekly(&sched, Some(0)).unwrap(), PropertyValue::Unsigned(7));
}

#[test]
fn schedule_weekly_schedule_index_1_returns_monday() {
    let sched = monday_schedule();
    assert_eq!(weekly(&sched, Some(1)).unwrap(), app(MONDAY));
    assert_eq!(weekly(&sched, Some(2)).unwrap(), app(EMPTY_DAY));
}

#[test]
fn schedule_weekly_schedule_index_7_returns_sunday() {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    sched
        .set_weekly_schedule(6, vec![make_tv(10, 0, PropertyValue::Boolean(false))])
        .unwrap();
    assert_eq!(
        weekly(&sched, Some(7)).unwrap(),
        app(&[0x0E, 0xB4, 10, 0, 0, 0, 0x10, 0x0F])
    );
}

#[test]
fn schedule_weekly_schedule_invalid_index_8_returns_error() {
    let sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    let result = weekly(&sched, Some(8));
    if let Err(Error::Remote(RemoteError::Error { code, .. })) = result {
        assert_eq!(code, ErrorCode::INVALID_ARRAY_INDEX);
    } else {
        panic!("expected Protocol error, got {result:?}");
    }
}

#[test]
fn schedule_weekly_schedule_refuses_a_day_index_past_sunday() {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    assert!(matches!(
        sched.set_weekly_schedule(7, vec![make_tv(8, 0, PropertyValue::Unsigned(1))]),
        Err(Error::Remote(RemoteError::Error { code, .. })) if code == ErrorCode::VALUE_OUT_OF_RANGE
    ));
    assert_eq!(
        weekly(&sched, None).unwrap(),
        PropertyValue::List(vec![app(EMPTY_DAY); 7])
    );
}

// --- Schedule effective_period ---

#[test]
fn schedule_effective_period_defaults_to_every_date() {
    // #996: a BACnetDateRange, never NULL. Both dates unspecified (0xFF octets)
    // makes a range that covers every date.
    let sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    assert_eq!(
        sched
            .read_property(PropertyIdentifier::EFFECTIVE_PERIOD, None)
            .unwrap(),
        app(&[0xA4, 0xFF, 0xFF, 0xFF, 0xFF, 0xA4, 0xFF, 0xFF, 0xFF, 0xFF])
    );
}

#[test]
fn schedule_effective_period_set_and_read() {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    let period = BACnetDateRange {
        start_date: Date {
            year: 124,
            month: 1,
            day: 1,
            day_of_week: 1,
        },
        end_date: Date {
            year: 124,
            month: 12,
            day: 31,
            day_of_week: 2,
        },
    };
    sched.set_effective_period(period).unwrap();
    // Two application Dates (tag 10, length 4: 0xA4), not an Octet String.
    assert_eq!(
        sched
            .read_property(PropertyIdentifier::EFFECTIVE_PERIOD, None)
            .unwrap(),
        app(&[0xA4, 124, 1, 1, 1, 0xA4, 124, 12, 31, 2])
    );
}

// --- Schedule exception_schedule ---

fn every_weekday(day_of_week: u8) -> SpecialEventPeriod {
    SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::WeekNDay(BACnetWeekNDay {
        month: BACnetWeekNDay::ANY,
        week_of_month: BACnetWeekNDay::ANY,
        day_of_week,
    }))
}

/// Every Sunday, Null at midnight, priority 16: the calendar-entry `[0]` frame
/// around weekNDay `[2]` (0x2B), the `[2]` time-value frame, priority `[3]`.
const SUNDAY_EVENT: &[u8] = &[
    0x0E, 0x2B, 0xFF, 0xFF, 7, 0x0F, 0x2E, 0xB4, 0, 0, 0, 0, 0x00, 0x2F, 0x39, 16,
];

fn sunday_event() -> BACnetSpecialEvent {
    BACnetSpecialEvent {
        period: every_weekday(7),
        list_of_time_values: vec![make_tv(0, 0, PropertyValue::Null)],
        event_priority: 16,
    }
}

fn exceptions(sched: &ScheduleObject, index: Option<u32>) -> Result<PropertyValue, Error> {
    sched.read_property(PropertyIdentifier::EXCEPTION_SCHEDULE, index)
}

#[test]
fn schedule_exception_schedule_empty_by_default() {
    let sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    assert_eq!(
        exceptions(&sched, None).unwrap(),
        PropertyValue::List(vec![])
    );
    assert_eq!(
        exceptions(&sched, Some(0)).unwrap(),
        PropertyValue::Unsigned(0)
    );
}

#[test]
fn schedule_exception_schedule_count_via_index_zero() {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    sched.add_exception(sunday_event()).unwrap();
    assert_eq!(
        exceptions(&sched, Some(0)).unwrap(),
        PropertyValue::Unsigned(1)
    );
}

#[test]
fn schedule_exception_schedule_reads_each_special_event_with_its_period() {
    // #996: the period, inline calendar entry or Calendar reference, used to
    // be dropped, and the priority was an application Unsigned.
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    sched.add_exception(sunday_event()).unwrap();
    sched
        .add_exception(BACnetSpecialEvent {
            period: every_weekday(1),
            list_of_time_values: vec![],
            event_priority: 14,
        })
        .unwrap();
    sched
        .add_exception(BACnetSpecialEvent {
            period: SpecialEventPeriod::CalendarReference(
                ObjectIdentifier::new(ObjectType::CALENDAR, 7).unwrap(),
            ),
            list_of_time_values: vec![],
            event_priority: 2,
        })
        .unwrap();
    let monday: &[u8] = &[0x0E, 0x2B, 0xFF, 0xFF, 1, 0x0F, 0x2E, 0x2F, 0x39, 14];
    // calendar-reference `[1]` (0x1C) holding Calendar 7, 0x01800007.
    let reference: &[u8] = &[0x1C, 0x01, 0x80, 0x00, 0x07, 0x2E, 0x2F, 0x39, 2];
    assert_eq!(
        exceptions(&sched, None).unwrap(),
        PropertyValue::List(vec![app(SUNDAY_EVENT), app(monday), app(reference)])
    );
    assert_eq!(
        exceptions(&sched, Some(0)).unwrap(),
        PropertyValue::Unsigned(3)
    );
    assert_eq!(exceptions(&sched, Some(1)).unwrap(), app(SUNDAY_EVENT));
    assert_eq!(exceptions(&sched, Some(3)).unwrap(), app(reference));
    for index in [4, u32::MAX] {
        assert!(matches!(
            exceptions(&sched, Some(index)),
            Err(Error::Remote(RemoteError::Error { code, .. }))
                if code == ErrorCode::INVALID_ARRAY_INDEX
        ));
    }
}

// --- Schedule list_of_object_property_references ---

#[test]
fn schedule_opr_list_empty_by_default() {
    let sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    let val = sched
        .read_property(PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES, None)
        .unwrap();
    assert_eq!(val, PropertyValue::ApplicationData(vec![]));
}

#[test]
fn schedule_opr_list_add_and_read() {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let r = BACnetObjectPropertyReference::new(oid, PropertyIdentifier::PRESENT_VALUE.to_raw());
    sched.add_object_property_reference(r.clone()).unwrap();

    let val = sched
        .read_property(PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::ApplicationData(vec![0x0c, 0, 0, 0, 1, 0x19, 85])
    );
}

#[test]
fn schedule_opr_list_multiple_references() {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    let oid1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let oid2 = ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 5).unwrap();
    sched
        .add_object_property_reference(BACnetObjectPropertyReference::new(
            oid1,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ))
        .unwrap();
    sched
        .add_object_property_reference(BACnetObjectPropertyReference::new(
            oid2,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ))
        .unwrap();

    let val = sched
        .read_property(PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES, None)
        .unwrap();
    assert_eq!(
        val,
        PropertyValue::ApplicationData(vec![
            0x0c, 0, 0, 0, 1, 0x19, 85, 0x0c, 0x01, 0, 0, 5, 0x19, 85,
        ])
    );
}

// --- Schedule property_list ---

#[test]
fn schedule_property_list_contains_new_properties() {
    let sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    let props = sched.property_list();
    assert!(props.contains(&PropertyIdentifier::WEEKLY_SCHEDULE));
    assert!(props.contains(&PropertyIdentifier::EXCEPTION_SCHEDULE));
    assert!(props.contains(&PropertyIdentifier::EFFECTIVE_PERIOD));
    assert!(props.contains(&PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES));
}
