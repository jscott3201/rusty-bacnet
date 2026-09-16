use super::*;
use bacnet_objects::{
    color::{ColorObject, ColorTemperatureObject},
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
            PropertyValue::CharacterString("long color label".repeat(100)),
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

// D65 default xy (0.3127f32 = 0x3EA01A37, 0.3290f32 = 0x3EA872B0).
const D65_XY: &[u8] = &[0x44, 0x3E, 0xA0, 0x1A, 0x37, 0x44, 0x3E, 0xA8, 0x72, 0xB0];
// Application-set xy (1.0f32 = 0x3F800000, 0.5f32 = 0x3F000000).
const SET_XY: &[u8] = &[0x44, 0x3F, 0x80, 0x00, 0x00, 0x44, 0x3F, 0x00, 0x00, 0x00];

#[test]
fn rpm_color_indexed_reads_and_bytes_are_unchanged() {
    for configured in [false, true] {
        let mut object = ColorObject::new(7, "CLR-7").unwrap();
        if configured {
            // Application-level color plus the two routed arms.
            object.set_present_value(1.0, 0.5);
            object
                .write_property(
                    P::COLOR_COMMAND,
                    None,
                    PropertyValue::OctetString(vec![0x01, 0x02]),
                    None,
                )
                .unwrap();
            object
                .write_property(
                    P::DEFAULT_FADE_TIME,
                    None,
                    PropertyValue::Unsigned(1000),
                    None,
                )
                .unwrap();
        }
        write_common(&mut object, configured);
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Independent application-value bytes pin the existing projection.
        // The xy lists are BACnetLIST-style productions, so an index is
        // PROPERTY_IS_NOT_AN_ARRAY on every scalar and list row.
        let xy: &[u8] = if configured { SET_XY } else { D65_XY };
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            (P::OBJECT_TYPE, None, Ok(&[0x91, 63])),
            (
                P::OBJECT_TYPE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::PRESENT_VALUE, None, Ok(xy)),
            (
                P::PRESENT_VALUE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::TRACKING_VALUE, None, Ok(xy)),
            (
                P::TRACKING_VALUE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::COLOR_COMMAND,
                None,
                Ok(if configured {
                    &[0x62, 0x01, 0x02]
                } else {
                    &[0x60]
                }),
            ),
            (
                P::COLOR_COMMAND,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::IN_PROGRESS, None, Ok(&[0x91, 0])),
            (
                P::IN_PROGRESS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::DEFAULT_COLOR, None, Ok(D65_XY)),
            (
                P::DEFAULT_COLOR,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::DEFAULT_FADE_TIME,
                None,
                Ok(if configured {
                    &[0x22, 0x03, 0xE8]
                } else {
                    &[0x21, 0x00]
                }),
            ),
            (
                P::DEFAULT_FADE_TIME,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::TRANSITION, None, Ok(&[0x91, 0])),
            (
                P::TRANSITION,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::STATUS_FLAGS,
                None,
                Ok(if configured {
                    &[0x82, 0x04, 0x10]
                } else {
                    &[0x82, 0x04, 0x00]
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
                    0x91, 0x1C, 0x91, 0x55, 0x91, 0xA4, 0x92, 0x01, 0xFC, 0x92, 0x01, 0x7A, 0x92,
                    0x01, 0xFE, 0x92, 0x01, 0x76, 0x92, 0x01, 0x81, 0x91, 0x6F, 0x91, 0x24, 0x91,
                    0x51, 0x91, 0x67,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 12])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 0x1C])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x91, 0x55])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x91, 0xA4])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x92, 0x01, 0xFC])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x92, 0x01, 0x7A])),
            (P::PROPERTY_LIST, Some(6), Ok(&[0x92, 0x01, 0xFE])),
            (P::PROPERTY_LIST, Some(7), Ok(&[0x92, 0x01, 0x76])),
            (P::PROPERTY_LIST, Some(8), Ok(&[0x92, 0x01, 0x81])),
            (P::PROPERTY_LIST, Some(9), Ok(&[0x91, 0x6F])),
            (P::PROPERTY_LIST, Some(10), Ok(&[0x91, 0x24])),
            (P::PROPERTY_LIST, Some(11), Ok(&[0x91, 0x51])),
            (P::PROPERTY_LIST, Some(12), Ok(&[0x91, 0x67])),
            (
                P::PROPERTY_LIST,
                Some(13),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::PROPERTY_LIST,
                Some(u32::MAX),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            // Unserved rows stay unknown; the array gate runs first.
            // (Priority_Array is array-classified by default, so an indexed
            // read passes the gate and reports unknown from dispatch.)
            (P::PRIORITY_ARRAY, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::DEVICE_TYPE,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::COV_INCREMENT, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (P::DEVICE_TYPE, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
        ];
        assert_cases(&db, oid, cases);
    }
}

