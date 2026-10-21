use super::*;
use bacnet_objects::{
    elevator::{ElevatorGroupObject, EscalatorObject},
    traits::BACnetObject,
};
use bacnet_types::constructed::{
    BACnetLandingCallStatus, LandingCallCommand, PropertyReference, ReadAccessSpecification,
};
use bacnet_types::enums::LiftCarDirection;
use bacnet_types::primitives::PropertyValue;
use PropertyIdentifier as P;

mod lift;

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
                    if class == ErrorClass::PROPERTY && code == expected));
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

fn write_description(object: &mut dyn BACnetObject) {
    object
        .write_property(
            P::DESCRIPTION,
            None,
            PropertyValue::CharacterString("long elevator label".repeat(100)),
            None,
        )
        .unwrap();
}

fn write_common(object: &mut dyn BACnetObject, configured: bool) {
    write_description(object);
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
fn rpm_elevator_group_indexed_reads_and_bytes_are_unchanged() {
    for configured in [false, true] {
        let mut object = ElevatorGroupObject::new(7, "EG-7").unwrap();
        if configured {
            let lift1 = ObjectIdentifier::new(ObjectType::LIFT, 1).unwrap();
            let lift2 = ObjectIdentifier::new(ObjectType::LIFT, 2).unwrap();
            object.add_member(lift1);
            object.add_member(lift2);
            object
                .set_machine_room_id(
                    ObjectIdentifier::new(ObjectType::POSITIVE_INTEGER_VALUE, 9).unwrap(),
                )
                .unwrap();
            object
                .write_property(P::GROUP_ID, None, PropertyValue::Unsigned(47), None)
                .unwrap();
            object
                .write_property(P::GROUP_MODE, None, PropertyValue::Enumerated(2), None)
                .unwrap();
            // BACnetLandingCallStatus: floor [0] 5, direction [1] UP.
            object
                .write_property(
                    P::LANDING_CALL_CONTROL,
                    None,
                    PropertyValue::ApplicationData(vec![0x09, 0x05, 0x19, 0x03]),
                    None,
                )
                .unwrap();
            object
                .set_landing_calls(vec![
                    BACnetLandingCallStatus {
                        floor_number: 2,
                        command: LandingCallCommand::Direction(LiftCarDirection::DOWN),
                        floor_text: None,
                    },
                    BACnetLandingCallStatus {
                        floor_number: 9,
                        command: LandingCallCommand::Destination(1),
                        floor_text: Some("L".into()),
                    },
                ])
                .unwrap();
        }
        // Elevator Group has no Out_Of_Service (Table 12-76).
        write_description(&mut object);
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Independent application-value bytes pin the existing projection.
        // Group_Members is BACnetARRAY (Table 12-76): index 0 is the member
        // count and index n one member (#1034); Landing_Calls is BACnetLIST
        // and rejects an index. LIFT is object type 59, so member references
        // encode as 0xC4 0x0E 0xC0 0x00 0x0N.
        let members: &[u8] = if configured {
            &[0xC4, 0x0E, 0xC0, 0x00, 0x01, 0xC4, 0x0E, 0xC0, 0x00, 0x02]
        } else {
            EMPTY
        };
        let first_member: ExpectedRead = if configured {
            Ok(&[0xC4, 0x0E, 0xC0, 0x00, 0x01])
        } else {
            Err(ErrorCode::INVALID_ARRAY_INDEX)
        };
        let second_member: ExpectedRead = if configured {
            Ok(&[0xC4, 0x0E, 0xC0, 0x00, 0x02])
        } else {
            Err(ErrorCode::INVALID_ARRAY_INDEX)
        };
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            // POSITIVE_INTEGER_VALUE is object type 48 (0x0C000000); with no
            // machine room number the instance is 4194303 (0x3FFFFF).
            (
                P::MACHINE_ROOM_ID,
                None,
                Ok(if configured {
                    &[0xC4, 0x0C, 0x00, 0x00, 0x09]
                } else {
                    &[0xC4, 0x0C, 0x3F, 0xFF, 0xFF]
                }),
            ),
            (
                P::MACHINE_ROOM_ID,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::GROUP_ID,
                None,
                Ok(if configured { &[0x21, 47] } else { &[0x21, 0] }),
            ),
            (
                P::GROUP_ID,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::GROUP_MEMBERS, None, Ok(members)),
            (
                P::GROUP_MEMBERS,
                Some(0),
                Ok(if configured { &[0x21, 2] } else { &[0x21, 0] }),
            ),
            (P::GROUP_MEMBERS, Some(1), first_member),
            (P::GROUP_MEMBERS, Some(2), second_member),
            (
                P::GROUP_MEMBERS,
                Some(3),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::GROUP_MEMBERS,
                Some(u32::MAX),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::GROUP_MODE,
                None,
                Ok(if configured { &[0x91, 2] } else { &[0x91, 0] }),
            ),
            (
                P::GROUP_MODE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            // A BACnetLIST of BACnetLandingCallStatus: concatenated
            // context-tagged SEQUENCEs, empty until the application sets one.
            (
                P::LANDING_CALLS,
                None,
                Ok(if configured {
                    &[
                        0x09, 0x02, 0x19, 0x04, 0x09, 0x09, 0x29, 0x01, 0x3A, 0x00, 0x4C,
                    ]
                } else {
                    EMPTY
                }),
            ),
            (
                P::LANDING_CALLS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::LANDING_CALLS,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::LANDING_CALL_CONTROL,
                None,
                Ok(if configured {
                    &[0x09, 0x05, 0x19, 0x03]
                } else {
                    &[0x09, 0x00, 0x19, 0x00]
                }),
            ),
            (
                P::LANDING_CALL_CONTROL,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::PROPERTY_LIST,
                None,
                Ok(&[
                    0x91, 28, 0x92, 0x01, 0xDA, 0x92, 0x01, 0xD1, 0x92, 0x01, 0x59, 0x92, 0x01,
                    0xD3, 0x92, 0x01, 0xD6, 0x92, 0x01, 0xD7,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 7])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x92, 0x01, 0xDA])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x92, 0x01, 0xD1])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x92, 0x01, 0x59])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x92, 0x01, 0xD3])),
            (P::PROPERTY_LIST, Some(6), Ok(&[0x92, 0x01, 0xD6])),
            (P::PROPERTY_LIST, Some(7), Ok(&[0x92, 0x01, 0xD7])),
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
            // Table 12-76 defines none of these, so the group doesn't serve
            // them (#997).
            (P::STATUS_FLAGS, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::STATUS_FLAGS,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::OUT_OF_SERVICE, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (P::RELIABILITY, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
        ];
        assert_cases(&db, oid, cases);
    }
}

