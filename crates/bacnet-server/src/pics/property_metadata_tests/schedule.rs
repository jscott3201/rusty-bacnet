use super::*;
use bacnet_objects::{schedule::ScheduleObject, traits::BACnetObject};
use bacnet_types::constructed::{
    BACnetCalendarEntry, BACnetSpecialEvent, BACnetTimeValue, SpecialEventPeriod,
};
use bacnet_types::primitives::{Date, PropertyValue, Time};
use PropertyIdentifier as P;

#[test]
fn pics_schedule_property_metadata_is_exact() {
    // Independent (identifier, optional, writable) rows in projection order.
    let expected = [
        (P::OBJECT_IDENTIFIER, false, false),
        (P::OBJECT_NAME, false, false),
        (P::DESCRIPTION, true, true),
        (P::OBJECT_TYPE, false, false),
        (P::PRESENT_VALUE, false, false),
        (P::SCHEDULE_DEFAULT, false, true),
        (P::WEEKLY_SCHEDULE, true, false),
        (P::EXCEPTION_SCHEDULE, true, false),
        (P::EFFECTIVE_PERIOD, false, false),
        (P::LIST_OF_OBJECT_PROPERTY_REFERENCES, false, false),
        (P::STATUS_FLAGS, false, false),
        (P::EVENT_STATE, true, false),
        (P::RELIABILITY, false, true),
        (P::OUT_OF_SERVICE, false, true),
        (P::PRIORITY_FOR_WRITING, false, false),
        (P::PROPERTY_LIST, false, false),
    ];
    // Empty, weekly-only, exception-only, and combined configurations expose
    // the same rows. Write capability also remains independent of current OOS.
    for configuration in 0..4 {
        for out_of_service in [false, true] {
            let mut object = ScheduleObject::new(7, "SCH-7", PropertyValue::Unsigned(42)).unwrap();
            let entries = vec![BACnetTimeValue {
                time: Time {
                    hour: 8,
                    minute: 30,
                    second: 0,
                    hundredths: 0,
                },
                value: vec![0x21, 42],
            }];
            if configuration & 1 != 0 {
                object.set_weekly_schedule(0, entries.clone());
            }
            if configuration & 2 != 0 {
                object.add_exception(BACnetSpecialEvent {
                    period: SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::Date(Date {
                        year: 126,
                        month: 9,
                        day: 14,
                        day_of_week: 1,
                    })),
                    list_of_time_values: entries,
                    event_priority: 3,
                });
            }
            object
                .write_property(
                    P::OUT_OF_SERVICE,
                    None,
                    PropertyValue::Boolean(out_of_service),
                    None,
                )
                .unwrap();
            let required = object.required_properties();
            let mut db = ObjectDatabase::new();
            db.add(Box::new(object)).unwrap();
            let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
            assert_eq!(pics.supported_object_types.len(), 1);
            let support = &pics.supported_object_types[0];
            assert_eq!(support.object_type, ObjectType::SCHEDULE);
            assert!(!support.createable);
            assert!(support.deleteable);
            let rows: Vec<_> = support
                .supported_properties
                .iter()
                .map(|row| {
                    assert!(row.access.readable);
                    (row.property_id, row.access.optional, row.access.writable)
                })
                .collect();
            assert_eq!(
                rows, expected,
                "configuration={configuration}, OOS={out_of_service}"
            );
            assert_eq!(
                rows.iter()
                    .filter_map(|&(p, optional, _)| (!optional).then_some(p))
                    .collect::<Vec<_>>(),
                required.as_ref()
            );
        }
    }
}
