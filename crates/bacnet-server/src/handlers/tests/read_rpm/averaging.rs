use super::*;
use bacnet_objects::{averaging::AveragingObject, traits::BACnetObject};
use bacnet_services::common::PropertyReference;
use bacnet_services::rpm::ReadAccessSpecification;
use bacnet_types::primitives::PropertyValue;
use PropertyIdentifier as P;

#[test]
fn rpm_averaging_indexed_reads_and_bytes_are_unchanged() {
    for configured in [false, true] {
        let mut object = AveragingObject::new(7, "AVG-7").unwrap();
        if configured {
            object.add_sample(10.0);
            object.add_sample(20.0);
            object.add_sample(30.0);
            let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
            object
                .write_property(
                    P::OBJECT_PROPERTY_REFERENCE,
                    None,
                    PropertyValue::List(vec![
                        PropertyValue::ObjectIdentifier(oid),
                        PropertyValue::Unsigned(P::PRESENT_VALUE.to_raw() as u64),
                    ]),
                    None,
                )
                .unwrap();
        }
        object
            .write_property(
                P::OUT_OF_SERVICE,
                None,
                PropertyValue::Boolean(configured),
                None,
            )
            .unwrap();
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Independent application-value bytes pin the existing projection.
        type ExpectedRead = Result<&'static [u8], ErrorCode>;
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            (
                P::PRESENT_VALUE,
                None,
                Ok(if configured {
                    &[0x44, 0x41, 0xA0, 0x00, 0x00]
                } else {
                    &[0x44, 0, 0, 0, 0]
                }),
            ),
            (
                P::PRESENT_VALUE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::PRESENT_VALUE,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::MINIMUM_VALUE,
                None,
                Ok(if configured {
                    &[0x44, 0x41, 0x20, 0x00, 0x00]
                } else {
                    &[0x44, 0, 0, 0, 0]
                }),
            ),
            (
                P::MINIMUM_VALUE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::MAXIMUM_VALUE,
                None,
                Ok(if configured {
                    &[0x44, 0x41, 0xF0, 0x00, 0x00]
                } else {
                    &[0x44, 0, 0, 0, 0]
                }),
            ),
            (
                P::MAXIMUM_VALUE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::AVERAGE_VALUE,
                None,
                Ok(if configured {
                    &[0x44, 0x41, 0xA0, 0x00, 0x00]
                } else {
                    &[0x44, 0, 0, 0, 0]
                }),
            ),
            (
                P::AVERAGE_VALUE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::ATTEMPTED_SAMPLES,
                None,
                Ok(if configured { &[0x21, 3] } else { &[0x21, 0] }),
            ),
            (
                P::ATTEMPTED_SAMPLES,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::VALID_SAMPLES,
                None,
                Ok(if configured { &[0x21, 3] } else { &[0x21, 0] }),
            ),
            (
                P::VALID_SAMPLES,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::OBJECT_PROPERTY_REFERENCE,
                None,
                Ok(if configured {
                    &[0xC4, 0x00, 0x00, 0x00, 0x01, 0x21, 85]
                } else {
                    &[0x00]
                }),
            ),
            (
                P::OBJECT_PROPERTY_REFERENCE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::OBJECT_PROPERTY_REFERENCE,
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
            (P::EVENT_STATE, None, Ok(&[0x91, 0])),
            (
                P::EVENT_STATE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::PROPERTY_LIST,
                None,
                Ok(&[
                    0x91, 28, 0x91, 85, 0x91, 136, 0x91, 135, 0x91, 125, 0x91, 124, 0x91, 146,
                    0x91, 78, 0x91, 111, 0x91, 81, 0x91, 103, 0x91, 36,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 12])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x91, 85])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x91, 136])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x91, 135])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x91, 125])),
            (P::PROPERTY_LIST, Some(6), Ok(&[0x91, 124])),
            (P::PROPERTY_LIST, Some(7), Ok(&[0x91, 146])),
            (P::PROPERTY_LIST, Some(8), Ok(&[0x91, 78])),
            (P::PROPERTY_LIST, Some(9), Ok(&[0x91, 111])),
            (P::PROPERTY_LIST, Some(10), Ok(&[0x91, 81])),
            (P::PROPERTY_LIST, Some(11), Ok(&[0x91, 103])),
            (P::PROPERTY_LIST, Some(12), Ok(&[0x91, 36])),
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
            (P::WINDOW_INTERVAL, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (P::WINDOW_SAMPLES, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::MINIMUM_VALUE_TIMESTAMP,
                None,
                Err(ErrorCode::UNKNOWN_PROPERTY),
            ),
        ];
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
        handle_read_property_multiple(&db, &request, &mut legacy).unwrap();
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
            let rp = handle_read_property(&db, &rp_request, &mut response);
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
        handle_rpm_budgeted(&db, &request, &mut bounded, budget).unwrap();
        assert_eq!(bounded, legacy);
        let mut prefix = BytesMut::from(&b"prefix"[..]);
        assert!(matches!(
            handle_rpm_budgeted(
                &db,
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
                &db,
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
}
