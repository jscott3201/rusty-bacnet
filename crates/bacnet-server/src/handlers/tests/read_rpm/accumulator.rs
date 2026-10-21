use super::*;
use bacnet_objects::{
    accumulator::{AccumulatorObject, PulseConverterObject},
    traits::BACnetObject,
};
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
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

fn write_common(object: &mut dyn BACnetObject, configured: bool) {
    object
        .write_property(
            P::DESCRIPTION,
            None,
            PropertyValue::CharacterString("long accumulator label".repeat(100)),
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
fn rpm_accumulator_indexed_reads_and_bytes_are_unchanged() {
    for configured in [false, true] {
        let mut object = AccumulatorObject::new(7, "ACC-7", EngineeringUnits::NO_UNITS).unwrap();
        if configured {
            // Application-level count plus the three routed Unsigned/Real arms.
            object.set_present_value(42);
            object
                .write_property(P::MAX_PRES_VALUE, None, PropertyValue::Unsigned(200), None)
                .unwrap();
            object
                .write_property(P::PULSE_RATE, None, PropertyValue::Real(2.5), None)
                .unwrap();
            object.set_scale(bacnet_types::constructed::BACnetScale::IntegerScale(-2));
            object
                .write_property(
                    P::LIMIT_MONITORING_INTERVAL,
                    None,
                    PropertyValue::Unsigned(60),
                    None,
                )
                .unwrap();
        }
        // Prescale is served only once set (an unset one is UNKNOWN_PROPERTY,
        // pinned in `pulse_converter_input_reference`), so set it for both.
        object.set_prescale(bacnet_types::constructed::BACnetPrescale {
            multiplier: 5,
            modulo_divide: 100,
        });
        write_common(&mut object, configured);
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Independent bytes pin the projection. Scale is a CHOICE of a REAL
        // under context tag 0 or an INTEGER under context tag 1, and Prescale
        // its multiplier under context tag 0 then its modulo divide under
        // context tag 1 (Clause 21, #1487); neither is an array, so an index
        // is PROPERTY_IS_NOT_AN_ARRAY on both. 2.5f32 encodes as 0x40200000;
        // 1.0f32 as 0x3F800000; -2 as the one octet 0xFE.
        let prescale: ExpectedRead = Ok(&[0x09, 5, 0x19, 100]);
        let scale: ExpectedRead = Ok(if configured {
            &[0x19, 0xFE]
        } else {
            &[0x0C, 0x3F, 0x80, 0x00, 0x00]
        });
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            (P::OBJECT_TYPE, None, Ok(&[0x91, 23])),
            (
                P::OBJECT_TYPE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::PRESENT_VALUE,
                None,
                Ok(if configured { &[0x21, 42] } else { &[0x21, 0] }),
            ),
            (
                P::PRESENT_VALUE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::MAX_PRES_VALUE,
                None,
                Ok(if configured {
                    &[0x21, 200]
                } else {
                    &[0x25, 8, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]
                }),
            ),
            (
                P::MAX_PRES_VALUE,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::SCALE, None, scale),
            (P::SCALE, Some(1), Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY)),
            (P::PRESCALE, None, prescale),
            (
                P::PRESCALE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::PULSE_RATE,
                None,
                Ok(if configured {
                    &[0x44, 0x40, 0x20, 0x00, 0x00]
                } else {
                    &[0x44, 0, 0, 0, 0]
                }),
            ),
            (
                P::PULSE_RATE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::UNITS, None, Ok(&[0x91, 95])),
            (P::UNITS, Some(0), Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY)),
            (
                P::LIMIT_MONITORING_INTERVAL,
                None,
                Ok(if configured { &[0x21, 60] } else { &[0x21, 0] }),
            ),
            (
                P::LIMIT_MONITORING_INTERVAL,
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
            (P::EVENT_STATE, None, Ok(&[0x91, 0])),
            (
                P::EVENT_STATE,
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
            (P::VALUE_BEFORE_CHANGE, None, Ok(&[0x21, 0])),
            (
                P::VALUE_BEFORE_CHANGE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::VALUE_SET, None, Ok(&[0x21, 0])),
            (
                P::VALUE_SET,
                Some(u32::MAX),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::PROPERTY_LIST,
                None,
                Ok(&[
                    0x91, 0x1C, 0x91, 0x55, 0x91, 0x41, 0x91, 0xBB, 0x91, 0xB9, 0x91, 0xBA, 0x91,
                    0x75, 0x91, 0xB6, 0x91, 0x6F, 0x91, 0x24, 0x91, 0x51, 0x91, 0x67, 0x91, 0xBE,
                    0x91, 0xBF,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 14])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 0x1C])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x91, 0x55])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x91, 0x41])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x91, 0xBB])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x91, 0xB9])),
            (P::PROPERTY_LIST, Some(6), Ok(&[0x91, 0xBA])),
            (P::PROPERTY_LIST, Some(7), Ok(&[0x91, 0x75])),
            (P::PROPERTY_LIST, Some(8), Ok(&[0x91, 0xB6])),
            (P::PROPERTY_LIST, Some(9), Ok(&[0x91, 0x6F])),
            (P::PROPERTY_LIST, Some(10), Ok(&[0x91, 0x24])),
            (P::PROPERTY_LIST, Some(11), Ok(&[0x91, 0x51])),
            (P::PROPERTY_LIST, Some(12), Ok(&[0x91, 0x67])),
            (P::PROPERTY_LIST, Some(13), Ok(&[0x91, 0xBE])),
            (P::PROPERTY_LIST, Some(14), Ok(&[0x91, 0xBF])),
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
            // Unserved Table 12-79 rows stay unknown; the array gate runs first.
            (P::DEVICE_TYPE, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::DEVICE_TYPE,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::VALUE_CHANGE_TIME, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (P::COUNT, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
        ];
        assert_cases(&db, oid, cases);
    }
}

