use super::*;
use PropertyIdentifier as P;
use bacnet_objects::{averaging::AveragingObject, traits::BACnetObject};
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::PropertyValue;

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
        // The reference and window writes reset the window, so they come
        // before the samples.
        object
            .write_property(
                P::OBJECT_PROPERTY_REFERENCE,
                None,
                // [0] analog-input 1, [1] present-value (#1182).
                PropertyValue::ApplicationData(vec![0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55]),
                None,
            )
            .unwrap();
        object
            .write_property(P::WINDOW_SAMPLES, None, PropertyValue::Unsigned(60), None)
            .unwrap();
        object
            .write_property(
                P::WINDOW_INTERVAL,
                None,
                PropertyValue::Unsigned(3600),
                None,
            )
            .unwrap();
        object.add_sample(10.0).unwrap();
        object.add_sample(20.0).unwrap();
        object.add_sample(30.0).unwrap();
    }
    object
}

#[test]
fn rpm_averaging_metadata_selectors_preserve_bytes_and_budgets() {
    let all = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::DESCRIPTION,
        P::OBJECT_TYPE,
        P::MINIMUM_VALUE,
        P::MAXIMUM_VALUE,
        P::AVERAGE_VALUE,
        P::ATTEMPTED_SAMPLES,
        P::VALID_SAMPLES,
        P::OBJECT_PROPERTY_REFERENCE,
        P::WINDOW_INTERVAL,
        P::WINDOW_SAMPLES,
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
        P::WINDOW_INTERVAL,
        P::WINDOW_SAMPLES,
    ];
    // Table 12-5 has no Present_Value, Status_Flags, Out_Of_Service,
    // Reliability or Event_State (#1064), so Description is the only optional
    // row Averaging serves.
    let optional = [P::DESCRIPTION];
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
        assert!(
            matches!(result, Err(Error::Remote(RemoteError::Error { class, code }))
            if class == ErrorClass::OBJECT
                && code == ErrorCode::UNSUPPORTED_OBJECT_TYPE)
        );
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
