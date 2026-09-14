use super::*;
use bacnet_objects::{schedule::CalendarObject, traits::BACnetObject};
use bacnet_types::constructed::{BACnetCalendarEntry, BACnetDateRange, BACnetWeekNDay};
use bacnet_types::primitives::Date;
use PropertyIdentifier as P;

fn calendar_object(configured: bool) -> CalendarObject {
    let mut object = CalendarObject::new(7, "CAL-7").unwrap();
    if configured {
        object.set_description("long calendar label".repeat(100));
        object.set_present_value(true);
        let date = Date {
            year: 126,
            month: 9,
            day: 14,
            day_of_week: 1,
        };
        for _ in 0..32 {
            object.add_date_entry(BACnetCalendarEntry::Date(date));
            object.add_date_entry(BACnetCalendarEntry::DateRange(BACnetDateRange {
                start_date: date,
                end_date: date,
            }));
            object.add_date_entry(BACnetCalendarEntry::WeekNDay(BACnetWeekNDay {
                month: 255,
                week_of_month: 255,
                day_of_week: 1,
            }));
        }
    }
    object
}

#[test]
fn rpm_calendar_metadata_selectors_preserve_bytes_and_budgets() {
    let all = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::DESCRIPTION,
        P::OBJECT_TYPE,
        P::PRESENT_VALUE,
        P::DATE_LIST,
        P::STATUS_FLAGS,
        P::EVENT_STATE,
        P::OUT_OF_SERVICE,
    ];
    let required = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::PRESENT_VALUE,
        P::DATE_LIST,
    ];
    let optional = [
        P::DESCRIPTION,
        P::STATUS_FLAGS,
        P::EVENT_STATE,
        P::OUT_OF_SERVICE,
    ];
    for configured in [false, true] {
        let object = calendar_object(configured);
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

#[test]
fn rpm_calendar_metadata_does_not_enable_create_object() {
    use bacnet_services::object_mgmt::{CreateObjectRequest, ObjectSpecifier};

    let oid = ObjectIdentifier::new(ObjectType::CALENDAR, 7).unwrap();
    for object_specifier in [
        ObjectSpecifier::Type(ObjectType::CALENDAR),
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

// Keep the Calendar PICS fixture with its handler contract while the shared
// PICS test file is capped; the pics:: filter still selects this test.
mod pics {
    use super::*;
    use crate::pics::{generate_pics, PicsConfig};
    use crate::server::ServerConfig;

    #[test]
    fn calendar_property_metadata_rows_and_createability_are_exact() {
        let expected = [
            (P::OBJECT_IDENTIFIER, false, false),
            (P::OBJECT_NAME, false, false),
            (P::DESCRIPTION, true, true),
            (P::OBJECT_TYPE, false, false),
            (P::PRESENT_VALUE, false, false),
            (P::DATE_LIST, false, false),
            (P::STATUS_FLAGS, true, false),
            (P::EVENT_STATE, true, false),
            (P::OUT_OF_SERVICE, true, false),
            (P::PROPERTY_LIST, false, false),
        ];
        for configured in [false, true] {
            let mut db = ObjectDatabase::new();
            db.add(Box::new(calendar_object(configured))).unwrap();
            let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
            assert_eq!(pics.supported_object_types.len(), 1);
            let support = &pics.supported_object_types[0];
            assert_eq!(support.object_type, ObjectType::CALENDAR);
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
            assert_eq!(rows, expected);
        }
    }
}