#[test]
fn rpm_color_temperature_indexed_reads_and_bytes_are_unchanged() {
    for configured in [false, true] {
        let mut object = ColorTemperatureObject::new(7, "CT-7").unwrap();
        if configured {
            // Present_Value needs no Out_Of_Service gate; the command and
            // fade defaults below are exercised through the read arms only.
            object.set_present_value(5000);
            object
                .write_property(
                    P::COLOR_COMMAND,
                    None,
                    PropertyValue::OctetString(vec![0x04, 0x05]),
                    None,
                )
                .unwrap();
        }
        write_common(&mut object, configured);
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Independent application-value bytes pin the existing projection.
        // 5000 = 0x1388, 4000 = 0x0FA0, 1000 = 0x03E8, 30000 = 0x7530.
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            (P::OBJECT_TYPE, None, Ok(&[0x91, 64])),
            (
                P::OBJECT_TYPE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::PRESENT_VALUE,
                None,
                Ok(if configured {
                    &[0x22, 0x13, 0x88]
                } else {
                    &[0x22, 0x0F, 0xA0]
                }),
            ),
            (
                P::PRESENT_VALUE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::TRACKING_VALUE,
                None,
                Ok(if configured {
                    &[0x22, 0x13, 0x88]
                } else {
                    &[0x22, 0x0F, 0xA0]
                }),
            ),
            (
                P::TRACKING_VALUE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::COLOR_COMMAND,
                None,
                Ok(if configured {
                    &[0x62, 0x04, 0x05]
                } else {
                    &[0x60]
                }),
            ),
            (
                P::COLOR_COMMAND,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::IN_PROGRESS, None, Ok(&[0x91, 0])),
            (
                P::IN_PROGRESS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::DEFAULT_COLOR_TEMPERATURE, None, Ok(&[0x22, 0x0F, 0xA0])),
            (
                P::DEFAULT_COLOR_TEMPERATURE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::DEFAULT_FADE_TIME, None, Ok(&[0x21, 0x00])),
            (
                P::DEFAULT_FADE_TIME,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::DEFAULT_RAMP_RATE, None, Ok(&[0x21, 0x64])),
            (
                P::DEFAULT_RAMP_RATE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::DEFAULT_STEP_INCREMENT, None, Ok(&[0x21, 0x32])),
            (
                P::DEFAULT_STEP_INCREMENT,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::TRANSITION, None, Ok(&[0x91, 0])),
            (
                P::TRANSITION,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::MIN_PRES_VALUE, None, Ok(&[0x22, 0x03, 0xE8])),
            (
                P::MIN_PRES_VALUE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::MAX_PRES_VALUE, None, Ok(&[0x22, 0x75, 0x30])),
            (
                P::MAX_PRES_VALUE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::STATUS_FLAGS,
                None,
                Ok(if configured {
                    &[0x82, 0x04, 0x10]
                } else {
                    &[0x82, 0x04, 0x00]
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
                    0x91, 0x1C, 0x91, 0x55, 0x91, 0xA4, 0x92, 0x01, 0xFC, 0x92, 0x01, 0x7A, 0x92,
                    0x01, 0xFD, 0x92, 0x01, 0x76, 0x92, 0x01, 0x77, 0x92, 0x01, 0x78, 0x92, 0x01,
                    0x81, 0x91, 0x45, 0x91, 0x41, 0x91, 0x6F, 0x91, 0x24, 0x91, 0x51, 0x91, 0x67,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 16])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 0x1C])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x91, 0x55])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x91, 0xA4])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x92, 0x01, 0xFC])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x92, 0x01, 0x7A])),
            (P::PROPERTY_LIST, Some(6), Ok(&[0x92, 0x01, 0xFD])),
            (P::PROPERTY_LIST, Some(7), Ok(&[0x92, 0x01, 0x76])),
            (P::PROPERTY_LIST, Some(8), Ok(&[0x92, 0x01, 0x77])),
            (P::PROPERTY_LIST, Some(9), Ok(&[0x92, 0x01, 0x78])),
            (P::PROPERTY_LIST, Some(10), Ok(&[0x92, 0x01, 0x81])),
            (P::PROPERTY_LIST, Some(11), Ok(&[0x91, 0x45])),
            (P::PROPERTY_LIST, Some(12), Ok(&[0x91, 0x41])),
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
            // Unserved rows stay unknown; the array gate runs first.
            // (Priority_Array is array-classified by default, so an indexed
            // read passes the gate and reports unknown from dispatch.)
            (P::PRIORITY_ARRAY, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::DEVICE_TYPE,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::COV_INCREMENT, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (P::DEVICE_TYPE, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
        ];
        assert_cases(&db, oid, cases);
    }
}
