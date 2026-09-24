use super::*;
use bacnet_objects::{
    elevator::{ElevatorGroupObject, EscalatorObject, LiftObject},
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
            PropertyValue::CharacterString("long elevator label".repeat(100)),
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
fn rpm_elevator_group_indexed_reads_and_bytes_are_unchanged() {
    for configured in [false, true] {
        let mut object = ElevatorGroupObject::new(7, "EG-7").unwrap();
        if configured {
            let lift1 = ObjectIdentifier::new(ObjectType::LIFT, 1).unwrap();
            let lift2 = ObjectIdentifier::new(ObjectType::LIFT, 2).unwrap();
            object.add_member(lift1);
            object.add_member(lift2);
            object
                .write_property(P::GROUP_ID, None, PropertyValue::Unsigned(47), None)
                .unwrap();
            object
                .write_property(P::GROUP_MODE, None, PropertyValue::Enumerated(2), None)
                .unwrap();
            object
                .write_property(
                    P::LANDING_CALL_CONTROL,
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
        // Group_Members is BACnetARRAY (Table 12-76), so the service gate
        // admits an index and the arm returns the whole value; Landing_Calls
        // is BACnetLIST and rejects one. LIFT is object type 59, so member
        // references encode as 0xC4 0x0E 0xC0 0x00 0x0N.
        let members: &[u8] = if configured {
            &[0xC4, 0x0E, 0xC0, 0x00, 0x01, 0xC4, 0x0E, 0xC0, 0x00, 0x02]
        } else {
            EMPTY
        };
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
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
            (P::GROUP_MEMBERS, Some(0), Ok(members)),
            (P::GROUP_MEMBERS, Some(1), Ok(members)),
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
            (P::LANDING_CALLS, None, Ok(&[0x21, 0])),
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
                Ok(if configured { &[0x91, 1] } else { &[0x91, 0] }),
            ),
            (
                P::LANDING_CALL_CONTROL,
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
                    0x91, 28, 0x92, 0x01, 0xD1, 0x92, 0x01, 0x59, 0x92, 0x01, 0xD3, 0x92, 0x01,
                    0xD6, 0x92, 0x01, 0xD7, 0x91, 111, 0x91, 81, 0x91, 103,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 9])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x92, 0x01, 0xD1])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x92, 0x01, 0x59])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x92, 0x01, 0xD3])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x92, 0x01, 0xD6])),
            (P::PROPERTY_LIST, Some(6), Ok(&[0x92, 0x01, 0xD7])),
            (P::PROPERTY_LIST, Some(7), Ok(&[0x91, 111])),
            (P::PROPERTY_LIST, Some(8), Ok(&[0x91, 81])),
            (P::PROPERTY_LIST, Some(9), Ok(&[0x91, 103])),
            (
                P::PROPERTY_LIST,
                Some(10),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::PROPERTY_LIST,
                Some(u32::MAX),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            // Unserved ElevatorGroup table rows stay unknown.
            (P::MACHINE_ROOM_ID, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::MACHINE_ROOM_ID,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
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
                .write_property(P::ESCALATOR_MODE, None, PropertyValue::Enumerated(3), None)
                .unwrap();
            object
                .write_property(
                    P::FAULT_SIGNALS,
                    None,
                    PropertyValue::List(vec![
                        PropertyValue::Enumerated(0),
                        PropertyValue::Enumerated(1024),
                    ]),
                    None,
                )
                .unwrap();
            object
                .write_property(P::ENERGY_METER, None, PropertyValue::Real(18.75), None)
                .unwrap();
            object
                .write_property(P::POWER_MODE, None, PropertyValue::Boolean(true), None)
                .unwrap();
            object
                .write_property(
                    P::OPERATION_DIRECTION,
                    None,
                    PropertyValue::Enumerated(2),
                    None,
                )
                .unwrap();
            object
                .write_property(P::PASSENGER_ALARM, None, PropertyValue::Boolean(true), None)
                .unwrap();
        }
        write_common(&mut object, configured);
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Independent application-value bytes pin the existing projection.
        // Fault_Signals is BACnetLIST (Table 12-78), so any index is
        // PROPERTY_IS_NOT_AN_ARRAY. 18.75f32 encodes as 0x41960000.
        let faults: &[u8] = if configured {
            &[0x91, 0, 0x92, 0x04, 0x00]
        } else {
            EMPTY
        };
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            (
                P::ESCALATOR_MODE,
                None,
                Ok(if configured { &[0x91, 3] } else { &[0x91, 0] }),
            ),
            (
                P::ESCALATOR_MODE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::FAULT_SIGNALS, None, Ok(faults)),
            (
                P::FAULT_SIGNALS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::FAULT_SIGNALS,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::ENERGY_METER,
                None,
                Ok(if configured {
                    &[0x44, 0x41, 0x96, 0x00, 0x00]
                } else {
                    &[0x44, 0, 0, 0, 0]
                }),
            ),
            (
                P::ENERGY_METER,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::ENERGY_METER_REF, None, Ok(&[0x60])),
            (
                P::ENERGY_METER_REF,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::POWER_MODE,
                None,
                Ok(if configured { &[0x11] } else { &[0x10] }),
            ),
            (
                P::POWER_MODE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::OPERATION_DIRECTION,
                None,
                Ok(if configured { &[0x91, 2] } else { &[0x91, 0] }),
            ),
            (
                P::OPERATION_DIRECTION,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::PASSENGER_ALARM,
                None,
                Ok(if configured { &[0x11] } else { &[0x10] }),
            ),
            (
                P::PASSENGER_ALARM,
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
                    0x91, 28, 0x92, 0x01, 0xCE, 0x92, 0x01, 0xCF, 0x92, 0x01, 0xCC, 0x92, 0x01,
                    0xCD, 0x92, 0x01, 0xDF, 0x92, 0x01, 0xDD, 0x92, 0x01, 0xDE, 0x91, 111, 0x91,
                    81, 0x91, 103,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 11])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x92, 0x01, 0xCE])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x92, 0x01, 0xCF])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x92, 0x01, 0xCC])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x92, 0x01, 0xCD])),
            (P::PROPERTY_LIST, Some(6), Ok(&[0x92, 0x01, 0xDF])),
            (P::PROPERTY_LIST, Some(7), Ok(&[0x92, 0x01, 0xDD])),
            (P::PROPERTY_LIST, Some(8), Ok(&[0x92, 0x01, 0xDE])),
            (P::PROPERTY_LIST, Some(9), Ok(&[0x91, 111])),
            (P::PROPERTY_LIST, Some(10), Ok(&[0x91, 81])),
            (P::PROPERTY_LIST, Some(11), Ok(&[0x91, 103])),
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
            // Unserved Escalator table rows stay unknown.
            (P::ELEVATOR_GROUP, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::ELEVATOR_GROUP,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::INSTALLATION_ID, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::INSTALLATION_ID,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
        ];
        assert_cases(&db, oid, cases);
    }
}

