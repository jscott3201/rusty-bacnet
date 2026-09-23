use super::*;
use bacnet_services::audit::AuditNotificationRequest;

fn enabled(confirmed: bool) -> bacnet_objects::audit::AuditReporterObject {
    let mut reporter = reporter();
    let mut flags = AuditOperationFlags::empty();
    flags.insert(AuditOperation::WRITE);
    flags.insert(AuditOperation::AUDITING_FAILURE);
    reporter.set_auditable_operations(flags);
    reporter.set_issue_confirmed_notifications(confirmed);
    reporter
}

fn records(fixture: &Fixture) -> Vec<BACnetAuditNotification> {
    fixture
        .transport
        .sent
        .lock()
        .unwrap()
        .iter()
        .map(|bytes| {
            let apdu = decode_apdu(decode_npdu(bytes.clone()).unwrap().payload).unwrap();
            let service = match apdu {
                Apdu::ConfirmedRequest(request) => {
                    assert!(!request.segmented);
                    assert_eq!(
                        request.service_choice,
                        ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION
                    );
                    request.service_request
                }
                Apdu::UnconfirmedRequest(request) => {
                    assert_eq!(
                        request.service_choice,
                        UnconfirmedServiceChoice::UNCONFIRMED_AUDIT_NOTIFICATION
                    );
                    request.service_request
                }
                other => panic!("unexpected {other:?}"),
            };
            let mut request = AuditNotificationRequest::decode(&service).unwrap();
            assert_eq!(request.notifications.len(), 1);
            request.notifications.remove(0)
        })
        .collect()
}

fn expected(count: u8, timestamp: u16) -> BACnetAuditNotification {
    BACnetAuditNotification {
        source_timestamp: None,
        target_timestamp: Some(BACnetTimeStamp::SequenceNumber(timestamp)),
        source_device: BACnetRecipient::Device(oid(ObjectType::DEVICE, 10)),
        source_object: None,
        operation: AuditOperation::AUDITING_FAILURE,
        source_comment: None,
        target_comment: None,
        invoke_id: None,
        source_user_id: None,
        source_user_role: None,
        target_device: BACnetRecipient::Device(oid(ObjectType::DEVICE, 10)),
        target_object: None,
        target_property: None,
        target_priority: None,
        target_value: None,
        // Independent application-tagged Unsigned vector (one-octet values).
        current_value: Some(vec![0x21, count]),
        result: None,
    }
}

async fn writes(fixture: &Fixture, count: usize) {
    assert!(matches!(
        dispatch(
            &fixture.server,
            ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
            wpm((0..count)
                .map(|_| element(PropertyIdentifier::PRESENT_VALUE, vec![0x91, 1]))
                .collect())
        )
        .await,
        Apdu::SimpleAck(_)
    ));
    settle().await;
}

fn ack(fixture: &Fixture, index: usize) {
    let request = confirmed_notification(&fixture.transport.sent, index);
    assert!(fixture.server.notification_transactions.admit_terminal(
        LOGGER,
        None,
        &Apdu::SimpleAck(SimpleAck {
            invoke_id: request.invoke_id,
            service_choice: request.service_choice
        })
    ));
}

async fn stop(fixture: &mut Fixture) {
    tokio::time::timeout(Duration::from_secs(1), fixture.server.stop())
        .await
        .unwrap()
        .unwrap();
    assert!(fixture.server.notification_transactions.workers_empty());
    assert_eq!(fixture.server.notification_transactions.active_count(), 0);
    assert_eq!(
        fixture.server.notification_transactions.audit_resources(),
        (false, 0, 64)
    );
}

#[tokio::test(start_paused = true)]
async fn audit_reporter_auditing_failure_unconfirmed_coalesces_64_active_drops() {
    let mut fixture = server(enabled(false)).await;
    fixture.transport.block.store(true, Ordering::Release);
    writes(&fixture, 68).await;
    writes(&fixture, 3).await;
    assert_eq!(fixture.writes.load(Ordering::Acquire), 71);
    assert_eq!(records(&fixture).len(), 64);
    assert_eq!(
        fixture.server.notification_transactions.audit_resources(),
        (true, 7, 0)
    );
    fixture.transport.block.store(false, Ordering::Release);
    fixture.transport.unblock.notify_one();
    settle().await;
    let records = records(&fixture);
    assert_eq!(records.len(), 65);
    assert_eq!(records[64], expected(7, 64));
    assert!(records[..64]
        .iter()
        .all(|record| record.operation == AuditOperation::WRITE && record.result.is_none()));
    assert_eq!(
        fixture.server.notification_transactions.audit_resources(),
        (false, 0, 1)
    );
    stop(&mut fixture).await;
}

