use super::*;

#[tokio::test]
async fn endpoint_rpm_prewire_profile_and_byte_bounds_with_and_without_source() {
    let (mut peer, mut requests) = network().await;
    let (mut sink, mut received) = network().await;
    for source in [false, true] {
        let mut session = if source {
            let mut db = database(false);
            db.get_mut(&oid(ObjectType::DEVICE, 123))
                .unwrap()
                .device_authority_internal()
                .unwrap()
                .provision_audit_recipient(BACnetRecipient::Device(oid(ObjectType::DEVICE, 999)))
                .unwrap();
            BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
                .role(SessionRole::ClientOnly)
                .queue_capacity(128)
                .database(db)
                .source_audit_device_binding(oid(ObjectType::DEVICE, 999), address(&sink))
                .build_session()
                .unwrap()
                .with_source_audit_reporter(selected())
        } else {
            BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
                .role(SessionRole::ClientOnly)
                .build_session()
                .unwrap()
        };
        session.start().await.unwrap();
        let client = session.cloned_client_handle().unwrap();
        let mut bad = vec![vec![]];
        let mut empty = specs();
        empty[0].list_of_property_references.clear();
        bad.push(empty);
        for property in [
            PropertyIdentifier::ALL,
            PropertyIdentifier::REQUIRED,
            PropertyIdentifier::OPTIONAL,
        ] {
            let mut s = specs();
            s[0].list_of_property_references[0].property_identifier = property;
            bad.push(s);
        }
        for kind in [ObjectType::DEVICE, ObjectType::NETWORK_PORT] {
            let mut s = specs();
            s[0].object_identifier = oid(kind, ObjectIdentifier::MAX_INSTANCE);
            bad.push(s);
        }
        let single = ReadAccessSpecification {
            object_identifier: target(),
            list_of_property_references: vec![PropertyReference {
                property_identifier: PropertyIdentifier::OBJECT_NAME,
                property_array_index: Some(0),
            }],
        };
        let mut too_many = single.clone();
        too_many.list_of_property_references =
            vec![single.list_of_property_references[0].clone(); 65];
        bad.push(vec![too_many]);
        let too_large = ReadAccessSpecification {
            object_identifier: target(),
            list_of_property_references: vec![
                PropertyReference {
                    property_identifier: PropertyIdentifier::from_raw(u32::MAX),
                    property_array_index: Some(u32::MAX)
                };
                64
            ],
        };
        bad.push(vec![too_large]);
        for specs in bad {
            assert!(client
                .read_property_multiple(peer.local_mac(), specs)
                .await
                .is_err());
            assert_eq!(session.active_leases(), 0);
        }
        assert!(requests.try_recv().is_err());
        assert!(received.try_recv().is_err());
        if source {
            assert_eq!(
                session.source_read.as_ref().unwrap().available_operations(),
                64
            );
        }
        let mut valid = single;
        valid.list_of_property_references[0].property_array_index = None;
        valid.list_of_property_references = vec![valid.list_of_property_references[0].clone(); 64];
        let read = start(&session, peer.local_mac(), vec![valid]);
        let env = receive(&mut requests).await;
        let (invoke, rpm) = request(&env);
        send(&peer, &env.source_mac, response(invoke, &success(&rpm))).await;
        assert!(read.await.unwrap().is_ok());
        if source {
            for record in records(&mut received, &sink, false, 64).await {
                assert_eq!(
                    record.source_timestamp,
                    Some(BACnetTimeStamp::SequenceNumber(0))
                );
            }
        }
        session.stop().await.unwrap();
        assert_eq!(session.active_leases(), 0);
    }
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

#[tokio::test]
async fn source_rpm_raw_ack_bound_and_segmented_terminal_fanout() {
    let (mut peer, mut requests) = network().await;
    let (mut sink, mut received) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    for segmented in [false, true] {
        let read = start(&session, peer.local_mac(), specs());
        let env = receive(&mut requests).await;
        let (invoke, rpm) = request(&env);
        let mut ack = success(&rpm);
        if !segmented {
            ack.list_of_read_access_results[0].list_of_results[0].property_value =
                Some(vec![0; 481]);
        }
        let Apdu::ComplexAck(mut pdu) = response(invoke, &ack) else {
            unreachable!()
        };
        if segmented {
            pdu.segmented = true;
            pdu.more_follows = true;
            pdu.sequence_number = Some(0);
            pdu.proposed_window_size = Some(1);
        }
        send(&peer, &env.source_mac, Apdu::ComplexAck(pdu)).await;
        assert!(read.await.unwrap().is_err());
        for record in records(&mut received, &sink, false, 5).await {
            assert!(record.result.is_some());
            assert!(matches!(record.target_device, BACnetRecipient::Address(_)));
        }
        if segmented {
            assert!(
                matches!(decode_apdu(receive(&mut requests).await.apdu).unwrap(),Apdu::Abort(pdu) if pdu.abort_reason==AbortReason::SEGMENTATION_NOT_SUPPORTED)
            );
        }
    }
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

#[tokio::test]
async fn source_rpm_cancelled_retry_retains_one_snapshot_and_stop_joins() {
    let (mut peer, mut requests) = network().await;
    let (mut old, mut old_records) = network().await;
    let (mut new, mut new_records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &old);
    session.client_config.apdu_retries = 1;
    session.start().await.unwrap();
    let read = start(&session, peer.local_mac(), specs());
    let first = receive(&mut requests).await;
    let (invoke, rpm) = request(&first);
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
    for env in [
        receive(&mut old_records).await,
        receive(&mut new_records).await,
    ] {
        assert_eq!(notification(&env, false).0.operation, AuditOperation::WRITE);
    }
    let retry = receive(&mut requests).await;
    assert_eq!(request(&retry), (invoke, rpm.clone()));
    send(&peer, &retry.source_mac, response(invoke, &success(&rpm))).await;
    for record in records(&mut old_records, &old, false, 5).await {
        assert_eq!(record.invoke_id, Some(invoke));
        assert_eq!(
            record.source_timestamp,
            Some(BACnetTimeStamp::SequenceNumber(0))
        );
    }
    let read = start(&session, peer.local_mac(), specs());
    let env = receive(&mut requests).await;
    let (invoke, rpm) = request(&env);
    send(&peer, &env.source_mac, response(invoke, &success(&rpm))).await;
    read.await.unwrap().unwrap();
    records(&mut new_records, &new, false, 5).await;
    assert!(old_records.try_recv().is_err());
    assert!(new_records.try_recv().is_err());
    let read = start(&session, peer.local_mac(), specs());
    receive(&mut requests).await;
    session.stop().await.unwrap();
    assert!(read.await.unwrap().is_err());
    assert_eq!(session.active_leases(), 0);
    peer.stop().await.unwrap();
    old.stop().await.unwrap();
    new.stop().await.unwrap();
}

#[tokio::test]
async fn source_rpm_drop_releases_pending_operation_and_database() {
    let (mut peer, mut requests) = network().await;
    let (mut sink, _) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    let db = Arc::clone(session.database.as_ref().unwrap());
    let coordinator = Arc::clone(&session.coordinator);
    let client = session.cloned_client_handle().unwrap();
    let read = start(&session, peer.local_mac(), specs());
    receive(&mut requests).await;
    drop(session);
    assert!(timeout(WAIT, read).await.unwrap().unwrap().is_err());
    assert!(client
        .read_property_multiple(peer.local_mac(), specs())
        .await
        .is_err());
    timeout(WAIT, async {
        loop {
            let mut db = db.write().await;
            if db.remove(&oid(ObjectType::DEVICE, 123)).is_ok() {
                break;
            }
            drop(db);
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(coordinator.active_count().unwrap(), 0);
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

#[tokio::test]
async fn source_rpm_partial_notification_admission_counts_only_dropped_occurrences() {
    let (mut peer, mut requests) = network().await;
    let (mut sink, mut received) = network().await;
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
            None,
        )
        .unwrap();
    let mut session = session(db, SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    let owner = Arc::clone(session.notifications.as_ref().unwrap());
    let held: Vec<_> = (0..62).map(|_| owner.try_admit_audit().unwrap()).collect();
    let read = start(&session, peer.local_mac(), specs());
    let env = receive(&mut requests).await;
    let (invoke, rpm) = request(&env);
    send(&peer, &env.source_mac, response(invoke, &success(&rpm))).await;
    assert!(read.await.unwrap().is_ok());
    let delivered = records(&mut received, &sink, false, 2).await;
    assert!(delivered
        .iter()
        .all(|r| r.operation == AuditOperation::READ));
    drop(held);
    let summary = notification(&receive(&mut received).await, false).0;
    assert_eq!(summary.operation, AuditOperation::AUDITING_FAILURE);
    assert_eq!(
        summary.target_timestamp,
        Some(BACnetTimeStamp::SequenceNumber(0))
    );
    assert_eq!(summary.current_value, Some(vec![0x21, 3]));
    assert!(summary.target_value.is_none());
    assert!(received.try_recv().is_err());
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}
