use bacnet_encoding::apdu::{decode_apdu, encode_apdu};
use bacnet_encoding::npdu::decode_npdu;
use bacnet_endpoint_core::endpoint_ingress::EndpointIngress;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_transport::loopback::LoopbackTransport;

use super::*;
use bacnet_transport::port::TransportProvenance;

fn read_property_request(invoke_id: u8) -> Bytes {
    let mut service_request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 7).unwrap(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    }
    .encode(&mut service_request);
    let request = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 480,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: service_request.freeze(),
    });
    let mut encoded = BytesMut::new();
    encode_apdu(&mut encoded, &request).unwrap();
    encoded.freeze()
}

#[tokio::test]
async fn responder_moves_reply_sender_once_and_preserves_routed_destination() {
    let (endpoint_transport, mut peer_transport) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut peer_rx = peer_transport.start().await.unwrap();
    let mut endpoint = EndpointIngress::new(endpoint_transport, 2);
    let ingress = endpoint.start().await.unwrap();
    let mut db = ObjectDatabase::new();
    let mut analog = AnalogInputObject::new(7, "shared-runtime-input", 0).unwrap();
    analog.set_present_value(42.0);
    db.add(Box::new(analog)).unwrap();
    let responder = EndpointResponder::new(Arc::new(RwLock::new(db)), ingress.egress.clone());
    let routed_source = NpduAddress {
        network: 77,
        mac_address: MacAddr::from_slice(&[0x44, 0x55]),
    };
    let (reply_tx, reply_rx) = oneshot::channel();
    let received = ReceivedApdu {
        direct_response: None,
        apdu: read_property_request(0x31),
        source_mac: MacAddr::from_slice(&[0x02]),
        ingress_network: None,
        source_network: Some(routed_source.clone()),
        link_layer_group: false,
        is_group: false,
        global_broadcast: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: Some(reply_tx),
    };

    assert!(responder.handle(received).await.unwrap());
    let npdu = decode_npdu(reply_rx.await.unwrap()).unwrap();
    assert_eq!(npdu.destination, Some(routed_source));
    let service_ack = match decode_apdu(npdu.payload).unwrap() {
        Apdu::ComplexAck(ack) => {
            assert_eq!(ack.invoke_id, 0x31);
            ack.service_ack
        }
        other => panic!("expected ComplexAck, got {other:?}"),
    };
    let ack = ReadPropertyACK::decode(&service_ack).unwrap();
    let mut expected = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut expected, &PropertyValue::Real(42.0))
        .unwrap();
    assert_eq!(ack.property_value, expected.to_vec());
    assert!(matches!(
        peer_rx.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));

    responder.close();
    assert!(matches!(
        responder
            .handle(ReceivedApdu {
                direct_response: None,
                apdu: read_property_request(0x32),
                source_mac: MacAddr::from_slice(&[0x02]),
                ingress_network: None,
                source_network: None,
                link_layer_group: false,
                is_group: false,
                global_broadcast: false,
                data_attributes: Vec::new(),
                provenance: TransportProvenance::unverified(),
                reply_tx: None,
            })
            .await,
        Err(Error::Encoding(message)) if message == "endpoint shutdown"
    ));
    endpoint.stop().await.unwrap();
    peer_transport.stop().await.unwrap();
}