#[tokio::test(start_paused = true)]
async fn audit_reporter_auditing_failure_confirmed_batches_are_sequential_and_bounded() {
    let mut fixture = server(enabled(true)).await;
    writes(&fixture, 68).await;
    ack(&fixture, 0);
    settle().await;
    assert_eq!(records(&fixture).len(), 65);
    assert_eq!(records(&fixture)[64], expected(4, 64));
    assert_eq!(fixture.server.notification_transactions.active_count(), 64);
    writes(&fixture, 5).await;
    assert_eq!(records(&fixture).len(), 65, "no second concurrent summary");
    assert_eq!(
        fixture.server.notification_transactions.audit_resources(),
        (true, 5, 0)
    );
    ack(&fixture, 64);
    settle().await;
    assert_eq!(records(&fixture).len(), 66);
    assert_eq!(records(&fixture)[65], expected(5, 68));
    ack(&fixture, 65);
    settle().await;
    assert_eq!(
        fixture.server.notification_transactions.audit_resources(),
        (false, 0, 1)
    );
    assert_eq!(fixture.writes.load(Ordering::Acquire), 73);
    stop(&mut fixture).await;
}

#[tokio::test(start_paused = true)]
async fn audit_reporter_auditing_failure_coordinator_exhaustion_flushes_on_release() {
    let mut fixture = server(enabled(true)).await;
    let mut held: Vec<_> = (0..256)
        .map(|_| {
            fixture
                .server
                .notification_transactions
                .reserve(
                    canonical_direct_peer(LOGGER),
                    ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
                )
                .unwrap()
        })
        .collect();
    writes(&fixture, 4).await;
    assert!(records(&fixture).is_empty());
    assert_eq!(
        fixture.server.notification_transactions.audit_resources(),
        (true, 4, 63)
    );
    // Capacity wait is not the delivery deadline and must not discard/retry.
    tokio::time::advance(Duration::from_secs(30)).await;
    settle().await;
    assert!(records(&fixture).is_empty());
    assert_eq!(
        fixture.server.notification_transactions.audit_resources(),
        (true, 4, 63)
    );
    drop(held.pop());
    settle().await;
    assert_eq!(records(&fixture), vec![expected(4, 0)]);
    assert_eq!(fixture.server.notification_transactions.active_count(), 256);
    ack(&fixture, 0);
    settle().await;
    drop(held);
    stop(&mut fixture).await;
}

#[tokio::test(start_paused = true)]
async fn audit_reporter_auditing_failure_filters_ignore_selection_and_priority() {
    for (level, bit, summary) in [
        (AuditLevel::NONE, true, false),
        (AuditLevel::AUDIT_ALL, false, false),
        (AuditLevel::AUDIT_CONFIG, true, true),
    ] {
        let mut reporter = enabled(false);
        reporter.set_audit_level(level).unwrap();
        if !bit {
            reporter.set_auditable_operations(AuditOperationFlags::empty());
        }
        reporter.set_monitored_objects(Some(vec![]));
        reporter.set_audit_priority_filter(BACnetPriorityFilter::empty());
        let mut fixture = server(reporter).await;
        let mut permits: Vec<_> = (0..64)
            .map(|_| {
                fixture
                    .server
                    .notification_transactions
                    .try_admit_audit()
                    .unwrap()
            })
            .collect();
        // Reporter Description bypasses ordinary selection/WRITE/priority.
        assert!(matches!(
            dispatch(
                &fixture.server,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                wp(
                    oid(ObjectType::AUDIT_REPORTER, 1),
                    PropertyIdentifier::DESCRIPTION,
                    vec![0x72, 0, b'x'],
                    Some(3)
                )
            )
            .await,
            Apdu::SimpleAck(_)
        ));
        settle().await;
        assert_eq!(
            fixture.server.notification_transactions.audit_resources().1,
            u64::from(summary)
        );
        drop(permits.pop());
        settle().await;
        assert_eq!(
            records(&fixture),
            if summary {
                vec![expected(1, 0)]
            } else {
                vec![]
            }
        );
        drop(permits);
        stop(&mut fixture).await;
    }
}

