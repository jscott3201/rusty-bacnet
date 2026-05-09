use super::*;
use bacnet_types::constructed::{
    BACnetCalendarEntry, BACnetDateRange, BACnetSpecialEvent, BACnetTimeValue, BACnetWeekNDay,
    SpecialEventPeriod,
};
use bacnet_types::primitives::{Date, Time};

// --- Calendar ---

#[test]
fn calendar_read_present_value_default() {
    let cal = CalendarObject::new(1, "CAL-1").unwrap();
    let val = cal
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Boolean(false));
}

#[test]
fn calendar_set_present_value() {
    let mut cal = CalendarObject::new(1, "CAL-1").unwrap();
    cal.set_present_value(true);
    let val = cal
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Boolean(true));
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
fn calendar_date_list_empty_by_default() {
    let cal = CalendarObject::new(1, "CAL-1").unwrap();
    let val = cal
        .read_property(PropertyIdentifier::DATE_LIST, None)
        .unwrap();
    assert_eq!(val, PropertyValue::List(vec![]));
}

#[test]
fn calendar_date_list_add_and_read_entries() {
    let mut cal = CalendarObject::new(1, "CAL-1").unwrap();

    // Add a Date entry
    let d = Date {
        year: 124,
        month: 3,
        day: 15,
        day_of_week: 5,
    };
    cal.add_date_entry(BACnetCalendarEntry::Date(d));

    // Add a DateRange entry
    let dr = BACnetDateRange {
        start_date: Date {
            year: 124,
            month: 6,
            day: 1,
            day_of_week: 6,
        },
        end_date: Date {
            year: 124,
            month: 6,
            day: 30,
            day_of_week: 0,
        },
    };
    cal.add_date_entry(BACnetCalendarEntry::DateRange(dr.clone()));

    // Add a WeekNDay entry
    let wnd = BACnetWeekNDay {
        month: BACnetWeekNDay::ANY,
        week_of_month: BACnetWeekNDay::ANY,
        day_of_week: 1,
    };
    cal.add_date_entry(BACnetCalendarEntry::WeekNDay(wnd.clone()));

    let val = cal
        .read_property(PropertyIdentifier::DATE_LIST, None)
        .unwrap();

    if let PropertyValue::List(items) = val {
        assert_eq!(items.len(), 3);
        assert_eq!(items[0], PropertyValue::Date(d));
        assert_eq!(items[1], PropertyValue::OctetString(dr.encode().to_vec()));
        assert_eq!(items[2], PropertyValue::OctetString(wnd.encode().to_vec()));
    } else {
        panic!("expected PropertyValue::List");
    }
}

#[test]
fn calendar_date_list_clear() {
    let mut cal = CalendarObject::new(1, "CAL-1").unwrap();
    let d = Date {
        year: 124,
        month: 1,
        day: 1,
        day_of_week: 1,
    };
    cal.add_date_entry(BACnetCalendarEntry::Date(d));
    // Confirm it was added
    let val = cal
        .read_property(PropertyIdentifier::DATE_LIST, None)
        .unwrap();
    if let PropertyValue::List(items) = &val {
        assert_eq!(items.len(), 1);
    } else {
        panic!("expected PropertyValue::List");
    }
    // Clear and verify empty
    cal.clear_date_list();
    let val = cal
        .read_property(PropertyIdentifier::DATE_LIST, None)
        .unwrap();
    assert_eq!(val, PropertyValue::List(vec![]));
}

#[test]
fn calendar_property_list_contains_date_list() {
    let cal = CalendarObject::new(1, "CAL-1").unwrap();
    let props = cal.property_list();
    assert!(props.contains(&PropertyIdentifier::DATE_LIST));
}

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

#[test]
fn schedule_set_present_value() {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    sched.set_present_value(PropertyValue::Real(65.0));
    let val = sched
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Real(65.0));
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

