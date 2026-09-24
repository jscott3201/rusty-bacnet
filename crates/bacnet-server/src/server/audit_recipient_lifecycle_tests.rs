//! Recipient admission, health generations and shutdown boundaries.
use super::*;

#[tokio::test(start_paused = true)]
async fn recipient_cancelled_stop_stays_sealed_until_retry_uninstalls() {
    let mut fixture = fixture(reporter()).await;
    fixture.transport.block.store(true, Ordering::Release);
    assert!(matches!(
        change(&fixture, &device(21)).await,
        Apdu::SimpleAck(_)
    ));
    settle().await;
    let database = Arc::clone(&fixture.server.db);
    let mut db = database.write().await;
    assert!(
        tokio::time::timeout(Duration::from_millis(10), fixture.server.stop())
            .await
            .is_err()
    );
    assert!(db.remove(&oid(ObjectType::DEVICE, 10)).is_err());
    assert!(db
        .get_mut(&oid(ObjectType::DEVICE, 10))
        .unwrap()
        .write_property(
            PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
            None,
            PropertyValue::Null,
            None
        )
        .is_err());
    assert_eq!(
        fixture.server.notification_transactions.audit_resources(),
        (false, 0, 64)
    );
    drop(db);
    fixture.server.stop().await.unwrap();
    fixture.server.stop().await.unwrap();
    assert!(fixture.server.notification_transactions.workers_empty());
    assert!(database
        .write()
        .await
        .remove(&oid(ObjectType::DEVICE, 10))
        .unwrap()
        .is_some());
}

fn complete(fixture: &Fixture, index: usize, mac: &[u8], success: bool) {
    let request = confirmed_notification(&fixture.transport.sent, index);
    let response = if success {
        Apdu::SimpleAck(SimpleAck {
            invoke_id: request.invoke_id,
            service_choice: request.service_choice,
        })
    } else {
        Apdu::Error(bacnet_encoding::apdu::ErrorPdu {
            invoke_id: request.invoke_id,
            service_choice: request.service_choice,
            error_class: ErrorClass::SERVICES,
            error_code: ErrorCode::OTHER,
            error_data: Bytes::new(),
        })
    };
    assert!(fixture
        .server
        .notification_transactions
        .admit_terminal(mac, None, &response));
}

