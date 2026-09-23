use super::{
    AccessCredentialObject, AccessRightsObject, AccessUserObject, CredentialDataInputObject,
};
use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead, RequiredWrite},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly},
};

// Canonical effective rows for the Access Identity quartet (ASHRAE 135-2020; PDF = printed + 2):
// - Access Credential (type 32, §12.35 Table 12-40; printed p. 400 / PDF p. 402)
// - Access User (type 35, §12.33 Table 12-38; printed p. 390 / PDF p. 392)
// - Access Rights (type 34, §12.34 Table 12-39; printed p. 394 / PDF p. 396)
// - Credential Data Input (type 37, §12.36 Table 12-43; printed p. 409 / PDF p. 411)
// Order preserves each legacy projection; Property_List is appended so the
// projection helper omits it while required_properties keeps it. Only
// implemented rows are described: table rows the objects do not serve stay
// absent until dispatch exists (credential/user Global_Identifier W,
// credential Reason_For_Disable/Activation_Time/Expiration_Time R, user
// Members R, rights Accompaniment O, CDI event/intrinsic rows). Shared
// conventions match metadata_topology.rs (Slice A): OI/ON/OT
// RequiredRead/ReadOnly with the explicit Object_Name denial, Description
// Optional/Always, table-absent-but-served Out_Of_Service
// RequiredRead/Always, Status_Flags/Reliability RequiredRead/ReadOnly
// (table R on all four quartet tables), Always-never-WhenOutOfService
// writability mirroring dispatch, presence None, not createable but
// deleteable with no overrides, and Property_List as the only array-gated row.
// Credential Present_Value is an implementation-extra row (Table 12-40 has no
// Present_Value row) with a routed Enumerated arm, so Optional/Always.
// Credential_Status/Assigned_Access_Rights/Authentication_Factors carry the
// table R code; the status arm makes CREDENTIAL_STATUS RequiredRead/Always
// while the count/list stay RequiredRead/ReadOnly. User Present_Value and
// Assigned_Access_Rights are implementation-extra rows (Table 12-38 has
// neither); the PV arm makes it Optional/Always while the count stays
// Optional/ReadOnly. User_Type/Credentials carry the table R code; the
// User_Type arm makes it RequiredRead/Always while Credentials stays
// RequiredRead/ReadOnly. Rights Global_Identifier carries the table W code
// with the routed Unsigned arm, so RequiredWrite/Always; the ±rules rows
// carry the table R code with no arm (counts only), so
// RequiredRead/ReadOnly. CDI Present_Value carries the table R code with
// footnote 1 (writable when Out_Of_Service) but dispatch denies every
// non-Description/Out_Of_Service write, so the metadata mirrors dispatch as
// RequiredRead/ReadOnly with no behavior change. Update_Time and
// Supported_Formats carry the table R code with no arm, so
// RequiredRead/ReadOnly; Supported_Format_Classes carries the table O code,
// so Optional/ReadOnly.
const ACCESS_CREDENTIAL_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, Optional, None, Always),
    PropertyMetadata::new(P::CREDENTIAL_STATUS, RequiredRead, None, Always),
    PropertyMetadata::new(P::ASSIGNED_ACCESS_RIGHTS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::AUTHENTICATION_FACTORS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::RELIABILITY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

const ACCESS_USER_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, Optional, None, Always),
    PropertyMetadata::new(P::USER_TYPE, RequiredRead, None, Always),
    PropertyMetadata::new(P::CREDENTIALS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::ASSIGNED_ACCESS_RIGHTS, Optional, None, ReadOnly),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::RELIABILITY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

const ACCESS_RIGHTS_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::GLOBAL_IDENTIFIER, RequiredWrite, None, Always),
    PropertyMetadata::new(P::POSITIVE_ACCESS_RULES, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::NEGATIVE_ACCESS_RULES, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::RELIABILITY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

const CREDENTIAL_DATA_INPUT_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::UPDATE_TIME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::SUPPORTED_FORMATS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::SUPPORTED_FORMAT_CLASSES, Optional, None, ReadOnly),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::RELIABILITY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_access_credential_object(
    _object: &AccessCredentialObject,
) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(ACCESS_CREDENTIAL_BASE)
}

pub(super) fn for_access_user_object(_object: &AccessUserObject) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(ACCESS_USER_BASE)
}

