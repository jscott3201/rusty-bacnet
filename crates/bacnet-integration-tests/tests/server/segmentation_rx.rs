use super::*;

// ---------------------------------------------------------------------------
// Server Rx Segmentation integration tests
// ---------------------------------------------------------------------------

fn read_property_service_payload() -> Vec<u8> {
    use bacnet_services::read_property::ReadPropertyRequest;

    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let rp_req = ReadPropertyRequest {
        object_identifier: ai_oid,
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    };
    let mut service_buf = BytesMut::new();
    rp_req.encode(&mut service_buf);
    service_buf.to_vec()
}

fn segmented_read_property_pdu(
    invoke_id: u8,
    seq: u8,
    more_follows: bool,
    proposed_window_size: Option<u8>,
    service_request: Bytes,
) -> Apdu {
    Apdu::ConfirmedRequest(ConfirmedRequestPdu {
        segmented: true,
        more_follows,
        segmented_response_accepted: true,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id,
        sequence_number: Some(seq),
        proposed_window_size,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request,
    })
}

async fn send_raw_apdu(
    raw_network: &Arc<NetworkLayer<BipTransport>>,
    server_mac: &[u8],
    apdu: Apdu,
) {
    let mut buf = BytesMut::new();
    encode_apdu(&mut buf, &apdu).expect("valid APDU encoding");
    raw_network
        .send_apdu(&buf, server_mac, true, NetworkPriority::NORMAL)
        .await
        .unwrap();
}

