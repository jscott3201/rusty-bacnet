use super::*;
use bacnet_objects::{
    lighting::{BinaryLightingOutputObject, LightingOutputObject},
    traits::BACnetObject,
};
use bacnet_services::common::PropertyReference;
use bacnet_services::rpm::ReadAccessSpecification;
use bacnet_types::primitives::PropertyValue;
use PropertyIdentifier as P;

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
            PropertyValue::CharacterString("long lighting label".repeat(100)),
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
fn rpm_lighting_output_indexed_reads_and_bytes_are_unchanged() {
    for configured in [false, true] {
        let mut object = LightingOutputObject::new(7, "LO-7").unwrap();
        if configured {
            object
                .write_property(P::PRESENT_VALUE, None, PropertyValue::Real(50.0), Some(8))
                .unwrap();
            object
                .write_property(
                    P::LIGHTING_COMMAND,
                    None,
                    PropertyValue::OctetString(vec![0x01, 0x02]),
                    None,
                )
                .unwrap();
            object
                .write_property(
                    P::LIGHTING_COMMAND_DEFAULT_PRIORITY,
                    None,
                    PropertyValue::Unsigned(8),
                    None,
                )
                .unwrap();
            object
                .write_property(P::RELINQUISH_DEFAULT, None, PropertyValue::Real(75.0), None)
                .unwrap();
            object
                .write_property(
                    P::BLINK_WARN_ENABLE,
                    None,
                    PropertyValue::Boolean(true),
                    None,
                )
                .unwrap();
            object
                .write_property(P::EGRESS_TIME, None, PropertyValue::Unsigned(600), None)
                .unwrap();
        }
        write_common(&mut object, configured);
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Independent application-value bytes pin the existing projection.
        // Priority_Array is BACnetARRAY (Table 12-64): None returns the whole
        // 16-slot list (Null 0x00, Real 0x44 + 4 bytes), Some(0) returns the
        // Unsigned size, and Some(1..=16) returns one slot. Every other
        // served row is scalar and rejects an index. 50.0f32 is 0x42480000,
        // 75.0f32 is 0x42960000.
        let pa_whole: &[u8] = if configured {
            &[
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x44, 0x42, 0x48, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
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
                Ok(if configured {
                    &[0x44, 0x42, 0x48, 0x00, 0x00]
                } else {
                    &[0x44, 0x00, 0x00, 0x00, 0x00]
                }),
            ),
            (
                P::PRESENT_VALUE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::TRACKING_VALUE, None, Ok(&[0x44, 0x00, 0x00, 0x00, 0x00])),
            (
                P::TRACKING_VALUE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::LIGHTING_COMMAND,
                None,
                Ok(if configured {
                    &[0x62, 0x01, 0x02]
                } else {
                    &[0x60]
                }),
            ),
            (
                P::LIGHTING_COMMAND,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::LIGHTING_COMMAND_DEFAULT_PRIORITY,
                None,
                Ok(if configured { &[0x21, 8] } else { &[0x21, 16] }),
            ),
            (
                P::LIGHTING_COMMAND_DEFAULT_PRIORITY,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::IN_PROGRESS, None, Ok(&[0x91, 0])),
            (
                P::IN_PROGRESS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::BLINK_WARN_ENABLE,
                None,
                Ok(if configured { &[0x11] } else { &[0x10] }),
            ),
            (
                P::BLINK_WARN_ENABLE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::EGRESS_TIME,
                None,
                Ok(if configured {
                    &[0x22, 0x02, 0x58]
                } else {
                    &[0x21, 0]
                }),
            ),
            (
                P::EGRESS_TIME,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::EGRESS_ACTIVE, None, Ok(&[0x10])),
            (
                P::EGRESS_ACTIVE,
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
            (P::PRIORITY_ARRAY, None, Ok(pa_whole)),
            (P::PRIORITY_ARRAY, Some(0), Ok(&[0x21, 16])),
            (P::PRIORITY_ARRAY, Some(1), Ok(&[0x00])),
            (
                P::PRIORITY_ARRAY,
                Some(8),
                Ok(if configured {
                    &[0x44, 0x42, 0x48, 0x00, 0x00]
                } else {
                    &[0x00]
                }),
            ),
            (P::PRIORITY_ARRAY, Some(16), Ok(&[0x00])),
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
                Ok(if configured {
                    &[0x44, 0x42, 0x96, 0x00, 0x00]
                } else {
                    &[0x44, 0x00, 0x00, 0x00, 0x00]
                }),
            ),
            (
                P::RELINQUISH_DEFAULT,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::DEFAULT_FADE_TIME, None, Ok(&[0x21, 0])),
            (
                P::DEFAULT_FADE_TIME,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::PROPERTY_LIST,
                None,
                Ok(&[
                    0x91, 28, 0x91, 85, 0x91, 164, 0x92, 0x01, 0x7C, 0x92, 0x01, 0x7D, 0x92, 0x01,
                    0x7A, 0x92, 0x01, 0x75, 0x92, 0x01, 0x79, 0x92, 0x01, 0x82, 0x91, 111, 0x91,
                    81, 0x91, 103, 0x91, 87, 0x91, 104, 0x92, 0x01, 0x76,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 15])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x91, 85])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x91, 164])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x92, 0x01, 0x7C])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x92, 0x01, 0x7D])),
            (P::PROPERTY_LIST, Some(6), Ok(&[0x92, 0x01, 0x7A])),
            (P::PROPERTY_LIST, Some(7), Ok(&[0x92, 0x01, 0x75])),
            (P::PROPERTY_LIST, Some(8), Ok(&[0x92, 0x01, 0x79])),
            (P::PROPERTY_LIST, Some(9), Ok(&[0x92, 0x01, 0x82])),
            (P::PROPERTY_LIST, Some(10), Ok(&[0x91, 111])),
            (P::PROPERTY_LIST, Some(11), Ok(&[0x91, 81])),
            (P::PROPERTY_LIST, Some(12), Ok(&[0x91, 103])),
            (P::PROPERTY_LIST, Some(13), Ok(&[0x91, 87])),
            (P::PROPERTY_LIST, Some(14), Ok(&[0x91, 104])),
            (P::PROPERTY_LIST, Some(15), Ok(&[0x92, 0x01, 0x76])),
            (
                P::PROPERTY_LIST,
                Some(16),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::PROPERTY_LIST,
                Some(u32::MAX),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            // Unserved Lighting Output table rows stay unknown.
            (P::DEFAULT_RAMP_RATE, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::DEFAULT_RAMP_RATE,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::DEFAULT_STEP_INCREMENT,
                None,
                Err(ErrorCode::UNKNOWN_PROPERTY),
            ),
            (P::FEEDBACK_VALUE, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::FEEDBACK_VALUE,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
        ];
        assert_cases(&db, oid, cases);
    }
}

