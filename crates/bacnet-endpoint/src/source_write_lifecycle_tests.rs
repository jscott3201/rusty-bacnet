use super::*;
use crate::source_audit::slots::SLOTS;
/// 64 audited writes, a lane's share to each of 16 stations, the first the
/// peer (#2652), admitted while the database is held: each holds a slot and
/// waits for the database, with no invoke ID and nothing sent, and a 65th
/// waits for a slot. A stop then seals admission at once, waits for the
/// database itself, and ends every one of them unsent: the peer is sent
/// nothing.
#[tokio::test(start_paused = true)]
async fn source_write_whole_operation_capacity_bounds_prelease_waiters_and_stop() {
    let (peer, mut requests) = network().await;
    let (sink, _records) = network().await;
    let mut session = session(write_database(false), SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    let client = client_of(&session);
    let source = Arc::clone(session.source_audit.as_ref().unwrap());
    let database = Arc::clone(session.database.as_ref().unwrap());
    let guard = database.write().await;
    let mut tasks = Vec::new();
    for n in 0..SLOTS {
        tasks.push(start_write(
            &client,
            &filling(&peer, n),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        ));
    }
    eventually(WAIT, "every source operation is in use", async || {
        source.available_operations() == 0
    })
    .await;
    assert_eq!(session.coordinator.active_count().unwrap(), 0);
    // One more waits for a slot, in its caller's task (#2617).
    let waiter = start_write(
        &client,
        peer.local_mac(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(!waiter.is_finished());
    tasks.push(waiter);
    // Seal while the admitted ones wait for the DB in the session's tasks
    // (#2755): no lease or egress yet.
    let coordinator = Arc::clone(&session.coordinator);
    let stop = session.stop();
    tokio::pin!(stop);
    assert!(timeout(Duration::from_millis(10), &mut stop).await.is_err());
    assert!(source.sealed());
    drop(guard);
    stop.await.unwrap();
    for task in tasks {
        assert!(timeout(WAIT, task).await.unwrap().unwrap().is_err());
    }
    assert!(requests.try_recv().is_err());
    assert_eq!(coordinator.active_count().unwrap(), 0);
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn source_write_audit_capacity_deadline_health_and_later_recovery() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    let mut session = session(write_database(true), SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    let client = client_of(&session);
    let mut first_notification = None;
    for _ in 0..64 {
        let read = start_write(
            &client,
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
        &client,
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
    eventually(WAIT, "63 invoke IDs are active", async || {
        session.coordinator.active_count().unwrap() == 63
    })
    .await;
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
        &client,
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
    eventually(WAIT, "Reliability is NO_FAULT_DETECTED", async || {
        reliability(&session).await == Reliability::NO_FAULT_DETECTED.to_raw()
    })
    .await;
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn source_write_stop_and_drop_cancel_owned_work_with_held_client_clones() {
    let (peer, mut requests) = network().await;
    let (sink, mut records) = network().await;
    for stop in [true, false] {
        let mut session = session(write_database(true), SessionRole::Both, &sink);
        session.client_config.apdu_timeout_ms = 60_000;
        session.start().await.unwrap();
        let client = client_of(&session);
        let weak_db = Arc::downgrade(session.database.as_ref().unwrap());
        let coordinator = Arc::clone(&session.coordinator);
        let pending = start_write(
            &client,
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
                    target(),
                    PropertyIdentifier::PRESENT_VALUE,
                    None,
                    vec![0],
                    None,
                )
                .await
                .is_err()
        );
        drop(client);
        eventually(
            WAIT,
            "invoke IDs are free and the database is dropped",
            async || coordinator.active_count().unwrap() == 0 && weak_db.upgrade().is_none(),
        )
        .await;
        assert!(records.try_recv().is_err());
    }
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

/// A write is recorded by the recipient and the Reporter's selection as
/// they stood when it was admitted, whatever changes before its answer.
#[tokio::test(start_paused = true)]
async fn source_write_captures_recipient_and_policy_before_peer_completion() {
    let (peer, mut requests) = network().await;
    let (old, mut old_records) = network().await;
    let (new, mut new_records) = network().await;
    let mut session = session(write_database(false), SessionRole::ClientOnly, &old);
    session.start().await.unwrap();
    let client = client_of(&session);
    let pending = write_value(
        &client,
        peer.local_mac(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
        vec![0],
        Some(8),
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
        .audit_reporter_mut()
        .expect("an Audit Reporter")
        .configure(
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
    assert_eq!(record.target_priority, Some(8));
    let pending = start_write(
        &client,
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
