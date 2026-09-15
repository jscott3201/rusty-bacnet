use super::*;
use bacnet_objects::{averaging::AveragingObject, traits::BACnetObject};
use bacnet_types::primitives::PropertyValue;
use PropertyIdentifier as P;

fn averaging_object(configured: bool) -> AveragingObject {
    let mut object = AveragingObject::new(7, "AVG-7").unwrap();
    if configured {
        object
            .write_property(
                P::DESCRIPTION,
                None,
                PropertyValue::CharacterString("long averaging label".repeat(100)),
                None,
            )
            .unwrap();
        object.add_sample(10.0);
        object.add_sample(20.0);
        object.add_sample(30.0);
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        object
            .write_property(
                P::OBJECT_PROPERTY_REFERENCE,
                None,
                PropertyValue::List(vec![
                    PropertyValue::ObjectIdentifier(oid),
                    PropertyValue::Unsigned(P::PRESENT_VALUE.to_raw() as u64),
                ]),
                None,
            )
            .unwrap();
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
    object
}

#[test]
fn rpm_averaging_metadata_selectors_preserve_bytes_and_budgets() {
    let all = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::DESCRIPTION,
        P::OBJECT_TYPE,
        P::PRESENT_VALUE,
        P::MINIMUM_VALUE,
        P::MAXIMUM_VALUE,
        P::AVERAGE_VALUE,
        P::ATTEMPTED_SAMPLES,
        P::VALID_SAMPLES,
        P::OBJECT_PROPERTY_REFERENCE,
        P::STATUS_FLAGS,
        P::OUT_OF_SERVICE,
        P::RELIABILITY,
        P::EVENT_STATE,
    ];
    let required = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::MINIMUM_VALUE,
        P::MAXIMUM_VALUE,
        P::AVERAGE_VALUE,
        P::ATTEMPTED_SAMPLES,
        P::VALID_SAMPLES,
        P::OBJECT_PROPERTY_REFERENCE,
    ];
    let optional = [
        P::DESCRIPTION,
        P::PRESENT_VALUE,
        P::STATUS_FLAGS,
        P::OUT_OF_SERVICE,
        P::RELIABILITY,
        P::EVENT_STATE,
    ];
    for configured in [false, true] {
        let object = averaging_object(configured);
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
fn rpm_averaging_metadata_does_not_enable_create_object() {
    use bacnet_services::object_mgmt::{CreateObjectRequest, ObjectSpecifier};

    let oid = ObjectIdentifier::new(ObjectType::AVERAGING, 7).unwrap();
    for object_specifier in [
        ObjectSpecifier::Type(ObjectType::AVERAGING),
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
fn averaging_delete_object_removes_averaging() {
    use bacnet_services::object_mgmt::DeleteObjectRequest;

    let mut db = ObjectDatabase::new();
    db.add(Box::new(AveragingObject::new(7, "AVG-7").unwrap()))
        .unwrap();
    let oid = ObjectIdentifier::new(ObjectType::AVERAGING, 7).unwrap();
    let mut request = BytesMut::new();
    DeleteObjectRequest {
        object_identifier: oid,
    }
    .encode(&mut request);
    handle_delete_object(&mut db, &request).unwrap();
    assert!(db.get(&oid).is_none());
}
