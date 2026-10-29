use super::*;
use PropertyIdentifier as P;
use bacnet_objects::{schedule::CalendarObject, traits::BACnetObject};
use bacnet_types::constructed::{BACnetCalendarEntry, BACnetDateRange, BACnetWeekNDay};
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::Date;

fn calendar_object(configured: bool) -> CalendarObject {
    let mut object = CalendarObject::new(7, "CAL-7").unwrap();
    if configured {
        object.set_description("long calendar label".repeat(100));
        let date = Date {
            year: 126,
            month: 9,
            day: 14,
            day_of_week: 1,
        };
        for _ in 0..32 {
            for entry in [
                BACnetCalendarEntry::Date(date),
                BACnetCalendarEntry::DateRange(BACnetDateRange {
                    start_date: date,
                    end_date: date,
                }),
                BACnetCalendarEntry::WeekNDay(BACnetWeekNDay {
                    month: 255,
                    week_of_month: 255,
                    day_of_week: 1,
                }),
            ] {
                object.add_date_entry(entry).unwrap();
            }
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
    ];
    let required = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::PRESENT_VALUE,
        P::DATE_LIST,
    ];
    // Description is the only optional row Calendar serves: Table 12-11 has no
    // Status_Flags, Event_State or Out_Of_Service (#984).
    let optional = [P::DESCRIPTION];
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
fn wp_calendar_properties_outside_table_12_11_are_unknown() {
    // #984: Calendar used to list these as read-only optional rows, so a write
    // failed WRITE_ACCESS_DENIED. Now the object has no such property at all.
    let object = calendar_object(true);
    let oid = object.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(object)).unwrap();
    let snapshot = |db: &ObjectDatabase| {
        let object = db.get(&oid).unwrap();
        [P::PRESENT_VALUE, P::DATE_LIST, P::PROPERTY_LIST]
            .map(|p| object.read_property(p, None).unwrap())
    };
    let before = snapshot(&db);
    for (property, value) in [
        (
            P::STATUS_FLAGS,
            PropertyValue::BitString {
                unused_bits: 4,
                data: vec![0],
            },
        ),
        (P::EVENT_STATE, PropertyValue::Enumerated(0)),
        (P::OUT_OF_SERVICE, PropertyValue::Boolean(true)),
        (P::OUT_OF_SERVICE, PropertyValue::Boolean(false)),
    ] {
        let mut property_value = BytesMut::new();
        encode_property_value(&mut property_value, &value).unwrap();
        let mut request = BytesMut::new();
        WritePropertyRequest {
            object_identifier: oid,
            property_identifier: property,
            property_array_index: None,
            property_value: property_value.to_vec(),
            priority: None,
        }
        .encode(&mut request)
        .unwrap();
        let result = handle_write_property(&mut db, &request);
        assert!(
            matches!(result, Err(Error::Remote(RemoteError::Error { class, code }))
                if class == ErrorClass::PROPERTY
                    && code == ErrorCode::UNKNOWN_PROPERTY),
            "{property:?} write must be UNKNOWN_PROPERTY, got {result:?}"
        );
    }
    assert_eq!(snapshot(&db), before);
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
        assert!(
            matches!(result, Err(Error::Remote(RemoteError::Error { class, code }))
            if class == ErrorClass::OBJECT
                && code == ErrorCode::UNSUPPORTED_OBJECT_TYPE)
        );
        assert!(response.is_empty());
        assert!(db.is_empty());
    }
}

// Keep the Calendar PICS fixture with its handler contract while the shared
// PICS test file is capped; the pics:: filter still selects this test.
mod pics {
    use super::*;
    use crate::pics::{PicsConfig, generate_pics};
    use crate::server::ServerConfig;

    #[test]
    fn calendar_property_metadata_rows_and_createability_are_exact() {
        let mut expected = [
            (P::OBJECT_IDENTIFIER, false, false),
            (P::OBJECT_NAME, false, false),
            (P::DESCRIPTION, true, true),
            (P::OBJECT_TYPE, false, false),
            (P::PRESENT_VALUE, false, false),
            (P::DATE_LIST, false, true),
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
            expected.sort_by_key(|row| row.0.to_raw());
            assert_eq!(rows, expected);
        }
    }
}
