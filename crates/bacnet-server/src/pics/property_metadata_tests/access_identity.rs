use super::*;
use bacnet_objects::{
    access_control::{
        AccessCredentialObject, AccessRightsObject, AccessUserObject, CredentialDataInputObject,
    },
    traits::BACnetObject,
};
use bacnet_types::primitives::PropertyValue;
use PropertyIdentifier as P;

fn expected_rows(kind: ObjectType) -> Vec<(P, bool, bool)> {
    // Independent (identifier, optional, writable) rows in projection order.
    // PICS corrections vs the historical heuristic: Object_Name is required
    // and read-only; CDI Present_Value is required and read-only;
    // Credential_Status, User_Type, and the rights Global_Identifier are
    // required and writable.
    match kind {
        ObjectType::ACCESS_CREDENTIAL => vec![
            (P::OBJECT_IDENTIFIER, false, false),
            (P::OBJECT_NAME, false, false),
            (P::DESCRIPTION, true, true),
            (P::OBJECT_TYPE, false, false),
            (P::PRESENT_VALUE, true, true),
            (P::CREDENTIAL_STATUS, false, true),
            (P::ASSIGNED_ACCESS_RIGHTS, false, false),
            (P::AUTHENTICATION_FACTORS, false, false),
            (P::STATUS_FLAGS, false, false),
            (P::OUT_OF_SERVICE, false, true),
            (P::RELIABILITY, false, false),
            (P::PROPERTY_LIST, false, false),
        ],
        ObjectType::ACCESS_USER => vec![
            (P::OBJECT_IDENTIFIER, false, false),
            (P::OBJECT_NAME, false, false),
            (P::DESCRIPTION, true, true),
            (P::OBJECT_TYPE, false, false),
            (P::PRESENT_VALUE, true, true),
            (P::USER_TYPE, false, true),
            (P::CREDENTIALS, false, false),
            (P::ASSIGNED_ACCESS_RIGHTS, true, false),
            (P::STATUS_FLAGS, false, false),
            (P::OUT_OF_SERVICE, false, true),
            (P::RELIABILITY, false, false),
            (P::PROPERTY_LIST, false, false),
        ],
        ObjectType::ACCESS_RIGHTS => vec![
            (P::OBJECT_IDENTIFIER, false, false),
            (P::OBJECT_NAME, false, false),
            (P::DESCRIPTION, true, true),
            (P::OBJECT_TYPE, false, false),
            (P::GLOBAL_IDENTIFIER, false, true),
            (P::POSITIVE_ACCESS_RULES, false, false),
            (P::NEGATIVE_ACCESS_RULES, false, false),
            (P::STATUS_FLAGS, false, false),
            (P::OUT_OF_SERVICE, false, true),
            (P::RELIABILITY, false, false),
            (P::PROPERTY_LIST, false, false),
        ],
        _ => vec![
            (P::OBJECT_IDENTIFIER, false, false),
            (P::OBJECT_NAME, false, false),
            (P::DESCRIPTION, true, true),
            (P::OBJECT_TYPE, false, false),
            (P::PRESENT_VALUE, false, false),
            (P::UPDATE_TIME, false, false),
            (P::SUPPORTED_FORMATS, false, false),
            (P::SUPPORTED_FORMAT_CLASSES, true, false),
            (P::STATUS_FLAGS, false, false),
            (P::OUT_OF_SERVICE, false, true),
            (P::RELIABILITY, false, false),
            (P::PROPERTY_LIST, false, false),
        ],
    }
}

#[test]
fn pics_access_identity_property_metadata_is_exact() {
    let fresh: [fn() -> (Box<dyn BACnetObject>, ObjectType); 4] = [
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
                object
                    .write_property(
                        P::OUT_OF_SERVICE,
                        None,
                        PropertyValue::Boolean(out_of_service),
                        None,
                    )
                    .unwrap();
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
                    rows, expected,
                    "{kind:?}, configured={configured}, OOS={out_of_service}"
                );
                assert_eq!(
                    rows.iter()
                        .filter_map(|&(p, optional, _)| (!optional).then_some(p))
                        .collect::<Vec<_>>(),
                    required.as_ref()
                );
            }
        }
    }
}
