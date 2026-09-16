use super::*;
use bacnet_objects::{
    access_control::{
        AccessCredentialObject, AccessRightsObject, AccessUserObject, CredentialDataInputObject,
    },
    traits::BACnetObject,
};
use bacnet_types::primitives::PropertyValue;
use PropertyIdentifier as P;

fn access_objects(configured: bool) -> [Box<dyn BACnetObject>; 4] {
    let mut credential = AccessCredentialObject::new(7, "CRED-7").unwrap();
    let mut user = AccessUserObject::new(7, "USER-7").unwrap();
    let mut rights = AccessRightsObject::new(7, "AR-7").unwrap();
    let cdi = CredentialDataInputObject::new(7, "CDI-7").unwrap();
    if configured {
        credential
            .write_property(P::PRESENT_VALUE, None, PropertyValue::Enumerated(1), None)
            .unwrap();
        credential
            .write_property(
                P::CREDENTIAL_STATUS,
                None,
                PropertyValue::Enumerated(2),
                None,
            )
            .unwrap();
        user.write_property(P::PRESENT_VALUE, None, PropertyValue::Enumerated(1), None)
            .unwrap();
        user.write_property(P::USER_TYPE, None, PropertyValue::Enumerated(2), None)
            .unwrap();
        rights
            .write_property(
                P::GLOBAL_IDENTIFIER,
                None,
                PropertyValue::Unsigned(77),
                None,
            )
            .unwrap();
        // CDI has no writable domain row: dispatch denies every
        // non-Description/Out_Of_Service write even while out of service.
    }
    let mut objects: [Box<dyn BACnetObject>; 4] = [
        Box::new(credential),
        Box::new(user),
        Box::new(rights),
        Box::new(cdi),
    ];
    for object in &mut objects {
        object
            .write_property(
                P::DESCRIPTION,
                None,
                PropertyValue::CharacterString("long access label".repeat(100)),
                None,
            )
            .unwrap();
        // Exercise the unconditional write route so large encodings persist.
        object
            .write_property(
                P::OUT_OF_SERVICE,
                None,
                PropertyValue::Boolean(configured),
                None,
            )
            .unwrap();
    }
    objects
}

fn expected_lists(kind: ObjectType) -> (Vec<P>, Vec<P>, Vec<P>) {
    let mut all = vec![
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::DESCRIPTION,
        P::OBJECT_TYPE,
    ];
    // PICS corrections vs the historical heuristic: Object_Name is required
    // and read-only (the heuristic called it writable); CDI Present_Value is
    // required and read-only (the heuristic called every Present_Value
    // writable); Credential_Status, User_Type, and the rights
    // Global_Identifier are required and writable (the heuristic called only
    // Description/Out_Of_Service/Present_Value writable).
    let middle: &[P] = match kind {
        ObjectType::ACCESS_CREDENTIAL => &[
            P::PRESENT_VALUE,
            P::CREDENTIAL_STATUS,
            P::ASSIGNED_ACCESS_RIGHTS,
            P::AUTHENTICATION_FACTORS,
        ],
        ObjectType::ACCESS_USER => &[
            P::PRESENT_VALUE,
            P::USER_TYPE,
            P::CREDENTIALS,
            P::ASSIGNED_ACCESS_RIGHTS,
        ],
        ObjectType::ACCESS_RIGHTS => &[
            P::GLOBAL_IDENTIFIER,
            P::POSITIVE_ACCESS_RULES,
            P::NEGATIVE_ACCESS_RULES,
        ],
        _ => &[
            P::PRESENT_VALUE,
            P::UPDATE_TIME,
            P::SUPPORTED_FORMATS,
            P::SUPPORTED_FORMAT_CLASSES,
        ],
    };
    all.extend_from_slice(middle);
    all.extend_from_slice(&[P::STATUS_FLAGS, P::OUT_OF_SERVICE, P::RELIABILITY]);
    let optional: &[P] = match kind {
        ObjectType::ACCESS_CREDENTIAL => &[P::DESCRIPTION, P::PRESENT_VALUE],
        ObjectType::ACCESS_USER => &[P::DESCRIPTION, P::PRESENT_VALUE, P::ASSIGNED_ACCESS_RIGHTS],
        ObjectType::ACCESS_RIGHTS => &[P::DESCRIPTION],
        _ => &[P::DESCRIPTION, P::SUPPORTED_FORMAT_CLASSES],
    };
    let required: Vec<_> = all
        .iter()
        .copied()
        .filter(|p| !optional.contains(p))
        .collect();
    (all, required, optional.to_vec())
}

#[test]
fn rpm_access_identity_metadata_selectors_preserve_bytes_and_budgets() {
    for configured in [false, true] {
        for object in access_objects(configured) {
            let oid = object.object_identifier();
            let (all, required, optional) = expected_lists(oid.object_type());
            let mut db = ObjectDatabase::new();
            db.add(object).unwrap();
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
fn rpm_access_identity_metadata_does_not_enable_create_object() {
    use bacnet_services::object_mgmt::{CreateObjectRequest, ObjectSpecifier};

    let cases = [
        (
            ObjectType::ACCESS_CREDENTIAL,
            ObjectIdentifier::new(ObjectType::ACCESS_CREDENTIAL, 7).unwrap(),
        ),
        (
            ObjectType::ACCESS_USER,
            ObjectIdentifier::new(ObjectType::ACCESS_USER, 7).unwrap(),
        ),
        (
            ObjectType::ACCESS_RIGHTS,
            ObjectIdentifier::new(ObjectType::ACCESS_RIGHTS, 7).unwrap(),
        ),
        (
            ObjectType::CREDENTIAL_DATA_INPUT,
            ObjectIdentifier::new(ObjectType::CREDENTIAL_DATA_INPUT, 7).unwrap(),
        ),
    ];
    for (kind, oid) in cases {
        for object_specifier in [
            ObjectSpecifier::Type(kind),
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
}

#[test]
fn access_identity_delete_object_removes_each_quartet_member() {
    use bacnet_services::object_mgmt::DeleteObjectRequest;

    for object in access_objects(false) {
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(object).unwrap();
        let mut request = BytesMut::new();
        DeleteObjectRequest {
            object_identifier: oid,
        }
        .encode(&mut request);
        handle_delete_object(&mut db, &request).unwrap();
        assert!(db.get(&oid).is_none());
    }
}
