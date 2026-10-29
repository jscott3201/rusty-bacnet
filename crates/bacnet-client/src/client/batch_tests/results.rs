use super::*;
use bacnet_encoding::{apdu::decode_apdu, npdu::decode_npdu};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::RemoteError;

async fn outcomes(
    client: &BACnetClient<LoopbackTransport>,
    service: u8,
) -> Vec<(usize, u32, Result<(), Error>)> {
    let oid = ObjectIdentifier::new(ObjectType::DEVICE, 10).unwrap();
    let pid = PropertyIdentifier::DESCRIPTION;
    let limit = NonZeroUsize::new(2);
    match service {
        12 => client
            .read_property_from_devices(
                vec![
                    DeviceReadRequest {
                        device_instance: 10,
                        object_identifier: oid,
                        property_identifier: pid,
                        property_array_index: None,
                    };
                    2
                ],
                limit,
            )
            .await
            .into_iter()
            .map(|r| {
                assert!(!format!("{r:?}").contains("property_value"));
                (
                    r.request_index,
                    r.device_instance,
                    r.result.map(|ack| {
                        assert_eq!(ack.property_value, vec![0x21, 42]);
                    }),
                )
            })
            .collect(),
        14 => client
            .read_property_multiple_from_devices(
                vec![
                    DeviceRpmRequest {
                        device_instance: 10,
                        specs: vec![bacnet_types::constructed::ReadAccessSpecification {
                            object_identifier: oid,
                            list_of_property_references: vec![
                                bacnet_types::constructed::PropertyReference {
                                    property_identifier: pid,
                                    property_array_index: None,
                                }
                            ],
                        }],
                    };
                    2
                ],
                limit,
            )
            .await
            .into_iter()
            .map(|r| {
                assert!(!format!("{r:?}").contains("property_value"));
                (
                    r.request_index,
                    r.device_instance,
                    r.result.map(|ack| {
                        assert_eq!(
                            ack.list_of_read_access_results[0].list_of_results[0].property_value,
                            Some(vec![0x21, 42])
                        );
                    }),
                )
            })
            .collect(),
        15 => client
            .write_property_to_devices(
                vec![
                    DeviceWriteRequest {
                        device_instance: 10,
                        object_identifier: oid,
                        property_identifier: pid,
                        property_array_index: None,
                        property_value: vec![0x21, 42],
                        priority: None,
                    };
                    2
                ],
                limit,
            )
            .await
            .into_iter()
            .map(|r| (r.request_index, r.device_instance, r.result))
            .collect(),
        _ => unreachable!(),
    }
}

async fn reversed(service: u8, reject: bool) {
    let (transport, mut peer) = LoopbackTransport::pair(vec![1], vec![2]);
    let mut rx = peer.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(transport)
        .build()
        .await
        .unwrap();
    client.add_device(10, &[2]).await.unwrap();
    let mut pending = Box::pin(outcomes(&client, service));
    let mut requests = Vec::new();
    for _ in 0..2 {
        let received = tokio::select! {
            _ = &mut pending => panic!("batch completed before replies"),
            frame = timeout(Duration::from_secs(2), rx.recv()) => frame.unwrap().unwrap(),
        };
        let Apdu::ConfirmedRequest(request) =
            decode_apdu(decode_npdu(received.npdu).unwrap().payload).unwrap()
        else {
            panic!("confirmed request")
        };
        requests.push(request);
    }
    assert_eq!(
        requests[0].service_request, requests[1].service_request,
        "identical input occurrences"
    );
    assert_ne!(requests[0].invoke_id, requests[1].invoke_id);
    let mut error = vec![1, 0];
    if reject {
        error.extend([0x60, requests[1].invoke_id, 9]);
    } else {
        error.extend([0x50, requests[1].invoke_id, service, 0x91, 2, 0x91, 32]);
    }
    peer.send_unicast(&error, &[1]).await.unwrap();
    // Poll the owning batch until B's lease is released. A remains unanswered;
    // this is an explicit completion barrier, not an elapsed-time ordering oracle.
    timeout(Duration::from_secs(2), async {
        loop {
            assert!(futures_util::poll!(pending.as_mut()).is_pending());
            if client.tsm.lock().await.coordinated_active_count() == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let mut ack = vec![
        1,
        0,
        if service == 15 { 0x20 } else { 0x30 },
        requests[0].invoke_id,
        service,
    ];
    match service {
        12 => {
            ack.extend_from_slice(&requests[0].service_request);
            ack.extend([0x3e, 0x21, 42, 0x3f]);
        }
        14 => {
            ack.push(0x0c);
            ack.extend(((8u32 << 22) | 10).to_be_bytes());
            ack.extend([0x1e, 0x29, 28, 0x4e, 0x21, 42, 0x4f, 0x1f]);
        }
        _ => {}
    }
    peer.send_unicast(&ack, &[1]).await.unwrap();
    let result = timeout(Duration::from_secs(2), pending).await.unwrap();
    assert_eq!(result.iter().map(|r| r.0).collect::<Vec<_>>(), vec![1, 0]);
    assert!(result.iter().all(|r| r.1 == 10));
    if reject {
        assert!(matches!(
            result[0].2,
            Err(Error::Remote(RemoteError::Reject {
                reason: RejectReason::UNRECOGNIZED_SERVICE
            }))
        ));
    } else {
        assert!(matches!(
            result[0].2,
            Err(Error::Remote(RemoteError::Error {
                class: ErrorClass::PROPERTY,
                code: ErrorCode::UNKNOWN_PROPERTY
            }))
        ));
    }
    assert!(result[1].2.is_ok());
    assert_eq!(client.tsm.lock().await.coordinated_active_count(), 0);
    assert_eq!(client.tsm.lock().await.pending_count(), 0);
    client.stop().await.unwrap();
    peer.stop().await.unwrap();
}

#[tokio::test]
async fn batch_identical_occurrences_reverse_mixed_success_all_families() {
    for service in [12, 14, 15] {
        reversed(service, false).await;
    }
}
#[tokio::test]
async fn batch_reject_keeps_typed_reason_and_occurrence() {
    reversed(12, true).await;
}
#[test]
fn batch_write_debug_omits_encoded_payload() {
    let request = DeviceWriteRequest {
        device_instance: 10,
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 10).unwrap(),
        property_identifier: PropertyIdentifier::DESCRIPTION,
        property_array_index: None,
        property_value: vec![222, 173, 190, 239],
        priority: None,
    };
    let debug = format!("{request:?}");
    assert!(debug.contains("property_value_len: 4"));
    assert!(!debug.contains("222"));
    assert_eq!(request.property_value, vec![222, 173, 190, 239]);
}
