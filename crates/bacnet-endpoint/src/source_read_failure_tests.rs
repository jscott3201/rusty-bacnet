use super::*;

fn failure_flags() -> AuditOperationFlags {
    let mut flags = AuditOperationFlags::empty();
    flags.insert(AuditOperation::READ);
    flags.insert(AuditOperation::AUDITING_FAILURE);
    flags
}
pub(super) fn failure_database(confirmed: bool) -> ObjectDatabase {
    let mut db = database(confirmed);
    db.get_mut(&selected())
        .unwrap()
        .configure_audit_reporter_internal(
            AuditLevel::AUDIT_ALL,
            failure_flags(),
            confirmed,
            None,
            BACnetPriorityFilter::empty(),
        )
        .unwrap();
    db.set_clock_reader(None);
    db
}
fn expected_summary(count: u64, timestamp: BACnetTimeStamp) -> BACnetAuditNotification {
    let mut current = BytesMut::new();
    bacnet_encoding::primitives::encode_app_unsigned(&mut current, count);
    BACnetAuditNotification {
        source_timestamp: None,
        target_timestamp: Some(timestamp),
        source_device: BACnetRecipient::Device(oid(ObjectType::DEVICE, 123)),
        source_object: None,
        operation: AuditOperation::AUDITING_FAILURE,
        source_comment: None,
        target_comment: None,
        invoke_id: None,
        source_user_id: None,
        source_user_role: None,
        target_device: BACnetRecipient::Device(oid(ObjectType::DEVICE, 123)),
        target_object: None,
        target_property: None,
        target_priority: None,
        target_value: None,
        current_value: Some(current.to_vec()),
        result: None,
    }
}
pub(super) async fn complete_read(
    session: &EndpointSession<BipTransport>,
    peer: &NetworkLayer<BipTransport>,
    requests: &mut mpsc::Receiver<ReceivedApdu>,
) {
    let read = start_read(
        session,
        peer.local_mac(),
        PropertyIdentifier::PRESENT_VALUE,
        None,
    );
    let request = receive(requests).await;
    let (invoke, rp) = read_request(&request);
    send(peer, &request.source_mac, ack(invoke, &rp)).await;
    assert!(read.await.unwrap().is_ok());
}

#[tokio::test]
async fn source_failure_permit_losses_wire_shape_reverse_completion_and_sequence_wrap() {
    let (mut peer, mut requests) = network().await;
    let (mut sink, mut records) = network().await;
    for confirmed in [false, true] {
        let mut db = failure_database(confirmed);
        for _ in 0..u16::MAX {
            db.next_event_sequence_number();
        }
        let mut session = session(db, SessionRole::Both, &sink);
        session.start().await.unwrap();
        let owner = session.notifications.as_ref().unwrap();
        let permits: Vec<_> = (0..64).map(|_| owner.try_admit_audit().unwrap()).collect();
        let mut operations = Vec::new();
        for _ in 0..3 {
            let read = start_read(
                &session,
                peer.local_mac(),
                PropertyIdentifier::PRESENT_VALUE,
                None,
            );
            let request = receive(&mut requests).await;
            operations.push((read, request));
        }
        for (read, request) in operations.into_iter().rev() {
            let (invoke, rp) = read_request(&request);
            send(&peer, &request.source_mac, ack(invoke, &rp)).await;
            assert!(read.await.unwrap().is_ok());
        }
        assert!(records.try_recv().is_err());
        drop(permits);
        let envelope = receive(&mut records).await;
        let (summary, invoke) = notification(&envelope, confirmed);
        assert_eq!(
            summary,
            expected_summary(3, BACnetTimeStamp::SequenceNumber(65535))
        );
        if let Some(invoke_id) = invoke {
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
        timeout(WAIT, async {
            while reliability(&session).await != Reliability::NO_FAULT_DETECTED.to_raw() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            records.try_recv().is_err(),
            "ordinary records are never replayed"
        );
        session.stop().await.unwrap();
    }
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

#[tokio::test]
async fn source_failure_pending_context_invalidates_without_another_read() {
    let (mut peer, mut requests) = network().await;
    let (mut sink, mut records) = network().await;
    for change in ["mode-aba", "level-aba", "filter-aba"] {
        let mut session = session(failure_database(true), SessionRole::ClientOnly, &sink);
        session.start().await.unwrap();
        let permits: Vec<_> = (0..64)
            .map(|_| {
                session
                    .notifications
                    .as_ref()
                    .unwrap()
                    .try_admit_audit()
                    .unwrap()
            })
            .collect();
        complete_read(&session, &peer, &mut requests).await;
        let old_status;
        {
            let mut db = session.database.as_ref().unwrap().write().await;
            old_status = db
                .get(&selected())
                .unwrap()
                .audit_reporter_internal()
                .unwrap()
                .status_internal();
            let object = db.get_mut(&selected()).unwrap();
            let (level, flags, confirmed) = match change {
                "mode-aba" => (AuditLevel::AUDIT_ALL, failure_flags(), false),
                "level-aba" => (AuditLevel::NONE, failure_flags(), true),
                "filter-aba" => (
                    AuditLevel::AUDIT_ALL,
                    AuditOperationFlags::from_bits(1).unwrap(),
                    true,
                ),
                _ => unreachable!(),
            };
            object
                .configure_audit_reporter_internal(
                    level,
                    flags,
                    confirmed,
                    None,
                    BACnetPriorityFilter::empty(),
                )
                .unwrap();
            object
                .configure_audit_reporter_internal(
                    AuditLevel::AUDIT_ALL,
                    failure_flags(),
                    true,
                    None,
                    BACnetPriorityFilter::empty(),
                )
                .unwrap();
        }
        drop(permits);
        assert!(
            timeout(Duration::from_millis(30), records.recv())
                .await
                .is_err(),
            "{change}"
        );
        assert_eq!(session.coordinator.active_count().unwrap(), 0, "{change}");
        session.stop().await.unwrap();
        drop(old_status); // retained stale Arcs did not preserve validity
    }
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

#[tokio::test]
async fn source_failure_pending_worker_stop_and_drop_reclaim_owner() {
    let (mut peer, mut requests) = network().await;
    let (mut sink, mut records) = network().await;
    for stop in [true, false] {
        let mut session = session(failure_database(true), SessionRole::ClientOnly, &sink);
        session.start().await.unwrap();
        let client = session.cloned_client_handle().unwrap();
        let weak_db = Arc::downgrade(session.database.as_ref().unwrap());
        let weak_source = Arc::downgrade(session.source_read.as_ref().unwrap());
        let coordinator = Arc::clone(&session.coordinator);
        let permits: Vec<_> = (0..64)
            .map(|_| {
                session
                    .notifications
                    .as_ref()
                    .unwrap()
                    .try_admit_audit()
                    .unwrap()
            })
            .collect();
        complete_read(&session, &peer, &mut requests).await;
        if stop {
            session.stop().await.unwrap();
        }
        drop(session);
        drop(permits);
        timeout(WAIT, async {
            while weak_db.upgrade().is_some() || weak_source.upgrade().is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(coordinator.active_count().unwrap(), 0);
        assert!(records.try_recv().is_err());
        drop(client);
    }
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}
