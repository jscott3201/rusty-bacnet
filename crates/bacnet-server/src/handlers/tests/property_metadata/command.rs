use super::*;
use bacnet_objects::{command::CommandObject, traits::BACnetObject};
use bacnet_types::primitives::PropertyValue;
use PropertyIdentifier as P;

fn command_object(configured: bool) -> CommandObject {
    let mut object = CommandObject::new(7, "CMD-7").unwrap();
    if configured {
        object
            .write_property(
                P::DESCRIPTION,
                None,
                PropertyValue::CharacterString("long command label".repeat(100)),
                None,
            )
            .unwrap();
        object
            .write_property(P::PRESENT_VALUE, None, PropertyValue::Unsigned(3), None)
            .unwrap();
        object.set_action(vec![vec![1, 2, 3], vec![4, 5]]);
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
fn rpm_command_metadata_selectors_preserve_bytes_and_budgets() {
    let all = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::DESCRIPTION,
        P::OBJECT_TYPE,
        P::PRESENT_VALUE,
        P::IN_PROCESS,
        P::ALL_WRITES_SUCCESSFUL,
        P::ACTION,
        P::STATUS_FLAGS,
        P::OUT_OF_SERVICE,
        P::RELIABILITY,
    ];
    let required = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::PRESENT_VALUE,
        P::IN_PROCESS,
        P::ALL_WRITES_SUCCESSFUL,
        P::ACTION,
    ];
    let optional = [
        P::DESCRIPTION,
        P::STATUS_FLAGS,
        P::OUT_OF_SERVICE,
        P::RELIABILITY,
    ];
    for configured in [false, true] {
        let object = command_object(configured);
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
fn rpm_command_metadata_does_not_enable_create_object() {
    use bacnet_services::object_mgmt::{CreateObjectRequest, ObjectSpecifier};

    let oid = ObjectIdentifier::new(ObjectType::COMMAND, 7).unwrap();
    for object_specifier in [
        ObjectSpecifier::Type(ObjectType::COMMAND),
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
fn command_delete_object_removes_command() {
    use bacnet_services::object_mgmt::DeleteObjectRequest;

    let mut db = ObjectDatabase::new();
    db.add(Box::new(CommandObject::new(7, "CMD-7").unwrap()))
        .unwrap();
    let oid = ObjectIdentifier::new(ObjectType::COMMAND, 7).unwrap();
    let mut request = BytesMut::new();
    DeleteObjectRequest {
        object_identifier: oid,
    }
    .encode(&mut request);
    handle_delete_object(&mut db, &request).unwrap();
    assert!(db.get(&oid).is_none());
}