#[tokio::test(start_paused = true)]
async fn audit_reporter_auditing_failure_delivery_failures_never_recurse() {
    for mode in [0, 1, 2] {
        let mut fixture = server(enabled(mode != 0)).await;
        let permits: Vec<_> = (0..64)
            .map(|_| {
                fixture
                    .server
                    .notification_transactions
                    .try_admit_audit()
                    .unwrap()
            })
            .collect();
        writes(&fixture, 2).await;
        fixture.transport.fail.store(mode == 0, Ordering::Release);
        drop(permits);
        settle().await;
        assert_eq!(records(&fixture), vec![expected(2, 0)]);
        if mode == 1 {
            let request = confirmed_notification(&fixture.transport.sent, 0);
            assert!(fixture.server.notification_transactions.admit_terminal(
                LOGGER,
                None,
                &Apdu::Reject(RejectPdu {
                    invoke_id: request.invoke_id,
                    reject_reason: RejectReason::OTHER
                })
            ));
        }
        tokio::time::advance(Duration::from_secs(30)).await;
        settle().await;
        assert_eq!(records(&fixture).len(), 1);
        assert_eq!(
            health(&fixture.server).await,
            Reliability::COMMUNICATION_FAILURE
        );
        assert_eq!(
            fixture.server.notification_transactions.audit_resources(),
            (false, 0, 64)
        );
        stop(&mut fixture).await;
    }
}

#[tokio::test(start_paused = true)]
async fn audit_reporter_auditing_failure_stop_cancels_waiters_and_inflight() {
    for capacity in [0, 1, 2] {
        let mut fixture = server(enabled(true)).await;
        let permits: Vec<_> = if capacity == 0 {
            (0..64)
                .map(|_| {
                    fixture
                        .server
                        .notification_transactions
                        .try_admit_audit()
                        .unwrap()
                })
                .collect()
        } else {
            vec![]
        };
        let mut held: Vec<_> = if capacity != 0 {
            (0..256)
                .map(|_| {
                    fixture
                        .server
                        .notification_transactions
                        .reserve(
                            canonical_direct_peer(LOGGER),
                            ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
                        )
                        .unwrap()
                })
                .collect()
        } else {
            vec![]
        };
        writes(&fixture, 2).await;
        if capacity == 2 {
            drop(held.pop());
            settle().await;
        }
        assert_eq!(records(&fixture).len(), usize::from(capacity == 2));
        assert!(fixture.server.notification_transactions.audit_resources().0);
        // Join the waiting/sending summary before releasing test-held resources.
        tokio::time::timeout(Duration::from_secs(1), fixture.server.stop())
            .await
            .unwrap()
            .unwrap();
        drop(permits);
        drop(held);
        assert!(fixture.server.notification_transactions.workers_empty());
        assert_eq!(fixture.server.notification_transactions.active_count(), 0);
        assert_eq!(
            fixture.server.notification_transactions.audit_resources(),
            (false, 0, 64)
        );
    }
}

#[tokio::test(start_paused = true)]
async fn audit_reporter_auditing_failure_late_completion_does_not_update_replacement() {
    let mut fixture = server(enabled(true)).await;
    let permits: Vec<_> = (0..64)
        .map(|_| {
            fixture
                .server
                .notification_transactions
                .try_admit_audit()
                .unwrap()
        })
        .collect();
    writes(&fixture, 2).await;
    drop(permits);
    settle().await;
    let replacement = enabled(true);
    let status = replacement.status_internal();
    status.set_configured(true);
    status.complete_delivery(status.begin_delivery(), false);
    {
        let mut db = fixture.server.db.write().await;
        db.remove(&oid(ObjectType::AUDIT_REPORTER, 1)).unwrap();
        db.add(Box::new(replacement)).unwrap();
    }
    ack(&fixture, 0);
    settle().await;
    assert_eq!(
        health(&fixture.server).await,
        Reliability::COMMUNICATION_FAILURE
    );
    assert_eq!(records(&fixture), vec![expected(2, 0)]);
    stop(&mut fixture).await;
}

