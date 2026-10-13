use super::*;
use bacnet_objects::{
    access_control::{
        AccessCredentialObject, AccessRightsObject, AccessUserObject, CredentialDataInputObject,
    },
    traits::BACnetObject,
};
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::primitives::PropertyValue;
use PropertyIdentifier as P;

const EMPTY: &[u8] = &[];

// Shared RP-vs-RPM parity plus budget parity over one case table.
type ExpectedRead = Result<&'static [u8], ErrorCode>;

fn assert_cases(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    cases: &[(P, Option<u32>, ExpectedRead)],
) {
    let mut request = BytesMut::new();
    ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: oid,
            list_of_property_references: cases
                .iter()
                .map(|&(p, i, _)| PropertyReference {
                    property_identifier: p,
                    property_array_index: i,
                })
                .collect(),
        }],
    }
    .encode(&mut request)
    .unwrap();
    let mut legacy = BytesMut::new();
    handle_read_property_multiple(db, &request, &mut legacy).unwrap();
    let ack = ReadPropertyMultipleACK::decode(&legacy).unwrap();
    assert_eq!(ack.list_of_read_access_results.len(), 1);
    let access = &ack.list_of_read_access_results[0];
    assert_eq!(access.object_identifier, oid);
    assert_eq!(access.list_of_results.len(), cases.len());
    for (result, &(p, i, expected)) in access.list_of_results.iter().zip(cases) {
        assert_eq!(result.property_identifier, p);
        // These table errors identify non-arrays or absent optional rows.
        let response_index = if matches!(
            expected,
            Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY | ErrorCode::UNKNOWN_PROPERTY)
        ) {
            None
        } else {
            i
        };
        assert_eq!(result.property_array_index, response_index);
        let mut rp_request = BytesMut::new();
        ReadPropertyRequest {
            object_identifier: oid,
            property_identifier: p,
            property_array_index: i,
        }
        .encode(&mut rp_request);
        let mut response = BytesMut::new();
        let rp = handle_read_property(db, &rp_request, &mut response);
        match expected {
            Ok(bytes) => {
                assert!(result.error.is_none(), "{p:?} {i:?}");
                assert_eq!(result.property_value.as_deref(), Some(bytes), "{p:?} {i:?}");
                rp.unwrap();
                let rp_ack = ReadPropertyACK::decode(&response).unwrap();
                assert_eq!(rp_ack.object_identifier, oid);
                assert_eq!(rp_ack.property_identifier, p);
                assert_eq!(rp_ack.property_array_index, i);
                assert_eq!(rp_ack.property_value, bytes);
            }
            Err(expected) => {
                assert!(result.property_value.is_none());
                assert_eq!(result.error, Some((ErrorClass::PROPERTY, expected)));
                assert!(matches!(rp, Err(Error::Protocol { class, code })
                    if class == ErrorClass::PROPERTY.to_raw() as u32 && code == expected.to_raw() as u32));
                assert!(response.is_empty());
            }
        }
    }
    use crate::handlers::{rpm_budget::handle_rpm_budgeted, ReadFailure};
    let budget = crate::server::ReadPropertyMultipleBudget {
        max_result_elements: cases.len(),
        max_service_ack_bytes: legacy.len(),
    };
    let mut bounded = BytesMut::new();
    handle_rpm_budgeted(db, &request, &mut bounded, budget).unwrap();
    assert_eq!(bounded, legacy);
    let mut prefix = BytesMut::from(&b"prefix"[..]);
    assert!(matches!(
        handle_rpm_budgeted(
            db,
            &request,
            &mut prefix,
            crate::server::ReadPropertyMultipleBudget {
                max_result_elements: cases.len() - 1,
                ..budget
            }
        ),
        Err(ReadFailure::Work)
    ));
    assert_eq!(&prefix[..], b"prefix");
    assert!(matches!(
        handle_rpm_budgeted(
            db,
            &request,
            &mut prefix,
            crate::server::ReadPropertyMultipleBudget {
                max_service_ack_bytes: legacy.len() - 1,
                ..budget
            }
        ),
        Err(ReadFailure::Bytes)
    ));
    assert_eq!(&prefix[..], b"prefix");
}