fn make_tv(hour: u8, minute: u8, raw_value: Vec<u8>) -> BACnetTimeValue {
    BACnetTimeValue {
        time: make_time(hour, minute),
        value: raw_value,
    }
}

#[test]
fn schedule_weekly_schedule_empty_by_default() {
    let sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    let val = sched
        .read_property(PropertyIdentifier::WEEKLY_SCHEDULE, None)
        .unwrap();
    if let PropertyValue::List(days) = val {
        assert_eq!(days.len(), 7);
        for day in &days {
            assert_eq!(*day, PropertyValue::List(vec![]));
        }
    } else {
        panic!("expected PropertyValue::List");
    }
}

#[test]
fn schedule_weekly_schedule_set_monday_read_no_index() {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    let entries = vec![
        make_tv(8, 0, vec![0x01]),  // 08:00
        make_tv(17, 0, vec![0x00]), // 17:00
    ];
    sched.set_weekly_schedule(0, entries.clone()); // Monday

    let val = sched
        .read_property(PropertyIdentifier::WEEKLY_SCHEDULE, None)
        .unwrap();
    if let PropertyValue::List(days) = val {
        assert_eq!(days.len(), 7);
        // Monday (index 0) should have 2 entries
        if let PropertyValue::List(monday_entries) = &days[0] {
            assert_eq!(monday_entries.len(), 2);
            // First entry: [Time(08:00), OctetString([0x01])]
            if let PropertyValue::List(pair) = &monday_entries[0] {
                assert_eq!(pair[0], PropertyValue::Time(make_time(8, 0)));
                assert_eq!(pair[1], PropertyValue::OctetString(vec![0x01]));
            } else {
                panic!("expected pair list");
            }
        } else {
            panic!("expected Monday list");
        }
        // Remaining days should be empty
        for day in days.iter().skip(1) {
            assert_eq!(*day, PropertyValue::List(vec![]));
        }
    } else {
        panic!("expected PropertyValue::List");
    }
}

#[test]
fn schedule_weekly_schedule_index_0_returns_count() {
    let sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    let val = sched
        .read_property(PropertyIdentifier::WEEKLY_SCHEDULE, Some(0))
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(7));
}

#[test]
fn schedule_weekly_schedule_index_1_returns_monday() {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    let entries = vec![make_tv(9, 30, vec![0xAB])];
    sched.set_weekly_schedule(0, entries); // Monday = day_index 0, array_index 1

    let val = sched
        .read_property(PropertyIdentifier::WEEKLY_SCHEDULE, Some(1))
        .unwrap();
    if let PropertyValue::List(items) = val {
        assert_eq!(items.len(), 1);
        if let PropertyValue::List(pair) = &items[0] {
            assert_eq!(pair[0], PropertyValue::Time(make_time(9, 30)));
            assert_eq!(pair[1], PropertyValue::OctetString(vec![0xAB]));
        } else {
            panic!("expected pair list");
        }
    } else {
        panic!("expected PropertyValue::List");
    }
}

#[test]
fn schedule_weekly_schedule_index_7_returns_sunday() {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    let entries = vec![make_tv(10, 0, vec![0xFF])];
    sched.set_weekly_schedule(6, entries); // Sunday = day_index 6, array_index 7

    let val = sched
        .read_property(PropertyIdentifier::WEEKLY_SCHEDULE, Some(7))
        .unwrap();
    if let PropertyValue::List(items) = val {
        assert_eq!(items.len(), 1);
    } else {
        panic!("expected PropertyValue::List");
    }
}

#[test]
fn schedule_weekly_schedule_invalid_index_8_returns_error() {
    let sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    let result = sched.read_property(PropertyIdentifier::WEEKLY_SCHEDULE, Some(8));
    assert!(result.is_err());
    if let Err(Error::Protocol { code, .. }) = result {
        assert_eq!(code, ErrorCode::INVALID_ARRAY_INDEX.to_raw() as u32);
    } else {
        panic!("expected Protocol error");
    }
}

