use super::*;
use bacnet_objects::access_control::{
    AccessCredentialObject, AccessRightsObject, AccessUserObject, CredentialDataInputObject,
};
use bacnet_types::primitives::PropertyValue;
use PropertyIdentifier as P;

fn expected_rows(kind: ObjectType) -> Vec<PropertyRow> {
    // Independent (identifier, optional, writable) rows in declaration order; PICS sorts by property ID.
    // PICS corrections vs the historical heuristic: Object_Name is required
    // and read-only; User_Type and the rights, credential and user
    // Global_Identifier are required and writable.
    match kind {
        ObjectType::ACCESS_CREDENTIAL => vec![
            (P::OBJECT_IDENTIFIER, false, false),
            (P::OBJECT_NAME, false, false),
            (P::DESCRIPTION, true, true),
            (P::OBJECT_TYPE, false, false),
            // Table 12-40 has no Present_Value row (#979). Credential_Status
            // is derived from Reason_For_Disable, so read-only (#1073).
            (P::CREDENTIAL_STATUS, false, false),
            (P::ASSIGNED_ACCESS_RIGHTS, false, false),
            (P::AUTHENTICATION_FACTORS, false, false),
            // Nor an Out_Of_Service row (#1064).
            (P::STATUS_FLAGS, false, false),
            (P::RELIABILITY, false, false),
            (P::GLOBAL_IDENTIFIER, false, true),
            (P::REASON_FOR_DISABLE, false, false),
            (P::ACTIVATION_TIME, false, true),
            (P::EXPIRATION_TIME, false, true),
            (P::CREDENTIAL_DISABLE, false, true),
            (P::PROPERTY_LIST, false, false),
        ],
        ObjectType::ACCESS_USER => vec![
            (P::OBJECT_IDENTIFIER, false, false),
            (P::OBJECT_NAME, false, false),
            (P::DESCRIPTION, true, true),
            (P::OBJECT_TYPE, false, false),
            // Table 12-38 has no Present_Value, Assigned_Access_Rights or
            // Out_Of_Service row (#1064).
            (P::USER_TYPE, false, true),
            (P::CREDENTIALS, false, false),
            (P::STATUS_FLAGS, false, false),
            (P::RELIABILITY, false, false),
            // Optional and read-only over the network (#1394).
            (P::MEMBERS, true, false),
            (P::MEMBER_OF, true, false),
            // The Table 12-38 W row (#1463).
            (P::GLOBAL_IDENTIFIER, false, true),
            (P::PROPERTY_LIST, false, false),
        ],
        ObjectType::ACCESS_RIGHTS => vec![
            (P::OBJECT_IDENTIFIER, false, false),
            (P::OBJECT_NAME, false, false),
            (P::DESCRIPTION, true, true),
            (P::OBJECT_TYPE, false, false),
            (P::GLOBAL_IDENTIFIER, false, true),
            // R rows the network may write (#1330).
            (P::POSITIVE_ACCESS_RULES, false, true),
            (P::NEGATIVE_ACCESS_RULES, false, true),
            // Table 12-39 has no Out_Of_Service row (#1064).
            (P::STATUS_FLAGS, false, false),
            (P::RELIABILITY, false, false),
            // Enable, property 133: required, and writable (#1332).
            (P::LOG_ENABLE, false, true),
            (P::PROPERTY_LIST, false, false),
        ],
        _ => vec![
            (P::OBJECT_IDENTIFIER, false, false),
            (P::OBJECT_NAME, false, false),
            (P::DESCRIPTION, true, true),
            (P::OBJECT_TYPE, false, false),
            // Writable while Out_Of_Service is TRUE (Table 12-43 footnote 1).
            (P::PRESENT_VALUE, false, true),
            (P::UPDATE_TIME, false, false),
            (P::SUPPORTED_FORMATS, false, false),
            (P::SUPPORTED_FORMAT_CLASSES, true, false),
            (P::STATUS_FLAGS, false, false),
            (P::OUT_OF_SERVICE, false, true),
            (P::RELIABILITY, false, true),
            (P::PROPERTY_LIST, false, false),
        ],
    }
}

