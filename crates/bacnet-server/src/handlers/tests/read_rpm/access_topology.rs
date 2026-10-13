use super::*;
use bacnet_objects::{
    access_control::{AccessDoorObject, AccessPointObject, AccessZoneObject},
    traits::BACnetObject,
};
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::AuthorizationMode;
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
    object
        .write_property(
            P::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(configured),
            None,
        )
        .unwrap();
}

#[test]
fn rpm_access_door_indexed_reads_and_bytes_are_unchanged() {
    for configured in [false, true] {
        let mut object = AccessDoorObject::new(7, "DOOR-7").unwrap();
        if configured {
            object
                .write_property(
                    P::PRESENT_VALUE,
                    None,
                    PropertyValue::Enumerated(1),
                    Some(8),
                )
                .unwrap();
            object
                .write_property(
                    P::RELINQUISH_DEFAULT,
                    None,
                    PropertyValue::Enumerated(1),
                    None,
                )
                .unwrap();
        }
        write_common(&mut object, configured);
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Independent application-value bytes pin the existing projection.
        // Priority_Array is BACnetARRAY (Table 12-30): index 0 is the slot
        // count, 1..=16 address slots, and 17 overflows. Door_Members is
        // BACnetLIST and rejects any index.
        let priority: &[u8] = if configured {
            &[
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x91, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00,
            ]
        } else {
            &[
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00,
            ]
        };
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
            (P::DOOR_STATUS, None, Ok(&[0x91, 0])),
            (
                P::DOOR_STATUS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::LOCK_STATUS, None, Ok(&[0x91, 0])),
            (
                P::LOCK_STATUS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            // Derived: the UNLOCK command leaves the door UNSECURED (#1148).
            (
                P::SECURED_STATUS,
                None,
                Ok(if configured { &[0x91, 1] } else { &[0x91, 0] }),
            ),
            (
                P::SECURED_STATUS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::DOOR_ALARM_STATE, None, Ok(&[0x91, 0])),
            (
                P::DOOR_ALARM_STATE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::DOOR_MEMBERS, None, Ok(EMPTY)),
            (P::DOOR_MEMBERS, Some(0), Ok(&[0x21, 0])),
            (
                P::DOOR_MEMBERS,
                Some(1),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (P::EVENT_STATE, None, Ok(&[0x91, 0])),
            (
                P::EVENT_STATE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::PRIORITY_ARRAY, None, Ok(priority)),
            (P::PRIORITY_ARRAY, Some(0), Ok(&[0x21, 16])),
            (P::PRIORITY_ARRAY, Some(1), Ok(&[0x00])),
            (
                P::PRIORITY_ARRAY,
                Some(8),
                Ok(if configured { &[0x91, 1] } else { &[0x00] }),
            ),
            (
                P::PRIORITY_ARRAY,
                Some(17),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::PRIORITY_ARRAY,
                Some(u32::MAX),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::RELINQUISH_DEFAULT,
                None,
                Ok(if configured { &[0x91, 1] } else { &[0x91, 0] }),
            ),
            (
                P::RELINQUISH_DEFAULT,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::STATUS_FLAGS,
                None,
                Ok(if configured {
                    &[0x82, 4, 0x10]
                } else {
                    &[0x82, 4, 0]
                }),
            ),
            (
                P::STATUS_FLAGS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::OUT_OF_SERVICE,
                None,
                Ok(if configured { &[0x11] } else { &[0x10] }),
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
            // The last twelve identifiers, from 234, are the rows #1149 added,
            // with the message texts and the inhibit pair (352, 355, 354)
            // after Event_Message_Texts (#1329).
            (
                P::PROPERTY_LIST,
                None,
                Ok(&[
                    0x91, 28, 0x91, 85, 0x91, 231, 0x91, 233, 0x91, 235, 0x91, 226, 0x91, 228,
                    0x91, 111, 0x91, 81, 0x91, 103, 0x91, 36, 0x91, 87, 0x91, 104, 0x91, 230, 0x91,
                    227, 0x91, 229, 0x92, 0x01, 0xAF, 0x91, 234, 0x91, 113, 0x91, 17, 0x91, 7,
                    0x91, 39, 0x91, 35, 0x91, 0, 0x91, 72, 0x91, 130, 0x92, 0x01, 0x5f, 0x92, 0x01,
                    0x60, 0x92, 0x01, 0x63, 0x92, 0x01, 0x62, 0x92, 0x01, 0x61, 0x92, 0x01, 0x64,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 32])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x91, 85])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x91, 231])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x91, 233])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x91, 235])),
            (P::PROPERTY_LIST, Some(6), Ok(&[0x91, 226])),
            (P::PROPERTY_LIST, Some(7), Ok(&[0x91, 228])),
            (P::PROPERTY_LIST, Some(8), Ok(&[0x91, 111])),
            (P::PROPERTY_LIST, Some(9), Ok(&[0x91, 81])),
            (P::PROPERTY_LIST, Some(10), Ok(&[0x91, 103])),
            (P::PROPERTY_LIST, Some(11), Ok(&[0x91, 36])),
            (P::PROPERTY_LIST, Some(12), Ok(&[0x91, 87])),
            (P::PROPERTY_LIST, Some(13), Ok(&[0x91, 104])),
            (P::PROPERTY_LIST, Some(14), Ok(&[0x91, 230])),
            (P::PROPERTY_LIST, Some(17), Ok(&[0x92, 0x01, 0xAF])),
            (P::PROPERTY_LIST, Some(18), Ok(&[0x91, 234])),
            (P::PROPERTY_LIST, Some(28), Ok(&[0x92, 0x01, 0x60])),
            (P::PROPERTY_LIST, Some(32), Ok(&[0x92, 0x01, 0x64])),
            (
                P::PROPERTY_LIST,
                Some(33),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::PROPERTY_LIST,
                Some(u32::MAX),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            // The required rows #1073 added: the default times in tenths
            // of a second, and the priority Present_Value comes from.
            (P::DOOR_PULSE_TIME, None, Ok(&[0x21, 50])),
            (
                P::DOOR_PULSE_TIME,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::DOOR_EXTENDED_PULSE_TIME, None, Ok(&[0x21, 150])),
            (P::DOOR_OPEN_TOO_LONG_TIME, None, Ok(&[0x22, 0x01, 0x2C])),
            (
                P::CURRENT_COMMAND_PRIORITY,
                None,
                Ok(if configured { &[0x21, 8] } else { &[0x00] }),
            ),
            (
                P::CURRENT_COMMAND_PRIORITY,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            // Door_Unlock_Delay_Time is a Table 12-30 O row with no read arm.
            (
                P::DOOR_UNLOCK_DELAY_TIME,
                None,
                Err(ErrorCode::UNKNOWN_PROPERTY),
            ),
        ];
        assert_cases(&db, oid, cases);
    }
}

