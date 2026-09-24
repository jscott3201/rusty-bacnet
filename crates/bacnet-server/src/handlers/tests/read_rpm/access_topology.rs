use super::*;
use bacnet_objects::{
    access_control::{AccessDoorObject, AccessPointObject, AccessZoneObject},
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
            (P::SECURED_STATUS, None, Ok(&[0x91, 0])),
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
            (
                P::DOOR_MEMBERS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::DOOR_MEMBERS,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
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
            (
                P::PROPERTY_LIST,
                None,
                Ok(&[
                    0x91, 28, 0x91, 85, 0x91, 231, 0x91, 233, 0x91, 235, 0x91, 226, 0x91, 228,
                    0x91, 111, 0x91, 81, 0x91, 103, 0x91, 36, 0x91, 87, 0x91, 104,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 13])),
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
            (
                P::PROPERTY_LIST,
                Some(14),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::PROPERTY_LIST,
                Some(u32::MAX),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            // Unserved Table 12-30 rows stay unknown.
            (P::DOOR_PULSE_TIME, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::DOOR_PULSE_TIME,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::CURRENT_COMMAND_PRIORITY,
                None,
                Err(ErrorCode::UNKNOWN_PROPERTY),
            ),
            (
                P::CURRENT_COMMAND_PRIORITY,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
        ];
        assert_cases(&db, oid, cases);
    }
}

#[test]
fn rpm_access_point_indexed_reads_and_bytes_are_unchanged() {
    for configured in [false, true] {
        let mut object = AccessPointObject::new(7, "AP-7").unwrap();
        if configured {
            object
                .write_property(P::PRESENT_VALUE, None, PropertyValue::Enumerated(2), None)
                .unwrap();
        }
        write_common(&mut object, configured);
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Access_Event_Time reads back the unspecified Date/Time pair, the
        // same bytes as the LoadControl Start_Time default.
        let unspec_event_time: &[u8] =
            &[0xa4, 0xff, 0xff, 0xff, 0xff, 0xb4, 0xff, 0xff, 0xff, 0xff];
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            (
                P::PRESENT_VALUE,
                None,
                Ok(if configured { &[0x91, 2] } else { &[0x91, 0] }),
            ),
            (
                P::PRESENT_VALUE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::ACCESS_EVENT, None, Ok(&[0x91, 0])),
            (
                P::ACCESS_EVENT,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::ACCESS_EVENT_TAG, None, Ok(&[0x21, 0])),
            (
                P::ACCESS_EVENT_TAG,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::ACCESS_EVENT_TIME, None, Ok(unspec_event_time)),
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
            (
                P::ACCESS_DOORS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::ACCESS_DOORS,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
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
                    0x91, 28, 0x91, 85, 0x91, 247, 0x92, 0x01, 0x42, 0x91, 250, 0x91, 246, 0x91,
                    36, 0x91, 111, 0x91, 81, 0x91, 103,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 10])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x91, 85])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x91, 247])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x92, 0x01, 0x42])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x91, 250])),
            (P::PROPERTY_LIST, Some(6), Ok(&[0x91, 246])),
            (P::PROPERTY_LIST, Some(7), Ok(&[0x91, 36])),
            (P::PROPERTY_LIST, Some(8), Ok(&[0x91, 111])),
            (P::PROPERTY_LIST, Some(9), Ok(&[0x91, 81])),
            (P::PROPERTY_LIST, Some(10), Ok(&[0x91, 103])),
            (
                P::PROPERTY_LIST,
                Some(11),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::PROPERTY_LIST,
                Some(u32::MAX),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            // Authentication_Status is the Table 12-36 R row with no arm.
            (
                P::AUTHENTICATION_STATUS,
                None,
                Err(ErrorCode::UNKNOWN_PROPERTY),
            ),
            (
                P::AUTHENTICATION_STATUS,
                Some(1),
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
                .write_property(P::PRESENT_VALUE, None, PropertyValue::Enumerated(1), None)
                .unwrap();
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
            (P::ACCESS_DOORS, None, Ok(EMPTY)),
            (
                P::ACCESS_DOORS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::ACCESS_DOORS,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
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
                    0x91, 28, 0x91, 85, 0x92, 0x01, 0x43, 0x92, 0x01, 0x22, 0x91, 246, 0x92, 0x01,
                    0x0c, 0x92, 0x01, 0x0d, 0x91, 111, 0x91, 81, 0x91, 103,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 10])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x91, 85])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x92, 0x01, 0x43])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x92, 0x01, 0x22])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x91, 246])),
            (P::PROPERTY_LIST, Some(6), Ok(&[0x92, 0x01, 0x0c])),
            (P::PROPERTY_LIST, Some(7), Ok(&[0x92, 0x01, 0x0d])),
            (P::PROPERTY_LIST, Some(8), Ok(&[0x91, 111])),
            (P::PROPERTY_LIST, Some(9), Ok(&[0x91, 81])),
            (P::PROPERTY_LIST, Some(10), Ok(&[0x91, 103])),
            (
                P::PROPERTY_LIST,
                Some(11),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::PROPERTY_LIST,
                Some(u32::MAX),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            // Occupancy_State is the Table 12-37 row with no arm.
            (P::OCCUPANCY_STATE, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::OCCUPANCY_STATE,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
        ];
        assert_cases(&db, oid, cases);
    }
}
