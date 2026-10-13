use super::*;
use crate::property_metadata::PropertyWriteCapability;
use crate::traits::BACnetObject;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;
use std::collections::HashSet;

fn assert_error(error: Error, expected: ErrorCode) {
    assert!(
        matches!(error, Error::Protocol { class, code }
            if class == ErrorClass::PROPERTY.to_raw() as u32
                && code == expected.to_raw() as u32),
        "expected {expected:?}, got {error:?}"
    );
}

fn assert_exact_sets(object: &dyn BACnetObject, all: &[P], required: &[P]) {
    let metadata = object.property_metadata();
    assert!(matches!(metadata, Cow::Borrowed(_)));
    assert_eq!(metadata.len(), all.len() + 1);
    assert_eq!(object.property_list().as_ref(), all);
    assert_eq!(object.required_properties().as_ref(), required);
    assert_eq!(
        metadata
            .iter()
            .map(|row| row.property_identifier)
            .collect::<HashSet<_>>()
            .len(),
        metadata.len()
    );
    assert!(!object.is_createable());
    assert!(object.is_deleteable());
    // Global_Identifier is the W row of Tables 12-38, 12-39 and 12-40.
    let required_write = matches!(
        object.object_identifier().object_type(),
        ObjectType::ACCESS_RIGHTS | ObjectType::ACCESS_CREDENTIAL | ObjectType::ACCESS_USER
    );
    for row in metadata.iter() {
        assert_eq!(row.presence_condition, None);
        let expected = if required_write && row.property_identifier == P::GLOBAL_IDENTIFIER {
            RequiredWrite
        } else if required.contains(&row.property_identifier) {
            RequiredRead
        } else {
            Optional
        };
        assert_eq!(row.conformance, expected, "{:?}", row.property_identifier);
        object.read_property(row.property_identifier, None).unwrap();
    }
}

fn assert_indexed_property_list(object: &dyn BACnetObject, all: &[P]) {
    let wire: Vec<_> = all
        .iter()
        .filter(|&&p| !matches!(p, P::OBJECT_IDENTIFIER | P::OBJECT_NAME | P::OBJECT_TYPE))
        .map(|p| PropertyValue::Enumerated(p.to_raw()))
        .collect();
    assert!(object.is_array_property(P::PROPERTY_LIST));
    assert_eq!(
        object.read_property(P::PROPERTY_LIST, None).unwrap(),
        PropertyValue::List(wire.clone())
    );
    assert_eq!(
        object.read_property(P::PROPERTY_LIST, Some(0)).unwrap(),
        PropertyValue::Unsigned(wire.len() as u64)
    );
    for (index, value) in wire.iter().enumerate() {
        assert_eq!(
            object
                .read_property(P::PROPERTY_LIST, Some(index as u32 + 1))
                .unwrap(),
            *value
        );
    }
    for index in [wire.len() as u32 + 1, u32::MAX] {
        assert_error(
            object
                .read_property(P::PROPERTY_LIST, Some(index))
                .unwrap_err(),
            ErrorCode::INVALID_ARRAY_INDEX,
        );
    }
}

