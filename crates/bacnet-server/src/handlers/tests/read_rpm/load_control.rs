use super::*;
use PropertyIdentifier as P;
use bacnet_objects::{load_control::LoadControlObject, traits::BACnetObject};
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::error::RemoteError;

#[test]
fn rpm_load_control_indexed_reads_and_bytes_are_unchanged() {
    for configured in [false, true] {
        let mut object = LoadControlObject::new(7, "LC-7").unwrap();
        if configured {
            object
                .write_property(
                    P::DESCRIPTION,
                    None,
                    bacnet_types::primitives::PropertyValue::CharacterString(
                        "long load control label".repeat(100),
                    ),
                    None,
                )
                .unwrap();
            // percent [0] 50.
            object
                .write_property(
                    P::REQUESTED_SHED_LEVEL,
                    None,
                    bacnet_types::primitives::PropertyValue::ApplicationData(vec![0x09, 50]),
                    None,
                )
                .unwrap();
            object
                .write_property(
                    P::SHED_DURATION,
                    None,
                    bacnet_types::primitives::PropertyValue::Unsigned(3600),
                    None,
                )
                .unwrap();
            object
                .set_shed_levels(vec![(1, "lights".into()), (2, "chiller".into())])
                .unwrap();
        }
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Independent application-value bytes pin the existing projection.
        type ExpectedRead = Result<&'static [u8], ErrorCode>;
        let unspec_start_time: &[u8] =
            &[0xa4, 0xff, 0xff, 0xff, 0xff, 0xb4, 0xff, 0xff, 0xff, 0xff];
        // The shed levels are BACnetShedLevel choices (#1133). They start at
        // level [1] 0; after the percent [0] write, Expected and Actual hold
        // the PERCENT default, 100, as the object stays SHED_INACTIVE.
        let (requested, expected_actual): (&[u8], &[u8]) = if configured {
            (&[0x09, 50], &[0x09, 100])
        } else {
            (&[0x19, 0], &[0x19, 0])
        };
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            (P::PRESENT_VALUE, None, Ok(&[0x91, 0])),
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
            (P::REQUESTED_SHED_LEVEL, None, Ok(requested)),
            (
                P::REQUESTED_SHED_LEVEL,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::REQUESTED_SHED_LEVEL,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::EXPECTED_SHED_LEVEL, None, Ok(expected_actual)),
            (
                P::EXPECTED_SHED_LEVEL,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::ACTUAL_SHED_LEVEL, None, Ok(expected_actual)),
            (
                P::ACTUAL_SHED_LEVEL,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::SHED_DURATION,
                None,
                Ok(if configured {
                    &[0x22, 0x0e, 0x10]
                } else {
                    &[0x21, 0]
                }),
            ),
            (
                P::SHED_DURATION,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::START_TIME, None, Ok(unspec_start_time)),
            (
                P::START_TIME,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::START_TIME,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            // Table 12-32 has no Out_Of_Service (#1064), so the flag stays
            // clear.
            (P::STATUS_FLAGS, None, Ok(&[0x82, 4, 0])),
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
                    0x91, 28, 0x91, 85, 0x91, 218, 0x91, 214, 0x91, 212, 0x91, 219, 0x91, 142,
                    0x91, 111, 0x91, 103, 0x91, 36, 0x91, 213, 0x91, 133, 0x91, 221, 0x91, 220,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 14])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x91, 85])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x91, 218])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x91, 214])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x91, 212])),
            (P::PROPERTY_LIST, Some(6), Ok(&[0x91, 219])),
            (P::PROPERTY_LIST, Some(7), Ok(&[0x91, 142])),
            (P::PROPERTY_LIST, Some(8), Ok(&[0x91, 111])),
            (P::PROPERTY_LIST, Some(9), Ok(&[0x91, 103])),
            (P::PROPERTY_LIST, Some(10), Ok(&[0x91, 36])),
            (P::PROPERTY_LIST, Some(11), Ok(&[0x91, 213])),
            (P::PROPERTY_LIST, Some(12), Ok(&[0x91, 133])),
            (P::PROPERTY_LIST, Some(13), Ok(&[0x91, 221])),
            (P::PROPERTY_LIST, Some(14), Ok(&[0x91, 220])),
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
            // The rows served since #2464: Duty_Window (minutes), Enable,
            // and the two arrays the embedder sizes, empty until it does.
            (P::DUTY_WINDOW, None, Ok(&[0x21, 0])),
            (
                P::DUTY_WINDOW,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::ENABLE, None, Ok(&[0x11])),
            (
                P::SHED_LEVELS,
                None,
                Ok(if configured { &[0x21, 1, 0x21, 2] } else { &[] }),
            ),
            (
                P::SHED_LEVELS,
                Some(0),
                Ok(if configured { &[0x21, 2] } else { &[0x21, 0] }),
            ),
            if configured {
                (P::SHED_LEVELS, Some(2), Ok(&[0x21, 2]))
            } else {
                (P::SHED_LEVELS, Some(1), Err(ErrorCode::INVALID_ARRAY_INDEX))
            },
            (
                P::SHED_LEVEL_DESCRIPTIONS,
                Some(0),
                Ok(if configured { &[0x21, 2] } else { &[0x21, 0] }),
            ),
            if configured {
                (
                    P::SHED_LEVEL_DESCRIPTIONS,
                    Some(1),
                    Ok(&[0x75, 7, 0, b'l', b'i', b'g', b'h', b't', b's']),
                )
            } else {
                (
                    P::SHED_LEVEL_DESCRIPTIONS,
                    Some(1),
                    Err(ErrorCode::INVALID_ARRAY_INDEX),
                )
            },
            // The optional rows this object leaves out stay unknown.
            (
                P::FULL_DUTY_BASELINE,
                None,
                Err(ErrorCode::UNKNOWN_PROPERTY),
            ),
            (P::STATE_DESCRIPTION, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
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
        .encode(&mut request)
        .unwrap();
        let mut unlimited = BytesMut::new();
        handle_read_property_multiple(&db, &request, &mut unlimited).unwrap();
        let ack = ReadPropertyMultipleACK::decode(&unlimited).unwrap();
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
                    assert!(
                        matches!(rp, Err(Error::Remote(RemoteError::Error { class, code }))
                        if class == ErrorClass::PROPERTY && code == expected)
                    );
                    assert!(response.is_empty());
                }
            }
        }
        use crate::handlers::rpm_budget::handle_rpm_budgeted;
        use bacnet_device_core::read::ReadFailure;
        let budget = bacnet_device_core::read::ReadPropertyMultipleBudget {
            max_result_elements: cases.len(),
            max_service_ack_bytes: unlimited.len(),
        };
        let mut bounded = BytesMut::new();
        handle_rpm_budgeted(&db, &request, &mut bounded, budget).unwrap();
        assert_eq!(bounded, unlimited);
        let mut prefix = BytesMut::from(&b"prefix"[..]);
        assert!(matches!(
            handle_rpm_budgeted(
                &db,
                &request,
                &mut prefix,
                bacnet_device_core::read::ReadPropertyMultipleBudget {
                    max_result_elements: cases.len() - 1,
                    ..budget
                }
            ),
            Err(ReadFailure::Work)
        ));
        assert_eq!(&prefix[..], b"prefix");
        assert!(matches!(
            handle_rpm_budgeted(
                &db,
                &request,
                &mut prefix,
                bacnet_device_core::read::ReadPropertyMultipleBudget {
                    max_service_ack_bytes: unlimited.len() - 1,
                    ..budget
                }
            ),
            Err(ReadFailure::Bytes)
        ));
        assert_eq!(&prefix[..], b"prefix");
    }
}
