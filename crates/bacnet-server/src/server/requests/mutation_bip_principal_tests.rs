//! B/IP requests carry no principal (#524), over a real UDP socket.
use super::*;
use crate::mutation::MutationAuthorizationContext;
use bacnet_objects::value_types::CharacterStringValueObject;
use bacnet_transport::port::{ScPrincipal, TransportProvenance};
use bacnet_types::enums::ObjectType;
use std::sync::Mutex as StdMutex;

#[tokio::test]
async fn bip_mutation_has_no_principal_and_a_denial_changes_nothing() {
    let target = ObjectIdentifier::new(ObjectType::CHARACTERSTRING_VALUE, 1).unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        CharacterStringValueObject::new(1, "value").unwrap(),
    ))
    .unwrap();
    let seen = Arc::new(StdMutex::new(Vec::<(
        Option<ScPrincipal>,
        TransportProvenance,
    )>::new()));
    let record = seen.clone();
    let mut server = BACnetServer::<BipTransport>::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .database(db)
        .enable_event_enrollment(false)
        .policy(
            crate::FnPolicy::new().mutation(move |context: &MutationAuthorizationContext| {
                record
                    .lock()
                    .unwrap()
                    .push((context.principal(), context.provenance));
                context.principal().is_some()
            }),
        )
        .build()
        .await
        .unwrap();
    let address = server
        .test_network()
        .transport()
        .bip_port()
        .unwrap()
        .endpoint;
    let before = server
        .database()
        .read()
        .await
        .get(&target)
        .unwrap()
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();

    let mut value = BytesMut::new();
    encode_property_value(&mut value, &PropertyValue::CharacterString("B/IP".into())).unwrap();
    let mut request = BytesMut::new();
    bacnet_services::write_property::WritePropertyRequest {
        object_identifier: target,
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        property_value: value.to_vec(),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: 9,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
            service_request: request.freeze(),
        }),
    )
    .unwrap();
    // Original-Unicast-NPDU (Annex J.2.11) around an NPDU expecting a reply.
    let mut frame = vec![0x81, 0x0a, 0, 0, 0x01, 0x04];
    frame.extend_from_slice(&apdu);
    let length = u16::try_from(frame.len()).unwrap().to_be_bytes();
    frame[2..4].copy_from_slice(&length);
    let peer = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    peer.send_to(&frame, address).await.unwrap();
    let mut reply = [0u8; 1500];
    let (len, _) = tokio::time::timeout(Duration::from_secs(3), peer.recv_from(&mut reply))
        .await
        .unwrap()
        .unwrap();
    let npdu = bacnet_encoding::npdu::decode_npdu(Bytes::copy_from_slice(&reply[4..len])).unwrap();
    let Apdu::Error(error) = bacnet_encoding::apdu::decode_apdu(npdu.payload).unwrap() else {
        panic!("expected a denial");
    };
    assert_eq!(
        (error.invoke_id, error.error_class, error.error_code),
        (9, ErrorClass::SERVICES, ErrorCode::SERVICE_REQUEST_DENIED)
    );

    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, None);
    assert!(seen[0].1.is_unverified());
    let after = server
        .database()
        .read()
        .await
        .get(&target)
        .unwrap()
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(after, before);
    assert_eq!(
        server
            .mutation_decision_counters()
            .write_property
            .deny_total,
        1
    );
    server.stop().await.unwrap();
}