#[test]
fn property_metadata_access_credential_exact_sets_readable_rows_and_indexed_list() {
    let object = AccessCredentialObject::new(1, "CRED-1").unwrap();
    let all = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::DESCRIPTION,
        P::OBJECT_TYPE,
        P::CREDENTIAL_STATUS,
        P::ASSIGNED_ACCESS_RIGHTS,
        P::AUTHENTICATION_FACTORS,
        P::STATUS_FLAGS,
        P::RELIABILITY,
        P::GLOBAL_IDENTIFIER,
        P::REASON_FOR_DISABLE,
        P::ACTIVATION_TIME,
        P::EXPIRATION_TIME,
        P::CREDENTIAL_DISABLE,
    ];
    let required = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::CREDENTIAL_STATUS,
        P::ASSIGNED_ACCESS_RIGHTS,
        P::AUTHENTICATION_FACTORS,
        P::STATUS_FLAGS,
        P::RELIABILITY,
        P::GLOBAL_IDENTIFIER,
        P::REASON_FOR_DISABLE,
        P::ACTIVATION_TIME,
        P::EXPIRATION_TIME,
        P::CREDENTIAL_DISABLE,
        P::PROPERTY_LIST,
    ];
    assert_exact_sets(&object, &all, &required);
    assert_indexed_property_list(&object, &all);
    // Table 12-40 has no Present_Value row (#979) and no Out_Of_Service
    // row (#1064).
    for property in [P::PRESENT_VALUE, P::OUT_OF_SERVICE] {
        assert_error(
            object.read_property(property, None).unwrap_err(),
            ErrorCode::UNKNOWN_PROPERTY,
        );
    }
    // No disable reason holds, so the derived status is ACTIVE (#1073).
    assert_eq!(
        object.read_property(P::CREDENTIAL_STATUS, None).unwrap(),
        PropertyValue::Enumerated(1)
    );
    for p in [
        P::ASSIGNED_ACCESS_RIGHTS,
        P::AUTHENTICATION_FACTORS,
        P::REASON_FOR_DISABLE,
    ] {
        assert_eq!(
            object.read_property(p, None).unwrap(),
            PropertyValue::List(vec![])
        );
    }
    // Table 12-40 types the two as BACnetARRAY[N]; Reason_For_Disable is
    // a BACnetLIST.
    assert!(object.is_array_property(P::AUTHENTICATION_FACTORS));
    assert!(object.is_array_property(P::ASSIGNED_ACCESS_RIGHTS));
    assert!(!object.is_array_property(P::REASON_FOR_DISABLE));
    assert!(object.is_list_property(P::REASON_FOR_DISABLE));
    assert!(!object.is_array_property(P::CREDENTIAL_STATUS));
}

#[test]
fn property_metadata_access_user_exact_sets_readable_rows_and_indexed_list() {
    let object = AccessUserObject::new(1, "USER-1").unwrap();
    let all = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::DESCRIPTION,
        P::OBJECT_TYPE,
        P::USER_TYPE,
        P::CREDENTIALS,
        P::STATUS_FLAGS,
        P::RELIABILITY,
        P::MEMBERS,
        P::MEMBER_OF,
        P::GLOBAL_IDENTIFIER,
    ];
    let required = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::USER_TYPE,
        P::CREDENTIALS,
        P::STATUS_FLAGS,
        P::RELIABILITY,
        P::GLOBAL_IDENTIFIER,
        P::PROPERTY_LIST,
    ];
    assert_exact_sets(&object, &all, &required);
    assert_indexed_property_list(&object, &all);
    // Table 12-38 has none of these rows (#1064).
    for property in [
        P::PRESENT_VALUE,
        P::ASSIGNED_ACCESS_RIGHTS,
        P::OUT_OF_SERVICE,
    ] {
        assert_error(
            object.read_property(property, None).unwrap_err(),
            ErrorCode::UNKNOWN_PROPERTY,
        );
    }
    assert_eq!(
        object.read_property(P::USER_TYPE, None).unwrap(),
        PropertyValue::Enumerated(0)
    );
    // No global identifier is assigned until one is written (#1463).
    assert_eq!(
        object.read_property(P::GLOBAL_IDENTIFIER, None).unwrap(),
        PropertyValue::Unsigned(0)
    );
    assert!(!object.is_array_property(P::GLOBAL_IDENTIFIER));
    // The three BACnetLISTs start empty (#1394).
    for p in [P::CREDENTIALS, P::MEMBERS, P::MEMBER_OF] {
        assert_eq!(
            object.read_property(p, None).unwrap(),
            PropertyValue::List(vec![])
        );
        assert!(!object.is_array_property(p));
        assert!(object.is_list_property(p));
    }
}

