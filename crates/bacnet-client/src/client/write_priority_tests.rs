use super::*;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_types::enums::{ObjectType, PropertyIdentifier, Segmentation};
use bacnet_types::primitives::ObjectIdentifier;

async fn setup() -> (
    BACnetClient<LoopbackTransport>,
    LoopbackTransport,
    mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>,
) {
    let (transport, mut peer) = LoopbackTransport::pair(vec![1], vec![2]);
    let received = peer.start().await.unwrap();
    let client = BACnetClient::generic_builder()
        .transport(transport)
        .build()
        .await
        .unwrap();
    client.add_device(10, &[2]).await.unwrap();
    client
        .add_routed_device(RoutedDeviceConfig {
            instance: 11,
            router_mac: vec![2],
            remote_network: 100,
            remote_mac: vec![3],
            max_apdu_length: 1476,
            segmentation_supported: Segmentation::NONE,
            max_segments_accepted: None,
        })
        .await
        .unwrap();
    (client, peer, received)
}

async fn write(
    client: &BACnetClient<LoopbackTransport>,
    path: u32,
    priority: Option<u8>,
) -> Result<(), Error> {
    let oid = ObjectIdentifier::new(ObjectType::DEVICE, 123).unwrap();
    if path == 0 {
        client
            .write_property(
                &[2],
                oid,
                PropertyIdentifier::DESCRIPTION,
                Some(0),
                vec![0],
                priority,
            )
            .await
    } else {
        client
            .write_property_to_device(
                path,
                oid,
                PropertyIdentifier::DESCRIPTION,
                Some(0),
                vec![0],
                priority,
            )
            .await
    }
}

#[tokio::test]
async fn write_priority_invalid_direct_local_routed_and_batch_never_admit_or_send() {
    let (mut client, mut peer, mut received) = setup().await;
    for priority in [0, 17, 255] {
        for path in [0, 10, 11, 999] {
            // Holding the lookup table also proves invalid input is rejected before resolution.
            let _lookup = client.device_table.lock().await;
            let error = timeout(Duration::from_secs(1), write(&client, path, Some(priority)))
                .await
                .unwrap()
                .unwrap_err();
            assert!(matches!(error, Error::Encoding(message) if message.contains("priority")));
            assert_eq!(client.tsm.lock().await.coordinated_active_count(), 0);
            assert_eq!(client.tsm.lock().await.pending_count(), 0);
            assert!(received.try_recv().is_err());
        }
        let results = client
            .write_property_to_devices(
                vec![DeviceWriteRequest {
                    device_instance: 11,
                    object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 123).unwrap(),
                    property_identifier: PropertyIdentifier::DESCRIPTION,
                    property_array_index: None,
                    property_value: vec![0],
                    priority: Some(priority),
                }],
                None,
            )
            .await;
        assert!(matches!(results[0].result, Err(Error::Encoding(_))));
    }
    assert!(timeout(Duration::from_millis(25), received.recv())
        .await
        .is_err());
    assert_eq!(client.tsm.lock().await.coordinated_active_count(), 0);
    client.stop().await.unwrap();
    peer.stop().await.unwrap();
}

#[tokio::test]
async fn write_priority_valid_direct_local_routed_preserve_exact_wire_and_complete() {
    let (mut client, mut peer, mut received) = setup().await;
    for path in [0, 10, 11] {
        for priority in std::iter::once(None).chain((1..=16).map(Some)) {
            let send = write(&client, path, priority);
            let reply = async {
                let received = timeout(Duration::from_secs(1), received.recv())
                    .await
                    .unwrap()
                    .unwrap();
                let npdu = decode_npdu(received.npdu).unwrap();
                if path == 11 {
                    let destination = npdu.destination.as_ref().unwrap();
                    assert_eq!(destination.network, 100);
                    assert_eq!(destination.mac_address.as_slice(), &[3]);
                } else {
                    assert!(npdu.destination.is_none());
                }
                let Apdu::ConfirmedRequest(request) = apdu::decode_apdu(npdu.payload).unwrap()
                else {
                    panic!("expected WP");
                };
                assert_eq!(
                    request.service_choice,
                    ConfirmedServiceChoice::WRITE_PROPERTY
                );
                // Independent bytes: Device123, Description, index0, application NULL.
                let mut expected = vec![
                    0x0c, 0x02, 0x00, 0x00, 123, 0x19, 28, 0x29, 0, 0x3e, 0, 0x3f,
                ];
                if let Some(priority) = priority {
                    expected.extend_from_slice(&[0x49, priority]);
                }
                assert_eq!(request.service_request.as_ref(), expected);
                let ack = Apdu::SimpleAck(SimpleAck {
                    invoke_id: request.invoke_id,
                    service_choice: request.service_choice,
                });
                if path == 11 {
                    super::tests::send_routed_response(&peer, &[1], 100, &[3], ack).await;
                } else {
                    let mut apdu = BytesMut::new();
                    encode_apdu(&mut apdu, &ack).unwrap();
                    let mut npdu = BytesMut::new();
                    encode_npdu(
                        &mut npdu,
                        &Npdu {
                            payload: apdu.freeze(),
                            ..Default::default()
                        },
                    )
                    .unwrap();
                    peer.send_unicast(&npdu, &[1]).await.unwrap();
                }
            };
            let (result, ()) = tokio::join!(send, reply);
            result.unwrap();
            assert_eq!(client.tsm.lock().await.coordinated_active_count(), 0);
        }
    }
    client.stop().await.unwrap();
    peer.stop().await.unwrap();
}
