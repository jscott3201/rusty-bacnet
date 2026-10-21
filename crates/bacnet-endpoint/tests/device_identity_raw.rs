//! Raw Device declarations remain distinct from Confirmed-Request header codes.
use bacnet_encoding::{
    apdu::{decode_apdu, encode_apdu, Apdu, ComplexAck},
    npdu::{decode_npdu, encode_npdu, Npdu},
};
use bacnet_endpoint::{DeviceIdentity, EndpointSession, SessionConfig, SessionRole};
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_transport::{loopback::LoopbackTransport, port::TransportPort};
use bacnet_types::{
    enums::{ConfirmedServiceChoice, ObjectType, PropertyIdentifier as P},
    primitives::{ObjectIdentifier, PropertyValue},
};
use bytes::BytesMut;
use std::time::Duration;

#[tokio::test]
async fn raw_identity_1474_floors_only_the_endpoint_request_header() {
    let identity = DeviceIdentity::new(1720, 42)
        .unwrap()
        .with_max_apdu(1474)
        .unwrap();
    assert_eq!(
        bacnet_endpoint::identity::client_config(&identity).max_apdu_length,
        1024
    );
    let (transport, mut peer) = LoopbackTransport::pair(vec![1], vec![2]);
    let mut incoming = peer.start().await.unwrap();
    let mut session = EndpointSession::new(transport, SessionRole::Both, SessionConfig::default())
        .unwrap()
        .device(identity);
    session.start().await.unwrap();
    assert_eq!(session.identity().unwrap().max_apdu_length(), 1474);
    session.broadcast_i_am().await.unwrap();
    let iam = incoming.recv().await.unwrap();
    let Apdu::UnconfirmedRequest(iam) =
        decode_apdu(decode_npdu(iam.npdu).unwrap().payload).unwrap()
    else {
        panic!("I-Am expected")
    };
    assert_eq!(
        bacnet_services::who_is::IAmRequest::decode(&iam.service_request)
            .unwrap()
            .max_apdu_length,
        1474
    );
    let oid = ObjectIdentifier::new(ObjectType::DEVICE, 22).unwrap();
    let client = session.client().unwrap();
    let response = async {
        let incoming = incoming.recv().await.unwrap();
        let Apdu::ConfirmedRequest(request) =
            decode_apdu(decode_npdu(incoming.npdu).unwrap().payload).unwrap()
        else {
            panic!("request")
        };
        assert_eq!(request.max_apdu_length, 1024);
        let request_data = ReadPropertyRequest::decode(&request.service_request).unwrap();
        assert_eq!(request_data.object_identifier, oid);
        let mut value = BytesMut::new();
        bacnet_encoding::primitives::encode_property_value(
            &mut value,
            &PropertyValue::Unsigned(17),
        )
        .unwrap();
        let mut service = BytesMut::new();
        ReadPropertyACK {
            object_identifier: oid,
            property_identifier: P::VENDOR_IDENTIFIER,
            property_array_index: None,
            property_value: value.to_vec(),
        }
        .encode(&mut service);
        let mut apdu = BytesMut::new();
        encode_apdu(
            &mut apdu,
            &Apdu::ComplexAck(ComplexAck {
                segmented: false,
                more_follows: false,
                invoke_id: request.invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: ConfirmedServiceChoice::READ_PROPERTY,
                service_ack: service.freeze(),
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
    let (read, ()) = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(
            client.read_property(&[2], oid, P::VENDOR_IDENTIFIER, None),
            response
        )
    })
    .await
    .unwrap();
    assert_eq!(read.unwrap().property_value, [0x21, 17]);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
}
