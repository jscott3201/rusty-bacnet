use super::*;
use PropertyIdentifier as P;
use bacnet_objects::{network_port::NetworkPortObject, traits::BACnetObject};
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::error::RemoteError;

#[test]
fn rpm_network_port_configured_profile_wire_bytes_and_dns_indices() {
    for configured in [false, true] {
        let object = NetworkPortObject::new_bip(
            7,
            "NP-7",
            bacnet_objects::network_port::BipPortConfig {
                ip_address: if configured {
                    [192, 168, 1, 100]
                } else {
                    [0; 4]
                },
                udp_port: if configured { 47809 } else { 47808 },
                network_number: if configured { 5 } else { 0 },
                ..Default::default()
            },
        )
        .unwrap();
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        // Independently specified application bytes, including property399 and DNS.
        type ExpectedRead = Result<&'static [u8], ErrorCode>;
        let cases: &[(P, Option<u32>, ExpectedRead)] = &[
            (P::NETWORK_TYPE, None, Ok(&[0x91, 5])),
            (
                P::NETWORK_TYPE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::PROTOCOL_LEVEL, None, Ok(&[0x91, 2])),
            (
                P::PROTOCOL_LEVEL,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::BACNET_IP_MODE, None, Ok(&[0x91, 0])),
            (
                P::BACNET_IP_MODE,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::APDU_LENGTH, None, Ok(&[0x22, 5, 0xc4])),
            (
                P::APDU_LENGTH,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::LINK_SPEED, None, Ok(&[0x44, 0, 0, 0, 0])),
            (
                P::LINK_SPEED,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::CHANGES_PENDING, None, Ok(&[0x10])),
            (
                P::CHANGES_PENDING,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::IP_SUBNET_MASK, None, Ok(&[0x64, 0, 0, 0, 0])),
            (
                P::IP_SUBNET_MASK,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (P::IP_DEFAULT_GATEWAY, None, Ok(&[0x64, 0, 0, 0, 0])),
            (
                P::IP_DEFAULT_GATEWAY,
                Some(0),
                Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            ),
            (
                P::NETWORK_NUMBER,
                None,
                Ok(if configured { &[0x21, 5] } else { &[0x21, 0] }),
            ),
            (
                P::NETWORK_NUMBER_QUALITY,
                None,
                Ok(if configured { &[0x91, 3] } else { &[0x91, 0] }),
            ),
            (
                P::MAC_ADDRESS,
                None,
                Ok(if configured {
                    &[0x65, 6, 192, 168, 1, 100, 0xba, 0xc1]
                } else {
                    &[0x65, 6, 0, 0, 0, 0, 0xba, 0xc0]
                }),
            ),
            (
                P::IP_ADDRESS,
                None,
                Ok(if configured {
                    &[0x64, 192, 168, 1, 100]
                } else {
                    &[0x64, 0, 0, 0, 0]
                }),
            ),
            (
                P::BACNET_IP_UDP_PORT,
                None,
                Ok(if configured {
                    &[0x22, 0xba, 0xc1]
                } else {
                    &[0x22, 0xba, 0xc0]
                }),
            ),
            (P::IP_DNS_SERVER, None, Ok(&[0x64, 0, 0, 0, 0])),
            (P::IP_DNS_SERVER, Some(0), Ok(&[0x21, 1])),
            (P::IP_DNS_SERVER, Some(1), Ok(&[0x64, 0, 0, 0, 0])),
            (
                P::IP_DNS_SERVER,
                Some(2),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::IP_DNS_SERVER,
                Some(u32::MAX),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
            ),
            (
                P::MAX_APDU_LENGTH_ACCEPTED,
                None,
                Err(ErrorCode::UNKNOWN_PROPERTY),
            ),
            (P::COMMAND_NP, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (P::EVENT_STATE, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
            (
                P::PROPERTY_LIST,
                None,
                Ok(&[
                    145, 28, 145, 111, 145, 81, 145, 103, 146, 1, 171, 146, 1, 226, 146, 1, 169,
                    146, 1, 170, 146, 1, 167, 146, 1, 143, 146, 1, 164, 146, 1, 160, 146, 1, 152,
                    146, 1, 144, 146, 1, 145, 146, 1, 155, 146, 1, 156, 146, 1, 150,
                ]),
            ),
            (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 18])),
            (P::PROPERTY_LIST, Some(1), Ok(&[145, 28])),
            (P::PROPERTY_LIST, Some(2), Ok(&[145, 111])),
            (P::PROPERTY_LIST, Some(3), Ok(&[145, 81])),
            (P::PROPERTY_LIST, Some(4), Ok(&[145, 103])),
            (P::PROPERTY_LIST, Some(5), Ok(&[146, 1, 171])),
            (P::PROPERTY_LIST, Some(6), Ok(&[146, 1, 226])),
            (P::PROPERTY_LIST, Some(7), Ok(&[146, 1, 169])),
            (P::PROPERTY_LIST, Some(8), Ok(&[146, 1, 170])),
            (P::PROPERTY_LIST, Some(9), Ok(&[146, 1, 167])),
            (P::PROPERTY_LIST, Some(10), Ok(&[146, 1, 143])),
            (P::PROPERTY_LIST, Some(11), Ok(&[146, 1, 164])),
            (P::PROPERTY_LIST, Some(12), Ok(&[146, 1, 160])),
            (P::PROPERTY_LIST, Some(13), Ok(&[146, 1, 152])),
            (P::PROPERTY_LIST, Some(14), Ok(&[146, 1, 144])),
            (P::PROPERTY_LIST, Some(15), Ok(&[146, 1, 145])),
            (P::PROPERTY_LIST, Some(16), Ok(&[146, 1, 155])),
            (P::PROPERTY_LIST, Some(17), Ok(&[146, 1, 156])),
            (P::PROPERTY_LIST, Some(18), Ok(&[146, 1, 150])),
            (
                P::PROPERTY_LIST,
                Some(19),
                Err(ErrorCode::INVALID_ARRAY_INDEX),
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
                    assert!(
                        matches!(rp, Err(Error::Remote(RemoteError::Error { class, code }))
                        if class == ErrorClass::PROPERTY && code == expected)
                    );
                    assert!(response.is_empty());
                }
            }
        }
        use crate::handlers::{ReadFailure, rpm_budget::handle_rpm_budgeted};
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
            Err(ReadFailure::Work)
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
            Err(ReadFailure::Bytes)
        ));
        assert_eq!(&prefix[..], b"prefix");
    }
}