#[test]
fn rpm_access_point_indexed_reads_and_bytes_are_unchanged() {
    for configured in [false, true] {
        let mut object = AccessPointObject::new(7, "AP-7").unwrap();
        write_common(&mut object, configured);
        if configured {
            // A client's DENY_ALL (2), once the application declares it
            // (#1307).
            object
                .set_supported_authorization_modes([
                    AuthorizationMode::AUTHORIZE,
                    AuthorizationMode::DENY_ALL,
                ])
                .unwrap();
            object
                .write_property(
                    P::AUTHORIZATION_MODE,
                    None,
                    PropertyValue::Enumerated(2),
                    None,
                )
                .unwrap();
        }
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Access_Event_Time is a BACnetTimeStamp: the unspecified date and
        // time framed as the datetime [2] choice (#1133). Going out of
        // service records OUT_OF_SERVICE (10) as transaction 1, stamped as
        // sequence number [1] 1 for want of a Device clock (#1248).
        let unspec_event_time: &[u8] = &[
            0x2e, 0xa4, 0xff, 0xff, 0xff, 0xff, 0xb4, 0xff, 0xff, 0xff, 0xff, 0x2f,
        ];
        let (access_event, access_event_tag, event_time): (&[u8], &[u8], &[u8]) = if configured {
            (&[0x91, 10], &[0x21, 1], &[0x19, 1])
        } else {
            (&[0x91, 0], &[0x21, 0], unspec_event_time)
        };
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            // Table 12-36 has no Present_Value row (#1064).
            (P::PRESENT_VALUE, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::PRESENT_VALUE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::ACCESS_EVENT, None, Ok(access_event)),
            (
                P::ACCESS_EVENT,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::ACCESS_EVENT_TAG, None, Ok(access_event_tag)),
            (
                P::ACCESS_EVENT_TAG,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::ACCESS_EVENT_TIME, None, Ok(event_time)),
            (
                P::ACCESS_EVENT_TIME,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::ACCESS_EVENT_TIME,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::ACCESS_DOORS, None, Ok(EMPTY)),
            (P::ACCESS_DOORS, Some(0), Ok(&[0x21, 0])),
            (
                P::ACCESS_DOORS,
                Some(1),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (P::EVENT_STATE, None, Ok(&[0x91, 0])),
            (
                P::EVENT_STATE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::STATUS_FLAGS,
                None,
                Ok(if configured {
                    &[0x82, 4, 0x10]
                } else {
                    &[0x82, 4, 0]
                }),
            ),
            (
                P::STATUS_FLAGS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::OUT_OF_SERVICE,
                None,
                Ok(if configured { &[0x11] } else { &[0x10] }),
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
                    0x91, 28, 0x91, 247, 0x92, 0x01, 0x42, 0x91, 250, 0x91, 246, 0x91, 36, 0x91,
                    111, 0x91, 81, 0x91, 103, 0x92, 0x01, 0x04, 0x91, 249, 0x91, 255, 0x92, 0x01,
                    0x21, 0x92, 0x01, 0x05, 0x91, 88, 0x91, 248,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 16])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x91, 247])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x92, 0x01, 0x42])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x91, 250])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x91, 246])),
            (P::PROPERTY_LIST, Some(6), Ok(&[0x91, 36])),
            (P::PROPERTY_LIST, Some(7), Ok(&[0x91, 111])),
            (P::PROPERTY_LIST, Some(8), Ok(&[0x91, 81])),
            (P::PROPERTY_LIST, Some(9), Ok(&[0x91, 103])),
            (P::PROPERTY_LIST, Some(10), Ok(&[0x92, 0x01, 0x04])),
            (P::PROPERTY_LIST, Some(11), Ok(&[0x91, 249])),
            (P::PROPERTY_LIST, Some(15), Ok(&[0x91, 88])),
            // Access_Event_Authentication_Factor (248), served since #1132.
            (P::PROPERTY_LIST, Some(16), Ok(&[0x91, 248])),
            (
                P::PROPERTY_LIST,
                Some(17),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::PROPERTY_LIST,
                Some(u32::MAX),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            // The Table 12-36 required rows #1284 added: DISABLED while out
            // of service, READY otherwise, and the no-credential reference.
            (
                P::AUTHENTICATION_STATUS,
                None,
                Ok(if configured { &[0x91, 2] } else { &[0x91, 1] }),
            ),
            (
                P::AUTHENTICATION_STATUS,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::ACCESS_EVENT_CREDENTIAL,
                None,
                Ok(&[0x1C, 0x08, 0x3F, 0xFF, 0xFF]),
            ),
            (
                P::ACCESS_EVENT_CREDENTIAL,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            // The UNDEFINED factor until an event carries one (#1132).
            (
                P::ACCESS_EVENT_AUTHENTICATION_FACTOR,
                None,
                Ok(&[0x09, 0x00, 0x19, 0x00, 0x28]),
            ),
            (
                P::ACCESS_EVENT_AUTHENTICATION_FACTOR,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            // The Table 12-36 required rows #1307 added: one policy, in
            // effect; AUTHORIZE until written; priority 16 until set.
            (P::ACTIVE_AUTHENTICATION_POLICY, None, Ok(&[0x21, 1])),
            (P::NUMBER_OF_AUTHENTICATION_POLICIES, None, Ok(&[0x21, 1])),
            (
                P::AUTHORIZATION_MODE,
                None,
                Ok(if configured { &[0x91, 2] } else { &[0x91, 0] }),
            ),
            (P::PRIORITY_FOR_WRITING, None, Ok(&[0x21, 16])),
            (
                P::ACTIVE_AUTHENTICATION_POLICY,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::NUMBER_OF_AUTHENTICATION_POLICIES,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::AUTHORIZATION_MODE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::PRIORITY_FOR_WRITING,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
        ];
        assert_cases(&db, oid, cases);
    }
}

#[test]
fn rpm_access_zone_indexed_reads_and_bytes_are_unchanged() {
    for configured in [false, true] {
        let mut object = AccessZoneObject::new(7, "ZONE-7").unwrap();
        if configured {
            object
                .write_property(
                    P::GLOBAL_IDENTIFIER,
                    None,
                    PropertyValue::Unsigned(99),
                    None,
                )
                .unwrap();
        }
        write_common(&mut object, configured);
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            // Table 12-37 has no Present_Value or Access_Doors row (#1064).
            (P::PRESENT_VALUE, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::PRESENT_VALUE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::GLOBAL_IDENTIFIER,
                None,
                Ok(if configured { &[0x21, 99] } else { &[0x21, 0] }),
            ),
            (
                P::GLOBAL_IDENTIFIER,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::OCCUPANCY_COUNT, None, Ok(&[0x21, 0])),
            (
                P::OCCUPANCY_COUNT,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            // Access_Doors is an array identifier wherever it appears, so an
            // index passes the gate and the zone, which lacks the row,
            // answers UNKNOWN_PROPERTY (#1169).
            (P::ACCESS_DOORS, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (P::ACCESS_DOORS, Some(0), Err(ErrorCode::UNKNOWN_PROPERTY)),
            (P::ACCESS_DOORS, Some(1), Err(ErrorCode::UNKNOWN_PROPERTY)),
            (P::ENTRY_POINTS, None, Ok(EMPTY)),
            (
                P::ENTRY_POINTS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::ENTRY_POINTS,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::EXIT_POINTS, None, Ok(EMPTY)),
            (
                P::EXIT_POINTS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::EXIT_POINTS,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::STATUS_FLAGS,
                None,
                Ok(if configured {
                    &[0x82, 4, 0x10]
                } else {
                    &[0x82, 4, 0]
                }),
            ),
            (
                P::STATUS_FLAGS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::OUT_OF_SERVICE,
                None,
                Ok(if configured { &[0x11] } else { &[0x10] }),
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
                    0x91, 28, 0x92, 0x01, 0x43, 0x92, 0x01, 0x22, 0x92, 0x01, 0x0c, 0x92, 0x01,
                    0x0d, 0x91, 111, 0x91, 81, 0x91, 103, 0x92, 0x01, 0x28, 0x91, 36, 0x92, 0x01,
                    0x24, 0x91, 176, 0x92, 0x01, 0x29, 0x92, 0x01, 0x26,
                    // The event rows #1305 added, and #1329's three after
                    // Event_Message_Texts.
                    0x91, 113, 0x91, 17, 0x91, 7, 0x91, 35, 0x91, 0, 0x91, 72, 0x91, 130, 0x92,
                    0x01, 0x5f, 0x92, 0x01, 0x60, 0x92, 0x01, 0x63, 0x92, 0x01, 0x62, 0x92, 0x01,
                    0x61, 0x92, 0x01, 0x64,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 27])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x92, 0x01, 0x43])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x92, 0x01, 0x22])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x92, 0x01, 0x0c])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x92, 0x01, 0x0d])),
            (P::PROPERTY_LIST, Some(6), Ok(&[0x91, 111])),
            (P::PROPERTY_LIST, Some(7), Ok(&[0x91, 81])),
            (P::PROPERTY_LIST, Some(8), Ok(&[0x91, 103])),
            (P::PROPERTY_LIST, Some(9), Ok(&[0x92, 0x01, 0x28])),
            (P::PROPERTY_LIST, Some(14), Ok(&[0x92, 0x01, 0x26])),
            (P::PROPERTY_LIST, Some(15), Ok(&[0x91, 113])),
            (P::PROPERTY_LIST, Some(24), Ok(&[0x92, 0x01, 0x63])),
            (P::PROPERTY_LIST, Some(27), Ok(&[0x92, 0x01, 0x64])),
            (
                P::PROPERTY_LIST,
                Some(28),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::PROPERTY_LIST,
                Some(u32::MAX),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            // The Table 12-37 rows #1284 added: a new zone counts from zero
            // with no limits, so Occupancy_State is NORMAL.
            (P::OCCUPANCY_STATE, None, Ok(&[0x91, 0])),
            (
                P::OCCUPANCY_STATE,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::EVENT_STATE, None, Ok(&[0x91, 0])),
            (P::OCCUPANCY_COUNT_ENABLE, None, Ok(&[0x11])),
            (P::ADJUST_VALUE, None, Ok(&[0x31, 0])),
            (
                P::ADJUST_VALUE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::OCCUPANCY_UPPER_LIMIT, None, Ok(&[0x21, 0])),
            (P::OCCUPANCY_LOWER_LIMIT, None, Ok(&[0x21, 0])),
        ];
        assert_cases(&db, oid, cases);
    }
}

#[path = "access_point_policies.rs"]
mod policies;
