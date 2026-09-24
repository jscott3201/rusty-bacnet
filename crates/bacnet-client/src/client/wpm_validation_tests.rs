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

fn specs(priority: Option<u8>) -> Vec<bacnet_services::wpm::WriteAccessSpecification> {
    use bacnet_services::{common::BACnetPropertyValue, wpm::WriteAccessSpecification};
    vec![WriteAccessSpecification {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 123).unwrap(),
        list_of_properties: vec![BACnetPropertyValue {
            property_identifier: PropertyIdentifier::DESCRIPTION,
            property_array_index: Some(0),
            value: vec![0],
            priority,
        }],
    }]
}
async fn write(
    client: &BACnetClient<LoopbackTransport>,
    path: u32,
    specs: Vec<bacnet_services::wpm::WriteAccessSpecification>,
) -> Result<(), Error> {
    if path == 0 {
        client.write_property_multiple(&[2], specs).await
    } else {
        client.write_property_multiple_to_device(path, specs).await
    }
}

#[tokio::test]
async fn wpm_validation_invalid_direct_local_routed_and_unknown_never_resolve_admit_or_send() {
    let (mut client, mut peer, mut received) = setup().await;
    let mut cases = vec![vec![]];
    let mut empty = specs(None);
    empty[0].list_of_properties.clear();
    cases.push(empty.clone());
    let mut late_empty = specs(None);
    late_empty.extend(empty);
    cases.push(late_empty);
    for selector in [
        PropertyIdentifier::ALL,
        PropertyIdentifier::REQUIRED,
        PropertyIdentifier::OPTIONAL,
    ] {
        let mut invalid = specs(None);
        invalid[0].list_of_properties[0].property_identifier = selector;
        let mut late = specs(Some(8));
        late.extend(invalid);
        cases.push(late);
    }
    for priority in [0, 17, 255] {
        let mut late = specs(None);
        late.extend(specs(Some(priority)));
        cases.push(late);
    }
    for invalid in cases {
        for path in [0, 10, 11, 999] {
            let _lookup = client.device_table.lock().await;
            let error = timeout(
                Duration::from_secs(1),
                write(&client, path, invalid.clone()),
            )
            .await
            .unwrap()
            .unwrap_err();
            assert!(matches!(error, Error::Encoding(_)));
            assert_eq!(client.tsm.lock().await.coordinated_active_count(), 0);
            assert_eq!(client.tsm.lock().await.pending_count(), 0);
            assert!(received.try_recv().is_err());
        }
    }
    assert!(timeout(Duration::from_millis(25), received.recv())
        .await
        .is_err());
    client.stop().await.unwrap();
    peer.stop().await.unwrap();
}

#[tokio::test]
async fn wpm_validation_valid_direct_local_routed_preserve_exact_wire_and_complete() {
    let (mut client, mut peer, mut received) = setup().await;
    for path in [0, 10, 11] {
        for priority in std::iter::once(None).chain((1..=16).map(Some)) {
            let send = write(&client, path, specs(priority));
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
                    panic!("expected WPM");
                };
                assert_eq!(
                    request.service_choice,
                    ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE
                );
                // Independent bytes: Device123, Description, index0, application NULL.
                let mut expected = vec![
                    0x0c, 0x02, 0x00, 0x00, 123, 0x1e, 0x09, 28, 0x19, 0, 0x2e, 0, 0x2f,
                ];
                if let Some(priority) = priority {
                    expected.extend_from_slice(&[0x39, priority]);
                }
                expected.push(0x1f);
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
