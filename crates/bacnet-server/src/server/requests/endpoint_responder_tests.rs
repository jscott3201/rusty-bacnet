use bacnet_encoding::apdu::{decode_apdu, encode_apdu};
use bacnet_encoding::npdu::decode_npdu;
use bacnet_endpoint_core::endpoint_ingress::EndpointIngress;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_transport::loopback::LoopbackTransport;

use super::*;

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
    let (endpoint_transport, _peer_transport) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
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
        apdu: read_property_request(0x31),
        source_mac: MacAddr::from_slice(&[0x02]),
        source_network: Some(routed_source.clone()),
        is_group: false,
        data_attributes: Vec::new(),
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

    responder.close();
    assert!(matches!(
        responder
            .handle(ReceivedApdu {
                apdu: read_property_request(0x32),
                source_mac: MacAddr::from_slice(&[0x02]),
                source_network: None,
                is_group: false,
                data_attributes: Vec::new(),
                reply_tx: None,
            })
            .await,
        Err(Error::Encoding(message)) if message == "endpoint shutdown"
    ));
    endpoint.stop().await.unwrap();
}