pub(super) fn for_access_rights_object(
    _object: &AccessRightsObject,
) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(ACCESS_RIGHTS_BASE)
}

pub(super) fn for_credential_data_input_object(
    _object: &CredentialDataInputObject,
) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(CREDENTIAL_DATA_INPUT_BASE)
}

#[cfg(test)]
mod tests {
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
        let required_write = object.object_identifier().object_type() == ObjectType::ACCESS_RIGHTS;
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
            P::PRESENT_VALUE,
            P::CREDENTIAL_STATUS,
            P::ASSIGNED_ACCESS_RIGHTS,
            P::AUTHENTICATION_FACTORS,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
        ];
        let required = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::CREDENTIAL_STATUS,
            P::ASSIGNED_ACCESS_RIGHTS,
            P::AUTHENTICATION_FACTORS,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
            P::PROPERTY_LIST,
        ];
        assert_exact_sets(&object, &all, &required);
        assert_indexed_property_list(&object, &all);
        assert_eq!(
            object.read_property(P::PRESENT_VALUE, None).unwrap(),
            PropertyValue::Enumerated(0)
        );
        assert_eq!(
            object.read_property(P::CREDENTIAL_STATUS, None).unwrap(),
            PropertyValue::Enumerated(0)
        );
        assert_eq!(
            object
                .read_property(P::ASSIGNED_ACCESS_RIGHTS, None)
                .unwrap(),
            PropertyValue::Unsigned(0)
        );
        assert_eq!(
            object
                .read_property(P::AUTHENTICATION_FACTORS, None)
                .unwrap(),
            PropertyValue::List(vec![])
        );
        assert!(!object.is_array_property(P::AUTHENTICATION_FACTORS));
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
            P::PRESENT_VALUE,
            P::USER_TYPE,
            P::CREDENTIALS,
            P::ASSIGNED_ACCESS_RIGHTS,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
        ];
        let required = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::USER_TYPE,
            P::CREDENTIALS,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
            P::PROPERTY_LIST,
        ];
        assert_exact_sets(&object, &all, &required);
        assert_indexed_property_list(&object, &all);
        assert_eq!(
            object.read_property(P::PRESENT_VALUE, None).unwrap(),
            PropertyValue::Enumerated(0)
        );
        assert_eq!(
            object.read_property(P::USER_TYPE, None).unwrap(),
            PropertyValue::Enumerated(0)
        );
        assert_eq!(
            object.read_property(P::CREDENTIALS, None).unwrap(),
            PropertyValue::List(vec![])
        );
        assert_eq!(
            object
                .read_property(P::ASSIGNED_ACCESS_RIGHTS, None)
                .unwrap(),
            PropertyValue::Unsigned(0)
        );
        assert!(!object.is_array_property(P::CREDENTIALS));
        assert!(!object.is_array_property(P::ASSIGNED_ACCESS_RIGHTS));
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
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
        ];
        let required = [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::GLOBAL_IDENTIFIER,
            P::POSITIVE_ACCESS_RULES,
            P::NEGATIVE_ACCESS_RULES,
            P::STATUS_FLAGS,
            P::OUT_OF_SERVICE,
            P::RELIABILITY,
            P::PROPERTY_LIST,
        ];
        assert_exact_sets(&object, &all, &required);
        assert_indexed_property_list(&object, &all);
        assert_eq!(
            object.read_property(P::GLOBAL_IDENTIFIER, None).unwrap(),
            PropertyValue::Unsigned(0)
        );
        assert_eq!(
            object
                .read_property(P::POSITIVE_ACCESS_RULES, None)
                .unwrap(),
            PropertyValue::Unsigned(0)
        );
        assert_eq!(
            object
                .read_property(P::NEGATIVE_ACCESS_RULES, None)
                .unwrap(),
            PropertyValue::Unsigned(0)
        );
        assert!(!object.is_array_property(P::POSITIVE_ACCESS_RULES));
        assert!(!object.is_array_property(P::NEGATIVE_ACCESS_RULES));
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
        assert_eq!(
            object.read_property(P::PRESENT_VALUE, None).unwrap(),
            PropertyValue::Enumerated(0)
        );
        match object.read_property(P::UPDATE_TIME, None).unwrap() {
            PropertyValue::List(items) => assert_eq!(items.len(), 2),
            other => panic!("expected List, got {other:?}"),
        }
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
        // Supported_Formats is BACnetARRAY per Table 12-43 but the default
        // array gate keeps Property_List as the only indexed row on this
        // family, so an index is rejected at the service gate.
        assert!(!object.is_array_property(P::SUPPORTED_FORMATS));
        assert!(!object.is_array_property(P::SUPPORTED_FORMAT_CLASSES));
        assert!(!object.is_array_property(P::UPDATE_TIME));
    }

    #[test]
    fn property_metadata_access_identity_write_capabilities_match_dispatch() {
        let cases: [(fn() -> Box<dyn BACnetObject>, &[P]); 4] = [
            (
                || Box::new(AccessCredentialObject::new(1, "CRED-1").unwrap()),
                &[
                    P::DESCRIPTION,
                    P::OUT_OF_SERVICE,
                    P::PRESENT_VALUE,
                    P::CREDENTIAL_STATUS,
                ],
            ),
            (
                || Box::new(AccessUserObject::new(1, "USER-1").unwrap()),
                &[
                    P::DESCRIPTION,
                    P::OUT_OF_SERVICE,
                    P::PRESENT_VALUE,
                    P::USER_TYPE,
                ],
            ),
            (
                || Box::new(AccessRightsObject::new(1, "AR-1").unwrap()),
                &[P::DESCRIPTION, P::OUT_OF_SERVICE, P::GLOBAL_IDENTIFIER],
            ),
            (
                || Box::new(CredentialDataInputObject::new(1, "CDI-1").unwrap()),
                &[P::DESCRIPTION, P::OUT_OF_SERVICE],
            ),
        ];
        for (make, writable) in cases {
            for out_of_service in [false, true] {
                let mut object = make();
                object
                    .write_property(
                        P::OUT_OF_SERVICE,
                        None,
                        PropertyValue::Boolean(out_of_service),
                        None,
                    )
                    .unwrap();
                let original = object.property_metadata().into_owned();
                for row in &original {
                    let p = row.property_identifier;
                    let capability = if writable.contains(&p) {
                        PropertyWriteCapability::Always
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
                    if capability.is_writable() {
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
                    P::OUT_OF_SERVICE,
                    None,
                    PropertyValue::Boolean(out_of_service),
                    None,
                )
                .unwrap();
            credential
                .write_property(P::PRESENT_VALUE, None, PropertyValue::Enumerated(3), None)
                .unwrap();
            credential
                .write_property(
                    P::CREDENTIAL_STATUS,
                    None,
                    PropertyValue::Enumerated(2),
                    None,
                )
                .unwrap();
            assert_eq!(
                credential.read_property(P::PRESENT_VALUE, None).unwrap(),
                PropertyValue::Enumerated(3)
            );
            assert_eq!(
                credential
                    .read_property(P::CREDENTIAL_STATUS, None)
                    .unwrap(),
                PropertyValue::Enumerated(2)
            );
            for (p, value) in [
                (P::PRESENT_VALUE, PropertyValue::Real(3.0)),
                (P::CREDENTIAL_STATUS, PropertyValue::Real(2.0)),
                (P::DESCRIPTION, PropertyValue::Unsigned(1)),
                (P::OUT_OF_SERVICE, PropertyValue::Unsigned(1)),
            ] {
                assert_error(
                    credential.write_property(p, None, value, None).unwrap_err(),
                    ErrorCode::INVALID_DATA_TYPE,
                );
            }
            for p in [
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
            user.write_property(
                P::OUT_OF_SERVICE,
                None,
                PropertyValue::Boolean(out_of_service),
                None,
            )
            .unwrap();
            user.write_property(P::PRESENT_VALUE, None, PropertyValue::Enumerated(1), None)
                .unwrap();
            user.write_property(P::USER_TYPE, None, PropertyValue::Enumerated(2), None)
                .unwrap();
            assert_eq!(
                user.read_property(P::PRESENT_VALUE, None).unwrap(),
                PropertyValue::Enumerated(1)
            );
            assert_eq!(
                user.read_property(P::USER_TYPE, None).unwrap(),
                PropertyValue::Enumerated(2)
            );
            for (p, value) in [
                (P::PRESENT_VALUE, PropertyValue::Real(1.0)),
                (P::USER_TYPE, PropertyValue::Real(2.0)),
                (P::DESCRIPTION, PropertyValue::Unsigned(1)),
                (P::OUT_OF_SERVICE, PropertyValue::Unsigned(1)),
            ] {
                assert_error(
                    user.write_property(p, None, value, None).unwrap_err(),
                    ErrorCode::INVALID_DATA_TYPE,
                );
            }
            for p in [
                P::CREDENTIALS,
                P::ASSIGNED_ACCESS_RIGHTS,
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
                    P::OUT_OF_SERVICE,
                    None,
                    PropertyValue::Boolean(out_of_service),
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
            assert_eq!(
                rights.read_property(P::GLOBAL_IDENTIFIER, None).unwrap(),
                PropertyValue::Unsigned(77)
            );
            for (p, value) in [
                (P::GLOBAL_IDENTIFIER, PropertyValue::Enumerated(77)),
                (P::DESCRIPTION, PropertyValue::Unsigned(1)),
                (P::OUT_OF_SERVICE, PropertyValue::Unsigned(1)),
            ] {
                assert_error(
                    rights.write_property(p, None, value, None).unwrap_err(),
                    ErrorCode::INVALID_DATA_TYPE,
                );
            }
            for p in [
                P::POSITIVE_ACCESS_RULES,
                P::NEGATIVE_ACCESS_RULES,
                P::STATUS_FLAGS,
                P::RELIABILITY,
            ] {
                let value = rights.read_property(p, None).unwrap();
                assert_error(
                    rights.write_property(p, None, value, None).unwrap_err(),
                    ErrorCode::WRITE_ACCESS_DENIED,
                );
                assert!(!rights.is_writable_property(p));
            }
            // CDI Present_Value carries the table R1 footnote but dispatch
            // denies every non-Description/Out_Of_Service write, so the
            // metadata stays ReadOnly and the denial holds while OOS too.
            let mut cdi = CredentialDataInputObject::new(1, "CDI-1").unwrap();
            cdi.write_property(
                P::OUT_OF_SERVICE,
                None,
                PropertyValue::Boolean(out_of_service),
                None,
            )
            .unwrap();
            assert!(!cdi.is_writable_property(P::PRESENT_VALUE));
            assert_error(
                cdi.write_property(P::PRESENT_VALUE, None, PropertyValue::Enumerated(1), None)
                    .unwrap_err(),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
            assert_eq!(
                cdi.read_property(P::PRESENT_VALUE, None).unwrap(),
                PropertyValue::Enumerated(0)
            );
            for p in [
                P::UPDATE_TIME,
                P::SUPPORTED_FORMATS,
                P::SUPPORTED_FORMAT_CLASSES,
                P::STATUS_FLAGS,
                P::RELIABILITY,
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
                ErrorCode::WRITE_ACCESS_DENIED,
            );
        }

        // Global_Identifier is the Table 12-40 W row with no read arm;
        // Activation_Time is the Table 12-40 R row with no read arm.
        let mut credential = AccessCredentialObject::new(1, "CRED-1").unwrap();
        assert_unserved(&mut credential, P::GLOBAL_IDENTIFIER);
        assert_unserved(&mut credential, P::ACTIVATION_TIME);
        // Global_Identifier is the Table 12-38 W row with no read arm;
        // Members is the Table 12-38 O row with no read arm.
        let mut user = AccessUserObject::new(1, "USER-1").unwrap();
        assert_unserved(&mut user, P::GLOBAL_IDENTIFIER);
        assert_unserved(&mut user, P::MEMBERS);
        // Accompaniment and Reliability_Evaluation_Inhibit are Table 12-39 O
        // rows with no read arm.
        let mut rights = AccessRightsObject::new(1, "AR-1").unwrap();
        assert_unserved(&mut rights, P::ACCOMPANIMENT);
        assert_unserved(&mut rights, P::RELIABILITY_EVALUATION_INHIBIT);
        // Event_State and Event_Detection_Enable are Table 12-43 O rows with
        // no read arm (no intrinsic reporting is modeled).
        let mut cdi = CredentialDataInputObject::new(1, "CDI-1").unwrap();
        assert_unserved(&mut cdi, P::EVENT_STATE);
        assert_unserved(&mut cdi, P::EVENT_DETECTION_ENABLE);
    }
}
