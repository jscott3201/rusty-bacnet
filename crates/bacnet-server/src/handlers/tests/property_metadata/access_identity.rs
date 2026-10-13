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
        // Credential_Status is derived (#1073); Credential_Disable drives it.
        credential
            .write_property(
                P::CREDENTIAL_DISABLE,
                None,
                PropertyValue::Enumerated(1),
                None,
            )
            .unwrap();
        user.write_property(P::USER_TYPE, None, PropertyValue::Enumerated(2), None)
            .unwrap();
        user.write_property(
            P::GLOBAL_IDENTIFIER,
            None,
            PropertyValue::Unsigned(78),
            None,
        )
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
        // Only Credential Data Input has Out_Of_Service (Table 12-43).
        if object.object_identifier().object_type() == ObjectType::CREDENTIAL_DATA_INPUT {
            object
                .write_property(
                    P::OUT_OF_SERVICE,
                    None,
                    PropertyValue::Boolean(configured),
                    None,
                )
                .unwrap();
        }
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
    // and read-only (the heuristic called it writable); User_Type and the
    // rights, credential and user Global_Identifier are required and
    // writable (the heuristic called only
    // Description/Out_Of_Service/Present_Value writable).
    let middle: &[P] = match kind {
        // Table 12-40 has no Present_Value row (#979).
        ObjectType::ACCESS_CREDENTIAL => &[
            P::CREDENTIAL_STATUS,
            P::ASSIGNED_ACCESS_RIGHTS,
            P::AUTHENTICATION_FACTORS,
        ],
        // Table 12-38 has no Present_Value or Assigned_Access_Rights (#1064).
        ObjectType::ACCESS_USER => &[P::USER_TYPE, P::CREDENTIALS],
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
    // Of the four tables, only Table 12-43 has Out_Of_Service (#1064).
    if kind == ObjectType::CREDENTIAL_DATA_INPUT {
        all.extend_from_slice(&[P::STATUS_FLAGS, P::OUT_OF_SERVICE, P::RELIABILITY]);
    } else {
        all.extend_from_slice(&[P::STATUS_FLAGS, P::RELIABILITY]);
    }
    // The Table 12-40 required rows #1073 added follow the status rows.
    if kind == ObjectType::ACCESS_CREDENTIAL {
        all.extend_from_slice(&[
            P::GLOBAL_IDENTIFIER,
            P::REASON_FOR_DISABLE,
            P::ACTIVATION_TIME,
            P::EXPIRATION_TIME,
            P::CREDENTIAL_DISABLE,
        ]);
    }
    // So does Table 12-39's Enable row, property 133 (#1332).
    if kind == ObjectType::ACCESS_RIGHTS {
        all.push(P::LOG_ENABLE);
    }
    // And Table 12-38's optional Members and Member_Of (#1394), then its
    // Global_Identifier (#1463).
    if kind == ObjectType::ACCESS_USER {
        all.extend_from_slice(&[P::MEMBERS, P::MEMBER_OF, P::GLOBAL_IDENTIFIER]);
    }
    let optional: &[P] = match kind {
        ObjectType::ACCESS_CREDENTIAL => &[P::DESCRIPTION],
        ObjectType::ACCESS_USER => &[P::DESCRIPTION, P::MEMBERS, P::MEMBER_OF],
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