#[test]
fn rpm_lift_indexed_reads_and_bytes_are_unchanged() {
    for configured in [false, true] {
        let mut object = LiftObject::new(7, "LIFT-7", 2).unwrap();
        if configured {
            object
                .write_property(P::TRACKING_VALUE, None, PropertyValue::Unsigned(2), None)
                .unwrap();
            object
                .write_property(P::CAR_POSITION, None, PropertyValue::Unsigned(2), None)
                .unwrap();
            object
                .write_property(
                    P::CAR_MOVING_DIRECTION,
                    None,
                    PropertyValue::Enumerated(2),
                    None,
                )
                .unwrap();
            object
                .write_property(P::CAR_LOAD, None, PropertyValue::Unsigned(50), None)
                .unwrap();
        }
        write_common(&mut object, configured);
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Independent application-value bytes pin the existing projection.
        // Floor_Number aliases Tracking_Value, so both move together.
        // "Floor N" encodes with an extended length octet: 0x75 0x08 0x00 +
        // text (tag 7, ANSI).
        let floor_text: &[u8] = &[
            0x75, 0x08, 0x00, b'F', b'l', b'o', b'o', b'r', b' ', b'1', 0x75, 0x08, 0x00, b'F',
            b'l', b'o', b'o', b'r', b' ', b'2',
        ];
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            (
                P::TRACKING_VALUE,
                None,
                Ok(if configured { &[0x21, 2] } else { &[0x21, 1] }),
            ),
            (
                P::TRACKING_VALUE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::CAR_POSITION,
                None,
                Ok(if configured { &[0x21, 2] } else { &[0x21, 1] }),
            ),
            (
                P::CAR_POSITION,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::CAR_MOVING_DIRECTION,
                None,
                Ok(if configured { &[0x91, 2] } else { &[0x91, 1] }),
            ),
            (
                P::CAR_MOVING_DIRECTION,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::CAR_DOOR_STATUS, None, Ok(EMPTY)),
            (
                P::CAR_DOOR_STATUS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::CAR_LOAD,
                None,
                Ok(if configured { &[0x21, 50] } else { &[0x21, 0] }),
            ),
            (
                P::CAR_LOAD,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::LANDING_DOOR_STATUS, None, Ok(&[0x21, 2])),
            (
                P::LANDING_DOOR_STATUS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::FLOOR_TEXT, None, Ok(floor_text)),
            (
                P::FLOOR_TEXT,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::FLOOR_TEXT,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::ENERGY_METER, None, Ok(&[0x44, 0, 0, 0, 0])),
            (
                P::ENERGY_METER,
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
                P::FLOOR_NUMBER,
                None,
                Ok(if configured { &[0x21, 2] } else { &[0x21, 1] }),
            ),
            (
                P::FLOOR_NUMBER,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::PROPERTY_LIST,
                None,
                Ok(&[
                    0x91, 28, 0x91, 164, 0x92, 0x01, 0xCA, 0x92, 0x01, 0xC9, 0x92, 0x01, 0xC2,
                    0x92, 0x01, 0xC6, 0x92, 0x01, 0xD8, 0x92, 0x01, 0xD0, 0x92, 0x01, 0xCC, 0x91,
                    111, 0x91, 81, 0x91, 103, 0x92, 0x01, 0xFA,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 13])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x91, 164])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x92, 0x01, 0xCA])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x92, 0x01, 0xC9])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x92, 0x01, 0xC2])),
            (P::PROPERTY_LIST, Some(6), Ok(&[0x92, 0x01, 0xC6])),
            (P::PROPERTY_LIST, Some(7), Ok(&[0x92, 0x01, 0xD8])),
            (P::PROPERTY_LIST, Some(8), Ok(&[0x92, 0x01, 0xD0])),
            (P::PROPERTY_LIST, Some(9), Ok(&[0x92, 0x01, 0xCC])),
            (P::PROPERTY_LIST, Some(10), Ok(&[0x91, 111])),
            (P::PROPERTY_LIST, Some(11), Ok(&[0x91, 81])),
            (P::PROPERTY_LIST, Some(12), Ok(&[0x91, 103])),
            (P::PROPERTY_LIST, Some(13), Ok(&[0x92, 0x01, 0xFA])),
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
            // Unserved Lift table rows stay unknown.
            (P::PASSENGER_ALARM, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::PASSENGER_ALARM,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::FAULT_SIGNALS, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::FAULT_SIGNALS,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
        ];
        assert_cases(&db, oid, cases);
    }
}
