use super::*;
use bacnet_objects::{timer::TimerObject, traits::BACnetObject};
use bacnet_types::primitives::{Date, PropertyValue, Time};
use PropertyIdentifier as P;

fn timer_object(configured: bool) -> TimerObject {
    let mut object = TimerObject::new(7, "TMR-7").unwrap();
    if configured {
        object
            .write_property(
                P::DESCRIPTION,
                None,
                PropertyValue::CharacterString("long timer label".repeat(100)),
                None,
            )
            .unwrap();
        object.start();
        object.set_initial_timeout(5000);
        let date = Date {
            year: 126,
            month: 9,
            day: 14,
            day_of_week: 1,
        };
        let time = Time {
            hour: 12,
            minute: 30,
            second: 15,
            hundredths: 25,
        };
        object.set_update_time(date, time);
        object.set_expiration_time(date, time);
    }
    // Exercise the unconditional write routes so large encodings persist.
    object
        .write_property(
            P::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(configured),
            None,
        )
        .unwrap();
    if configured {
        object
            .write_property(P::PRESENT_VALUE, None, PropertyValue::Enumerated(1), None)
            .unwrap();
        object
            .write_property(
                P::INITIAL_TIMEOUT,
                None,
                PropertyValue::Unsigned(1_000_000),
                None,
            )
            .unwrap();
    }
    object
}

#[test]
fn rpm_timer_metadata_selectors_preserve_bytes_and_budgets() {
    let all = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::DESCRIPTION,
        P::OBJECT_TYPE,
        P::PRESENT_VALUE,
        P::TIMER_STATE,
        P::TIMER_RUNNING,
        P::INITIAL_TIMEOUT,
        P::UPDATE_TIME,
        P::EXPIRATION_TIME,
        P::STATUS_FLAGS,
        P::OUT_OF_SERVICE,
        P::RELIABILITY,
        P::EVENT_STATE,
    ];
    let required = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::PRESENT_VALUE,
        P::TIMER_STATE,
        P::TIMER_RUNNING,
        P::UPDATE_TIME,
        P::EXPIRATION_TIME,
        P::STATUS_FLAGS,
        P::RELIABILITY,
    ];
    let optional = [
        P::DESCRIPTION,
        P::INITIAL_TIMEOUT,
        P::OUT_OF_SERVICE,
        P::EVENT_STATE,
    ];
    for configured in [false, true] {
        let object = timer_object(configured);
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
fn rpm_timer_metadata_does_not_enable_create_object() {
    use bacnet_services::object_mgmt::{CreateObjectRequest, ObjectSpecifier};

    let oid = ObjectIdentifier::new(ObjectType::TIMER, 7).unwrap();
    for object_specifier in [
        ObjectSpecifier::Type(ObjectType::TIMER),
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

#[test]
fn timer_delete_object_removes_timer() {
    use bacnet_services::object_mgmt::DeleteObjectRequest;

    let mut db = ObjectDatabase::new();
    db.add(Box::new(TimerObject::new(7, "TMR-7").unwrap()))
        .unwrap();
    let oid = ObjectIdentifier::new(ObjectType::TIMER, 7).unwrap();
    let mut request = BytesMut::new();
    DeleteObjectRequest {
        object_identifier: oid,
    }
    .encode(&mut request);
    handle_delete_object(&mut db, &request).unwrap();
    assert!(db.get(&oid).is_none());
}
