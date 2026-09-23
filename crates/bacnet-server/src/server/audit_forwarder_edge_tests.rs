use super::*;

#[tokio::test(start_paused = true)]
async fn audit_forwarding_expired_before_first_poll_never_sends() {
    let mut f = ready().await;
    f.unconfirmed(payload(false)).await;
    // Advance the clock before the spawned delivery gets its first poll.
    tokio::time::advance(Duration::from_secs(3)).await;
    settle().await;
    assert!(f.requests().is_empty());
    assert_eq!(f.server.notification_transactions.active_count(), 0);
    assert_eq!(
        f.server.notification_transactions.audit_resources(),
        (false, 0, 64)
    );
    assert_eq!(
        f.reliability().await,
        PropertyValue::Enumerated(Reliability::COMMUNICATION_FAILURE.to_raw())
    );
    f.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn audit_forwarding_routed_ack_uses_final_peer_not_router() {
    let mut f = fixture(
        Some(parent()),
        Some(DeviceBinding::routed(oid(ObjectType::DEVICE, 20), 200, [9], [2]).unwrap()),
    )
    .await;
    f.unconfirmed(payload(false)).await;
    settle().await;
    let wire = f.wire.sent.lock().unwrap()[0].clone();
    let npdu = decode_npdu(wire).unwrap();
    assert_eq!(
        npdu.destination,
        Some(NpduAddress {
            network: 200,
            mac_address: MacAddr::from_slice(&[9])
        })
    );
    let req = f.requests().remove(0);
    assert!(!f.ack(req.invoke_id, &[2], req.service_choice));
    assert!(f.server.notification_transactions.admit_terminal(
        &[2],
        Some(&NpduAddress {
            network: 200,
            mac_address: MacAddr::from_slice(&[9])
        }),
        &Apdu::SimpleAck(SimpleAck {
            invoke_id: req.invoke_id,
            service_choice: req.service_choice
        })
    ));
    settle().await;
    assert_eq!(f.server.notification_transactions.active_count(), 0);
    f.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn audit_forwarding_observed_binding_and_local_alias_do_not_send() {
    let mut f = fixture(Some(parent()), None).await;
    f.server.device_bindings.write().await.observe_i_am_at(
        oid(ObjectType::DEVICE, 20),
        &[2],
        None,
        Instant::now(),
        |_| false,
    );
    f.unconfirmed(payload(false)).await;
    settle().await;
    assert!(f.requests().is_empty());
    assert_eq!(
        f.reliability().await,
        PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw())
    );
    f.server.stop().await.unwrap();
    let mut f = fixture(
        Some(parent()),
        Some(DeviceBinding::local(oid(ObjectType::DEVICE, 20), [1]).unwrap()),
    )
    .await;
    assert_eq!(
        f.reliability().await,
        PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw())
    );
    f.unconfirmed(payload(false)).await;
    settle().await;
    assert!(f.requests().is_empty());
    f.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn audit_forwarding_denial_disabled_and_malformed_batches_are_silent() {
    let mut f = ready().await;
    f.server.config.audit_notification_authorizer = Some(Arc::new(|_| false));
    f.server.config.unconfirmed_audit_notification_authorizer = Some(Arc::new(|_| false));
    assert!(matches!(
        f.confirmed(1, &[3], payload(false)).await,
        Some(Apdu::Error(_))
    ));
    f.unconfirmed(payload(false)).await;
    f.server.config.audit_notification_authorizer = Some(Arc::new(|_| true));
    f.server.config.unconfirmed_audit_notification_authorizer = Some(Arc::new(|_| true));
    let mut malformed = payload(false).to_vec();
    malformed.pop();
    assert!(matches!(
        f.confirmed(2, &[3], malformed.clone().into()).await,
        Some(Apdu::Error(_))
    ));
    f.unconfirmed(malformed.into()).await;
    f.server
        .db
        .write()
        .await
        .get_mut(&oid(ObjectType::AUDIT_LOG, 7))
        .unwrap()
        .write_property(
            PropertyIdentifier::LOG_ENABLE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();
    let before = f.store.snapshot.lock().unwrap().clone();
    assert!(matches!(
        f.confirmed(3, &[3], payload(false)).await,
        Some(Apdu::Error(_))
    ));
    f.unconfirmed(payload(false)).await;
    settle().await;
    assert!(f.requests().is_empty());
    assert_eq!(*f.store.snapshot.lock().unwrap(), before);
    f.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn audit_forwarding_dcc_oversize_and_send_errors_leave_local_success() {
    let mut f = ready().await;
    f.server.comm_state.store(2, Ordering::Release); // DISABLE_INITIATION
    assert!(matches!(
        f.confirmed(1, &[3], payload(false)).await,
        Some(Apdu::SimpleAck(_))
    ));
    settle().await;
    assert!(f.requests().is_empty());
    f.server.comm_state.store(0, Ordering::Release);
    f.server.config.max_apdu_length = 50;
    assert!(matches!(
        f.confirmed(2, &[3], payload(false)).await,
        Some(Apdu::SimpleAck(_))
    ));
    settle().await;
    assert!(
        f.requests().is_empty(),
        "oversized forward is not segmented"
    );
    f.server.config.max_apdu_length = 1476;
    f.wire.fail.store(true, Ordering::Release);
    assert!(matches!(
        f.confirmed(3, &[3], payload(false)).await,
        Some(Apdu::SimpleAck(_))
    ));
    settle().await;
    tokio::time::advance(Duration::from_secs(3)).await;
    settle().await;
    assert_eq!(f.requests().len(), 1);
    assert_eq!(
        f.reliability().await,
        PropertyValue::Enumerated(Reliability::COMMUNICATION_FAILURE.to_raw())
    );
    assert_eq!(
        f.store
            .snapshot
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .completed_receipts
            .len(),
        3
    );
    assert_eq!(f.server.notification_transactions.active_count(), 0);
    f.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn audit_forwarding_terminal_failures_and_late_success_cannot_hide_new_failure() {
    let mut f = ready().await;
    f.unconfirmed(payload(false)).await;
    f.unconfirmed(payload(false)).await;
    settle().await;
    let requests = f.requests();
    let failure = Apdu::Reject(RejectPdu {
        invoke_id: requests[1].invoke_id,
        reject_reason: RejectReason::OTHER,
    });
    assert!(f
        .server
        .notification_transactions
        .admit_terminal(&[2], None, &failure));
    settle().await;
    assert!(f.ack(requests[0].invoke_id, &[2], requests[0].service_choice));
    settle().await;
    assert_eq!(
        f.reliability().await,
        PropertyValue::Enumerated(Reliability::COMMUNICATION_FAILURE.to_raw())
    );
    for abort in [false, true] {
        f.unconfirmed(payload(false)).await;
        settle().await;
        let req = f.requests().pop().unwrap();
        let terminal = if abort {
            // Preserve the existing Notification coordinator direction policy.
            Apdu::Abort(AbortPdu {
                invoke_id: req.invoke_id,
                sent_by_server: false,
                abort_reason: AbortReason::OTHER,
            })
        } else {
            Apdu::Error(ErrorPdu {
                invoke_id: req.invoke_id,
                service_choice: req.service_choice,
                error_class: ErrorClass::SERVICES,
                error_code: ErrorCode::SERVICE_REQUEST_DENIED,
                error_data: Bytes::new(),
            })
        };
        assert!(f
            .server
            .notification_transactions
            .admit_terminal(&[2], None, &terminal));
        settle().await;
        assert_eq!(f.server.notification_transactions.active_count(), 0);
    }
    assert_eq!(f.requests().len(), 4);
    f.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn audit_forwarding_reopen_keeps_receipt_but_never_replays_forwarding_progress() {
    let mut f = ready().await;
    f.wire.block.store(true, Ordering::Release);
    let data = payload(false);
    assert!(matches!(
        f.confirmed(201, &[3], data.clone()).await,
        Some(Apdu::SimpleAck(_))
    ));
    settle().await;
    f.server.stop().await.unwrap();
    let snapshot = f.store.snapshot.lock().unwrap().clone();
    let mut reopened = fixture_with(
        10,
        Some(parent()),
        Some(DeviceBinding::local(oid(ObjectType::DEVICE, 20), [2]).unwrap()),
        f.store.clone(),
    )
    .await;
    settle().await;
    assert!(reopened.requests().is_empty());
    assert!(reopened.confirmed(201, &[3], data).await.is_none());
    assert_eq!(*reopened.store.snapshot.lock().unwrap(), snapshot);
    assert!(reopened.requests().is_empty());
    reopened.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn audit_forwarding_isolated_cycle_is_content_bounded_not_global_deduplication() {
    let mut a = ready().await;
    let mut b_parent = parent();
    b_parent.device_identifier = Some(oid(ObjectType::DEVICE, 10));
    let mut b = fixture_with(
        20,
        Some(b_parent),
        Some(DeviceBinding::local(oid(ObjectType::DEVICE, 10), [2]).unwrap()),
        Arc::new(MemoryPersistence::default()),
    )
    .await;
    a.unconfirmed(payload(true)).await;
    settle().await;
    let outgoing = a.requests()[0].clone();
    assert!(matches!(
        b.confirmed(outgoing.invoke_id, &[2], outgoing.service_request.clone())
            .await,
        Some(Apdu::SimpleAck(_))
    ));
    assert!(a.ack(outgoing.invoke_id, &[2], outgoing.service_choice));
    settle().await;
    let returning = b.requests()[0].clone();
    assert!(matches!(
        a.confirmed(returning.invoke_id, &[2], returning.service_request)
            .await,
        Some(Apdu::SimpleAck(_))
    ));
    assert!(b.ack(returning.invoke_id, &[2], returning.service_choice));
    settle().await;
    assert_eq!(a.requests().len(), 1);
    assert_eq!(b.requests().len(), 1);
    assert_eq!(a.server.notification_transactions.active_count(), 0);
    assert_eq!(b.server.notification_transactions.active_count(), 0);
    // Single-actor content is not a complementary match. There is deliberately
    // no cross-peer/hop dedupe claim: bounded overload/deadline tests cover that
    // case, and applications must configure an acyclic hierarchy.
    a.server.stop().await.unwrap();
    b.server.stop().await.unwrap();
}