#[test]
fn rpm_escalator_indexed_reads_and_bytes_are_unchanged() {
    for configured in [false, true] {
        let mut object = EscalatorObject::new(7, "ESC-7").unwrap();
        if configured {
            object
                .set_elevator_group(ObjectIdentifier::new(ObjectType::ELEVATOR_GROUP, 3).unwrap())
                .unwrap();
            object.set_group_id(47);
            object.set_installation_id(255);
            for (p, value) in [
                (P::ESCALATOR_MODE, PropertyValue::Enumerated(3)),
                (
                    P::FAULT_SIGNALS,
                    PropertyValue::List(vec![
                        PropertyValue::Enumerated(0),
                        PropertyValue::Enumerated(1024),
                    ]),
                ),
                (P::ENERGY_METER, PropertyValue::Real(18.75)),
                (P::POWER_MODE, PropertyValue::Boolean(true)),
                (P::OPERATION_DIRECTION, PropertyValue::Enumerated(2)),
                (P::PASSENGER_ALARM, PropertyValue::Boolean(true)),
            ] {
                object.write_property(p, None, value, None).unwrap();
            }
        }
        write_common(&mut object, configured);
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Independent application-value bytes pin the Table 12-78 projection.
        // Fault_Signals is BACnetLIST (Table 12-78), so any index is
        // PROPERTY_IS_NOT_AN_ARRAY. 18.75f32 encodes as 0x41960000.
        let faults: &[u8] = if configured {
            &[0x91, 0, 0x92, 0x04, 0x00]
        } else {
            EMPTY
        };
        let not_array = Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY);
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            (
                P::STATUS_FLAGS,
                None,
                Ok(if configured {
                    &[0x82, 4, 0x10]
                } else {
                    &[0x82, 4, 0]
                }),
            ),
            (P::STATUS_FLAGS, Some(0), not_array),
            // ELEVATOR_GROUP is object type 57 (0x0E400000); with no group
            // the instance is 4194303 (0x3FFFFF).
            (
                P::ELEVATOR_GROUP,
                None,
                Ok(if configured {
                    &[0xC4, 0x0E, 0x40, 0x00, 0x03]
                } else {
                    &[0xC4, 0x0E, 0x7F, 0xFF, 0xFF]
                }),
            ),
            (P::ELEVATOR_GROUP, Some(1), not_array),
            (
                P::GROUP_ID,
                None,
                Ok(if configured { &[0x21, 47] } else { &[0x21, 0] }),
            ),
            (P::GROUP_ID, Some(0), not_array),
            (
                P::INSTALLATION_ID,
                None,
                Ok(if configured {
                    &[0x21, 0xFF]
                } else {
                    &[0x21, 0]
                }),
            ),
            (P::INSTALLATION_ID, Some(1), not_array),
            (
                P::POWER_MODE,
                None,
                Ok(if configured { &[0x11] } else { &[0x10] }),
            ),
            (P::POWER_MODE, Some(0), not_array),
            (
                P::OPERATION_DIRECTION,
                None,
                Ok(if configured { &[0x91, 2] } else { &[0x91, 0] }),
            ),
            (P::OPERATION_DIRECTION, Some(0), not_array),
            (
                P::ESCALATOR_MODE,
                None,
                Ok(if configured { &[0x91, 3] } else { &[0x91, 0] }),
            ),
            (P::ESCALATOR_MODE, Some(0), not_array),
            (
                P::ENERGY_METER,
                None,
                Ok(if configured {
                    &[0x44, 0x41, 0x96, 0x00, 0x00]
                } else {
                    &[0x44, 0, 0, 0, 0]
                }),
            ),
            (P::ENERGY_METER, Some(0), not_array),
            // An uninitialized BACnetDeviceObjectReference: no device [0],
            // object [1] Accumulator (type 23) instance 4194303.
            (
                P::ENERGY_METER_REF,
                None,
                Ok(&[0x1C, 0x05, 0xFF, 0xFF, 0xFF]),
            ),
            (P::ENERGY_METER_REF, Some(0), not_array),
            (P::RELIABILITY, None, Ok(&[0x91, 0])),
            (P::RELIABILITY, Some(0), not_array),
            (
                P::OUT_OF_SERVICE,
                None,
                Ok(if configured { &[0x11] } else { &[0x10] }),
            ),
            (P::OUT_OF_SERVICE, Some(0), not_array),
            (P::FAULT_SIGNALS, None, Ok(faults)),
            (P::FAULT_SIGNALS, Some(0), not_array),
            (P::FAULT_SIGNALS, Some(1), not_array),
            (
                P::PASSENGER_ALARM,
                None,
                Ok(if configured { &[0x11] } else { &[0x10] }),
            ),
            (P::PASSENGER_ALARM, Some(0), not_array),
            (
                P::PROPERTY_LIST,
                None,
                Ok(&[
                    0x91, 28, 0x91, 111, 0x92, 0x01, 0xCB, 0x92, 0x01, 0xD1, 0x92, 0x01, 0xD5,
                    0x92, 0x01, 0xDF, 0x92, 0x01, 0xDD, 0x92, 0x01, 0xCE, 0x92, 0x01, 0xCC, 0x92,
                    0x01, 0xCD, 0x91, 103, 0x91, 81, 0x92, 0x01, 0xCF, 0x92, 0x01, 0xDE,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 14])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x92, 0x01, 0xCB])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x92, 0x01, 0xD5])),
            (P::PROPERTY_LIST, Some(14), Ok(&[0x92, 0x01, 0xDE])),
            (
                P::PROPERTY_LIST,
                Some(15),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::PROPERTY_LIST,
                Some(u32::MAX),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            // Unserved rows stay unknown.
            (P::EVENT_STATE, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (P::CAR_POSITION, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (P::CAR_POSITION, Some(1), not_array),
        ];
        assert_cases(&db, oid, cases);
    }
}