#[test]
fn schedule_weekly_schedule_out_of_bounds_day_index_ignored() {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    // day_index 7 is out of bounds; should be silently ignored
    sched.set_weekly_schedule(7, vec![make_tv(8, 0, vec![0x01])]);
    // All days should still be empty
    let val = sched
        .read_property(PropertyIdentifier::WEEKLY_SCHEDULE, None)
        .unwrap();
    if let PropertyValue::List(days) = val {
        for day in &days {
            assert_eq!(*day, PropertyValue::List(vec![]));
        }
    } else {
        panic!("expected PropertyValue::List");
    }
}

// --- Schedule effective_period ---

#[test]
fn schedule_effective_period_default_null() {
    let sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    let val = sched
        .read_property(PropertyIdentifier::EFFECTIVE_PERIOD, None)
        .unwrap();
    assert_eq!(val, PropertyValue::Null);
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
    sched.set_effective_period(period.clone());
    let val = sched
        .read_property(PropertyIdentifier::EFFECTIVE_PERIOD, None)
        .unwrap();
    assert_eq!(val, PropertyValue::OctetString(period.encode().to_vec()));
}

// --- Schedule exception_schedule ---

#[test]
fn schedule_exception_schedule_empty_by_default() {
    let sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    let val = sched
        .read_property(PropertyIdentifier::EXCEPTION_SCHEDULE, None)
        .unwrap();
    assert_eq!(val, PropertyValue::List(vec![]));
}

#[test]
fn schedule_exception_schedule_count_via_index_zero() {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    let event = BACnetSpecialEvent {
        period: SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::WeekNDay(BACnetWeekNDay {
            month: BACnetWeekNDay::ANY,
            week_of_month: BACnetWeekNDay::ANY,
            day_of_week: 7,
        })),
        list_of_time_values: vec![make_tv(0, 0, vec![0x00])],
        event_priority: 16,
    };
    sched.add_exception(event);
    let val = sched
        .read_property(PropertyIdentifier::EXCEPTION_SCHEDULE, Some(0))
        .unwrap();
    assert_eq!(val, PropertyValue::Unsigned(1));
}

#[test]
fn schedule_exception_schedule_add_and_read() {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    let event = BACnetSpecialEvent {
        period: SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::WeekNDay(BACnetWeekNDay {
            month: BACnetWeekNDay::ANY,
            week_of_month: BACnetWeekNDay::ANY,
            day_of_week: 7, // Sunday
        })),
        list_of_time_values: vec![make_tv(0, 0, vec![0x00])],
        event_priority: 16,
    };
    sched.add_exception(event);
    let val = sched
        .read_property(PropertyIdentifier::EXCEPTION_SCHEDULE, None)
        .unwrap();
    // Should be a List with one event entry
    if let PropertyValue::List(events) = &val {
        assert_eq!(events.len(), 1);
    } else {
        panic!("expected List, got {val:?}");
    }

    // Add a second exception
    let event2 = BACnetSpecialEvent {
        period: SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::WeekNDay(BACnetWeekNDay {
            month: BACnetWeekNDay::ANY,
            week_of_month: BACnetWeekNDay::ANY,
            day_of_week: 1, // Monday
        })),
        list_of_time_values: vec![],
        event_priority: 14,
    };
    sched.add_exception(event2);
    let val = sched
        .read_property(PropertyIdentifier::EXCEPTION_SCHEDULE, None)
        .unwrap();
    if let PropertyValue::List(events) = &val {
        assert_eq!(events.len(), 2);
    } else {
        panic!("expected List, got {val:?}");
    }

    // array_index 0 returns count
    let count = sched
        .read_property(PropertyIdentifier::EXCEPTION_SCHEDULE, Some(0))
        .unwrap();
    assert_eq!(count, PropertyValue::Unsigned(2));
}

// --- Schedule list_of_object_property_references ---