#[tokio::test(start_paused = true)]
async fn recipient_confirmed_pair_failure_wins_in_either_completion_order() {
    for failure_first in [true, false] {
        let mut reporter = reporter();
        reporter.set_issue_confirmed_notifications(true).unwrap();
        let mut fixture = fixture(reporter).await;
        assert!(matches!(
            change(&fixture, &device(21)).await,
            Apdu::SimpleAck(_)
        ));
        settle().await;
        assert_eq!(fixture.server.notification_transactions.active_count(), 2);
        for index in if failure_first { [0, 1] } else { [1, 0] } {
            complete(
                &fixture,
                index,
                if index == 0 { LOGGER } else { NEW_LOGGER },
                index == 1,
            );
            settle().await;
        }
        assert_eq!(
            health(&fixture.server).await,
            Reliability::COMMUNICATION_FAILURE
        );
        assert_eq!(
            read_recipient(&fixture).await,
            PropertyValue::ApplicationData(value(&device(21)))
        );
        assert_eq!(fixture.server.notification_transactions.active_count(), 0);
        assert_eq!(
            fixture.server.notification_transactions.audit_resources(),
            (false, 0, 64)
        );
        fixture.server.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn recipient_old_generation_completion_cannot_poison_new_pair_health() {
    let mut reporter = reporter();
    reporter.set_issue_confirmed_notifications(true).unwrap();
    let mut fixture = fixture(reporter).await;
    assert!(matches!(
        write_value(&fixture.server, None).await,
        Apdu::SimpleAck(_)
    ));
    settle().await;
    assert!(matches!(
        change(&fixture, &device(21)).await,
        Apdu::SimpleAck(_)
    ));
    settle().await;
    complete(&fixture, 1, LOGGER, true);
    complete(&fixture, 2, NEW_LOGGER, true);
    settle().await;
    complete(&fixture, 0, LOGGER, false);
    settle().await;
    assert_eq!(
        health(&fixture.server).await,
        Reliability::NO_FAULT_DETECTED
    );
    assert_eq!(
        fixture
            .server
            .db
            .read()
            .await
            .reserve_event_sequence_number()
            .number(),
        2
    );
    fixture.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn recipient_aba_retires_and_wakes_pending_summary_without_new_ordinary_work() {
    let mut reporter = reporter();
    let mut flags = AuditOperationFlags::empty();
    flags.insert(AuditOperation::WRITE);
    flags.insert(AuditOperation::AUDITING_FAILURE);
    reporter.set_auditable_operations(flags).unwrap();
    let mut fixture = fixture(reporter).await;
    let held = (0..63)
        .map(|_| {
            fixture
                .server
                .notification_transactions
                .try_admit_audit()
                .unwrap()
        })
        .collect::<Vec<_>>();
    fixture.transport.block.store(true, Ordering::Release);
    for _ in 0..2 {
        assert!(matches!(
            write_value(&fixture.server, None).await,
            Apdu::SimpleAck(_)
        ));
    }
    settle().await;
    assert_eq!(
        fixture.server.notification_transactions.audit_resources(),
        (true, 1, 0)
    );
    let mut db = fixture.server.db.write().await;
    drop(held);
    for next in [21, 20] {
        db.get_mut(&oid(ObjectType::DEVICE, 10))
            .unwrap()
            .write_property(
                PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
                None,
                PropertyValue::ApplicationData(value(&device(next))),
                None,
            )
            .unwrap();
    }
    assert_eq!(
        fixture.server.notification_transactions.audit_resources().1,
        0,
        "retired count is gone while the old worker still owns its lease"
    );
    drop(db);
    fixture.transport.block.store(false, Ordering::Release);
    fixture.transport.unblock.notify_waiters();
    settle().await;
    let records = notifications(&fixture.transport.sent);
    assert_eq!(records.len(), 5);
    assert!(records
        .iter()
        .all(|request| request.notifications[0].operation == AuditOperation::WRITE));
    assert_eq!(
        fixture.server.notification_transactions.audit_resources(),
        (false, 0, 64)
    );
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn recipient_no_runtime_and_closed_admission_fail_before_commit() {
    let mut fixture = fixture(reporter()).await;
    let db = Arc::clone(&fixture.server.db);
    let result = std::thread::spawn(move || {
        db.blocking_write()
            .get_mut(&oid(ObjectType::DEVICE, 10))
            .unwrap()
            .write_property(
                PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
                None,
                PropertyValue::ApplicationData(value(&device(21))),
                None,
            )
    })
    .join()
    .unwrap();
    assert!(result.is_err());
    fixture.server.notification_transactions.close();
    assert!(fixture
        .server
        .write_local(
            &oid(ObjectType::DEVICE, 10),
            PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
            None,
            PropertyValue::ApplicationData(value(&device(21))),
            None
        )
        .await
        .is_err());
    assert_eq!(
        read_recipient(&fixture).await,
        PropertyValue::ApplicationData(value(&device(20)))
    );
    assert_eq!(
        fixture
            .server
            .db
            .read()
            .await
            .reserve_event_sequence_number()
            .number(),
        0
    );
    assert!(fixture.transport.sent.lock().unwrap().is_empty());
    fixture.server.stop().await.unwrap();
}

#[tokio::test]
async fn recipient_configured_routed_old_route_is_preserved_and_observed_new_is_denied() {
    let mut fixture = try_server(
        reporter(),
        &[10],
        Some(device(20)),
        vec![
            DeviceBinding::routed(oid(ObjectType::DEVICE, 20), 200, [9], LOGGER).unwrap(),
            DeviceBinding::local(oid(ObjectType::DEVICE, 21), NEW_LOGGER).unwrap(),
        ],
    )
    .await
    .unwrap();
    fixture
        .server
        .device_bindings
        .write()
        .await
        .observe_i_am_at(
            oid(ObjectType::DEVICE, 22),
            LOGGER,
            None,
            Instant::now(),
            |_| false,
        );
    assert!(matches!(
        change(&fixture, &device(22)).await,
        Apdu::Error(_)
    ));
    assert_eq!(
        fixture
            .server
            .db
            .read()
            .await
            .reserve_event_sequence_number()
            .number(),
        0
    );
    assert!(matches!(
        change(&fixture, &device(21)).await,
        Apdu::SimpleAck(_)
    ));
    settle().await;
    let sent = fixture.transport.sent.lock().unwrap().clone();
    assert_eq!(
        decode_npdu(sent[0].clone()).unwrap().destination,
        Some(NpduAddress {
            network: 200,
            mac_address: MacAddr::from_slice(&[9])
        })
    );
    assert_eq!(decode_npdu(sent[1].clone()).unwrap().destination, None);
    fixture.server.stop().await.unwrap();
}