#[test]
fn property_metadata_access_rights_exact_sets_readable_rows_and_indexed_list() {
    let object = AccessRightsObject::new(1, "AR-1").unwrap();
    let all = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::DESCRIPTION,
        P::OBJECT_TYPE,
        P::GLOBAL_IDENTIFIER,
        P::POSITIVE_ACCESS_RULES,
        P::NEGATIVE_ACCESS_RULES,
        P::STATUS_FLAGS,
        P::RELIABILITY,
        P::LOG_ENABLE,
    ];
    let required = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::GLOBAL_IDENTIFIER,
        P::POSITIVE_ACCESS_RULES,
        P::NEGATIVE_ACCESS_RULES,
        P::STATUS_FLAGS,
        P::RELIABILITY,
        P::LOG_ENABLE,
        P::PROPERTY_LIST,
    ];
    assert_exact_sets(&object, &all, &required);
    assert_indexed_property_list(&object, &all);
    // Table 12-39 has no Out_Of_Service row (#1064).
    assert_error(
        object.read_property(P::OUT_OF_SERVICE, None).unwrap_err(),
        ErrorCode::UNKNOWN_PROPERTY,
    );
    assert_eq!(
        object.read_property(P::GLOBAL_IDENTIFIER, None).unwrap(),
        PropertyValue::Unsigned(0)
    );
    // Enable (property 133) starts TRUE and is no array (#1332).
    assert_eq!(
        object.read_property(P::LOG_ENABLE, None).unwrap(),
        PropertyValue::Boolean(true)
    );
    assert!(!object.is_array_property(P::LOG_ENABLE));
    // Both rule properties are arrays, empty until rules are set.
    for p in [P::POSITIVE_ACCESS_RULES, P::NEGATIVE_ACCESS_RULES] {
        assert!(object.is_array_property(p));
        assert_eq!(
            object.read_property(p, None).unwrap(),
            PropertyValue::List(vec![])
        );
        assert_eq!(
            object.read_property(p, Some(0)).unwrap(),
            PropertyValue::Unsigned(0)
        );
        assert_error(
            object.read_property(p, Some(1)).unwrap_err(),
            ErrorCode::INVALID_ARRAY_INDEX,
        );
    }
}

#[test]
fn property_metadata_credential_data_input_exact_sets_readable_rows_and_indexed_list() {
    let object = CredentialDataInputObject::new(1, "CDI-1").unwrap();
    let all = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::DESCRIPTION,
        P::OBJECT_TYPE,
        P::PRESENT_VALUE,
        P::UPDATE_TIME,
        P::SUPPORTED_FORMATS,
        P::SUPPORTED_FORMAT_CLASSES,
        P::STATUS_FLAGS,
        P::OUT_OF_SERVICE,
        P::RELIABILITY,
    ];
    let required = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::OBJECT_TYPE,
        P::PRESENT_VALUE,
        P::UPDATE_TIME,
        P::SUPPORTED_FORMATS,
        P::STATUS_FLAGS,
        P::OUT_OF_SERVICE,
        P::RELIABILITY,
        P::PROPERTY_LIST,
    ];
    assert_exact_sets(&object, &all, &required);
    assert_indexed_property_list(&object, &all);
    // The UNDEFINED factor and the unspecified date and time, in their
    // Clause 21 forms (#1133).
    assert_eq!(
        object.read_property(P::PRESENT_VALUE, None).unwrap(),
        PropertyValue::ApplicationData(vec![0x09, 0x00, 0x19, 0x00, 0x28])
    );
    assert!(matches!(
        object.read_property(P::UPDATE_TIME, None).unwrap(),
        PropertyValue::ApplicationData(bytes) if bytes.first() == Some(&0x2E)
    ));
    assert_eq!(
        object.read_property(P::SUPPORTED_FORMATS, None).unwrap(),
        PropertyValue::List(vec![])
    );
    assert_eq!(
        object
            .read_property(P::SUPPORTED_FORMAT_CLASSES, None)
            .unwrap(),
        PropertyValue::List(vec![])
    );
    // Table 12-43 types both format rows as BACnetARRAY[N] (#1169).
    assert!(object.is_array_property(P::SUPPORTED_FORMATS));
    assert!(object.is_array_property(P::SUPPORTED_FORMAT_CLASSES));
    assert!(!object.is_array_property(P::UPDATE_TIME));
}

