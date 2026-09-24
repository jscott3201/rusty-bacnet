use super::*;
use bacnet_objects::{command::CommandObject, traits::BACnetObject};
use bacnet_services::common::PropertyReference;
use bacnet_services::rpm::ReadAccessSpecification;
use PropertyIdentifier as P;

#[test]
fn rpm_command_indexed_reads_and_bytes_are_unchanged() {
    for configured in [false, true] {
        let mut object = CommandObject::new(7, "CMD-7").unwrap();
        if configured {
            object
                .write_property(
                    P::DESCRIPTION,
                    None,
                    bacnet_types::primitives::PropertyValue::CharacterString(
                        "long command label".repeat(100),
                    ),
                    None,
                )
                .unwrap();
            object
                .write_property(
                    P::PRESENT_VALUE,
                    None,
                    bacnet_types::primitives::PropertyValue::Unsigned(3),
                    None,
                )
                .unwrap();
            object.set_action(vec![vec![1, 2, 3], vec![4, 5]]);
        }
        object
            .write_property(
                P::OUT_OF_SERVICE,
                None,
                bacnet_types::primitives::PropertyValue::Boolean(configured),
                None,
            )
            .unwrap();
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Independent application-value bytes pin the existing projection.
        type ExpectedRead = Result<&'static [u8], ErrorCode>;
        // Action is BACnetARRAY but the object arm returns the whole list
        // regardless of the index; the service gate admits the index and the
        // object ignores it.
        let action_bytes: &[u8] = if configured {
            &[0x63, 1, 2, 3, 0x62, 4, 5]
        } else {
            &[]
        };
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            (
                P::PRESENT_VALUE,
                None,
                Ok(if configured { &[0x21, 3] } else { &[0x21, 0] }),
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
            (P::IN_PROCESS, None, Ok(&[0x10])),
            (
                P::IN_PROCESS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::IN_PROCESS,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::ALL_WRITES_SUCCESSFUL, None, Ok(&[0x11])),
            (
                P::ALL_WRITES_SUCCESSFUL,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::ACTION, None, Ok(action_bytes)),
            (P::ACTION, Some(0), Ok(action_bytes)),
            (P::ACTION, Some(1), Ok(action_bytes)),
            (P::ACTION, Some(u32::MAX), Ok(action_bytes)),
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
                    0x91, 28, 0x91, 85, 0x91, 47, 0x91, 9, 0x91, 2, 0x91, 111, 0x91, 81, 0x91, 103,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 8])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x91, 85])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x91, 47])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x91, 9])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x91, 2])),
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
            (P::ACTION_TEXT, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::ACTION_TEXT,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::EVENT_STATE, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::EVENT_STATE,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
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
        .encode(&mut request)
        .unwrap();
        let mut legacy = BytesMut::new();
        handle_read_property_multiple(&db, &request, &mut legacy).unwrap();
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
