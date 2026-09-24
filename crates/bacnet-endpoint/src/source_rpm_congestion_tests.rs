use super::super::queued::Gated;
use super::*;
use bacnet_transport::loopback::LoopbackTransport;
use tokio::sync::{Notify, Semaphore};

#[tokio::test]
async fn source_rpm_default_queue_accounts_local_losses_and_recovers_summary() {
    congestion(0).await;
}
#[tokio::test]
async fn source_rpm_full_queue_summary_wait_is_owned_and_stop_cancels() {
    congestion(1).await;
}
#[tokio::test]
async fn source_rpm_full_queue_summary_generation_replacement_discards_old_count() {
    congestion(2).await;
}
async fn congestion(mode: u8) {
    let source_mac = encode_bip_mac([127, 0, 0, 1], 30001);
    let peer_mac = encode_bip_mac([127, 0, 0, 1], 30002);
    let (transport, peer) = LoopbackTransport::pair(source_mac.to_vec(), peer_mac.to_vec());
    let entered = Arc::new(Notify::new());
    let gate = Arc::new(Semaphore::new(1));
    let mut db = database(false);
    let mut flags = AuditOperationFlags::empty();
    flags.insert(AuditOperation::READ);
    flags.insert(AuditOperation::AUDITING_FAILURE);
    db.get_mut(&selected())
        .unwrap()
        .configure_audit_reporter_internal(
            AuditLevel::AUDIT_ALL,
            flags,
            false,
            None,
            BACnetPriorityFilter::empty(),
        )
        .unwrap();
    db.get_mut(&oid(ObjectType::DEVICE, 123))
        .unwrap()
        .device_authority_internal()
        .unwrap()
        .provision_audit_recipient(BACnetRecipient::Device(oid(ObjectType::DEVICE, 999)))
        .unwrap();
    let mut session = EndpointSession::new(
        Gated {
            inner: transport,
            entered: entered.clone(),
            gate: gate.clone(),
        },
        SessionRole::ClientOnly,
        SessionConfig::default(),
    )
    .unwrap()
    .with_database(db)
    .with_source_audit_reporter(selected());
    session.source_audit_bindings.push((
        oid(ObjectType::DEVICE, 999),
        "127.0.0.1:30002".parse().unwrap(),
    ));
    let mut peer = NetworkLayer::new(peer);
    let mut received = peer.start().await.unwrap();
    session.start().await.unwrap();
    let specs = vec![ReadAccessSpecification {
        object_identifier: target(),
        list_of_property_references: vec![
            PropertyReference {
                property_identifier: PropertyIdentifier::OBJECT_NAME,
                property_array_index: None
            };
            64
        ],
    }];
    let read = start(&session, &peer_mac, specs);
    let env = receive(&mut received).await;
    let (invoke, rpm) = request(&env);
    entered.notified().await; // consume the request's transport-entry marker
                              // Fill the default16-slot queue behind one held unrelated transport send.
                              // This deterministically forces the summary itself to encounter QueueFull.
    let egress = session.egress.as_ref().unwrap();
    let enqueue = || {
        egress
            .admit_apdu(
                vec![0x10, 8],
                bacnet_endpoint_core::endpoint_ingress::EndpointApduDestination::Direct {
                    destination_mac: MacAddr::from_slice(&peer_mac),
                },
                false,
                NetworkPriority::NORMAL,
                Vec::new(),
                None,
            )
            .unwrap()
    };
    let mut unrelated = vec![enqueue()];
    timeout(WAIT, entered.notified()).await.unwrap();
    unrelated.extend((0..16).map(|_| enqueue()));
    let mut encoded = BytesMut::new();
    encode_apdu(&mut encoded, &response(invoke, &success(&rpm))).unwrap();
    peer.send_apdu(&encoded, &source_mac, false, NetworkPriority::NORMAL)
        .await
        .unwrap();
    assert!(read.await.unwrap().is_ok());
    // All notification admissions and the first full-queue summary attempt settle
    // while transport remains held. No deadline or speculative retry is needed.
    timeout(WAIT, async {
        loop {
            let db = session.database.as_ref().unwrap().read().await;
            let state = db
                .get(&selected())
                .unwrap()
                .read_property(PropertyIdentifier::RELIABILITY, None)
                .unwrap();
            if state == PropertyValue::Enumerated(Reliability::COMMUNICATION_FAILURE.to_raw()) {
                break;
            }
            drop(db);
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    timeout(
        WAIT,
        session
            .source_read
            .as_ref()
            .unwrap()
            .summary_queue_full
            .notified(),
    )
    .await
    .unwrap();
    if mode == 1 {
        let db = Arc::clone(session.database.as_ref().unwrap());
        let guard = db.write().await;
        assert!(timeout(Duration::from_millis(20), session.stop())
            .await
            .is_err());
        drop(guard);
        timeout(WAIT, session.stop()).await.unwrap().unwrap();
        assert_eq!(session.active_leases(), 0);
        peer.stop().await.unwrap();
        return;
    }
    if mode == 2 {
        session
            .write_audit_recipient(Some(BACnetRecipient::Address(
                bacnet_types::constructed::BACnetAddress {
                    network_number: 0,
                    mac_address: MacAddr::from_slice(&peer_mac),
                },
            )))
            .await
            .unwrap();
        gate.add_permits(128);
        for send in unrelated {
            assert!(send.complete().await.result.is_ok());
        }
        // The only queued packets are the17 unrelated commands. Superseded
        // summary count must not reappear once egress capacity returns.
        for _ in 0..17 {
            assert!(
                matches!(decode_apdu(receive(&mut received).await.apdu).unwrap(),Apdu::UnconfirmedRequest(pdu) if pdu.service_choice==UnconfirmedServiceChoice::WHO_IS)
            );
        }
        let read = start(
            &session,
            &peer_mac,
            vec![ReadAccessSpecification {
                object_identifier: target(),
                list_of_property_references: vec![PropertyReference {
                    property_identifier: PropertyIdentifier::OBJECT_NAME,
                    property_array_index: None,
                }],
            }],
        );
        let env = receive(&mut received).await;
        let (invoke, rpm) = request(&env);
        let mut bytes = BytesMut::new();
        encode_apdu(&mut bytes, &response(invoke, &success(&rpm))).unwrap();
        peer.send_apdu(&bytes, &source_mac, false, NetworkPriority::NORMAL)
            .await
            .unwrap();
        read.await.unwrap().unwrap();
        let record = notification(&receive(&mut received).await, false).0;
        assert_eq!(record.operation, AuditOperation::READ);
        assert_ne!(
            record.source_timestamp,
            Some(BACnetTimeStamp::SequenceNumber(0))
        );
        assert!(received.try_recv().is_err());
        session.stop().await.unwrap();
        peer.stop().await.unwrap();
        return;
    }
    gate.add_permits(128);
    let mut delivered = 0u64;
    let mut lost = 0u64;
    let mut summaries = 0;
    while delivered + lost < 64 {
        let envelope = receive(&mut received).await;
        if matches!(decode_apdu(envelope.apdu.clone()).unwrap(), Apdu::UnconfirmedRequest(ref pdu) if pdu.service_choice == UnconfirmedServiceChoice::WHO_IS)
        {
            continue;
        }
        let record = notification(&envelope, false).0;
        if record.operation == AuditOperation::READ {
            delivered += 1;
            assert_eq!(
                record.source_timestamp,
                Some(BACnetTimeStamp::SequenceNumber(0))
            );
            assert!(record.current_value.is_none());
        } else {
            assert_eq!(record.operation, AuditOperation::AUDITING_FAILURE);
            summaries += 1;
            assert_eq!(
                record.target_timestamp,
                Some(BACnetTimeStamp::SequenceNumber(0))
            );
            let (PropertyValue::Unsigned(count), _) =
                bacnet_encoding::primitives::decode_application_value(
                    record.current_value.as_ref().unwrap(),
                    0,
                )
                .unwrap()
            else {
                panic!()
            };
            lost += count;
        }
    }
    assert_eq!(delivered + lost, 64);
    assert_eq!(delivered, 0);
    assert!(lost > 0);
    assert!(summaries >= 1);
    timeout(WAIT, async {
        loop {
            let db = session.database.as_ref().unwrap().read().await;
            if db
                .get(&selected())
                .unwrap()
                .read_property(PropertyIdentifier::RELIABILITY, None)
                .unwrap()
                == PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
            {
                break;
            }
            drop(db);
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    for send in unrelated {
        assert!(send.complete().await.result.is_ok());
    }
    assert!(received.try_recv().is_err());
    session.stop().await.unwrap();
    assert_eq!(session.active_leases(), 0);
    peer.stop().await.unwrap();
}
