use super::*;
use bacnet_objects::{
    access_control::{
        AccessCredentialObject, AccessRightsObject, AccessUserObject, CredentialDataInputObject,
    },
    traits::BACnetObject,
};
use bacnet_services::common::PropertyReference;
use bacnet_services::rpm::ReadAccessSpecification;
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
    .encode(&mut request);
    let mut legacy = BytesMut::new();
    handle_read_property_multiple(db, &request, &mut legacy).unwrap();
    let ack = ReadPropertyMultipleACK::decode(&legacy).unwrap();
    assert_eq!(ack.list_of_read_access_results.len(), 1);
    let access = &ack.list_of_read_access_results[0];
    assert_eq!(access.object_identifier, oid);
    assert_eq!(access.list_of_results.len(), cases.len());
    for (result, &(p, i, expected)) in access.list_of_results.iter().zip(cases) {
        assert_eq!(result.property_identifier, p);
        assert_eq!(result.property_array_index, i);
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
    use crate::handlers::rpm_budget::{handle_rpm_budgeted, RpmFailure};
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
        Err(RpmFailure::Work)
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
        Err(RpmFailure::Bytes)
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
    object
        .write_property(
            P::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(configured),
            None,
        )
        .unwrap();
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
            object
                .write_property(P::PRESENT_VALUE, None, PropertyValue::Enumerated(1), None)
                .unwrap();
            object
                .write_property(
                    P::CREDENTIAL_STATUS,
                    None,
                    PropertyValue::Enumerated(2),
                    None,
                )
                .unwrap();
        }
        write_common(&mut object, configured);
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Independent application-value bytes pin the existing projection.
        // Authentication_Factors is BACnetLIST and rejects any index.
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            (
                P::PRESENT_VALUE,
                None,
                Ok(if configured { &[0x91, 1] } else { &[0x91, 0] }),
            ),
            (
                P::PRESENT_VALUE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::CREDENTIAL_STATUS,
                None,
                Ok(if configured { &[0x91, 2] } else { &[0x91, 0] }),
            ),
            (
                P::CREDENTIAL_STATUS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::ASSIGNED_ACCESS_RIGHTS, None, Ok(&[0x21, 0])),
            (
                P::ASSIGNED_ACCESS_RIGHTS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::AUTHENTICATION_FACTORS, None, Ok(EMPTY)),
            (
                P::AUTHENTICATION_FACTORS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::AUTHENTICATION_FACTORS,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
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
                    0x91, 28, 0x91, 85, 0x92, 0x01, 0x08, 0x92, 0x01, 0x00, 0x92, 0x01, 0x01, 0x91,
                    111, 0x91, 81, 0x91, 103,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 8])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x91, 85])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x92, 0x01, 0x08])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x92, 0x01, 0x00])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x92, 0x01, 0x01])),
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
            // Global_Identifier is the Table 12-40 W row with no read arm;
            // Activation_Time is the Table 12-40 R row with no read arm.
            (P::GLOBAL_IDENTIFIER, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::GLOBAL_IDENTIFIER,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::ACTIVATION_TIME, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::ACTIVATION_TIME,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
        ];
        assert_cases(&db, oid, cases);
    }
}

#[test]
fn rpm_access_user_indexed_reads_and_bytes_are_unchanged() {
    for configured in [false, true] {
        let mut object = AccessUserObject::new(7, "USER-7").unwrap();
        if configured {
            object
                .write_property(P::PRESENT_VALUE, None, PropertyValue::Enumerated(1), None)
                .unwrap();
            object
                .write_property(P::USER_TYPE, None, PropertyValue::Enumerated(2), None)
                .unwrap();
        }
        write_common(&mut object, configured);
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Credentials is BACnetLIST and rejects any index.
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            (
                P::PRESENT_VALUE,
                None,
                Ok(if configured { &[0x91, 1] } else { &[0x91, 0] }),
            ),
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
            (P::CREDENTIALS, None, Ok(EMPTY)),
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
            (P::ASSIGNED_ACCESS_RIGHTS, None, Ok(&[0x21, 0])),
            (
                P::ASSIGNED_ACCESS_RIGHTS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
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
                    0x91, 28, 0x91, 85, 0x92, 0x01, 0x3E, 0x92, 0x01, 0x09, 0x92, 0x01, 0x00, 0x91,
                    111, 0x91, 81, 0x91, 103,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 8])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x91, 85])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x92, 0x01, 0x3E])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x92, 0x01, 0x09])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x92, 0x01, 0x00])),
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
            // Global_Identifier is the Table 12-38 W row with no read arm;
            // Members is the Table 12-38 O row with no read arm.
            (P::GLOBAL_IDENTIFIER, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::GLOBAL_IDENTIFIER,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::MEMBERS, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::MEMBERS,
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
            (
                P::GLOBAL_IDENTIFIER,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::POSITIVE_ACCESS_RULES, None, Ok(&[0x21, 0])),
            (
                P::POSITIVE_ACCESS_RULES,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::NEGATIVE_ACCESS_RULES, None, Ok(&[0x21, 0])),
            (
                P::NEGATIVE_ACCESS_RULES,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
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
                    0x91, 28, 0x92, 0x01, 0x43, 0x92, 0x01, 0x2E, 0x92, 0x01, 0x20, 0x91, 111,
                    0x91, 81, 0x91, 103,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 7])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x92, 0x01, 0x43])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x92, 0x01, 0x2E])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x92, 0x01, 0x20])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x91, 111])),
            (P::PROPERTY_LIST, Some(6), Ok(&[0x91, 81])),
            (P::PROPERTY_LIST, Some(7), Ok(&[0x91, 103])),
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
        // Update_Time reads back the unspecified Date/Time pair, the same
        // bytes as the Access Point Access_Event_Time default. The format
        // lists are BACnetLIST/BACnetARRAY rows the default array gate
        // rejects an index on, so every index overflows to NOT_AN_ARRAY.
        let unspec_time: &[u8] = &[0xa4, 0xff, 0xff, 0xff, 0xff, 0xb4, 0xff, 0xff, 0xff, 0xff];
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            (P::PRESENT_VALUE, None, Ok(&[0x91, 0])),
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
            (
                P::SUPPORTED_FORMATS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::SUPPORTED_FORMATS,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::SUPPORTED_FORMAT_CLASSES, None, Ok(EMPTY)),
            (
                P::SUPPORTED_FORMAT_CLASSES,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::SUPPORTED_FORMAT_CLASSES,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
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