#[tokio::test]
async fn responder_routes_reply_to_original_source_via_immediate_router() {
    let (endpoint_transport, mut router_transport) =
        LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut router_rx = router_transport.start().await.unwrap();
    let mut endpoint = EndpointIngress::new(endpoint_transport, 2);
    let ingress = endpoint.start().await.unwrap();
    let mut db = ObjectDatabase::new();
    let mut analog = AnalogInputObject::new(7, "routed-input", 0).unwrap();
    analog.set_present_value(42.0);
    db.add(Box::new(analog)).unwrap();
    let responder = EndpointResponder::new(Arc::new(RwLock::new(db)), ingress.egress);
    let routed_source = NpduAddress {
        network: 77,
        mac_address: MacAddr::from_slice(&[0x44, 0x55]),
    };

    assert!(responder
        .handle(ReceivedApdu {
            direct_response: None,
            apdu: read_property_request(0x41),
            source_mac: MacAddr::from_slice(&[0x02]),
            ingress_network: None,
            source_network: Some(routed_source.clone()),
            link_layer_group: false,
            is_group: false,
            global_broadcast: false,
            data_attributes: Vec::new(),
            provenance: TransportProvenance::unverified(),
            reply_tx: None,
        })
        .await
        .unwrap());

    let sent = tokio::time::timeout(std::time::Duration::from_secs(1), router_rx.recv())
        .await
        .expect("routed response timed out")
        .expect("router link closed");
    assert_eq!(sent.source_mac.as_slice(), &[0x01]);
    let npdu = decode_npdu(sent.npdu).unwrap();
    assert_eq!(npdu.destination, Some(routed_source));
    assert!(!npdu.expecting_reply);
    assert_eq!(npdu.priority, NetworkPriority::NORMAL);
    match decode_apdu(npdu.payload).unwrap() {
        Apdu::ComplexAck(ack) => assert_eq!(ack.invoke_id, 0x41),
        other => panic!("expected ComplexAck, got {other:?}"),
    }

    endpoint.stop().await.unwrap();
    router_transport.stop().await.unwrap();
}

#[tokio::test]
async fn responder_ignores_write_group_and_leaves_its_channels_alone() {
    use bacnet_objects::channel::ChannelObject;
    use bacnet_services::write_group::{GroupChannelValue, WriteGroupRequest};
    use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
    use bacnet_types::enums::WriteStatus;

    let (endpoint_transport, mut peer_transport) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut peer_rx = peer_transport.start().await.unwrap();
    let mut endpoint = EndpointIngress::new(endpoint_transport, 2);
    let ingress = endpoint.start().await.unwrap();
    let ch = ObjectIdentifier::new(ObjectType::CHANNEL, 1).unwrap();
    let mut channel = ChannelObject::new(1, "CH-1", 11).unwrap();
    channel
        .set_members(vec![BACnetDeviceObjectPropertyReference::new_local(
            ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 7).unwrap(),
            PropertyIdentifier::OUT_OF_SERVICE.to_raw(),
        )])
        .unwrap();
    channel.set_control_groups(vec![27]).unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(channel)).unwrap();
    let responder = EndpointResponder::new(Arc::new(RwLock::new(db)), ingress.egress);

    // The endpoint executes no unconfirmed service, WriteGroup included,
    // whether it arrives as a broadcast or directed to it.
    let mut service = BytesMut::new();
    WriteGroupRequest {
        group_number: std::num::NonZeroU32::new(27).unwrap(),
        write_priority: 8,
        change_list: vec![GroupChannelValue {
            channel: 11,
            override_priority: None,
            value: vec![0x11],
        }],
        inhibit_delay: None,
    }
    .encode(&mut service)
    .unwrap();
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
            service_choice: UnconfirmedServiceChoice::WRITE_GROUP,
            service_request: service.freeze(),
        }),
    )
    .unwrap();
    for is_group in [true, false] {
        let (reply_tx, reply_rx) = oneshot::channel();
        assert!(!responder
            .handle(ReceivedApdu {
                direct_response: None,
                apdu: apdu.clone().freeze(),
                source_mac: MacAddr::from_slice(&[0x02]),
                ingress_network: None,
                source_network: None,
                link_layer_group: is_group,
                is_group,
                global_broadcast: false,
                data_attributes: Vec::new(),
                provenance: TransportProvenance::unverified(),
                reply_tx: Some(reply_tx),
            })
            .await
            .unwrap());
        assert!(reply_rx.await.is_err(), "WriteGroup is answered");
    }
    let db = responder.db().read().await;
    let channel = db.get(&ch).unwrap();
    assert_eq!(
        channel
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Null
    );
    assert_eq!(
        channel
            .read_property(PropertyIdentifier::WRITE_STATUS, None)
            .unwrap(),
        PropertyValue::Enumerated(WriteStatus::IDLE.to_raw())
    );
    drop(db);
    assert!(matches!(
        peer_rx.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
    endpoint.stop().await.unwrap();
    peer_transport.stop().await.unwrap();
}
