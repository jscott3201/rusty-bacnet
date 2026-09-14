use super::*;
use bacnet_objects::{schedule::ScheduleObject, traits::BACnetObject};
use bacnet_types::constructed::{
    BACnetCalendarEntry, BACnetDateRange, BACnetObjectPropertyReference, BACnetSpecialEvent,
    BACnetTimeValue, SpecialEventPeriod,
};
use bacnet_types::primitives::{Date, PropertyValue, Time};
use PropertyIdentifier as P;

fn schedule_object(configuration: u8, out_of_service: bool) -> ScheduleObject {
    let mut object = ScheduleObject::new(7, "SCH-7", PropertyValue::Unsigned(42)).unwrap();
    if configuration != 0 {
        object.set_description("long schedule label".repeat(100));
        let date = Date {
            year: 126,
            month: 9,
            day: 14,
            day_of_week: 1,
        };
        object.set_effective_period(BACnetDateRange {
            start_date: date,
            end_date: date,
        });
        let entries: Vec<_> = (0..32)
            .map(|minute| BACnetTimeValue {
                time: Time {
                    hour: 8,
                    minute,
                    second: 0,
                    hundredths: 0,
                },
                value: vec![0x21, minute],
            })
            .collect();
        if configuration & 1 != 0 {
            object.set_weekly_schedule(0, entries.clone());
            object.set_weekly_schedule(6, entries.clone());
        }
        if configuration & 2 != 0 {
            object.add_exception(BACnetSpecialEvent {
                period: SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::Date(date)),
                list_of_time_values: entries,
                event_priority: 3,
            });
        }
        object.add_object_property_reference(BACnetObjectPropertyReference::new(
            ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 2).unwrap(),
            P::PRESENT_VALUE.to_raw(),
        ));
    }
    object
        .write_property(
            P::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(out_of_service),
            None,
        )
        .unwrap();
    object
}

#[test]
fn rpm_schedule_metadata_selectors_preserve_bytes_and_budgets() {
    // Independent fixtures preserve the old prefix and include the readable priority.
    let all = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::DESCRIPTION,
        P::OBJECT_TYPE,
        P::PRESENT_VALUE,
        P::SCHEDULE_DEFAULT,
        P::WEEKLY_SCHEDULE,
        P::EXCEPTION_SCHEDULE,
        P::EFFECTIVE_PERIOD,
        P::LIST_OF_OBJECT_PROPERTY_REFERENCES,
        P::STATUS_FLAGS,
        P::EVENT_STATE,
        P::RELIABILITY,
        P::OUT_OF_SERVICE,
        P::PRIORITY_FOR_WRITING,
    ];
    let required = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::PRESENT_VALUE,
        P::SCHEDULE_DEFAULT,
        P::EFFECTIVE_PERIOD,
        P::LIST_OF_OBJECT_PROPERTY_REFERENCES,
        P::STATUS_FLAGS,
        P::RELIABILITY,
        P::OUT_OF_SERVICE,
        P::PRIORITY_FOR_WRITING,
    ];
    let optional = [
        P::DESCRIPTION,
        P::WEEKLY_SCHEDULE,
        P::EXCEPTION_SCHEDULE,
        P::EVENT_STATE,
    ];
    for configuration in 0..4 {
        for out_of_service in [false, true] {
            let object = schedule_object(configuration, out_of_service);
            let oid = object.object_identifier();
            let mut db = ObjectDatabase::new();
            db.add(Box::new(object)).unwrap();
            for (selector, expected) in [
                (P::ALL, all.as_slice()),
                (P::REQUIRED, required.as_slice()),
                (P::OPTIONAL, optional.as_slice()),
                (P::PROPERTY_LIST, &[P::PROPERTY_LIST]),
            ] {
                assert_rpm_selector_bytes(&db, oid, selector, expected);
            }
        }
    }
}

#[test]
fn rpm_schedule_metadata_does_not_enable_create_object() {
    use bacnet_services::object_mgmt::{CreateObjectRequest, ObjectSpecifier};

    let oid = ObjectIdentifier::new(ObjectType::SCHEDULE, 7).unwrap();
    for object_specifier in [
        ObjectSpecifier::Type(ObjectType::SCHEDULE),
        ObjectSpecifier::Identifier(oid),
    ] {
        let mut db = ObjectDatabase::new();
        let mut request = BytesMut::new();
        CreateObjectRequest {
            object_specifier,
            list_of_initial_values: vec![],
        }
        .encode(&mut request);
        let mut response = BytesMut::new();
        let result = handle_create_object(&mut db, &request, &mut response);
        assert!(matches!(result, Err(Error::Protocol { class, code })
            if class == ErrorClass::OBJECT.to_raw() as u32
                && code == ErrorCode::UNSUPPORTED_OBJECT_TYPE.to_raw() as u32));
        assert!(response.is_empty());
        assert!(db.is_empty());
    }
}
