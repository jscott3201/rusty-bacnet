use super::*;
#[tokio::test]
async fn source_write_whole_operation_capacity_bounds_prelease_waiters_and_stop() {
    let (mut peer, mut requests) = network().await;
    let (mut sink, _records) = network().await;
    let mut session = session(write_database(false), SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    let database = Arc::clone(session.database.as_ref().unwrap());
    let guard = database.write().await;
    let mut tasks = Vec::new();
    for _ in 0..64 {
        tasks.push(start_write(
            &session,
            peer.local_mac(),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        ));
    }
    timeout(WAIT, async {
        while session
            .source_audit
            .as_ref()
            .unwrap()
            .available_operations()
            != 0
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(session.coordinator.active_count().unwrap(), 0);
    assert!(
        start_write(
            &session,
            peer.local_mac(),
            PropertyIdentifier::PRESENT_VALUE,
            None
        )
        .await
        .unwrap()
        .is_err()
    );
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
async fn source_write_audit_capacity_deadline_health_and_later_recovery() {
    let (mut peer, mut requests) = network().await;
    let (mut sink, mut records) = network().await;
    let mut session = session(write_database(true), SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    let mut first_notification = None;
    for _ in 0..64 {
        let read = start_write(
            &session,
            peer.local_mac(),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        );
        let request = receive(&mut requests).await;
        let (invoke, rp) = write_request(&request);
        send(&peer, &request.source_mac, write_ack(invoke, &rp)).await;
        assert!(read.await.unwrap().is_ok());
        let envelope = receive(&mut records).await;
        notification(&envelope, true); // withhold audit ACK
        if first_notification.is_none() {
            first_notification = Some(envelope);
        }
    }
    assert_eq!(session.coordinator.active_count().unwrap(), 64);
    let read = start_write(
        &session,
        peer.local_mac(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    let request = receive(&mut requests).await;
    let (invoke, rp) = write_request(&request);
    send(&peer, &request.source_mac, write_ack(invoke, &rp)).await;
    assert!(
        read.await.unwrap().is_ok(),
        "delivery exhaustion cannot replace RP result"
    );
    assert_eq!(
        reliability(&session).await,
        Reliability::COMMUNICATION_FAILURE.to_raw()
    );
    assert!(
        timeout(Duration::from_millis(20), records.recv())
            .await
            .is_err()
    );
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
    let read = start_write(
        &session,
        peer.local_mac(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    let request = receive(&mut requests).await;
    let (invoke, rp) = write_request(&request);
    send(&peer, &request.source_mac, write_ack(invoke, &rp)).await;
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
async fn source_write_stop_and_drop_cancel_owned_work_with_held_client_clones() {
    let (mut peer, mut requests) = network().await;
    let (mut sink, mut records) = network().await;
    for stop in [true, false] {
        let mut session = session(write_database(true), SessionRole::Both, &sink);
        session.client_config.apdu_timeout_ms = 60_000;
        session.start().await.unwrap();
        let client = session.cloned_client_handle().unwrap();
        let weak_db = Arc::downgrade(session.database.as_ref().unwrap());
        let coordinator = Arc::clone(&session.coordinator);
        let pending = start_write(
            &session,
            peer.local_mac(),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        );
        let request = receive(&mut requests).await;
        let (invoke, rp) = write_request(&request);
        if stop {
            session.stop().await.unwrap();
        }
        drop(session);
        assert!(timeout(WAIT, pending).await.unwrap().unwrap().is_err());
        send(&peer, &request.source_mac, write_ack(invoke, &rp)).await;
        assert!(
            client
                .write_property(
                    peer.local_mac(),
                    WritePropertyRequest {
                        object_identifier: target(),
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                        property_value: vec![0],
                        priority: None,
                    },
                    Commandability::Commandable,
                )
                .await
                .is_err()
        );
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
async fn source_write_captures_recipient_and_policy_before_peer_completion() {
    let (mut peer, mut requests) = network().await;
    let (mut old, mut old_records) = network().await;
    let (mut new, mut new_records) = network().await;
    let mut session = session(write_database(false), SessionRole::ClientOnly, &old);
    session.start().await.unwrap();
    let pending = start_write(
        &session,
        peer.local_mac(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    let envelope = receive(&mut requests).await;
    let (invoke, wp) = write_request(&envelope);
    session
        .write_audit_recipient(Some(BACnetRecipient::Address(
            bacnet_types::constructed::BACnetAddress {
                network_number: 0,
                mac_address: MacAddr::from_slice(new.local_mac()),
            },
        )))
        .await
        .unwrap();
    notification(&receive(&mut old_records).await, false);
    notification(&receive(&mut new_records).await, false);
    session
        .database
        .as_ref()
        .unwrap()
        .write()
        .await
        .get_mut(&selected())
        .unwrap()
        .configure_audit_reporter_internal(
            AuditLevel::NONE,
            AuditOperationFlags::empty(),
            false,
            None,
            BACnetPriorityFilter::empty(),
            None,
        )
        .unwrap();
    send(&peer, &envelope.source_mac, write_ack(invoke, &wp)).await;
    pending.await.unwrap().unwrap();
    let (record, _) = notification(&receive(&mut old_records).await, false);
    assert_eq!(record.operation, AuditOperation::WRITE);
    assert_eq!(record.invoke_id, Some(invoke));
    assert_eq!(record.target_priority, Some(16));
    let pending = start_write(
        &session,
        peer.local_mac(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    let envelope = receive(&mut requests).await;
    let (invoke, wp) = write_request(&envelope);
    send(&peer, &envelope.source_mac, write_ack(invoke, &wp)).await;
    pending.await.unwrap().unwrap();
    assert!(
        timeout(Duration::from_millis(30), new_records.recv())
            .await
            .is_err()
    );
    assert!(old_records.try_recv().is_err());
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    old.stop().await.unwrap();
    new.stop().await.unwrap();
}
