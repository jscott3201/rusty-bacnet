use super::*;

/// A segmented answer is taken segment by segment, each acknowledged, and
/// recorded once it is whole, as an unsegmented one is.
#[tokio::test(start_paused = true)]
async fn source_rpm_segmented_answer_is_reassembled_and_recorded() {
    use bacnet_encoding::apdu::SegmentAck;
    let (peer, mut requests) = network().await;
    let (sink, mut received) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    let client = client_of(&session);
    let read = start(&client, peer.local_mac(), specs());
    let env = receive(&mut requests).await;
    let (invoke, rpm) = request(&env);
    let ack = success(&rpm);
    let Apdu::ComplexAck(whole) = response(invoke, &ack) else {
        unreachable!()
    };
    let half = whole.service_ack.len() / 2;
    for (sequence, more) in [(0, true), (1, false)] {
        let mut segment = whole.clone();
        segment.segmented = true;
        segment.more_follows = more;
        segment.sequence_number = Some(sequence);
        segment.proposed_window_size = Some(1);
        segment.service_ack = if more {
            whole.service_ack.slice(..half)
        } else {
            whole.service_ack.slice(half..)
        };
        send(&peer, &env.source_mac, Apdu::ComplexAck(segment)).await;
        let acknowledged = decode_apdu(receive(&mut requests).await.apdu).unwrap();
        assert!(
            matches!(
                &acknowledged,
                Apdu::SegmentAck(SegmentAck { invoke_id, sequence_number, negative_ack: false, .. })
                    if *invoke_id == invoke && *sequence_number == sequence
            ),
            "segment {sequence}: {acknowledged:?}"
        );
    }
    assert_eq!(read.await.unwrap().unwrap(), ack);
    for record in records(&mut received, &sink, false, 5).await {
        assert_eq!(record.result, None);
        assert_eq!(record.invoke_id, Some(invoke));
    }
    assert!(received.try_recv().is_err());
    drop(client);
    session.stop().await.unwrap();
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

/// A caller that goes away loses nothing: the session's task retries the
/// request unchanged and records its answer, to the recipient the request
/// was admitted with; a stop then ends a request still waiting.
#[tokio::test(start_paused = true)]
async fn source_rpm_cancelled_retry_retains_one_snapshot_and_stop_joins() {
    let (peer, mut requests) = network().await;
    let (old, mut old_records) = network().await;
    let (new, mut new_records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &old);
    session.client_config.apdu_retries = 1;
    session.start().await.unwrap();
    let client = client_of(&session);
    let read = start(&client, peer.local_mac(), specs());
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
    let read = start(&client, peer.local_mac(), specs());
    let env = receive(&mut requests).await;
    let (invoke, rpm) = request(&env);
    send(&peer, &env.source_mac, response(invoke, &success(&rpm))).await;
    read.await.unwrap().unwrap();
    records(&mut new_records, &new, false, 5).await;
    assert!(old_records.try_recv().is_err());
    assert!(new_records.try_recv().is_err());
    let read = start(&client, peer.local_mac(), specs());
    receive(&mut requests).await;
    session.stop().await.unwrap();
    assert!(read.await.unwrap().is_err());
    assert_eq!(session.active_leases(), 0);
    peer.stop().await.unwrap();
    old.stop().await.unwrap();
    new.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn source_rpm_drop_releases_pending_operation_and_database() {
    let (peer, mut requests) = network().await;
    let (sink, _) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    let db = Arc::clone(session.database.as_ref().unwrap());
    let coordinator = Arc::clone(&session.coordinator);
    let client = client_of(&session);
    let read = start(&client, peer.local_mac(), specs());
    receive(&mut requests).await;
    drop(session);
    assert!(timeout(WAIT, read).await.unwrap().unwrap().is_err());
    assert!(
        client
            .read_property_multiple(peer.local_mac(), specs())
            .await
            .is_err()
    );
    drop(client);
    eventually(WAIT, "the local Device can be removed", async || {
        db.write()
            .await
            .remove(&oid(ObjectType::DEVICE, 123))
            .is_ok()
    })
    .await;
    assert_eq!(coordinator.active_count().unwrap(), 0);
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn source_rpm_partial_notification_admission_counts_only_dropped_occurrences() {
    let (peer, mut requests) = network().await;
    let (sink, mut received) = network().await;
    let mut db = database(false);
    let mut flags = AuditOperationFlags::empty();
    flags.insert(AuditOperation::READ);
    flags.insert(AuditOperation::AUDITING_FAILURE);
    db.get_mut(&selected())
        .unwrap()
        .audit_reporter_mut()
        .expect("an Audit Reporter")
        .configure(
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
    let client = client_of(&session);
    let read = start(&client, peer.local_mac(), specs());
    let env = receive(&mut requests).await;
    let (invoke, rpm) = request(&env);
    send(&peer, &env.source_mac, response(invoke, &success(&rpm))).await;
    assert!(read.await.unwrap().is_ok());
    let delivered = records(&mut received, &sink, false, 2).await;
    assert!(
        delivered
            .iter()
            .all(|r| r.operation == AuditOperation::READ)
    );
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
