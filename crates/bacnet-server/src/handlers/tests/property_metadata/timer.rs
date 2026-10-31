use super::*;
use PropertyIdentifier as P;
use bacnet_objects::{timer::TimerObject, traits::BACnetObject};
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::PropertyValue;

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
        object.set_pres_value_range(1_000, 4_000_000_000).unwrap();
        object.set_default_timeout(3_000_000_000).unwrap();
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
            .write_property(P::TIMER_RUNNING, None, PropertyValue::Boolean(true), None)
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
        P::STATUS_FLAGS,
        P::EVENT_STATE,
        P::RELIABILITY,
        P::OUT_OF_SERVICE,
        P::TIMER_STATE,
        P::TIMER_RUNNING,
        P::UPDATE_TIME,
        P::LAST_STATE_CHANGE,
        P::EXPIRATION_TIME,
        P::INITIAL_TIMEOUT,
        P::DEFAULT_TIMEOUT,
        P::MIN_PRES_VALUE,
        P::MAX_PRES_VALUE,
    ];
    // Table 12-75's R rows, and the range its footnote 4 requires of a
    // timer whose Present_Value is writable in service (#2192).
    let required = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::PRESENT_VALUE,
        P::STATUS_FLAGS,
        P::TIMER_STATE,
        P::TIMER_RUNNING,
        P::MIN_PRES_VALUE,
        P::MAX_PRES_VALUE,
    ];
    let optional = [
        P::DESCRIPTION,
        P::EVENT_STATE,
        P::RELIABILITY,
        P::OUT_OF_SERVICE,
        P::UPDATE_TIME,
        P::LAST_STATE_CHANGE,
        P::EXPIRATION_TIME,
        P::INITIAL_TIMEOUT,
        P::DEFAULT_TIMEOUT,
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
        assert!(
            matches!(result, Err(Error::Remote(RemoteError::Error { class, code }))
            if class == ErrorClass::OBJECT
                && code == ErrorCode::DYNAMIC_CREATION_NOT_SUPPORTED)
        );
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