#[test]
fn pics_access_identity_property_metadata_is_exact() {
    let fresh: [FreshObject; 4] = [
        || {
            (
                Box::new(AccessCredentialObject::new(7, "CRED-7").unwrap()),
                ObjectType::ACCESS_CREDENTIAL,
            )
        },
        || {
            (
                Box::new(AccessUserObject::new(7, "USER-7").unwrap()),
                ObjectType::ACCESS_USER,
            )
        },
        || {
            (
                Box::new(AccessRightsObject::new(7, "AR-7").unwrap()),
                ObjectType::ACCESS_RIGHTS,
            )
        },
        || {
            (
                Box::new(CredentialDataInputObject::new(7, "CDI-7").unwrap()),
                ObjectType::CREDENTIAL_DATA_INPUT,
            )
        },
    ];
    for make in fresh {
        let expected = expected_rows(make().1);
        for configured in [false, true] {
            for out_of_service in [false, true] {
                let (mut object, kind) = make();
                if configured {
                    object
                        .write_property(
                            P::DESCRIPTION,
                            None,
                            PropertyValue::CharacterString("long access label".repeat(100)),
                            None,
                        )
                        .unwrap();
                }
                if kind == ObjectType::CREDENTIAL_DATA_INPUT {
                    object
                        .write_property(
                            P::OUT_OF_SERVICE,
                            None,
                            PropertyValue::Boolean(out_of_service),
                            None,
                        )
                        .unwrap();
                }
                let required = object.required_properties();
                let mut db = ObjectDatabase::new();
                db.add(object).unwrap();
                let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
                assert_eq!(pics.supported_object_types.len(), 1);
                let support = &pics.supported_object_types[0];
                assert_eq!(support.object_type, kind);
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
                assert_eq!(
                    rows,
                    sorted_rows(&expected),
                    "{kind:?}, configured={configured}, OOS={out_of_service}"
                );
                assert_eq!(
                    rows.iter()
                        .filter_map(|&(p, optional, _)| (!optional).then_some(p))
                        .collect::<Vec<_>>(),
                    sorted_required(required.as_ref())
                );
            }
        }
    }
}

/// Accompaniment is a per-instance Table 12-39 O row (#1393): the PICS lists
/// it, optional and writable, once any Access Rights object serves it.
#[test]
fn pics_access_rights_lists_accompaniment_once_an_object_serves_it() {
    let row = |pics: &Pics| {
        pics.supported_object_types[0]
            .supported_properties
            .iter()
            .find(|row| row.property_id == P::ACCOMPANIMENT)
            .map(|row| {
                (
                    row.access.readable,
                    row.access.optional,
                    row.access.writable,
                )
            })
    };
    let mut db = ObjectDatabase::new();
    db.add(Box::new(AccessRightsObject::new(1, "AR-1").unwrap()))
        .unwrap();
    let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
    assert_eq!(row(&pics), None);

    let mut rights = AccessRightsObject::new(2, "AR-2").unwrap();
    let credential = ObjectIdentifier::new(ObjectType::ACCESS_CREDENTIAL, 5).unwrap();
    rights.set_accompaniment(Some(credential.into())).unwrap();
    db.add(Box::new(rights)).unwrap();
    let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
    assert_eq!(pics.supported_object_types.len(), 1);
    assert_eq!(row(&pics), Some((true, true, true)));
}

/// Authorization_Exemptions is a per-instance Table 12-40 O row (#1331): the
/// PICS lists it, optional and read-only, once any Access Credential serves
/// it.
#[test]
fn pics_access_credential_lists_authorization_exemptions_once_an_object_serves_it() {
    let row = |pics: &Pics| {
        pics.supported_object_types[0]
            .supported_properties
            .iter()
            .find(|row| row.property_id == P::AUTHORIZATION_EXEMPTIONS)
            .map(|row| {
                (
                    row.access.readable,
                    row.access.optional,
                    row.access.writable,
                )
            })
    };
    let mut db = ObjectDatabase::new();
    db.add(Box::new(AccessCredentialObject::new(1, "CRED-1").unwrap()))
        .unwrap();
    let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
    assert_eq!(row(&pics), None);

    let mut credential = AccessCredentialObject::new(2, "CRED-2").unwrap();
    credential
        .set_authorization_exemptions(Some(vec![
            bacnet_types::enums::AuthorizationExemption::ACCESS_RIGHTS,
        ]))
        .unwrap();
    db.add(Box::new(credential)).unwrap();
    let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
    assert_eq!(pics.supported_object_types.len(), 1);
    assert_eq!(row(&pics), Some((true, true, false)));
}
