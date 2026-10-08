//! Receiving identity survives routed envelopes and full-server reassembly.
use super::*;
use bacnet_encoding::npdu::decode_npdu;

async fn send(peer: &UdpSocket, address: SocketAddrV4, apdu: &[u8], routed: bool) {
    // Remote source network 91 deliberately equals the unrelated configured row.
    let mut frame = if routed {
        vec![0x81, 0x0a, 0, 0, 1, 0x0c, 0, 91, 1, 0x44]
    } else {
        vec![0x81, 0x0a, 0, 0, 1, 4]
    };
    frame.extend_from_slice(apdu);
    let len = (frame.len() as u16).to_be_bytes();
    frame[2..4].copy_from_slice(&len);
    peer.send_to(&frame, address).await.unwrap();
}
async fn receive(peer: &UdpSocket, routed: bool) -> Apdu {
    let mut frame = [0; 2048];
    let (size, _) = tokio::time::timeout(Duration::from_secs(3), peer.recv_from(&mut frame))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&frame[..2], &[0x81, 0x0a]);
    let npdu = decode_npdu(Bytes::copy_from_slice(&frame[4..size])).unwrap();
    if routed {
        let destination = npdu.destination.unwrap();
        assert_eq!(destination.network, 91);
        assert_eq!(destination.mac_address.as_slice(), &[0x44]);
    }
    decode_apdu(npdu.payload).unwrap()
}
fn assert_identity(apdu: Apdu) {
    let Apdu::ComplexAck(ack) = apdu else {
        panic!("expected concrete RP ACK: {apdu:?}");
    };
    let ack = ReadPropertyACK::decode(&ack.service_ack).unwrap();
    assert_eq!(ack.object_identifier, port());
    assert_eq!(
        decode_application_value(&ack.property_value, 0).unwrap().0,
        PropertyValue::ObjectIdentifier(port())
    );
}
#[tokio::test]
async fn registered_port_routed_read_uses_receiving_owner_not_remote_number() {
    for full in [true, false] {
        let mut owner = Owner::start(full, true).await;
        let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        send(
            &peer,
            owner.address(),
            &rp(wildcard(), P::OBJECT_IDENTIFIER),
            true,
        )
        .await;
        assert_identity(receive(&peer, true).await);
        owner.stop().await;
    }
}
#[tokio::test]
async fn registered_port_reassembled_routed_read_and_endpoint_segmentation_boundary() {
    for full in [true, false] {
        let mut owner = if full {
            let id = identity().with_segmentation(bacnet_types::enums::Segmentation::BOTH);
            let mut config = id.server_config();
            config.registered_network_port = Some(port());
            config.segmentation_supported = bacnet_types::enums::Segmentation::BOTH;
            Owner::Server(Box::new(
                bacnet_server::server::BACnetServer::start(
                    config,
                    id.build_database().unwrap(),
                    BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST),
                )
                .await
                .unwrap(),
            ))
        } else {
            Owner::start(false, true).await
        };
        let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let Apdu::ConfirmedRequest(mut request) =
            decode_apdu(rp(wildcard(), P::OBJECT_IDENTIFIER)).unwrap()
        else {
            panic!()
        };
        let service = request.service_request.clone();
        request.segmented = true;
        request.proposed_window_size = Some(1);
        for sequence in 0..2 {
            request.sequence_number = Some(sequence);
            request.more_follows = sequence == 0;
            request.service_request = if sequence == 0 {
                service.slice(..3)
            } else {
                service.slice(3..)
            };
            let mut encoded = BytesMut::new();
            encode_apdu(&mut encoded, &Apdu::ConfirmedRequest(request.clone())).unwrap();
            send(&peer, owner.address(), &encoded, true).await;
            let response = receive(&peer, true).await;
            if !full {
                assert!(
                    matches!(response, Apdu::Abort(abort) if abort.abort_reason == bacnet_types::enums::AbortReason::SEGMENTATION_NOT_SUPPORTED)
                );
                break;
            }
            assert!(matches!(response, Apdu::SegmentAck(_)));
        }
        if full {
            assert_identity(receive(&peer, true).await);
        }
        owner.stop().await;
    }
}
