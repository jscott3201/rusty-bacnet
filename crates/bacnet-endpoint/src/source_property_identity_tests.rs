//! A source records only the concrete identity established by a valid ACK.
use super::*;

fn start_object(
    session: &EndpointSession<BipTransport>,
    mac: &[u8],
    object: ObjectIdentifier,
) -> tokio::task::JoinHandle<Result<ReadPropertyACK, Error>> {
    let client = session.cloned_client_handle().unwrap();
    let mac = mac.to_vec();
    tokio::spawn(async move {
        client
            .read_property(&mac, object, PropertyIdentifier::OBJECT_NAME, None)
            .await
    })
}
fn start_alias(
    session: &EndpointSession<BipTransport>,
    mac: &[u8],
    kind: ObjectType,
) -> tokio::task::JoinHandle<Result<ReadPropertyACK, Error>> {
    start_object(session, mac, oid(kind, ObjectIdentifier::WILDCARD_INSTANCE))
}
async fn released(session: &EndpointSession<BipTransport>) {
    timeout(WAIT, async {
        while session.active_leases() != 0
            || session.source_read.as_ref().unwrap().available_operations() != 64
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
fn value_free(record: &BACnetAuditNotification) {
    assert!(record.target_value.is_none() && record.current_value.is_none());
    assert_eq!(record.operation, AuditOperation::READ);
    assert_eq!(
        record.source_device,
        BACnetRecipient::Device(oid(ObjectType::DEVICE, 123))
    );
}

#[tokio::test]
async fn source_read_alias_success_records_peer_concrete_object_both_roles_modes() {
    let (mut peer, mut requests) = network().await;
    let (mut sink, mut records) = network().await;
    for role in [SessionRole::ClientOnly, SessionRole::Both] {
        for confirmed in [false, true] {
            let mut session = session(database(confirmed), role, &sink);
            session.start().await.unwrap();
            for (sequence, (kind, instance, concrete)) in [
                (ObjectType::DEVICE, ObjectIdentifier::WILDCARD_INSTANCE, 10),
                (ObjectType::DEVICE, 22, 22),
                (
                    ObjectType::NETWORK_PORT,
                    ObjectIdentifier::WILDCARD_INSTANCE,
                    10,
                ),
            ]
            .into_iter()
            .enumerate()
            {
                let read = start_object(&session, peer.local_mac(), oid(kind, instance));
                let envelope = receive(&mut requests).await;
                let (invoke, mut rp) = read_request(&envelope);
                assert_eq!(rp.object_identifier, oid(kind, instance));
                rp.object_identifier = oid(kind, concrete);
                send(&peer, &envelope.source_mac, ack(invoke, &rp)).await;
                let actual = read.await.unwrap().unwrap();
                assert_eq!(actual.object_identifier, rp.object_identifier);
                assert_eq!(actual.property_value, [0x21, 42]);
                let envelope = receive(&mut records).await;
                let (record, delivery) = notification(&envelope, confirmed);
                value_free(&record);
                assert_eq!(record.target_object, Some(oid(kind, concrete)));
                if kind == ObjectType::DEVICE {
                    assert_eq!(
                        record.target_device,
                        BACnetRecipient::Device(oid(kind, concrete))
                    );
                } else {
                    assert!(matches!(record.target_device, BACnetRecipient::Address(_)));
                }
                assert_eq!(record.result, None);
                assert_eq!(record.invoke_id, Some(invoke));
                assert_eq!(
                    record.source_timestamp,
                    Some(BACnetTimeStamp::SequenceNumber(sequence as u16))
                );
                if let Some(invoke_id) = delivery {
                    send(
                        &sink,
                        &envelope.source_mac,
                        Apdu::SimpleAck(SimpleAck {
                            invoke_id,
                            service_choice: ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
                        }),
                    )
                    .await;
                }
                released(&session).await;
            }
            assert!(records.try_recv().is_err());
            session.stop().await.unwrap();
        }
    }
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

#[tokio::test]
async fn source_read_alias_failure_keeps_requested_identity_without_trusting_bad_ack() {
    let (mut peer, mut requests) = network().await;
    let (mut sink, mut records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    let mut sequence = 0;
    for kind in [ObjectType::DEVICE, ObjectType::NETWORK_PORT] {
        for failure in 0..7 {
            let read = start_alias(&session, peer.local_mac(), kind);
            let envelope = receive(&mut requests).await;
            let (invoke, mut rp) = read_request(&envelope);
            let requested = rp.object_identifier;
            rp.object_identifier = oid(kind, 10);
            match failure {
                0 => rp.object_identifier = requested,
                1 => rp.object_identifier = oid(ObjectType::ANALOG_INPUT, 10),
                2 => rp.property_identifier = PropertyIdentifier::DESCRIPTION,
                3 => rp.property_array_index = Some(0),
                _ => {}
            }
            let mut response = ack(invoke, &rp);
            if failure == 4 {
                let Apdu::ComplexAck(ref mut body) = response else {
                    unreachable!()
                };
                body.service_ack = Bytes::from_static(&[0]);
            }
            if failure == 5 {
                response = Apdu::Error(ErrorPdu {
                    invoke_id: invoke,
                    service_choice: ConfirmedServiceChoice::READ_PROPERTY,
                    error_class: ErrorClass::OBJECT,
                    error_code: ErrorCode::UNKNOWN_OBJECT,
                    error_data: Bytes::new(),
                });
            }
            if failure != 6 {
                send(&peer, &envelope.source_mac, response).await;
            }
            let result = read.await.unwrap();
            match failure {
                5 => assert!(matches!(result, Err(Error::Protocol { .. }))),
                6 => assert!(matches!(result, Err(Error::Timeout(_)))),
                _ => assert!(matches!(result, Err(Error::Decoding { .. }))),
            }
            let (record, _) = notification(&receive(&mut records).await, false);
            value_free(&record);
            assert_eq!(record.target_object, Some(requested));
            assert!(matches!(record.target_device, BACnetRecipient::Address(_)));
            assert_eq!(record.invoke_id, Some(invoke));
            assert_eq!(
                record.source_timestamp,
                Some(BACnetTimeStamp::SequenceNumber(sequence))
            );
            sequence += 1;
            assert_eq!(
                record.result,
                Some(match failure {
                    5 => (ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT),
                    6 => (ErrorClass::COMMUNICATION, ErrorCode::TIMEOUT),
                    _ => (ErrorClass::COMMUNICATION, ErrorCode::OTHER),
                })
            );
            released(&session).await;
        }
    }
    assert!(records.try_recv().is_err());
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

#[tokio::test]
async fn source_read_alias_cancelled_retry_resolves_identity_on_original_recipient_snapshot() {
    let (mut peer, mut requests) = network().await;
    let (mut old, mut old_records) = network().await;
    let (mut new, mut new_records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &old);
    session.client_config.apdu_retries = 1;
    session.start().await.unwrap();
    let read = start_alias(&session, peer.local_mac(), ObjectType::DEVICE);
    let first = receive(&mut requests).await;
    let (invoke, mut rp) = read_request(&first);
    read.abort();
    assert!(read.await.unwrap_err().is_cancelled());
    session
        .write_audit_recipient(Some(BACnetRecipient::Address(
            bacnet_types::constructed::BACnetAddress {
                network_number: 0,
                mac_address: MacAddr::from_slice(new.local_mac()),
            },
        )))
        .await
        .unwrap();
    for envelope in [
        receive(&mut old_records).await,
        receive(&mut new_records).await,
    ] {
        assert_eq!(
            notification(&envelope, false).0.operation,
            AuditOperation::WRITE
        );
    }
    let retry = receive(&mut requests).await;
    assert_eq!(read_request(&retry), (invoke, rp.clone()));
    rp.object_identifier = oid(ObjectType::DEVICE, 10);
    send(&peer, &retry.source_mac, ack(invoke, &rp)).await;
    let (record, _) = notification(&receive(&mut old_records).await, false);
    value_free(&record);
    assert_eq!(record.target_object, Some(rp.object_identifier));
    assert_eq!(
        record.target_device,
        BACnetRecipient::Device(rp.object_identifier)
    );
    assert_eq!(record.result, None);
    assert_eq!(record.invoke_id, Some(invoke));
    assert_eq!(
        record.source_timestamp,
        Some(BACnetTimeStamp::SequenceNumber(0))
    );
    released(&session).await;
    assert!(timeout(Duration::from_millis(30), old_records.recv())
        .await
        .is_err());
    // Subsequent admitted work uses the new route and concrete peer identity.
    let read = start_alias(&session, peer.local_mac(), ObjectType::NETWORK_PORT);
    let envelope = receive(&mut requests).await;
    let (invoke, mut rp) = read_request(&envelope);
    rp.object_identifier = oid(ObjectType::NETWORK_PORT, 17);
    send(&peer, &envelope.source_mac, ack(invoke, &rp)).await;
    read.await.unwrap().unwrap();
    let (record, _) = notification(&receive(&mut new_records).await, false);
    assert_eq!(record.target_object, Some(rp.object_identifier));
    value_free(&record);
    released(&session).await;
    assert!(old_records.try_recv().is_err());
    assert!(new_records.try_recv().is_err());
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    old.stop().await.unwrap();
    new.stop().await.unwrap();
}

#[tokio::test]
async fn source_read_range_concrete_device_ack_establishes_target_device_for_this_record() {
    use bacnet_services::read_range::{ReadRangeAck, ReadRangeRequest};
    let (mut peer, mut requests) = network().await;
    let (mut sink, mut records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    let client = session.cloned_client_handle().unwrap();
    let mac = peer.local_mac().to_vec();
    let object = oid(ObjectType::DEVICE, 10);
    let read = tokio::spawn(async move {
        client
            .read_range(&mac, object, PropertyIdentifier::OBJECT_LIST, None, None)
            .await
    });
    let envelope = receive(&mut requests).await;
    let Apdu::ConfirmedRequest(request) = decode_apdu(envelope.apdu.clone()).unwrap() else {
        panic!()
    };
    assert_eq!(request.service_choice, ConfirmedServiceChoice::READ_RANGE);
    assert_eq!(
        ReadRangeRequest::decode(&request.service_request)
            .unwrap()
            .object_identifier,
        object
    );
    let mut body = BytesMut::new();
    ReadRangeAck {
        object_identifier: object,
        property_identifier: PropertyIdentifier::OBJECT_LIST,
        property_array_index: None,
        result_flags: (true, true, false),
        item_count: 1,
        item_data: vec![0xc4, 2, 0, 0, 10],
        first_sequence_number: None,
    }
    .encode(&mut body);
    send(
        &peer,
        &envelope.source_mac,
        Apdu::ComplexAck(ComplexAck {
            segmented: false,
            more_follows: false,
            invoke_id: request.invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::READ_RANGE,
            service_ack: body.freeze(),
        }),
    )
    .await;
    assert_eq!(read.await.unwrap().unwrap().item_data, [0xc4, 2, 0, 0, 10]);
    let (record, _) = notification(&receive(&mut records).await, false);
    assert_eq!(record.target_object, Some(object));
    assert_eq!(record.target_device, BACnetRecipient::Device(object));
    assert_eq!(record.result, None);
    value_free(&record);
    released(&session).await;
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}
