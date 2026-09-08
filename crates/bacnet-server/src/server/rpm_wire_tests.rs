use super::*;
use bacnet_services::{
    common::PropertyReference,
    rpm::{ReadAccessSpecification, ReadPropertyMultipleRequest},
};

#[tokio::test]
async fn rpm_whole_abort_direct_routed_reply_and_segmentation_matrix() {
    for work in [false, true] {
        for segmented_response_accepted in [false, true] {
            for routed in [false, true] {
                for reply in [false, true] {
                    let budget = ReadPropertyMultipleBudget {
                        max_result_elements: if work { 1 } else { 2 },
                        max_service_ack_bytes: if work { 16384 } else { 7 },
                    };
                    let (mut server, tx, mut started) = fixture_with_config(
                        "rpm",
                        ServerConfig {
                            read_property_multiple_budget: budget,
                            segmentation_supported: Segmentation::BOTH,
                            ..Default::default()
                        },
                    )
                    .await;
                    let source = routed.then(|| NpduAddress {
                        network: 7,
                        mac_address: MacAddr::from_slice(&[9]),
                    });
                    let mut service = BytesMut::new();
                    ReadPropertyMultipleRequest {
                        list_of_read_access_specs: vec![ReadAccessSpecification {
                            object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 4194303)
                                .unwrap(),
                            list_of_property_references: vec![
                                PropertyReference {
                                    property_identifier: PropertyIdentifier::OBJECT_NAME,
                                    property_array_index: None,
                                };
                                2
                            ],
                        }],
                    }
                    .encode(&mut service);
                    let mut payload = BytesMut::new();
                    encode_apdu(
                        &mut payload,
                        &Apdu::ConfirmedRequest(ConfirmedRequestPdu {
                            segmented: false,
                            more_follows: false,
                            segmented_response_accepted,
                            max_segments: None,
                            max_apdu_length: 50,
                            invoke_id: 42,
                            sequence_number: None,
                            proposed_window_size: None,
                            service_choice: ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
                            service_request: service.freeze(),
                        }),
                    )
                    .unwrap();
                    let mut npdu = BytesMut::new();
                    encode_npdu(
                        &mut npdu,
                        &Npdu {
                            source: source.clone(),
                            payload: payload.freeze(),
                            ..Default::default()
                        },
                    )
                    .unwrap();
                    let (reply_tx, reply_rx) = oneshot::channel();
                    tx.send(ReceivedNpdu {
                        npdu: npdu.freeze(),
                        source_mac: MacAddr::from_slice(&[1]),
                        link_layer_group: false,
                        data_attributes: Vec::new(),
                        reply_tx: reply.then_some(reply_tx),
                    })
                    .await
                    .unwrap();
                    let response = if reply {
                        let bytes = tokio::time::timeout(Duration::from_secs(2), reply_rx)
                            .await
                            .unwrap()
                            .unwrap();
                        let decoded = decode_npdu(bytes).unwrap();
                        assert_eq!(decoded.destination, source);
                        assert!(server.network.transport().frames.lock().unwrap().is_empty());
                        apdu::decode_apdu(decoded.payload).unwrap()
                    } else {
                        let _completion =
                            tokio::time::timeout(Duration::from_secs(2), started.recv())
                                .await
                                .unwrap()
                                .unwrap();
                        let transport = server.network.transport();
                        assert_eq!(
                            transport.routes.lock().unwrap().as_slice(),
                            &[(source, MacAddr::from_slice(&[1]))]
                        );
                        let frames = transport.frames.lock().unwrap();
                        assert_eq!(frames.len(), 1, "no partial ComplexACK before Abort");
                        frames[0].clone()
                    };
                    assert!(
                        matches!(response, Apdu::Abort(AbortPdu { sent_by_server: true, invoke_id: 42, abort_reason }) if abort_reason == if work { AbortReason::OUT_OF_RESOURCES } else { AbortReason::BUFFER_OVERFLOW }),
                        "{response:?}"
                    );
                    server.stop().await.unwrap();
                }
            }
        }
    }
}