fn write_common(object: &mut dyn BACnetObject, configured: bool) {
    object
        .write_property(
            P::DESCRIPTION,
            None,
            PropertyValue::CharacterString("long access label".repeat(100)),
            None,
        )
        .unwrap();
    // Of the quartet, only Credential Data Input has Out_Of_Service (Table
    // 12-43); the other three tables have none (#1064).
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

fn status_flags_bytes(configured: bool) -> &'static [u8] {
    if configured {
        &[0x82, 4, 0x10]
    } else {
        &[0x82, 4, 0]
    }
}

fn out_of_service_bytes(configured: bool) -> &'static [u8] {
    if configured {
        &[0x11]
    } else {
        &[0x10]
    }
}

#[test]
fn rpm_access_credential_indexed_reads_and_bytes_are_unchanged() {
    for configured in [false, true] {
        let mut object = AccessCredentialObject::new(7, "CRED-7").unwrap();
        if configured {
            // Credential_Disable DISABLE adds DISABLED (0) to
            // Reason_For_Disable, which makes the status INACTIVE (#1073).
            object
                .write_property(
                    P::CREDENTIAL_DISABLE,
                    None,
                    PropertyValue::Enumerated(1),
                    None,
                )
                .unwrap();
            object
                .write_property(
                    P::GLOBAL_IDENTIFIER,
                    None,
                    PropertyValue::Unsigned(77),
                    None,
                )
                .unwrap();
        }
        write_common(&mut object, configured);
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Independent application-value bytes pin the projection. The two
        // BACnetARRAYs admit an index (empty here, so only the size reads);
        // Reason_For_Disable is a BACnetLIST and rejects one. Both window
        // ends read as the open all-X'FF' date and time.
        let open_window: &[u8] = &[0xa4, 0xff, 0xff, 0xff, 0xff, 0xb4, 0xff, 0xff, 0xff, 0xff];
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            (
                P::CREDENTIAL_STATUS,
                None,
                Ok(if configured { &[0x91, 0] } else { &[0x91, 1] }),
            ),
            (
                P::CREDENTIAL_STATUS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::ASSIGNED_ACCESS_RIGHTS, None, Ok(EMPTY)),
            (P::ASSIGNED_ACCESS_RIGHTS, Some(0), Ok(&[0x21, 0])),
            (
                P::ASSIGNED_ACCESS_RIGHTS,
                Some(1),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (P::AUTHENTICATION_FACTORS, None, Ok(EMPTY)),
            (P::AUTHENTICATION_FACTORS, Some(0), Ok(&[0x21, 0])),
            (
                P::AUTHENTICATION_FACTORS,
                Some(1),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            // No Out_Of_Service row (#1064), so the flag stays clear.
            (P::STATUS_FLAGS, None, Ok(status_flags_bytes(false))),
            (
                P::STATUS_FLAGS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::OUT_OF_SERVICE, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::OUT_OF_SERVICE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::RELIABILITY, None, Ok(&[0x91, 0])),
            (
                P::RELIABILITY,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::GLOBAL_IDENTIFIER,
                None,
                Ok(if configured { &[0x21, 77] } else { &[0x21, 0] }),
            ),
            (
                P::GLOBAL_IDENTIFIER,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::REASON_FOR_DISABLE,
                None,
                Ok(if configured { &[0x91, 0] } else { EMPTY }),
            ),
            (
                P::REASON_FOR_DISABLE,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::ACTIVATION_TIME, None, Ok(open_window)),
            (
                P::ACTIVATION_TIME,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::EXPIRATION_TIME, None, Ok(open_window)),
            (
                P::CREDENTIAL_DISABLE,
                None,
                Ok(if configured { &[0x91, 1] } else { &[0x91, 0] }),
            ),
            (
                P::PROPERTY_LIST,
                None,
                Ok(&[
                    0x91, 28, 0x92, 0x01, 0x08, 0x92, 0x01, 0x00, 0x92, 0x01, 0x01, 0x91, 111,
                    0x91, 103, 0x92, 0x01, 0x43, 0x92, 0x01, 0x2F, 0x91, 0xFE, 0x92, 0x01, 0x0E,
                    0x92, 0x01, 0x07,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 11])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x92, 0x01, 0x08])),
            (P::PROPERTY_LIST, Some(7), Ok(&[0x92, 0x01, 0x43])),
            (P::PROPERTY_LIST, Some(9), Ok(&[0x91, 0xFE])),
            (P::PROPERTY_LIST, Some(11), Ok(&[0x92, 0x01, 0x07])),
            (
                P::PROPERTY_LIST,
                Some(12),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::PROPERTY_LIST,
                Some(u32::MAX),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            // Present_Value is no Table 12-40 row (#979); Days_Remaining is
            // a Table 12-40 O row with no read arm.
            (P::PRESENT_VALUE, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::PRESENT_VALUE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::DAYS_REMAINING, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
        ];
        assert_cases(&db, oid, cases);
    }
}

#[test]
fn rpm_access_user_indexed_reads_and_bytes_are_unchanged() {
    for configured in [false, true] {
        use bacnet_types::constructed::BACnetDeviceObjectReference as Reference;
        // Access Credential 1 and Access User 2 here; Access Credential 4 and
        // Access User 5 in Device 9 (#1394).
        const CREDENTIALS: &[u8] = &[
            0x1C, 0x08, 0x00, 0x00, 0x01, 0x0C, 0x02, 0x00, 0x00, 0x09, 0x1C, 0x08, 0x00, 0x00,
            0x04,
        ];
        const MEMBERS: &[u8] = &[0x1C, 0x08, 0xC0, 0x00, 0x02];
        const MEMBER_OF: &[u8] = &[0x0C, 0x02, 0x00, 0x00, 0x09, 0x1C, 0x08, 0xC0, 0x00, 0x05];
        let in_device_9 = |object_type, instance| Reference {
            device_identifier: Some(ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap()),
            object_identifier: ObjectIdentifier::new(object_type, instance).unwrap(),
        };
        let mut object = AccessUserObject::new(7, "USER-7").unwrap();
        if configured {
            object
                .write_property(P::USER_TYPE, None, PropertyValue::Enumerated(2), None)
                .unwrap();
            object
                .write_property(
                    P::GLOBAL_IDENTIFIER,
                    None,
                    PropertyValue::Unsigned(77),
                    None,
                )
                .unwrap();
            let here = |object_type, instance| -> Reference {
                ObjectIdentifier::new(object_type, instance).unwrap().into()
            };
            object
                .set_credentials([
                    here(ObjectType::ACCESS_CREDENTIAL, 1),
                    in_device_9(ObjectType::ACCESS_CREDENTIAL, 4),
                ])
                .unwrap();
            object
                .set_members([here(ObjectType::ACCESS_USER, 2)])
                .unwrap();
            object
                .set_member_of([in_device_9(ObjectType::ACCESS_USER, 5)])
                .unwrap();
        }
        let listed =
            |octets: &'static [u8]| -> ExpectedRead { Ok(if configured { octets } else { EMPTY }) };
        write_common(&mut object, configured);
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Credentials, Members and Member_Of are BACnetLISTs and reject any
        // index.
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            // Table 12-38 has no Present_Value or Assigned_Access_Rights
            // (#1064).
            (P::PRESENT_VALUE, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::PRESENT_VALUE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::USER_TYPE,
                None,
                Ok(if configured { &[0x91, 2] } else { &[0x91, 0] }),
            ),
            (
                P::USER_TYPE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::CREDENTIALS, None, listed(CREDENTIALS)),
            (
                P::CREDENTIALS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::CREDENTIALS,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::MEMBERS, None, listed(MEMBERS)),
            (
                P::MEMBERS,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::MEMBER_OF, None, listed(MEMBER_OF)),
            (
                P::MEMBER_OF,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            // Assigned_Access_Rights is a BACnetARRAY on the one table that
            // has it (Table 12-40), so the index passes the gate and the
            // user answers that it has no such property.
            (
                P::ASSIGNED_ACCESS_RIGHTS,
                None,
                Err(ErrorCode::UNKNOWN_PROPERTY),
            ),
            (
                P::ASSIGNED_ACCESS_RIGHTS,
                Some(0),
                Err(ErrorCode::UNKNOWN_PROPERTY),
            ),
            // No Out_Of_Service row (#1064), so the flag stays clear.
            (P::STATUS_FLAGS, None, Ok(status_flags_bytes(false))),
            (
                P::STATUS_FLAGS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::OUT_OF_SERVICE, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::OUT_OF_SERVICE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::RELIABILITY, None, Ok(&[0x91, 0])),
            (
                P::RELIABILITY,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::PROPERTY_LIST,
                None,
                Ok(&[
                    0x91, 28, 0x92, 0x01, 0x3E, 0x92, 0x01, 0x09, 0x91, 111, 0x91, 103, 0x92, 0x01,
                    0x1E, 0x91, 159, 0x92, 0x01, 0x43,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 8])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x92, 0x01, 0x3E])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x92, 0x01, 0x09])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x91, 111])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x91, 103])),
            // Members (286) and Member_Of (159), the O rows #1394 serves.
            (P::PROPERTY_LIST, Some(6), Ok(&[0x92, 0x01, 0x1E])),
            (P::PROPERTY_LIST, Some(7), Ok(&[0x91, 159])),
            // Global_Identifier (323), the W row #1463 serves.
            (P::PROPERTY_LIST, Some(8), Ok(&[0x92, 0x01, 0x43])),
            (
                P::PROPERTY_LIST,
                Some(9),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::PROPERTY_LIST,
                Some(u32::MAX),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            // Global_Identifier, the Table 12-38 W row (#1463): an
            // Unsigned32, and no array.
            (
                P::GLOBAL_IDENTIFIER,
                None,
                Ok(if configured { &[0x21, 77] } else { &[0x21, 0] }),
            ),
            (
                P::GLOBAL_IDENTIFIER,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
        ];
        assert_cases(&db, oid, cases);
    }
}