#[test]
fn schedule_opr_list_empty_by_default() {
    let sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    let val = sched
        .read_property(PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES, None)
        .unwrap();
    assert_eq!(val, PropertyValue::List(vec![]));
}

#[test]
fn schedule_opr_list_add_and_read() {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let r = BACnetObjectPropertyReference::new(oid, PropertyIdentifier::PRESENT_VALUE.to_raw());
    sched.add_object_property_reference(r.clone());

    let val = sched
        .read_property(PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES, None)
        .unwrap();
    if let PropertyValue::List(items) = val {
        assert_eq!(items.len(), 1);
        if let PropertyValue::List(pair) = &items[0] {
            assert_eq!(pair[0], PropertyValue::ObjectIdentifier(oid));
            assert_eq!(
                pair[1],
                PropertyValue::Enumerated(PropertyIdentifier::PRESENT_VALUE.to_raw())
            );
        } else {
            panic!("expected pair list");
        }
    } else {
        panic!("expected PropertyValue::List");
    }
}

#[test]
fn schedule_opr_list_multiple_references() {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    let oid1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let oid2 = ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 5).unwrap();
    sched.add_object_property_reference(BACnetObjectPropertyReference::new(
        oid1,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    ));
    sched.add_object_property_reference(BACnetObjectPropertyReference::new(
        oid2,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    ));

    let val = sched
        .read_property(PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES, None)
        .unwrap();
    if let PropertyValue::List(items) = val {
        assert_eq!(items.len(), 2);
    } else {
        panic!("expected PropertyValue::List");
    }
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

// --- Schedule evaluate() ---

#[test]
fn evaluate_returns_default_when_no_entries() {
    let sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    let value = sched.evaluate(0, 12, 0); // Monday noon
    assert_eq!(value, PropertyValue::Real(72.0));
}

#[test]
fn evaluate_returns_weekly_value() {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    // Monday: 08:00 → occupied, 17:00 → unoccupied
    sched.set_weekly_schedule(
        0,
        vec![make_tv(8, 0, vec![0x01]), make_tv(17, 0, vec![0x00])],
    );

    // Before first entry → default
    assert_eq!(sched.evaluate(0, 7, 59), PropertyValue::Real(72.0));
    // At 08:00 → occupied
    assert_eq!(
        sched.evaluate(0, 8, 0),
        PropertyValue::OctetString(vec![0x01])
    );
    // At 12:00 → still occupied (last entry before current time)
    assert_eq!(
        sched.evaluate(0, 12, 0),
        PropertyValue::OctetString(vec![0x01])
    );
    // At 17:00 → unoccupied
    assert_eq!(
        sched.evaluate(0, 17, 0),
        PropertyValue::OctetString(vec![0x00])
    );
    // At 23:59 → still unoccupied
    assert_eq!(
        sched.evaluate(0, 23, 59),
        PropertyValue::OctetString(vec![0x00])
    );
}

#[test]
fn evaluate_different_day_returns_default() {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    // Only Monday has entries
    sched.set_weekly_schedule(0, vec![make_tv(8, 0, vec![0x01])]);

    // Tuesday should return default
    assert_eq!(sched.evaluate(1, 12, 0), PropertyValue::Real(72.0));
}

#[test]
fn evaluate_exception_overrides_weekly() {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    // Monday: 08:00 → 0x01
    sched.set_weekly_schedule(0, vec![make_tv(8, 0, vec![0x01])]);

    // Exception: all day → 0xFF (higher priority)
    sched.add_exception(BACnetSpecialEvent {
        period: SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::WeekNDay(BACnetWeekNDay {
            month: BACnetWeekNDay::ANY,
            week_of_month: BACnetWeekNDay::ANY,
            day_of_week: BACnetWeekNDay::ANY,
        })),
        list_of_time_values: vec![make_tv(0, 0, vec![0xFF])],
        event_priority: 10,
    });

    // Exception should win over weekly schedule
    assert_eq!(
        sched.evaluate(0, 12, 0),
        PropertyValue::OctetString(vec![0xFF])
    );
}

