use super::*;
use bacnet_objects::{load_control::LoadControlObject, traits::BACnetObject};
use bacnet_types::primitives::PropertyValue;
use PropertyIdentifier as P;

fn load_control_object(configured: bool) -> LoadControlObject {
    let mut object = LoadControlObject::new(7, "LC-7").unwrap();
    if configured {
        object
            .write_property(
                P::DESCRIPTION,
                None,
                PropertyValue::CharacterString("long load control label".repeat(100)),
                None,
            )
            .unwrap();
        object
            .write_property(
                P::REQUESTED_SHED_LEVEL,
                None,
                PropertyValue::ApplicationData(vec![0x09, 50]),
                None,
            )
            .unwrap();
        object
            .write_property(P::SHED_DURATION, None, PropertyValue::Unsigned(3600), None)
            .unwrap();
    }
    object
}

#[test]
fn rpm_load_control_metadata_selectors_preserve_bytes_and_budgets() {
    let all = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::DESCRIPTION,
        P::OBJECT_TYPE,
        P::PRESENT_VALUE,
        P::REQUESTED_SHED_LEVEL,
        P::EXPECTED_SHED_LEVEL,
        P::ACTUAL_SHED_LEVEL,
        P::SHED_DURATION,
        P::START_TIME,
        P::STATUS_FLAGS,
        P::RELIABILITY,
        P::EVENT_STATE,
    ];
    let required = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::PRESENT_VALUE,
        P::REQUESTED_SHED_LEVEL,
        P::EXPECTED_SHED_LEVEL,
        P::ACTUAL_SHED_LEVEL,
        P::SHED_DURATION,
        P::START_TIME,
        P::EVENT_STATE,
    ];
    // Table 12-32 has no Out_Of_Service (#1064).
    let optional = [P::DESCRIPTION, P::STATUS_FLAGS, P::RELIABILITY];
    for configured in [false, true] {
        let object = load_control_object(configured);
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
fn rpm_load_control_metadata_does_not_enable_create_object() {
    use bacnet_services::object_mgmt::{CreateObjectRequest, ObjectSpecifier};

    let oid = ObjectIdentifier::new(ObjectType::LOAD_CONTROL, 7).unwrap();
    for object_specifier in [
        ObjectSpecifier::Type(ObjectType::LOAD_CONTROL),
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
            if class == ErrorClass::OBJECT
                && code == ErrorCode::UNSUPPORTED_OBJECT_TYPE));
        assert!(response.is_empty());
        assert!(db.is_empty());
    }
}

#[test]
fn load_control_delete_object_removes_load_control() {
    use bacnet_services::object_mgmt::DeleteObjectRequest;

    let mut db = ObjectDatabase::new();
    db.add(Box::new(LoadControlObject::new(7, "LC-7").unwrap()))
        .unwrap();
    let oid = ObjectIdentifier::new(ObjectType::LOAD_CONTROL, 7).unwrap();
    let mut request = BytesMut::new();
    DeleteObjectRequest {
        object_identifier: oid,
    }
    .encode(&mut request);
    handle_delete_object(&mut db, &request).unwrap();
    assert!(db.get(&oid).is_none());
}
