//! Both segmentation directions through the trusted outbound transport intake.
use super::*;
use bacnet_encoding::apdu::SegmentAck;
use bacnet_types::enums::Segmentation;

#[tokio::test]
async fn outbound_tls_server_reassembles_requests_and_completes_segmented_replies() {
    let ca = TestCa::new();
    let (mut peer, port) = Peer::start(&ca).await;
    let name = "segmented".repeat(40);
    let config = bacnet_server::server::ServerConfig {
        device: bacnet_objects::DeviceIdentity::default()
            .with_max_apdu(480)
            .expect("valid Device APDU")
            .with_segmentation(Segmentation::BOTH),
        ..Default::default()
    };
    let mut server =
        bacnet_server::server::BACnetServer::start_clockless(config, database(&name), port)
            .await
            .unwrap();
    // Reusing the same InvokeID after the final ACK also observes pending
    // ownership release. Ordered frames traverse the same actual TLS socket.
    for _ in 0..2 {
        let Apdu::ConfirmedRequest(mut request) = read_name(81) else {
            unreachable!()
        };
        request.max_apdu_length = 50;
        request.max_segments = Some(16);
        let bytes = request.service_request.clone();
        let split = bytes.len() / 2;
        for (sequence, piece) in [bytes.slice(..split), bytes.slice(split..)]
            .into_iter()
            .enumerate()
        {
            request.segmented = true;
            request.more_follows = sequence == 0;
            request.sequence_number = Some(sequence as u8);
            request.proposed_window_size = Some(1);
            request.service_request = piece;
            peer.send(&Apdu::ConfirmedRequest(request.clone())).await;
            assert!(
                matches!(peer.receive().await, Apdu::SegmentAck(a) if a.sent_by_server && !a.negative_ack && a.invoke_id==81 && a.sequence_number==sequence as u8)
            );
        }
        let mut payload = BytesMut::new();
        let mut sequence = 0;
        loop {
            let Apdu::ComplexAck(segment) = peer.receive().await else {
                panic!("segmented reply")
            };
            assert!(segment.segmented);
            assert_eq!(segment.invoke_id, 81);
            assert_eq!(segment.sequence_number, Some(sequence));
            assert_eq!(segment.proposed_window_size, Some(1));
            assert_eq!(
                segment.service_choice,
                ConfirmedServiceChoice::READ_PROPERTY
            );
            assert!(segment.service_ack.len() + 5 <= 50);
            payload.extend_from_slice(&segment.service_ack);
            peer.send(&Apdu::SegmentAck(SegmentAck {
                sent_by_server: false,
                negative_ack: false,
                invoke_id: 81,
                sequence_number: sequence,
                actual_window_size: 1,
            }))
            .await;
            if !segment.more_follows {
                break;
            }
            sequence += 1;
            assert!(sequence < 16);
        }
        assert!(sequence > 0);
        let result = ReadPropertyACK::decode(&payload).unwrap();
        assert_eq!(result.object_identifier, csv_oid());
        assert_eq!(result.property_identifier, PropertyIdentifier::OBJECT_NAME);
        assert!(result.property_value.ends_with(name.as_bytes()));
    }
    server.stop().await.unwrap();
    peer.stop().await;
}
