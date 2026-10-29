use super::*;
use bacnet_types::error::RemoteError;

#[tokio::test]
async fn source_write_preflight_is_silent_with_and_without_reporter() {
    let (mut peer, mut requests) = network().await;
    let (mut sink, mut records) = network().await;
    for reported in [false, true] {
        let mut endpoint = if reported {
            session(write_database(false), SessionRole::ClientOnly, &sink)
        } else {
            BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
                .role(SessionRole::ClientOnly)
                .build_session()
                .unwrap()
        };
        endpoint.start().await.unwrap();
        let client = endpoint.cloned_client_handle().unwrap();
        for (value, priority) in [
            (vec![0x3e], None),
            ([vec![0; 33], vec![0x3e]].concat(), None),
            (vec![0; 2000], None),
            (vec![0], Some(0)),
            (vec![0], Some(17)),
        ] {
            assert!(
                client
                    .write_property(
                        peer.local_mac(),
                        WritePropertyRequest {
                            object_identifier: target(),
                            property_identifier: PropertyIdentifier::PRESENT_VALUE,
                            property_array_index: None,
                            property_value: value,
                            priority,
                        },
                        Commandability::Commandable,
                    )
                    .await
                    .is_err()
            );
            assert_eq!(endpoint.active_leases(), 0);
        }
        for mac in [
            vec![1],
            encode_bip_mac([255; 4], 47808).to_vec(),
            encode_bip_mac([127, 0, 0, 1], 0).to_vec(),
            encode_bip_mac([224, 0, 0, 1], 47808).to_vec(),
        ] {
            assert!(
                client
                    .write_property(
                        &mac,
                        WritePropertyRequest {
                            object_identifier: target(),
                            property_identifier: PropertyIdentifier::PRESENT_VALUE,
                            property_array_index: None,
                            property_value: vec![0],
                            priority: None,
                        },
                        Commandability::Noncommandable,
                    )
                    .await
                    .is_err()
            );
        }
        assert!(requests.try_recv().is_err());
        assert!(records.try_recv().is_err());
        // Preflight refusal has not consumed the source's sequence number.
        let task = write_value(
            &endpoint,
            peer.local_mac(),
            PropertyIdentifier::PRESENT_VALUE,
            None,
            vec![0],
            None,
            Commandability::Commandable,
        );
        let envelope = receive(&mut requests).await;
        let (invoke, wp) = write_request(&envelope);
        send(&peer, &envelope.source_mac, write_ack(invoke, &wp)).await;
        assert!(task.await.unwrap().is_ok());
        if reported {
            let (record, _) = notification(&receive(&mut records).await, false);
            assert_eq!(
                record.source_timestamp,
                Some(BACnetTimeStamp::SequenceNumber(0))
            );
        } else {
            assert!(records.try_recv().is_err());
        }
        endpoint.stop().await.unwrap();
    }
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

#[tokio::test]
async fn source_write_before_admission_cancel_and_after_admission_retry_are_bounded() {
    let (mut peer, mut requests) = network().await;
    let (mut sink, mut records) = network().await;
    let mut session = session(write_database(false), SessionRole::ClientOnly, &sink);
    session.client_config.apdu_retries = 1;
    session.start().await.unwrap();
    let db = Arc::clone(session.database.as_ref().unwrap());
    let guard = db.write().await;
    let pending = start_write(
        &session,
        peer.local_mac(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    timeout(WAIT, async {
        while session
            .source_audit
            .as_ref()
            .unwrap()
            .available_operations()
            == 64
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    pending.abort();
    let _ = pending.await;
    drop(guard);
    assert_eq!(
        session
            .source_audit
            .as_ref()
            .unwrap()
            .available_operations(),
        64
    );
    assert_eq!(session.active_leases(), 0);
    assert!(requests.try_recv().is_err());
    assert!(records.try_recv().is_err());
    let pending = start_write(
        &session,
        peer.local_mac(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    let first = receive(&mut requests).await;
    let (invoke, wp) = write_request(&first);
    pending.abort();
    let _ = pending.await;
    let retry = receive(&mut requests).await;
    assert_eq!(retry.apdu, first.apdu);
    send(&peer, &retry.source_mac, write_ack(invoke, &wp)).await;
    let (record, _) = notification(&receive(&mut records).await, false);
    assert_eq!(record.invoke_id, Some(invoke));
    assert_eq!(record.result, None);
    assert_eq!(
        record.source_timestamp,
        Some(BACnetTimeStamp::SequenceNumber(0))
    );
    assert!(
        timeout(Duration::from_millis(150), records.recv())
            .await
            .is_err()
    );
    assert_eq!(session.active_leases(), 0);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

#[tokio::test]
async fn endpoint_write_non_bip_without_reporter_is_refused_and_reads_remain_available() {
    use bacnet_transport::loopback::LoopbackTransport;
    let (transport, peer) = LoopbackTransport::pair(vec![1], vec![2]);
    let mut session =
        EndpointSession::new(transport, SessionRole::ClientOnly, SessionConfig::default()).unwrap();
    let mut peer = NetworkLayer::new(peer);
    let mut requests = peer.start().await.unwrap();
    session.start().await.unwrap();
    let client = session.cloned_client_handle().unwrap();
    assert!(
        client
            .write_property(
                &encode_bip_mac([127, 0, 0, 1], 47808),
                WritePropertyRequest {
                    object_identifier: target(),
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    property_value: vec![0],
                    priority: None,
                },
                Commandability::Noncommandable,
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("IPv4 B/IP")
    );
    assert_eq!(session.active_leases(), 0);
    assert!(requests.try_recv().is_err());
    let read = tokio::spawn(async move {
        client
            .read_property(&[2], target(), PropertyIdentifier::PRESENT_VALUE, None)
            .await
    });
    let envelope = receive(&mut requests).await;
    let (invoke, rp) = read_request(&envelope);
    let mut encoded = BytesMut::new();
    encode_apdu(&mut encoded, &ack(invoke, &rp)).unwrap();
    peer.send_apdu(&encoded, &[1], false, NetworkPriority::Normal)
        .await
        .unwrap();
    assert!(read.await.unwrap().is_ok());
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
}

#[tokio::test]
async fn source_write_empty_recipient_list_success_and_empty_scalar_remote_error() {
    use bacnet_objects::{analog::AnalogValueObject, notification_class::NotificationClass};
    use bacnet_server::server::{BACnetServer, ServerConfig};
    use bacnet_types::enums::EngineeringUnits;
    let (mut sink, mut records) = network().await;
    let mut db = crate::DeviceIdentity::new(456, 42)
        .unwrap()
        .build_database()
        .unwrap();
    db.add(Box::new(NotificationClass::new(1, "empty-list").unwrap()))
        .unwrap();
    db.add(Box::new(
        AnalogValueObject::new(7, "scalar", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    let mut remote = BACnetServer::start_clockless(
        ServerConfig {
            device: bacnet_objects::DeviceIdentity::new(456, 42).unwrap(),
            ..Default::default()
        },
        db,
        BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST),
    )
    .await
    .unwrap();
    let mut session = session(write_database(false), SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    for (object, property, success) in [
        (
            oid(ObjectType::NOTIFICATION_CLASS, 1),
            PropertyIdentifier::RECIPIENT_LIST,
            true,
        ),
        (target(), PropertyIdentifier::PRESENT_VALUE, false),
    ] {
        let result = session
            .client()
            .unwrap()
            .write_property(
                remote.local_mac(),
                WritePropertyRequest {
                    object_identifier: object,
                    property_identifier: property,
                    property_array_index: None,
                    property_value: vec![],
                    priority: None,
                },
                Commandability::Noncommandable,
            )
            .await;
        assert_eq!(result.is_ok(), success);
        if !success {
            assert!(matches!(
                result,
                Err(Error::Remote(RemoteError::Error { .. }))
            ));
        }
        let (record, _) = notification(&receive(&mut records).await, false);
        assert_eq!(record.target_value, Some(vec![]));
        assert_eq!(record.target_object, Some(object));
        assert!(record.current_value.is_none());
        assert_eq!(record.result.is_none(), success);
    }
    session.stop().await.unwrap();
    remote.stop().await.unwrap();
    sink.stop().await.unwrap();
}

#[tokio::test]
async fn source_write_wrong_service_error_cannot_finish_or_steal_retry_identity() {
    let (mut peer, mut requests) = network().await;
    let (mut sink, mut records) = network().await;
    let mut session = session(write_database(false), SessionRole::ClientOnly, &sink);
    session.client_config.apdu_retries = 1;
    session.start().await.unwrap();
    let pending = start_write(
        &session,
        peer.local_mac(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    let first = receive(&mut requests).await;
    let (invoke, wp) = write_request(&first);
    send(
        &peer,
        &first.source_mac,
        Apdu::Error(ErrorPdu {
            invoke_id: invoke,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            error_class: ErrorClass::PROPERTY,
            error_code: ErrorCode::WRITE_ACCESS_DENIED,
            error_data: Bytes::new(),
        }),
    )
    .await;
    let retry = receive(&mut requests).await;
    assert_eq!(retry.apdu, first.apdu);
    assert!(!pending.is_finished());
    assert!(records.try_recv().is_err());
    send(&peer, &retry.source_mac, write_ack(invoke, &wp)).await;
    pending.await.unwrap().unwrap();
    let (record, _) = notification(&receive(&mut records).await, false);
    assert_eq!(record.invoke_id, Some(invoke));
    assert_eq!(record.result, None);
    assert!(records.try_recv().is_err());
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}