#[test]
fn rpm_access_rights_indexed_reads_and_bytes_are_unchanged() {
    for configured in [false, true] {
        let mut object = AccessRightsObject::new(7, "AR-7").unwrap();
        if configured {
            object
                .write_property(
                    P::GLOBAL_IDENTIFIER,
                    None,
                    PropertyValue::Unsigned(77),
                    None,
                )
                .unwrap();
            object.set_enable(false);
        }
        write_common(&mut object, configured);
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            (
                P::GLOBAL_IDENTIFIER,
                None,
                Ok(if configured { &[0x21, 77] } else { &[0x21, 0] }),
            ),
            // Enable, property 133 (#1332): an application BOOLEAN.
            (
                P::LOG_ENABLE,
                None,
                Ok(if configured { &[0x10] } else { &[0x11] }),
            ),
            (
                P::LOG_ENABLE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::GLOBAL_IDENTIFIER,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            // Empty arrays: no octets whole, a zero size at index 0.
            (P::POSITIVE_ACCESS_RULES, None, Ok(&[])),
            (P::POSITIVE_ACCESS_RULES, Some(0), Ok(&[0x21, 0])),
            (
                P::POSITIVE_ACCESS_RULES,
                Some(1),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (P::NEGATIVE_ACCESS_RULES, None, Ok(&[])),
            (P::NEGATIVE_ACCESS_RULES, Some(0), Ok(&[0x21, 0])),
            // No Out_Of_Service row (#1064), so the flag stays clear.
            (P::STATUS_FLAGS, None, Ok(status_flags_bytes(false))),
            (
                P::STATUS_FLAGS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::OUT_OF_SERVICE, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::OUT_OF_SERVICE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::RELIABILITY, None, Ok(&[0x91, 0])),
            (
                P::RELIABILITY,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::PROPERTY_LIST,
                None,
                Ok(&[
                    0x91, 28, 0x92, 0x01, 0x43, 0x92, 0x01, 0x2E, 0x92, 0x01, 0x20, 0x91, 111,
                    0x91, 103, 0x91, 133,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 7])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x92, 0x01, 0x43])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x92, 0x01, 0x2E])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x92, 0x01, 0x20])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x91, 111])),
            (P::PROPERTY_LIST, Some(6), Ok(&[0x91, 103])),
            (P::PROPERTY_LIST, Some(7), Ok(&[0x91, 133])),
            (
                P::PROPERTY_LIST,
                Some(8),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::PROPERTY_LIST,
                Some(u32::MAX),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            // Accompaniment and Reliability_Evaluation_Inhibit are Table
            // 12-39 O rows with no read arm.
            (P::ACCOMPANIMENT, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::ACCOMPANIMENT,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::RELIABILITY_EVALUATION_INHIBIT,
                None,
                Err(ErrorCode::UNKNOWN_PROPERTY),
            ),
            (
                P::RELIABILITY_EVALUATION_INHIBIT,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
        ];
        assert_cases(&db, oid, cases);
    }
}