#[test]
fn rpm_binary_lighting_output_indexed_reads_and_bytes_are_unchanged() {
    for configured in [false, true] {
        let mut object = BinaryLightingOutputObject::new(7, "BLO-7").unwrap();
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
            object
                .write_property(
                    P::BLINK_WARN_ENABLE,
                    None,
                    PropertyValue::Boolean(true),
                    None,
                )
                .unwrap();
            object
                .write_property(P::EGRESS_TIME, None, PropertyValue::Unsigned(5), None)
                .unwrap();
        }
        write_common(&mut object, configured);
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Independent application-value bytes pin the existing projection.
        // Priority_Array is BACnetARRAY (Table 12-69) with the same
        // whole/size/slot gating as Lighting Output; Enumerated ON is
        // 0x91 0x01 and Null is 0x00.
        let pa_expected: &[u8] = if configured {
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
            (
                P::BLINK_WARN_ENABLE,
                None,
                Ok(if configured { &[0x11] } else { &[0x10] }),
            ),
            (
                P::BLINK_WARN_ENABLE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::EGRESS_TIME,
                None,
                Ok(if configured { &[0x21, 5] } else { &[0x21, 0] }),
            ),
            (
                P::EGRESS_TIME,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::EGRESS_ACTIVE, None, Ok(&[0x10])),
            (
                P::EGRESS_ACTIVE,
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
            (P::PRIORITY_ARRAY, None, Ok(pa_expected)),
            (P::PRIORITY_ARRAY, Some(0), Ok(&[0x21, 16])),
            (P::PRIORITY_ARRAY, Some(1), Ok(&[0x00])),
            (
                P::PRIORITY_ARRAY,
                Some(8),
                Ok(if configured { &[0x91, 1] } else { &[0x00] }),
            ),
            (P::PRIORITY_ARRAY, Some(16), Ok(&[0x00])),
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
                P::PROPERTY_LIST,
                None,
                Ok(&[
                    0x91, 28, 0x91, 85, 0x92, 0x01, 0x75, 0x92, 0x01, 0x79, 0x92, 0x01, 0x82, 0x91,
                    111, 0x91, 81, 0x91, 103, 0x91, 87, 0x91, 104,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 10])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x91, 85])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x92, 0x01, 0x75])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x92, 0x01, 0x79])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x92, 0x01, 0x82])),
            (P::PROPERTY_LIST, Some(6), Ok(&[0x91, 111])),
            (P::PROPERTY_LIST, Some(7), Ok(&[0x91, 81])),
            (P::PROPERTY_LIST, Some(8), Ok(&[0x91, 103])),
            (P::PROPERTY_LIST, Some(9), Ok(&[0x91, 87])),
            (P::PROPERTY_LIST, Some(10), Ok(&[0x91, 104])),
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
            // Unserved Binary Lighting Output table rows stay unknown.
            (P::FEEDBACK_VALUE, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::FEEDBACK_VALUE,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::POLARITY, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::POLARITY,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
        ];
        assert_cases(&db, oid, cases);
    }
}
