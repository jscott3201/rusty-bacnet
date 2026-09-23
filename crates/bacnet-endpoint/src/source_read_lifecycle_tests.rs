use super::*;

#[tokio::test]
async fn source_read_filters_priority_independence_and_unsupported_destinations() {
    let (mut peer, mut requests) = network().await;
    let (mut sink, mut records) = network().await;
    let mut session = session(database(false), SessionRole::Both, &sink);
    session.start().await.unwrap();
    for (level, bits, property, reports) in [
        (AuditLevel::NONE, 1, PropertyIdentifier::OBJECT_NAME, false),
        (
            AuditLevel::AUDIT_ALL,
            2,
            PropertyIdentifier::OBJECT_NAME,
            false,
        ),
        (
            AuditLevel::AUDIT_CONFIG,
            1,
            PropertyIdentifier::PRESENT_VALUE,
            false,
        ),
        (
            AuditLevel::AUDIT_CONFIG,
            1,
            PropertyIdentifier::OBJECT_NAME,
            true,
        ),
        (
            AuditLevel::AUDIT_ALL,
            1,
            PropertyIdentifier::PRESENT_VALUE,
            true,
        ),
    ] {
        session
            .database
            .as_ref()
            .unwrap()
            .write()
            .await
            .get_mut(&selected())
            .unwrap()
            .configure_audit_reporter_internal(
                level,
                AuditOperationFlags::from_bits(bits).unwrap(),
                false,
                None,
                BACnetPriorityFilter::empty(),
            )
            .unwrap();
        let read = start_read(&session, peer.local_mac(), property, None);
        let request = receive(&mut requests).await;
        let (invoke, rp) = read_request(&request);
        send(&peer, &request.source_mac, ack(invoke, &rp)).await;
        assert!(read.await.unwrap().is_ok());
        if reports {
            notification(&receive(&mut records).await, false);
        } else {
            assert!(timeout(Duration::from_millis(20), records.recv())
                .await
                .is_err());
        }
    }
    let client = session.cloned_client_handle().unwrap();
    // Unpolled futures do not reserve leases or produce traffic.
    drop(client.read_property(
        peer.local_mac(),
        target(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    ));
    for destination in [
        bacnet_endpoint_core::endpoint_ingress::EndpointApduDestination::Direct {
            destination_mac: MacAddr::from_slice(&[1]),
        },
        bacnet_endpoint_core::endpoint_ingress::EndpointApduDestination::Direct {
            destination_mac: MacAddr::from_slice(&encode_bip_mac([255; 4], 47808)),
        },
        bacnet_endpoint_core::endpoint_ingress::EndpointApduDestination::Routed {
            router_mac: MacAddr::from_slice(peer.local_mac()),
            destination_network: 7,
            destination_mac: MacAddr::from_slice(&[2]),
        },
    ] {
        assert!(client
            .read_property_with_destination(
                destination,
                Vec::new(),
                target(),
                PropertyIdentifier::PRESENT_VALUE,
                None
            )
            .await
            .is_err());
    }
    assert!(requests.try_recv().is_err());
    assert!(records.try_recv().is_err());
    assert_eq!(session.coordinator.active_count().unwrap(), 0);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

#[tokio::test]
async fn source_read_whole_operation_capacity_bounds_prelease_waiters_and_stop() {
    let (mut peer, mut requests) = network().await;
    let (mut sink, _records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    let database = Arc::clone(session.database.as_ref().unwrap());
    let guard = database.write().await;
    let mut tasks = Vec::new();
    for _ in 0..64 {
        tasks.push(start_read(
            &session,
            peer.local_mac(),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        ));
    }
    timeout(WAIT, async {
        while session.source_read.as_ref().unwrap().available_operations() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(session.coordinator.active_count().unwrap(), 0);
    assert!(start_read(
        &session,
        peer.local_mac(),
        PropertyIdentifier::PRESENT_VALUE,
        None
    )
    .await
    .unwrap()
    .is_err());
    // Seal while admitted caller futures wait on the DB: no task/lease/egress yet.
    let token = session.shared.token.clone();
    let stop = session.stop();
    tokio::pin!(stop);
    assert!(timeout(Duration::from_millis(10), &mut stop).await.is_err());
    assert!(!token.is_open());
    drop(guard);
    stop.await.unwrap();
    for task in tasks {
        assert!(timeout(WAIT, task).await.unwrap().unwrap().is_err());
    }
    assert!(requests.try_recv().is_err());
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

#[tokio::test]
async fn source_read_audit_capacity_deadline_health_and_later_recovery() {
    let (mut peer, mut requests) = network().await;
    let (mut sink, mut records) = network().await;
    let mut session = session(database(true), SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    let mut first_notification = None;
    for _ in 0..64 {
        let read = start_read(
            &session,
            peer.local_mac(),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        );
        let request = receive(&mut requests).await;
        let (invoke, rp) = read_request(&request);
        send(&peer, &request.source_mac, ack(invoke, &rp)).await;
        assert!(read.await.unwrap().is_ok());
        let envelope = receive(&mut records).await;
        notification(&envelope, true); // withhold audit ACK
        if first_notification.is_none() {
            first_notification = Some(envelope);
        }
    }
    assert_eq!(session.coordinator.active_count().unwrap(), 64);
    let read = start_read(
        &session,
        peer.local_mac(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    let request = receive(&mut requests).await;
    let (invoke, rp) = read_request(&request);
    send(&peer, &request.source_mac, ack(invoke, &rp)).await;
    assert!(
        read.await.unwrap().is_ok(),
        "delivery exhaustion cannot replace RP result"
    );
    assert_eq!(
        reliability(&session).await,
        Reliability::COMMUNICATION_FAILURE.to_raw()
    );
    assert!(timeout(Duration::from_millis(20), records.recv())
        .await
        .is_err());
    let envelope = first_notification.unwrap();
    let (_, invoke) = notification(&envelope, true);
    send(
        &sink,
        &envelope.source_mac,
        Apdu::SimpleAck(SimpleAck {
            invoke_id: invoke.unwrap(),
            service_choice: ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
        }),
    )
    .await;
    timeout(WAIT, async {
        while session.coordinator.active_count().unwrap() != 63 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        reliability(&session).await,
        Reliability::COMMUNICATION_FAILURE.to_raw(),
        "earlier success cannot erase later overload"
    );
    // Absolute deadlines reclaim all shared IDs without audit retries or backlog.
    timeout(Duration::from_secs(4), async {
        while session.coordinator.active_count().unwrap() != 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(records.try_recv().is_err());
    let read = start_read(
        &session,
        peer.local_mac(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    let request = receive(&mut requests).await;
    let (invoke, rp) = read_request(&request);
    send(&peer, &request.source_mac, ack(invoke, &rp)).await;
    assert!(read.await.unwrap().is_ok());
    let envelope = receive(&mut records).await;
    let (_, invoke) = notification(&envelope, true);
    send(
        &sink,
        &envelope.source_mac,
        Apdu::SimpleAck(SimpleAck {
            invoke_id: invoke.unwrap(),
            service_choice: ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
        }),
    )
    .await;
    timeout(WAIT, async {
        while reliability(&session).await != Reliability::NO_FAULT_DETECTED.to_raw() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

#[tokio::test]
async fn source_read_stop_and_drop_cancel_owned_work_with_held_client_clones() {
    let (mut peer, mut requests) = network().await;
    let (mut sink, mut records) = network().await;
    for stop in [true, false] {
        let mut session = session(database(true), SessionRole::Both, &sink);
        session.client_config.apdu_timeout_ms = 60_000;
        session.start().await.unwrap();
        let client = session.cloned_client_handle().unwrap();
        let weak_db = Arc::downgrade(session.database.as_ref().unwrap());
        let coordinator = Arc::clone(&session.coordinator);
        let pending = start_read(
            &session,
            peer.local_mac(),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        );
        let request = receive(&mut requests).await;
        let (invoke, rp) = read_request(&request);
        if stop {
            session.stop().await.unwrap();
        }
        drop(session);
        assert!(timeout(WAIT, pending).await.unwrap().unwrap().is_err());
        send(&peer, &request.source_mac, ack(invoke, &rp)).await;
        assert!(client
            .read_property(
                peer.local_mac(),
                target(),
                PropertyIdentifier::PRESENT_VALUE,
                None
            )
            .await
            .is_err());
        timeout(WAIT, async {
            while coordinator.active_count().unwrap() != 0 || weak_db.upgrade().is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(records.try_recv().is_err());
    }
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

#[tokio::test]
async fn source_read_and_target_reporting_remain_independent_over_bip() {
    use bacnet_server::server::{AuditReporterConfig, BACnetServer, DeviceBinding};
    let (mut sink, mut records) = network().await;
    let mut db = crate::DeviceIdentity::new(456, 42)
        .unwrap()
        .build_database()
        .unwrap();
    db.add(Box::new(
        bacnet_objects::analog::AnalogValueObject::new(7, "Target", 0).unwrap(),
    ))
    .unwrap();
    let mut reporter = AuditReporterObject::new(1, "Target Reporter").unwrap();
    reporter
        .configure_audit_reporter_internal(
            AuditLevel::AUDIT_ALL,
            AuditOperationFlags::from_bits(1).unwrap(),
            false,
            None,
            BACnetPriorityFilter::all(),
        )
        .unwrap();
    db.add(Box::new(reporter)).unwrap();
    db.get_mut(&oid(ObjectType::DEVICE, 456))
        .unwrap()
        .device_authority_internal()
        .unwrap()
        .provision_audit_recipient(bacnet_types::constructed::BACnetRecipient::Device(oid(
            ObjectType::DEVICE,
            999,
        )))
        .unwrap();
    let mut target_server = BACnetServer::builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .database(db)
        .audit_reporter(AuditReporterConfig {
            reporter: selected(),
        })
        .device_binding(
            DeviceBinding::local(oid(ObjectType::DEVICE, 999), sink.local_mac()).unwrap(),
        )
        .unwrap()
        .build()
        .await
        .unwrap();
    let mut session = session(database(false), SessionRole::Both, &sink);
    session.start().await.unwrap();
    assert!(session
        .client()
        .unwrap()
        .read_property(
            target_server.local_mac(),
            target(),
            PropertyIdentifier::PRESENT_VALUE,
            None
        )
        .await
        .is_ok());
    let (first, _) = notification(&receive(&mut records).await, false);
    let (second, _) = notification(&receive(&mut records).await, false);
    let (source, target_record) = if first.source_timestamp.is_some() {
        (first, second)
    } else {
        (second, first)
    };
    assert!(source.target_timestamp.is_none());
    assert!(target_record.target_timestamp.is_some());
    assert_eq!(source.invoke_id, target_record.invoke_id);
    assert_eq!(source.target_object, target_record.target_object);
    assert!(matches!(source.target_device, BACnetRecipient::Address(_)));
    assert_eq!(
        target_record.target_device,
        BACnetRecipient::Device(oid(ObjectType::DEVICE, 456))
    );
    assert_eq!(source.result, None);
    assert_eq!(target_record.result, None);
    session.stop().await.unwrap();
    target_server.stop().await.unwrap();
    sink.stop().await.unwrap();
}