#[test]
fn rpm_credential_data_input_indexed_reads_and_bytes_are_unchanged() {
    for configured in [false, true] {
        let mut object = CredentialDataInputObject::new(7, "CDI-7").unwrap();
        write_common(&mut object, configured);
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Present_Value is the UNDEFINED BACnetAuthenticationFactor: format
        // type [0] 0, format class [1] 0, an empty value [2]. Update_Time is
        // a BACnetTimeStamp, the unspecified date and time framed as the
        // datetime [2] choice, the same bytes as the Access Point
        // Access_Event_Time default (#1133). The two format rows are empty
        // BACnetARRAYs: index 0 is their size and index 1 is past the end
        // (#1169).
        let unspec_time: &[u8] = &[
            0x2e, 0xa4, 0xff, 0xff, 0xff, 0xff, 0xb4, 0xff, 0xff, 0xff, 0xff, 0x2f,
        ];
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            (P::PRESENT_VALUE, None, Ok(&[0x09, 0x00, 0x19, 0x00, 0x28])),
            (
                P::PRESENT_VALUE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::UPDATE_TIME, None, Ok(unspec_time)),
            (
                P::UPDATE_TIME,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::UPDATE_TIME,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::SUPPORTED_FORMATS, None, Ok(EMPTY)),
            (P::SUPPORTED_FORMATS, Some(0), Ok(&[0x21, 0])),
            (
                P::SUPPORTED_FORMATS,
                Some(1),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (P::SUPPORTED_FORMAT_CLASSES, None, Ok(EMPTY)),
            (P::SUPPORTED_FORMAT_CLASSES, Some(0), Ok(&[0x21, 0])),
            (
                P::SUPPORTED_FORMAT_CLASSES,
                Some(1),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (P::STATUS_FLAGS, None, Ok(status_flags_bytes(configured))),
            (
                P::STATUS_FLAGS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::OUT_OF_SERVICE,
                None,
                Ok(out_of_service_bytes(configured)),
            ),
            (
                P::OUT_OF_SERVICE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::RELIABILITY, None, Ok(&[0x91, 0])),
            (
                P::RELIABILITY,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::PROPERTY_LIST,
                None,
                Ok(&[
                    0x91, 28, 0x91, 85, 0x91, 189, 0x92, 0x01, 0x30, 0x92, 0x01, 0x31, 0x91, 111,
                    0x91, 81, 0x91, 103,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 8])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x91, 85])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x91, 189])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x92, 0x01, 0x30])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x92, 0x01, 0x31])),
            (P::PROPERTY_LIST, Some(6), Ok(&[0x91, 111])),
            (P::PROPERTY_LIST, Some(7), Ok(&[0x91, 81])),
            (P::PROPERTY_LIST, Some(8), Ok(&[0x91, 103])),
            (
                P::PROPERTY_LIST,
                Some(9),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::PROPERTY_LIST,
                Some(u32::MAX),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            // Event_State and Event_Detection_Enable are Table 12-43 O rows
            // with no read arm (no intrinsic reporting is modeled).
            (P::EVENT_STATE, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::EVENT_STATE,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::EVENT_DETECTION_ENABLE,
                None,
                Err(ErrorCode::UNKNOWN_PROPERTY),
            ),
            (
                P::EVENT_DETECTION_ENABLE,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
        ];
        assert_cases(&db, oid, cases);
    }
}