#[test]
fn property_metadata_access_identity_write_capabilities_match_dispatch() {
    // Constructor paired with the properties it must always accept writes
    // for, and those it must accept only while Out_Of_Service is TRUE.
    type WriteCase = (fn() -> Box<dyn BACnetObject>, &'static [P], &'static [P]);
    let cases: [WriteCase; 4] = [
        (
            || Box::new(AccessCredentialObject::new(1, "CRED-1").unwrap()),
            &[
                P::DESCRIPTION,
                P::GLOBAL_IDENTIFIER,
                P::ACTIVATION_TIME,
                P::EXPIRATION_TIME,
                P::CREDENTIAL_DISABLE,
            ],
            &[],
        ),
        (
            || Box::new(AccessUserObject::new(1, "USER-1").unwrap()),
            &[P::DESCRIPTION, P::USER_TYPE, P::GLOBAL_IDENTIFIER],
            &[],
        ),
        (
            || Box::new(AccessRightsObject::new(1, "AR-1").unwrap()),
            &[
                P::DESCRIPTION,
                P::GLOBAL_IDENTIFIER,
                P::POSITIVE_ACCESS_RULES,
                P::NEGATIVE_ACCESS_RULES,
                P::LOG_ENABLE,
            ],
            &[],
        ),
        (
            || Box::new(CredentialDataInputObject::new(1, "CDI-1").unwrap()),
            &[P::DESCRIPTION, P::OUT_OF_SERVICE],
            // Table 12-43 footnote 1 (#1168).
            &[P::PRESENT_VALUE, P::RELIABILITY],
        ),
    ];
    for (make, writable, when_out_of_service) in cases {
        for out_of_service in [false, true] {
            let mut object = make();
            // Of the four, only Credential Data Input has Out_Of_Service
            // (Table 12-43); the other tables have none (#1064).
            if object.object_identifier().object_type() == ObjectType::CREDENTIAL_DATA_INPUT {
                object
                    .write_property(
                        P::OUT_OF_SERVICE,
                        None,
                        PropertyValue::Boolean(out_of_service),
                        None,
                    )
                    .unwrap();
            }
            let original = object.property_metadata().into_owned();
            for row in &original {
                let p = row.property_identifier;
                let capability = if writable.contains(&p) {
                    PropertyWriteCapability::Always
                } else if when_out_of_service.contains(&p) {
                    PropertyWriteCapability::WhenOutOfService
                } else {
                    PropertyWriteCapability::ReadOnly
                };
                assert_eq!(row.write_capability, capability, "{p:?}");
                assert_eq!(
                    object.is_writable_property(p),
                    capability.is_writable(),
                    "{p:?}"
                );
                let value = object.read_property(p, None).unwrap();
                let result = object.write_property(p, None, value, None);
                if capability == PropertyWriteCapability::Always
                    || (capability == PropertyWriteCapability::WhenOutOfService && out_of_service)
                {
                    result.unwrap();
                } else {
                    assert_error(result.unwrap_err(), ErrorCode::WRITE_ACCESS_DENIED);
                }
            }
            // Object_Name has no network write route: a rename falls
            // through to WRITE_ACCESS_DENIED even with a well-formed value.
            assert!(!object.is_writable_property(P::OBJECT_NAME));
            assert_error(
                object
                    .write_property(
                        P::OBJECT_NAME,
                        None,
                        PropertyValue::CharacterString("renamed".into()),
                        None,
                    )
                    .unwrap_err(),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
            assert_eq!(object.property_metadata().as_ref(), original);
        }
    }
}

#[test]
fn property_metadata_access_identity_writes_store_verbatim_and_deny() {
    for out_of_service in [false, true] {
        let mut credential = AccessCredentialObject::new(1, "CRED-1").unwrap();
        credential
            .write_property(
                P::CREDENTIAL_DISABLE,
                None,
                PropertyValue::Enumerated(1),
                None,
            )
            .unwrap();
        assert_eq!(
            credential
                .read_property(P::CREDENTIAL_STATUS, None)
                .unwrap(),
            PropertyValue::Enumerated(0)
        );
        for (p, value) in [
            (P::CREDENTIAL_DISABLE, PropertyValue::Real(1.0)),
            (P::GLOBAL_IDENTIFIER, PropertyValue::Enumerated(1)),
            (P::ACTIVATION_TIME, PropertyValue::Unsigned(1)),
            (P::DESCRIPTION, PropertyValue::Unsigned(1)),
        ] {
            assert_error(
                credential.write_property(p, None, value, None).unwrap_err(),
                ErrorCode::INVALID_DATA_TYPE,
            );
        }
        for p in [
            P::CREDENTIAL_STATUS,
            P::REASON_FOR_DISABLE,
            P::ASSIGNED_ACCESS_RIGHTS,
            P::AUTHENTICATION_FACTORS,
            P::STATUS_FLAGS,
            P::RELIABILITY,
        ] {
            let value = credential.read_property(p, None).unwrap();
            assert_error(
                credential.write_property(p, None, value, None).unwrap_err(),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
            assert!(!credential.is_writable_property(p));
        }
        let mut user = AccessUserObject::new(1, "USER-1").unwrap();
        user.write_property(P::USER_TYPE, None, PropertyValue::Enumerated(2), None)
            .unwrap();
        assert_eq!(
            user.read_property(P::USER_TYPE, None).unwrap(),
            PropertyValue::Enumerated(2)
        );
        // Global_Identifier is an Unsigned32 (#1463), stored as written.
        user.write_property(
            P::GLOBAL_IDENTIFIER,
            None,
            PropertyValue::Unsigned(u32::MAX.into()),
            None,
        )
        .unwrap();
        assert_eq!(
            user.read_property(P::GLOBAL_IDENTIFIER, None).unwrap(),
            PropertyValue::Unsigned(u32::MAX.into())
        );
        for (p, value) in [
            (P::USER_TYPE, PropertyValue::Real(2.0)),
            (P::GLOBAL_IDENTIFIER, PropertyValue::Enumerated(77)),
            (P::DESCRIPTION, PropertyValue::Unsigned(1)),
        ] {
            assert_error(
                user.write_property(p, None, value, None).unwrap_err(),
                ErrorCode::INVALID_DATA_TYPE,
            );
        }
        // Past 32 bits it is out of range, and the value held stays.
        assert_error(
            user.write_property(
                P::GLOBAL_IDENTIFIER,
                None,
                PropertyValue::Unsigned(u64::from(u32::MAX) + 1),
                None,
            )
            .unwrap_err(),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_eq!(
            user.read_property(P::GLOBAL_IDENTIFIER, None).unwrap(),
            PropertyValue::Unsigned(u32::MAX.into())
        );
        user.set_global_identifier(77);
        assert_eq!(
            user.read_property(P::GLOBAL_IDENTIFIER, None).unwrap(),
            PropertyValue::Unsigned(77)
        );
        for p in [
            P::CREDENTIALS,
            P::MEMBERS,
            P::MEMBER_OF,
            P::STATUS_FLAGS,
            P::RELIABILITY,
        ] {
            let value = user.read_property(p, None).unwrap();
            assert_error(
                user.write_property(p, None, value, None).unwrap_err(),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
            assert!(!user.is_writable_property(p));
        }
        let mut rights = AccessRightsObject::new(1, "AR-1").unwrap();
        rights
            .write_property(
                P::GLOBAL_IDENTIFIER,
                None,
                PropertyValue::Unsigned(77),
                None,
            )
            .unwrap();
        assert_eq!(
            rights.read_property(P::GLOBAL_IDENTIFIER, None).unwrap(),
            PropertyValue::Unsigned(77)
        );
        rights
            .write_property(P::LOG_ENABLE, None, PropertyValue::Boolean(false), None)
            .unwrap();
        assert_eq!(
            rights.read_property(P::LOG_ENABLE, None).unwrap(),
            PropertyValue::Boolean(false)
        );
        for (p, value) in [
            (P::GLOBAL_IDENTIFIER, PropertyValue::Enumerated(77)),
            (P::DESCRIPTION, PropertyValue::Unsigned(1)),
            (P::LOG_ENABLE, PropertyValue::Enumerated(1)),
            (P::POSITIVE_ACCESS_RULES, PropertyValue::Boolean(true)),
        ] {
            assert_error(
                rights.write_property(p, None, value, None).unwrap_err(),
                ErrorCode::INVALID_DATA_TYPE,
            );
        }
        assert!(!rights.enable());
        for p in [P::STATUS_FLAGS, P::RELIABILITY] {
            let value = rights.read_property(p, None).unwrap();
            assert_error(
                rights.write_property(p, None, value, None).unwrap_err(),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
            assert!(!rights.is_writable_property(p));
        }
        // CDI Present_Value and Reliability take writes only while
        // Out_Of_Service is TRUE (#1168): in service an Enumerated
        // Present_Value is refused before its datatype is looked at, and
        // out of service it is the wrong datatype.
        let mut cdi = CredentialDataInputObject::new(1, "CDI-1").unwrap();
        cdi.write_property(
            P::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(out_of_service),
            None,
        )
        .unwrap();
        assert!(cdi.is_writable_property(P::PRESENT_VALUE));
        assert_error(
            cdi.write_property(P::PRESENT_VALUE, None, PropertyValue::Enumerated(1), None)
                .unwrap_err(),
            if out_of_service {
                ErrorCode::INVALID_DATA_TYPE
            } else {
                ErrorCode::WRITE_ACCESS_DENIED
            },
        );
        assert_eq!(
            cdi.read_property(P::PRESENT_VALUE, None).unwrap(),
            PropertyValue::ApplicationData(vec![0x09, 0x00, 0x19, 0x00, 0x28])
        );
        for p in [
            P::UPDATE_TIME,
            P::SUPPORTED_FORMATS,
            P::SUPPORTED_FORMAT_CLASSES,
            P::STATUS_FLAGS,
        ] {
            let value = cdi.read_property(p, None).unwrap();
            assert_error(
                cdi.write_property(p, None, value, None).unwrap_err(),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
            assert!(!cdi.is_writable_property(p));
        }
    }
}

#[test]
fn property_metadata_access_identity_unserved_rows_stay_unknown() {
    fn assert_unserved(object: &mut dyn BACnetObject, p: P) {
        assert!(!object.is_writable_property(p));
        assert_error(
            object.read_property(p, None).unwrap_err(),
            ErrorCode::UNKNOWN_PROPERTY,
        );
        assert_error(
            object
                .write_property(p, None, PropertyValue::Null, None)
                .unwrap_err(),
            ErrorCode::UNKNOWN_PROPERTY,
        );
    }

    // Days_Remaining and Trace_Flag are Table 12-40 O rows with no read
    // arm; Present_Value (#979) and Out_Of_Service (#1064) are no Table
    // 12-40 rows at all.
    let mut credential = AccessCredentialObject::new(1, "CRED-1").unwrap();
    assert_unserved(&mut credential, P::DAYS_REMAINING);
    assert_unserved(&mut credential, P::TRACE_FLAG);
    assert_unserved(&mut credential, P::PRESENT_VALUE);
    assert_unserved(&mut credential, P::OUT_OF_SERVICE);
    // Present_Value, Assigned_Access_Rights and Out_Of_Service are no
    // Table 12-38 rows (#1064).
    let mut user = AccessUserObject::new(1, "USER-1").unwrap();
    assert_unserved(&mut user, P::PRESENT_VALUE);
    assert_unserved(&mut user, P::ASSIGNED_ACCESS_RIGHTS);
    assert_unserved(&mut user, P::OUT_OF_SERVICE);
    // Accompaniment is a Table 12-39 O row served only once the
    // application sets it (#1393); Reliability_Evaluation_Inhibit is one
    // with no read arm; Out_Of_Service is no Table 12-39 row (#1064).
    let mut rights = AccessRightsObject::new(1, "AR-1").unwrap();
    assert_unserved(&mut rights, P::ACCOMPANIMENT);
    assert_unserved(&mut rights, P::RELIABILITY_EVALUATION_INHIBIT);
    assert_unserved(&mut rights, P::OUT_OF_SERVICE);
    // Event_State and Event_Detection_Enable are Table 12-43 O rows with
    // no read arm (no intrinsic reporting is modeled).
    let mut cdi = CredentialDataInputObject::new(1, "CDI-1").unwrap();
    assert_unserved(&mut cdi, P::EVENT_STATE);
    assert_unserved(&mut cdi, P::EVENT_DETECTION_ENABLE);
}
