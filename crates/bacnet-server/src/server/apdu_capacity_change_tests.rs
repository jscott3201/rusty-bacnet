//! #2093: when the transport's receive capacity falls after start, the
//! server's Max_APDU_Length_Accepted property and I-Am stay as resolved at
//! start, and each reply honours its requester's declared limit, not the
//! receive capacity, which only bounds what comes in.
use super::*;
use bacnet_encoding::apdu::ComplexAck;

/// A confirmed ReadProperty of `object`'s Object_Name from a peer that
/// accepts APDUs of `max_apdu_length` octets, in segments.
fn read_name(object: ObjectIdentifier, max_apdu_length: u16, invoke_id: u8) -> Bytes {
    let mut service_request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: object,
        property_identifier: PropertyIdentifier::OBJECT_NAME,
        property_array_index: None,
    }
    .encode(&mut service_request);
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: true,
            max_segments: Some(16),
            max_apdu_length,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_request: service_request.freeze(),
        }),
    )
    .unwrap();
    apdu.freeze()
}

#[tokio::test(start_paused = true)]
async fn max_apdu_stays_fixed_when_the_receive_capacity_falls() {
    let (transport, sent, tx) = discovery_transport();
    let handle = transport.handle();
    let mut server = BACnetServer::generic_builder()
        .device(
            bacnet_objects::DeviceIdentity::default()
                .with_segmentation(bacnet_types::enums::Segmentation::BOTH),
        )
        .transport(transport)
        .build()
        .await
        .unwrap();
    let device = server.config.device.device_oid();
    // An object whose name alone is over 480 octets.
    let long = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    server
        .db
        .write()
        .await
        .add(Box::new(
            AnalogInputObject::new(1, "x".repeat(1000), EngineeringUnits::NO_UNITS).unwrap(),
        ))
        .unwrap();
    let max_apdu = || async {
        server
            .read_local(&device, PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED, None)
            .await
            .unwrap()
    };
    assert_eq!(max_apdu().await, PropertyValue::Unsigned(1476));

    // The link's capacity falls after start, as on an SC hub failover.
    handle.set_receive_capacity(480);
    assert_eq!(
        server
            .test_network()
            .transport()
            .local_receive_apdu_capacity(),
        480
    );
    assert_eq!(max_apdu().await, PropertyValue::Unsigned(1476));
    assert_eq!(server.config.device.max_apdu_length(), 1476);
    let before = sent.len();
    server.broadcast_i_am().await.unwrap();
    sent.wait_for_len(before + 1).await;
    let Apdu::UnconfirmedRequest(i_am) = sent.frame(before).apdu() else {
        panic!("I-Am expected");
    };
    assert_eq!(
        IAmRequest::decode(&i_am.service_request)
            .unwrap()
            .max_apdu_length,
        1476
    );

    // A requester declaring 480: the name is over 480 octets, so the reply
    // goes out in segments of at most 480.
    let before = sent.len();
    tx.send(wrap_apdu(read_name(long, 480, 9), &[0x0A, 0, 0, 2], None))
        .await
        .unwrap();
    sent.wait_for_len(before + 1).await;
    let frame = sent.frame(before);
    let apdu_len = frame.decode_npdu().payload.len();
    assert!(apdu_len <= 480, "{apdu_len} octets");
    assert!(
        matches!(
            frame.apdu(),
            Apdu::ComplexAck(ComplexAck {
                segmented: true,
                more_follows: true,
                sequence_number: Some(0),
                ..
            })
        ),
        "{:?}",
        frame.apdu()
    );

    // A requester declaring 1476 gets the whole name in one APDU: the
    // fallen receive capacity doesn't shrink replies.
    let before = sent.len();
    tx.send(wrap_apdu(read_name(long, 1476, 10), &[0x0A, 0, 0, 3], None))
        .await
        .unwrap();
    sent.wait_for_len(before + 1).await;
    let frame = sent.frame(before);
    let apdu_len = frame.decode_npdu().payload.len();
    assert!(apdu_len > 480 && apdu_len <= 1476, "{apdu_len} octets");
    assert!(
        matches!(
            frame.apdu(),
            Apdu::ComplexAck(ComplexAck {
                segmented: false,
                invoke_id: 10,
                ..
            })
        ),
        "{:?}",
        frame.apdu()
    );
    server.stop().await.unwrap();
}
