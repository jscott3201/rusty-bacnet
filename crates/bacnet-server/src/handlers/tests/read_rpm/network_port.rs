use super::*;
use bacnet_objects::{network_port::NetworkPortObject, traits::BACnetObject};
use bacnet_services::common::PropertyReference;
use bacnet_services::rpm::ReadAccessSpecification;
use PropertyIdentifier as P;

#[test]
fn rpm_network_port_indexed_reads_and_bytes_are_unchanged() {
    for configured in [false, true] {
        let mut object = NetworkPortObject::new(7, "NP-7", 0).unwrap();
        if configured {
            object
                .write_property(
                    P::DESCRIPTION,
                    None,
                    bacnet_types::primitives::PropertyValue::CharacterString(
                        "long network port label".repeat(100),
                    ),
                    None,
                )
                .unwrap();
            object
                .write_property(
                    P::IP_ADDRESS,
                    None,
                    bacnet_types::primitives::PropertyValue::OctetString(vec![192, 168, 1, 100]),
                    None,
                )
                .unwrap();
            object
                .write_property(
                    P::BACNET_IP_UDP_PORT,
                    None,
                    bacnet_types::primitives::PropertyValue::Unsigned(47809),
                    None,
                )
                .unwrap();
            object
                .write_property(
                    P::NETWORK_NUMBER,
                    None,
                    bacnet_types::primitives::PropertyValue::Unsigned(5),
                    None,
                )
                .unwrap();
            object
                .write_property(
                    P::MAC_ADDRESS,
                    None,
                    bacnet_types::primitives::PropertyValue::OctetString(vec![
                        0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01,
                    ]),
                    None,
                )
                .unwrap();
            object
                .write_property(
                    P::COMMAND_NP,
                    None,
                    bacnet_types::primitives::PropertyValue::Enumerated(1),
                    None,
                )
                .unwrap();
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
            (P::NETWORK_TYPE, None, Ok(&[0x91, 0])),
            (
                P::NETWORK_TYPE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::NETWORK_NUMBER,
                None,
                Ok(if configured { &[0x21, 5] } else { &[0x21, 0] }),
            ),
            (
                P::NETWORK_NUMBER,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::MAC_ADDRESS,
                None,
                Ok(if configured {
                    &[0x65, 0x06, 0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01]
                } else {
                    &[0x60]
                }),
            ),
            (
                P::MAC_ADDRESS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::MAX_APDU_LENGTH_ACCEPTED, None, Ok(&[0x22, 0x05, 0xC4])),
            (
                P::MAX_APDU_LENGTH_ACCEPTED,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::LINK_SPEED, None, Ok(&[0x44, 0, 0, 0, 0])),
            (
                P::LINK_SPEED,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::CHANGES_PENDING,
                None,
                Ok(if configured { &[0x11] } else { &[0x10] }),
            ),
            (
                P::CHANGES_PENDING,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::COMMAND_NP,
                None,
                Ok(if configured { &[0x91, 1] } else { &[0x91, 0] }),
            ),
            (
                P::COMMAND_NP,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::IP_ADDRESS,
                None,
                Ok(if configured {
                    &[0x64, 0xC0, 0xA8, 0x01, 0x64]
                } else {
                    &[0x64, 0, 0, 0, 0]
                }),
            ),
            (
                P::IP_ADDRESS,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::IP_DEFAULT_GATEWAY, None, Ok(&[0x64, 0, 0, 0, 0])),
            (
                P::IP_DEFAULT_GATEWAY,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::IP_SUBNET_MASK, None, Ok(&[0x64, 0xFF, 0xFF, 0xFF, 0x00])),
            (
                P::IP_SUBNET_MASK,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::BACNET_IP_UDP_PORT,
                None,
                Ok(if configured {
                    &[0x22, 0xBA, 0xC1]
                } else {
                    &[0x22, 0xBA, 0xC0]
                }),
            ),
            (
                P::BACNET_IP_UDP_PORT,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::PROPERTY_LIST,
                None,
                Ok(&[
                    0x91, 28, 0x91, 111, 0x91, 81, 0x91, 103, 0x92, 0x01, 0xAB, 0x92, 0x01, 0xA9,
                    0x92, 0x01, 0xA7, 0x91, 62, 0x92, 0x01, 0xA4, 0x92, 0x01, 0xA0, 0x92, 0x01,
                    0xA1, 0x92, 0x01, 0x90, 0x92, 0x01, 0x91, 0x92, 0x01, 0x9B, 0x92, 0x01, 0x9C,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 15])),
            (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[0x91, 111])),
            (P::PROPERTY_LIST, Some(3), Ok(&[0x91, 81])),
            (P::PROPERTY_LIST, Some(4), Ok(&[0x91, 103])),
            (P::PROPERTY_LIST, Some(5), Ok(&[0x92, 0x01, 0xAB])),
            (P::PROPERTY_LIST, Some(6), Ok(&[0x92, 0x01, 0xA9])),
            (P::PROPERTY_LIST, Some(7), Ok(&[0x92, 0x01, 0xA7])),
            (P::PROPERTY_LIST, Some(8), Ok(&[0x91, 62])),
            (P::PROPERTY_LIST, Some(9), Ok(&[0x92, 0x01, 0xA4])),
            (P::PROPERTY_LIST, Some(10), Ok(&[0x92, 0x01, 0xA0])),
            (P::PROPERTY_LIST, Some(11), Ok(&[0x92, 0x01, 0xA1])),
            (P::PROPERTY_LIST, Some(12), Ok(&[0x92, 0x01, 0x90])),
            (P::PROPERTY_LIST, Some(13), Ok(&[0x92, 0x01, 0x91])),
            (P::PROPERTY_LIST, Some(14), Ok(&[0x92, 0x01, 0x9B])),
            (P::PROPERTY_LIST, Some(15), Ok(&[0x92, 0x01, 0x9C])),
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
            // Unserved NetworkPort table rows stay unknown. APDU_Length (399)
            // is the table row with no dispatch arm (MAX_APDU_LENGTH_ACCEPTED
            // (62) is the served mirror); Protocol_Level and Event_State
            // likewise have no arm.
            (P::EVENT_STATE, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::EVENT_STATE,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::APDU_LENGTH, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::APDU_LENGTH,
                Some(1),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::PROTOCOL_LEVEL, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::PROTOCOL_LEVEL,
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