/// When a client sends a segmented ConfirmedRequest (ReadProperty split across
/// two segments), the server should reassemble it, process the full request,
/// and return a correct ReadPropertyACK.
#[tokio::test]
async fn server_handles_segmented_request() {
    use bacnet_encoding::apdu::{self, encode_apdu, Apdu, ConfirmedRequest as ConfirmedRequestPdu};
    use bacnet_network::layer::NetworkLayer;
    use bacnet_services::read_property::ReadPropertyRequest;
    use bacnet_types::enums::ConfirmedServiceChoice;
    use bytes::BytesMut;
    use tokio::time::Duration;

    // 1. Build a server with an AnalogInput (present_value = 72.5).
    let mut server = make_server().await;
    let server_mac = server.local_mac().to_vec();

    // 2. Build a raw NetworkLayer to act as a "manual client".
    let raw_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    let mut raw_network = NetworkLayer::new(raw_transport);
    let mut raw_rx = raw_network.start().await.unwrap();
    let raw_network = std::sync::Arc::new(raw_network);

    // 3. Encode a ReadProperty service request for AI:1 Present_Value.
    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let rp_req = ReadPropertyRequest {
        object_identifier: ai_oid,
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    };
    let mut service_buf = BytesMut::new();
    rp_req.encode(&mut service_buf);
    let full_payload = service_buf.to_vec();

    // 4. Split the payload into 2 segments (artificially, even though it is small).
    let mid = full_payload.len() / 2;
    let seg0_data = Bytes::copy_from_slice(&full_payload[..mid]);
    let seg1_data = Bytes::copy_from_slice(&full_payload[mid..]);

    let invoke_id: u8 = 42;

    // 5. Send segment 0 (more_follows = true).
    let seg0 = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
        segmented: true,
        more_follows: true,
        segmented_response_accepted: true,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id,
        sequence_number: Some(0),
        proposed_window_size: Some(1),
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: seg0_data,
    });

    let mut buf = BytesMut::new();
    encode_apdu(&mut buf, &seg0).expect("valid APDU encoding");
    raw_network
        .send_apdu(
            &buf,
            &server_mac,
            true,
            bacnet_types::enums::NetworkPriority::NORMAL,
        )
        .await
        .unwrap();

    // 6. Wait for the SegmentAck from the server.
    let ack0 = tokio::time::timeout(Duration::from_secs(3), raw_rx.recv())
        .await
        .expect("Timed out waiting for SegmentAck 0")
        .expect("Channel closed while waiting for SegmentAck 0");
    let decoded_ack0 = apdu::decode_apdu(ack0.apdu.clone()).unwrap();
    match decoded_ack0 {
        Apdu::SegmentAck(sa) => {
            assert_eq!(sa.invoke_id, invoke_id);
            assert_eq!(sa.sequence_number, 0);
            assert!(sa.sent_by_server);
            assert!(!sa.negative_ack);
        }
        other => panic!("Expected SegmentAck, got {:?}", other),
    }

    // 7. Send segment 1 (more_follows = false -- last segment).
    let seg1 = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
        segmented: true,
        more_follows: false,
        segmented_response_accepted: true,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id,
        sequence_number: Some(1),
        proposed_window_size: Some(1),
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: seg1_data,
    });

    let mut buf = BytesMut::new();
    encode_apdu(&mut buf, &seg1).expect("valid APDU encoding");
    raw_network
        .send_apdu(
            &buf,
            &server_mac,
            true,
            bacnet_types::enums::NetworkPriority::NORMAL,
        )
        .await
        .unwrap();

    // 8. Wait for SegmentAck for segment 1, then the ReadPropertyACK response.
    //    The server sends a SegmentAck for every segment, then processes the
    //    reassembled request and sends the ComplexAck response.
    let mut got_seg_ack_1 = false;
    let mut got_response = false;

    for _ in 0..3 {
        let received = tokio::time::timeout(Duration::from_secs(3), raw_rx.recv())
            .await
            .expect("Timed out waiting for response")
            .expect("Channel closed while waiting for response");
        let decoded = apdu::decode_apdu(received.apdu.clone()).unwrap();
        match decoded {
            Apdu::SegmentAck(sa) => {
                assert_eq!(sa.invoke_id, invoke_id);
                assert_eq!(sa.sequence_number, 1);
                got_seg_ack_1 = true;
            }
            Apdu::ComplexAck(ack) => {
                assert_eq!(ack.invoke_id, invoke_id);
                assert_eq!(ack.service_choice, ConfirmedServiceChoice::READ_PROPERTY);
                // Decode the ReadPropertyACK to verify the value.
                let rp_ack =
                    bacnet_services::read_property::ReadPropertyACK::decode(&ack.service_ack)
                        .unwrap();
                assert_eq!(rp_ack.object_identifier, ai_oid);
                assert_eq!(
                    rp_ack.property_identifier,
                    PropertyIdentifier::PRESENT_VALUE
                );
                let (val, _) = bacnet_encoding::primitives::decode_application_value(
                    &rp_ack.property_value,
                    0,
                )
                .unwrap();
                assert_eq!(val, bacnet_types::primitives::PropertyValue::Real(72.5));
                got_response = true;
            }
            other => panic!("Unexpected APDU: {:?}", other),
        }
        if got_seg_ack_1 && got_response {
            break;
        }
    }

    assert!(
        got_seg_ack_1,
        "Should have received SegmentAck for segment 1"
    );
    assert!(
        got_response,
        "Should have received ReadPropertyACK response"
    );

    server.stop().await.unwrap();
}

#[tokio::test]
async fn server_aborts_segmented_request_with_invalid_window_size() {
    let mut server = make_server().await;
    let server_mac = server.local_mac().to_vec();

    let raw_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    let mut raw_network = NetworkLayer::new(raw_transport);
    let mut raw_rx = raw_network.start().await.unwrap();
    let raw_network = Arc::new(raw_network);

    let payload = read_property_service_payload();
    let invoke_id = 43;
    let seg0 = segmented_read_property_pdu(
        invoke_id,
        0,
        true,
        Some(1),
        Bytes::copy_from_slice(&payload[..payload.len() / 2]),
    );
    let mut buf = BytesMut::new();
    encode_apdu(&mut buf, &seg0).expect("valid APDU encoding");
    buf[4] = 0;
    raw_network
        .send_apdu(&buf, &server_mac, true, NetworkPriority::NORMAL)
        .await
        .unwrap();

    let received = tokio::time::timeout(Duration::from_secs(3), raw_rx.recv())
        .await
        .expect("Timed out waiting for Abort")
        .expect("Channel closed while waiting for Abort");
    let decoded = bacnet_encoding::apdu::decode_apdu(received.apdu).unwrap();
    match decoded {
        Apdu::Abort(abort) => {
            assert!(abort.sent_by_server);
            assert_eq!(abort.invoke_id, invoke_id);
            assert_eq!(abort.abort_reason, AbortReason::WINDOW_SIZE_OUT_OF_RANGE);
        }
        other => panic!("Expected Abort, got {:?}", other),
    }

    server.stop().await.unwrap();
}

