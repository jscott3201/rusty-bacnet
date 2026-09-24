use super::*;
use bacnet_services::list_manipulation::ListElementRequest;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_types::{
    enums::{ObjectType, PropertyIdentifier},
    primitives::ObjectIdentifier,
};

async fn write(
    client: &BACnetClient<LoopbackTransport>,
    add: bool,
    bytes: Vec<u8>,
    index: Option<u32>,
) -> Result<(), Error> {
    let oid = ObjectIdentifier::new(ObjectType::NOTIFICATION_CLASS, 1).unwrap();
    let pid = PropertyIdentifier::from_raw(600);
    if add {
        client.add_list_element(&[2], oid, pid, index, bytes).await
    } else {
        client
            .remove_list_element(&[2], oid, pid, index, bytes)
            .await
    }
}

#[tokio::test]
async fn list_validation_invalid_requests_do_not_admit_or_send() {
    let (transport, mut peer) = LoopbackTransport::pair(vec![1], vec![2]);
    let mut received = peer.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(transport)
        .build()
        .await
        .unwrap();
    let cases = [
        (vec![], None),
        (vec![0], Some(0)),
        (vec![0x21, 7, 0x22, 1], None),
        (vec![0x0e, 0x1f], Some(1)),
        (vec![0x3f], None),
    ];
    for add in [false, true] {
        for (bytes, index) in &cases {
            let error = timeout(
                Duration::from_secs(1),
                write(&client, add, bytes.clone(), *index),
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
async fn list_validation_both_services_preserve_structural_values_and_exact_wire() {
    let (transport, mut peer) = LoopbackTransport::pair(vec![1], vec![2]);
    let mut received = peer.start().await.unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(transport)
        .build()
        .await
        .unwrap();
    for add in [false, true] {
        for body in [
            vec![0x60],
            vec![0x10, 0x11, 0x21, 7],
            vec![0xf8, 254],
            vec![0x0e, 0x1e, 0x08, 0x1f, 0x0f],
            vec![0xd1, 0],
        ] {
            for index in [None, Some(1), Some(u32::MAX)] {
                let send = write(&client, add, body.clone(), index);
                let reply = async {
                    let received = timeout(Duration::from_secs(1), received.recv())
                        .await
                        .unwrap()
                        .unwrap();
                    let npdu = bacnet_encoding::npdu::decode_npdu(received.npdu).unwrap();
                    let Apdu::ConfirmedRequest(request) = apdu::decode_apdu(npdu.payload).unwrap()
                    else {
                        panic!("expected list request")
                    };
                    assert_eq!(
                        request.service_choice,
                        if add {
                            ConfirmedServiceChoice::ADD_LIST_ELEMENT
                        } else {
                            ConfirmedServiceChoice::REMOVE_LIST_ELEMENT
                        }
                    );
                    let mut expected = vec![0x0c, 0x03, 0xc0, 0, 1, 0x1a, 2, 0x58];
                    match index {
                        None => {}
                        Some(1) => expected.extend_from_slice(&[0x29, 1]),
                        Some(_) => expected.extend_from_slice(&[0x2c, 255, 255, 255, 255]),
                    }
                    expected.push(0x3e);
                    expected.extend_from_slice(&body);
                    expected.push(0x3f);
                    assert_eq!(request.service_request.as_ref(), expected);
                    assert_eq!(
                        ListElementRequest::decode(&request.service_request)
                            .unwrap()
                            .list_of_elements,
                        body
                    );
                    let mut apdu = BytesMut::new();
                    encode_apdu(
                        &mut apdu,
                        &Apdu::SimpleAck(SimpleAck {
                            invoke_id: request.invoke_id,
                            service_choice: request.service_choice,
                        }),
                    )
                    .unwrap();
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
                };
                let (result, ()) = tokio::join!(send, reply);
                result.unwrap();
                assert_eq!(client.tsm.lock().await.coordinated_active_count(), 0);
            }
        }
    }
    client.stop().await.unwrap();
    peer.stop().await.unwrap();
}