#[test]
fn evaluate_out_of_service_returns_present_value() {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    sched.set_weekly_schedule(0, vec![make_tv(8, 0, vec![0x01])]);
    sched.set_present_value(PropertyValue::Real(55.0));
    sched.out_of_service = true;

    assert_eq!(sched.evaluate(0, 12, 0), PropertyValue::Real(55.0));
}

#[test]
fn evaluate_exception_priority_lowest_number_wins() {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    // Two exceptions, priority 15 (lower prio) and priority 5 (higher prio)
    sched.add_exception(BACnetSpecialEvent {
        period: SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::WeekNDay(BACnetWeekNDay {
            month: BACnetWeekNDay::ANY,
            week_of_month: BACnetWeekNDay::ANY,
            day_of_week: BACnetWeekNDay::ANY,
        })),
        list_of_time_values: vec![make_tv(0, 0, vec![0xAA])],
        event_priority: 15,
    });
    sched.add_exception(BACnetSpecialEvent {
        period: SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::WeekNDay(BACnetWeekNDay {
            month: BACnetWeekNDay::ANY,
            week_of_month: BACnetWeekNDay::ANY,
            day_of_week: BACnetWeekNDay::ANY,
        })),
        list_of_time_values: vec![make_tv(0, 0, vec![0xBB])],
        event_priority: 5,
    });

    // Priority 5 (lower number = higher priority) should win
    assert_eq!(
        sched.evaluate(0, 12, 0),
        PropertyValue::OctetString(vec![0xBB])
    );
}

// --- Schedule tick_schedule ---

#[test]
fn tick_schedule_returns_none_when_no_refs() {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    sched.set_weekly_schedule(0, vec![make_tv(8, 0, vec![0x01])]);
    // No property references → None
    assert!(sched.tick_schedule(0, 12, 0).is_none());
}

#[test]
fn tick_schedule_returns_none_when_value_unchanged() {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    sched.add_object_property_reference(BACnetObjectPropertyReference::new(
        oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    ));
    // No weekly entries → evaluates to default (Real(72.0)) which matches present_value
    assert!(sched.tick_schedule(0, 12, 0).is_none());
}

#[test]
fn tick_schedule_returns_value_and_refs_on_change() {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    let target_oid = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 5).unwrap();
    sched.add_object_property_reference(BACnetObjectPropertyReference::new(
        target_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    ));
    sched.set_weekly_schedule(0, vec![make_tv(8, 0, vec![0x01])]);

    let result = sched.tick_schedule(0, 12, 0);
    assert!(result.is_some());
    let (value, refs) = result.unwrap();
    assert_eq!(value, PropertyValue::OctetString(vec![0x01]));
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].0, target_oid);
    assert_eq!(refs[0].1, PropertyIdentifier::PRESENT_VALUE.to_raw());
}

#[test]
fn tick_schedule_updates_present_value() {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    sched.add_object_property_reference(BACnetObjectPropertyReference::new(
        oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    ));
    sched.set_weekly_schedule(0, vec![make_tv(8, 0, vec![0x01])]);

    let _ = sched.tick_schedule(0, 12, 0);
    assert_eq!(
        *sched.present_value(),
        PropertyValue::OctetString(vec![0x01])
    );

    // Second call with same time → no change
    assert!(sched.tick_schedule(0, 12, 0).is_none());
}

#[test]
fn tick_schedule_returns_none_when_out_of_service() {
    let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    sched.add_object_property_reference(BACnetObjectPropertyReference::new(
        oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    ));
    sched.set_weekly_schedule(0, vec![make_tv(8, 0, vec![0x01])]);
    sched.out_of_service = true;

    assert!(sched.tick_schedule(0, 12, 0).is_none());
}