#[test]
fn rpm_pulse_converter_indexed_reads_and_bytes_are_unchanged() {
    for configured in [false, true] {
        let mut object =
            PulseConverterObject::new(7, "PC-7", EngineeringUnits::DEGREES_CELSIUS).unwrap();
        write_common(&mut object, configured);
        if configured {
            // The Present_Value gate needs Out_Of_Service TRUE first.
            object
                .write_property(P::PRESENT_VALUE, None, PropertyValue::Real(12.5), None)
                .unwrap();
            object.add_pulses(7).unwrap();
            object
                .write_property(P::SCALE_FACTOR, None, PropertyValue::Real(2.5), None)
                .unwrap();
            // 5 / 2.5 takes 2 off Count: 7 becomes 5.
            object
                .write_property(P::ADJUST_VALUE, None, PropertyValue::Real(5.0), None)
                .unwrap();
            object
                .write_property(P::COV_INCREMENT, None, PropertyValue::Real(0.5), None)
                .unwrap();
            // [0] accumulator 1, [1] present-value (#1312).
            object
                .write_property(
                    P::INPUT_REFERENCE,
                    None,
                    PropertyValue::ApplicationData(vec![0x0C, 0x05, 0xC0, 0x00, 0x01, 0x19, 0x55]),
                    None,
                )
                .unwrap();
        }
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Independent application-value bytes pin the existing projection.
        // 12.5f32 encodes as 0x41480000, 2.5f32 as 0x40200000, 5.0f32 as
        // 0x40A00000, 0.5f32 as 0x3F000000. The Accumulator(23) instance-1
        // reference is its Clause 21 members: [0] 0x0C 0x05 0xC0 0x00 0x01,
        // then [1] present-value 0x19 0x55 (#1312). With no Device clock the two
        // BACnetDateTime rows are an all-unspecified Date (0xA4) then Time
        // (0xB4).
        const UNSPECIFIED: &[u8] = &[0xA4, 0xFF, 0xFF, 0xFF, 0xFF, 0xB4, 0xFF, 0xFF, 0xFF, 0xFF];
        let input_reference: &[u8] = if configured {
            &[0x0C, 0x05, 0xC0, 0x00, 0x01, 0x19, 0x55]
        } else {
            // Unset: Accumulator 4194303's present-value (#1417).
            &[0x0C, 0x05, 0xFF, 0xFF, 0xFF, 0x19, 0x55]
        };
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            (P::OBJECT_TYPE, None, Ok(&[0x91, 24])),
            (
                P::OBJECT_TYPE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::PRESENT_VALUE,
                None,
                Ok(if configured {
                    &[0x44, 0x41, 0x48, 0x00, 0x00]
                } else {
                    &[0x44, 0, 0, 0, 0]
                }),
            ),
            (
                P::PRESENT_VALUE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::UNITS, None, Ok(&[0x91, 62])),
            (P::UNITS, Some(0), Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY)),
            (
                P::SCALE_FACTOR,
                None,
                Ok(if configured {
                    &[0x44, 0x40, 0x20, 0x00, 0x00]
                } else {
                    &[0x44, 0x3F, 0x80, 0x00, 0x00]
                }),
            ),
            (
                P::SCALE_FACTOR,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::ADJUST_VALUE,
                None,
                Ok(if configured {
                    &[0x44, 0x40, 0xA0, 0x00, 0x00]
                } else {
                    &[0x44, 0, 0, 0, 0]
                }),
            ),
            (
                P::ADJUST_VALUE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::COUNT,
                None,
                Ok(if configured { &[0x21, 5] } else { &[0x21, 0] }),
            ),
            (P::COUNT, Some(1), Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY)),
            (P::UPDATE_TIME, None, Ok(UNSPECIFIED)),
            (
                P::UPDATE_TIME,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::COUNT_CHANGE_TIME, None, Ok(UNSPECIFIED)),
            (
                P::COUNT_CHANGE_TIME,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::COUNT_BEFORE_CHANGE,
                None,
                Ok(if configured { &[0x21, 7] } else { &[0x21, 0] }),
            ),
            (
                P::COUNT_BEFORE_CHANGE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::COV_INCREMENT,
                None,
                Ok(if configured {
                    &[0x44, 0x3F, 0x00, 0x00, 0x00]
                } else {
                    &[0x44, 0, 0, 0, 0]
                }),
            ),
            (
                P::COV_INCREMENT,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::COV_PERIOD, None, Ok(&[0x21, 0])),
            (
                P::COV_PERIOD,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::INPUT_REFERENCE, None, Ok(input_reference)),
            (
                P::INPUT_REFERENCE,
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
            (P::EVENT_STATE, None, Ok(&[0x91, 0])),
            (
                P::EVENT_STATE,
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
                    0x91, 0x1C, 0x91, 0x55, 0x91, 0x75, 0x91, 0xBC, 0x91, 0xB0, 0x91, 0xB1, 0x91,
                    0xBD, 0x91, 0xB3, 0x91, 0xB2, 0x91, 0x16, 0x91, 0xB4, 0x91, 0xB5, 0x91, 0x6F,
                    0x91, 0x24, 0x91, 0x51, 0x91, 0x67,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 16])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 0x1C])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x91, 0x55])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x91, 0x75])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x91, 0xBC])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x91, 0xB0])),
            (P::PROPERTY_LIST, Some(6), Ok(&[0x91, 0xB1])),
            (P::PROPERTY_LIST, Some(7), Ok(&[0x91, 0xBD])),
            (P::PROPERTY_LIST, Some(8), Ok(&[0x91, 0xB3])),
            (P::PROPERTY_LIST, Some(9), Ok(&[0x91, 0xB2])),
            (P::PROPERTY_LIST, Some(10), Ok(&[0x91, 0x16])),
            (P::PROPERTY_LIST, Some(11), Ok(&[0x91, 0xB4])),
            (P::PROPERTY_LIST, Some(12), Ok(&[0x91, 0xB5])),
            (P::PROPERTY_LIST, Some(13), Ok(&[0x91, 0x6F])),
            (P::PROPERTY_LIST, Some(14), Ok(&[0x91, 0x24])),
            (P::PROPERTY_LIST, Some(15), Ok(&[0x91, 0x51])),
            (P::PROPERTY_LIST, Some(16), Ok(&[0x91, 0x67])),
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
            // Unserved optional Table 12-27 rows stay unknown; the array gate
            // runs first.
            (
                P::NOTIFICATION_CLASS,
                None,
                Err(ErrorCode::UNKNOWN_PROPERTY),
            ),
            (
                P::NOTIFICATION_CLASS,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::HIGH_LIMIT, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
        ];
        assert_cases(&db, oid, cases);
    }
}