#[tokio::test]
async fn server_naks_segmented_request_gap_with_last_good_sequence() {
    let mut server = make_server().await;
    let server_mac = server.local_mac().to_vec();

    let raw_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    let mut raw_network = NetworkLayer::new(raw_transport);
    let mut raw_rx = raw_network.start().await.unwrap();
    let raw_network = Arc::new(raw_network);

    let payload = read_property_service_payload();
    let third = payload.len() / 3;
    let invoke_id = 44;
    let seg0 = segmented_read_property_pdu(
        invoke_id,
        0,
        true,
        Some(2),
        Bytes::copy_from_slice(&payload[..third]),
    );
    send_raw_apdu(&raw_network, &server_mac, seg0).await;

    assert!(
        tokio::time::timeout(Duration::from_millis(200), raw_rx.recv())
            .await
            .is_err(),
        "server should not ACK before the negotiated window boundary"
    );

    let seg2 = segmented_read_property_pdu(
        invoke_id,
        2,
        true,
        Some(2),
        Bytes::copy_from_slice(&payload[third * 2..]),
    );
    send_raw_apdu(&raw_network, &server_mac, seg2).await;

    let received = tokio::time::timeout(Duration::from_secs(3), raw_rx.recv())
        .await
        .expect("Timed out waiting for negative SegmentAck")
        .expect("Channel closed while waiting for negative SegmentAck");
    let decoded = bacnet_encoding::apdu::decode_apdu(received.apdu).unwrap();
    match decoded {
        Apdu::SegmentAck(sa) => {
            assert!(sa.sent_by_server);
            assert!(sa.negative_ack);
            assert_eq!(sa.invoke_id, invoke_id);
            assert_eq!(sa.sequence_number, 0);
            assert_eq!(sa.actual_window_size, 2);
        }
        other => panic!("Expected SegmentAck, got {:?}", other),
    }

    server.stop().await.unwrap();
}

#[tokio::test]
async fn server_acks_segmented_request_at_window_boundary() {
    let mut server = make_server().await;
    let server_mac = server.local_mac().to_vec();

    let raw_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    let mut raw_network = NetworkLayer::new(raw_transport);
    let mut raw_rx = raw_network.start().await.unwrap();
    let raw_network = Arc::new(raw_network);

    let payload = read_property_service_payload();
    let mid = payload.len() / 2;
    let invoke_id = 45;

    let seg0 = segmented_read_property_pdu(
        invoke_id,
        0,
        true,
        Some(2),
        Bytes::copy_from_slice(&payload[..mid]),
    );
    send_raw_apdu(&raw_network, &server_mac, seg0).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(200), raw_rx.recv())
            .await
            .is_err(),
        "server should wait for the second segment before ACKing a two-segment window"
    );

    let seg1 = segmented_read_property_pdu(
        invoke_id,
        1,
        true,
        Some(2),
        Bytes::copy_from_slice(&payload[mid..]),
    );
    send_raw_apdu(&raw_network, &server_mac, seg1).await;

    let received = tokio::time::timeout(Duration::from_secs(3), raw_rx.recv())
        .await
        .expect("Timed out waiting for SegmentAck")
        .expect("Channel closed while waiting for SegmentAck");
    let decoded = bacnet_encoding::apdu::decode_apdu(received.apdu).unwrap();
    match decoded {
        Apdu::SegmentAck(sa) => {
            assert!(sa.sent_by_server);
            assert!(!sa.negative_ack);
            assert_eq!(sa.invoke_id, invoke_id);
            assert_eq!(sa.sequence_number, 1);
            assert_eq!(sa.actual_window_size, 2);
        }
        other => panic!("Expected SegmentAck, got {:?}", other),
    }

    server.stop().await.unwrap();
}
