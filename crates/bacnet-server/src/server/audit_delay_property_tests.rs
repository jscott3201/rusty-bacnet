use super::*;

#[tokio::test]
async fn delayed_target_audit_pair_wire_presence_scalar_null_and_wpm_prefix() {
    use PropertyIdentifier as P;
    for present in [false, true] {
        let configured = if present {
            super::super::batching::delayed(0)
        } else {
            reporter()
        };
        let mut f = server(configured).await;
        let optional = rpm_wire(&f.server, P::OPTIONAL, None).await;
        let list = read_wire(&f.server, P::PROPERTY_LIST, None).await.unwrap();
        for property in [P::MAXIMUM_SEND_DELAY, P::SEND_NOW] {
            assert_eq!(
                optional.iter().any(|r| r.property_identifier == property),
                present
            );
            let mut id = BytesMut::new();
            bacnet_encoding::primitives::encode_app_enumerated(&mut id, property.to_raw());
            assert_eq!(
                list.windows(id.len()).any(|bytes| bytes == id.as_ref()),
                present
            );
            for index in [None, Some(0), Some(1)] {
                let response = read_wire(&f.server, property, index).await;
                if !present && index.is_none() {
                    assert_eq!(
                        response,
                        Err((ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY))
                    );
                } else if index.is_some() {
                    assert_eq!(
                        response,
                        Err((ErrorClass::PROPERTY, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY))
                    );
                } else {
                    assert!(response.is_ok());
                }
                for multiple in [false, true] {
                    let mut bytes = BytesMut::new();
                    let service = if multiple {
                        WritePropertyMultipleRequest {
                            list_of_write_access_specs: vec![WriteAccessSpecification {
                                object_identifier: oid(ObjectType::AUDIT_REPORTER, 1),
                                list_of_properties: vec![BACnetPropertyValue {
                                    property_identifier: property,
                                    property_array_index: index,
                                    value: vec![0],
                                    priority: None,
                                }],
                            }],
                        }
                        .encode(&mut bytes)
                        .unwrap();
                        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE
                    } else {
                        WritePropertyRequest {
                            object_identifier: oid(ObjectType::AUDIT_REPORTER, 1),
                            property_identifier: property,
                            property_array_index: index,
                            property_value: vec![0],
                            priority: None,
                        }
                        .encode(&mut bytes)
                        .unwrap();
                        ConfirmedServiceChoice::WRITE_PROPERTY
                    };
                    let response = dispatch(&f.server, service, bytes.freeze()).await;
                    if present && index.is_none() {
                        assert!(matches!(response, Apdu::SimpleAck(_)));
                    } else {
                        let Apdu::Error(error) = response else {
                            panic!("expected error {response:?}");
                        };
                        assert_eq!(
                            error.error_code,
                            if index.is_some() {
                                ErrorCode::PROPERTY_IS_NOT_AN_ARRAY
                            } else {
                                ErrorCode::UNKNOWN_PROPERTY
                            }
                        );
                    }
                }
            }
        }
        if present {
            let mut bytes = BytesMut::new();
            WritePropertyMultipleRequest {
                list_of_write_access_specs: vec![WriteAccessSpecification {
                    object_identifier: oid(ObjectType::AUDIT_REPORTER, 1),
                    list_of_properties: vec![
                        element(P::MAXIMUM_SEND_DELAY, vec![0x21, 2]),
                        element(P::MAXIMUM_SEND_DELAY, vec![0x22, 0x0e, 0x11]),
                        element(P::MAXIMUM_SEND_DELAY, vec![0x21, 3]),
                    ],
                }],
            }
            .encode(&mut bytes)
            .unwrap();
            assert!(matches!(
                dispatch(
                    &f.server,
                    ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
                    bytes.freeze()
                )
                .await,
                Apdu::Error(_)
            ));
            assert_eq!(
                read_wire(&f.server, P::MAXIMUM_SEND_DELAY, None)
                    .await
                    .unwrap(),
                vec![0x21, 2]
            );
        }
        f.server.stop().await.unwrap();
    }
}