#[tokio::test(start_paused = true)]
async fn audit_reporter_auditing_failure_excludes_nonresource_failures_under_load() {
    let mut fixture = server(enabled(false)).await;
    let permits: Vec<_> = (0..64)
        .map(|_| {
            fixture
                .server
                .notification_transactions
                .try_admit_audit()
                .unwrap()
        })
        .collect();
    fixture.server.config.max_apdu_length = 1;
    writes(&fixture, 2).await;
    fixture.server.config.max_apdu_length = 1476;
    fixture.server.comm_state.store(2, Ordering::Release);
    writes(&fixture, 2).await;
    fixture.server.comm_state.store(0, Ordering::Release);
    fixture.server.config.mutation_policy = crate::mutation::MutationPolicy::DenyAll;
    assert!(matches!(
        write_value(&fixture.server, None).await,
        Apdu::Error(_)
    ));
    fixture.server.config.mutation_policy = crate::mutation::MutationPolicy::Permissive;
    dispatch(
        &fixture.server,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        Bytes::new(),
    )
    .await;
    *fixture.execution_error.lock().unwrap() = Some(Error::Timeout(Duration::from_secs(1)));
    write_value(&fixture.server, None).await;
    drop(permits);
    settle().await;
    assert!(records(&fixture).is_empty());
    assert_eq!(
        fixture.server.notification_transactions.audit_resources(),
        (false, 0, 64)
    );
    // A normal send failure also does not become a dropped-record summary.
    fixture.transport.fail.store(true, Ordering::Release);
    writes(&fixture, 1).await;
    assert_eq!(records(&fixture).len(), 1);
    assert_eq!(records(&fixture)[0].operation, AuditOperation::WRITE);
    assert_eq!(
        fixture.server.notification_transactions.audit_resources(),
        (false, 0, 64)
    );
    stop(&mut fixture).await;
}

#[tokio::test(start_paused = true)]
async fn audit_reporter_auditing_failure_replacement_supersedes_old_context_without_transferring_drops(
) {
    let mut fixture = server(enabled(true)).await;
    let permits: Vec<_> = (0..64)
        .map(|_| {
            fixture
                .server
                .notification_transactions
                .try_admit_audit()
                .unwrap()
        })
        .collect();
    writes(&fixture, 2).await;
    {
        let mut db = fixture.server.db.write().await;
        db.remove(&oid(ObjectType::AUDIT_REPORTER, 1)).unwrap();
        db.add(Box::new(enabled(true))).unwrap();
    }
    writes(&fixture, 3).await;
    assert_eq!(
        fixture.server.notification_transactions.audit_resources(),
        (true, 3, 0)
    );
    drop(permits);
    settle().await;
    assert_eq!(records(&fixture), vec![expected(3, 2)]);
    ack(&fixture, 0);
    settle().await;
    assert_eq!(
        health(&fixture.server).await,
        Reliability::NO_FAULT_DETECTED,
        "only the replacement context summary owns its health completion"
    );
    stop(&mut fixture).await;
}

#[tokio::test(start_paused = true)]
async fn audit_reporter_auditing_failure_counts_execution_error_but_not_normal_ack_timeout() {
    let mut fixture = server(enabled(true)).await;
    writes(&fixture, 1).await;
    tokio::time::advance(Duration::from_secs(3)).await;
    settle().await;
    assert_eq!(
        records(&fixture).len(),
        1,
        "normal ACK timeout is not a resource drop"
    );
    assert_eq!(
        fixture.server.notification_transactions.audit_resources(),
        (false, 0, 64)
    );
    let permits: Vec<_> = (0..64)
        .map(|_| {
            fixture
                .server
                .notification_transactions
                .try_admit_audit()
                .unwrap()
        })
        .collect();
    *fixture.execution_error.lock().unwrap() = Some(Error::Protocol {
        class: ErrorClass::PROPERTY.to_raw() as u32,
        code: ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
    });
    assert!(matches!(
        write_value(&fixture.server, None).await,
        Apdu::Error(_)
    ));
    settle().await;
    assert_eq!(fixture.writes.load(Ordering::Acquire), 1);
    assert_eq!(
        fixture.server.notification_transactions.audit_resources(),
        (true, 1, 0)
    );
    drop(permits);
    settle().await;
    assert_eq!(records(&fixture)[1], expected(1, 1));
    fixture.server.comm_state.store(2, Ordering::Release);
    stop(&mut fixture).await;
}
