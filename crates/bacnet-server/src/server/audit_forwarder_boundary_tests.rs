use super::*;
use bacnet_objects::audit::AuditLogSnapshot;
use bacnet_types::constructed::{BACnetAuditLogDatum, BACnetAuditNotification};
use bacnet_types::primitives::{BACnetTimeStamp, Time};

async fn ready_capacity(capacity: u32) -> Fixture {
    fixture_with_capacity(
        10,
        Some(parent()),
        Some(DeviceBinding::local(oid(ObjectType::DEVICE, 20), [2]).unwrap()),
        Arc::new(MemoryPersistence::default()),
        capacity,
    )
    .await
}

fn snapshot(f: &Fixture) -> AuditLogSnapshot {
    f.store.snapshot.lock().unwrap().clone().unwrap()
}

fn stored(snapshot: &AuditLogSnapshot, index: usize) -> &BACnetAuditNotification {
    let BACnetAuditLogDatum::AuditNotification(item) = &snapshot.records[index].record.datum else {
        panic!("expected notification")
    };
    item
}

async fn assert_live(f: &Fixture, expected: &AuditLogSnapshot) {
    let db = f.server.db.read().await;
    let log = db.get(&oid(ObjectType::AUDIT_LOG, 7)).unwrap();
    assert_eq!(
        log.read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
            .unwrap(),
        PropertyValue::Unsigned(expected.total_record_count)
    );
    assert_eq!(
        log.read_property(PropertyIdentifier::RECORD_COUNT, None)
            .unwrap(),
        PropertyValue::Unsigned(expected.records.len() as u64)
    );
    let query = bacnet_types::constructed::BACnetAuditLogQueryParameters::BySource {
        source_device_identifier: oid(ObjectType::DEVICE, 999),
        source_device_address: None,
        source_object_identifier: None,
        operations: None,
        successful_actions_only: bacnet_types::enums::BACnetSuccessFilter::ALL,
    };
    let page = log
        .audit_log_storage_internal()
        .unwrap()
        .query(&query, None, 100);
    assert_eq!(
        page.records,
        expected.records.iter().rev().cloned().collect::<Vec<_>>()
    );
    assert!(page.no_more_items);
}

async fn ack_last(f: &Fixture) {
    settle().await;
    let req = f.requests().pop().unwrap();
    assert!(f.ack(req.invoke_id, &[2], req.service_choice));
    settle().await;
}

#[tokio::test(start_paused = true)]
async fn audit_forwarding_full_ring_evicts_atomically_without_delivery_rollback() {
    for capacity in [1, 2] {
        let mut f = ready_capacity(capacity).await;
        for invoke in 1..=capacity as u8 {
            assert!(matches!(
                f.confirmed(invoke, &[3], payload(false)).await,
                Some(Apdu::SimpleAck(_))
            ));
            ack_last(&f).await;
        }
        // Each accepted batch changes a full ring. Success, send failure and
        // missing ACK must all leave the same durable/live survivors intact.
        for (index, outcome) in ["ack", "send-error", "timeout"].into_iter().enumerate() {
            let before = snapshot(&f);
            let commits = f.store.commits.load(Ordering::Acquire);
            let sends = f.requests().len();
            f.wire
                .fail
                .store(outcome == "send-error", Ordering::Release);
            let mut item = notification(AuditOperation::GENERAL);
            item.invoke_id = Some(50 + index as u8);
            let data = request_bytes(vec![item.clone()]);
            let invoke = 201 + index as u8;
            assert!(matches!(
                f.confirmed(invoke, &[3], data.clone()).await,
                Some(Apdu::SimpleAck(_))
            ));
            let accepted = snapshot(&f);
            assert_eq!(f.store.commits.load(Ordering::Acquire), commits + 1);
            assert_eq!(accepted.generation, before.generation + 1);
            assert_eq!(accepted.total_record_count, before.total_record_count + 1);
            assert_eq!(accepted.records.len(), capacity as usize);
            assert_eq!(
                &accepted.records[..capacity as usize - 1],
                &before.records[1..]
            );
            assert_eq!(
                accepted.records.last().unwrap().sequence_number,
                accepted.total_record_count
            );
            assert_eq!(stored(&accepted, capacity as usize - 1), &item);
            assert_eq!(
                accepted.completed_receipts.len(),
                before.completed_receipts.len() + 1
            );
            settle().await;
            let requests = f.requests();
            assert_eq!(requests.len(), sends + 1);
            assert_eq!(requests.last().unwrap().service_request, data);
            assert_ne!(requests.last().unwrap().invoke_id, invoke);
            if outcome == "ack" {
                ack_last(&f).await;
            } else {
                tokio::time::advance(Duration::from_secs(3)).await;
                settle().await;
                assert_eq!(
                    f.reliability().await,
                    PropertyValue::Enumerated(Reliability::COMMUNICATION_FAILURE.to_raw())
                );
            }
            assert_eq!(snapshot(&f), accepted);
            assert_live(&f, &accepted).await;
            assert!(f.confirmed(invoke, &[3], data).await.is_none());
            assert_eq!(f.requests().len(), sends + 1);
            assert_eq!(f.server.notification_transactions.active_count(), 0);
        }
        let before = snapshot(&f);
        let sends = f.requests().len();
        f.store.fail.store(true, Ordering::Release);
        let data = request_bytes(vec![notification(AuditOperation::READ)]);
        let Some(Apdu::Error(error)) = f.confirmed(210, &[3], data.clone()).await else {
            panic!("storage unavailable must not be mistaken for normal ring eviction")
        };
        // The injected backend returns Error::Transport; preserve the server's
        // existing non-protocol error mapping rather than inventing a new one.
        assert_eq!(error.error_class, ErrorClass::SERVICES);
        assert_eq!(error.error_code, ErrorCode::OTHER);
        settle().await;
        assert_eq!(snapshot(&f), before);
        assert_live(&f, &before).await;
        assert_eq!(f.requests().len(), sends);
        f.store.fail.store(false, Ordering::Release);
        f.wire.fail.store(false, Ordering::Release);
        assert!(f.confirmed(210, &[3], data.clone()).await.is_none());
        // The existing server-lifetime guard also remembers error responses.
        // Isolate durable receipt behavior using a fresh guard, as after restart.
        f.server.confirmed_request_tracker = Arc::new(ConfirmedRequestTracker::default());
        assert!(
            matches!(f.confirmed(210, &[3], data).await, Some(Apdu::SimpleAck(_))),
            "failed commit retained no receipt"
        );
        ack_last(&f).await;
        assert_eq!(f.requests().len(), sends + 1);
        f.server.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn audit_forwarding_partial_batch_is_one_commit_and_one_complete_wire_request() {
    let mut f = ready_capacity(2).await;
    let source = notification(AuditOperation::WRITE);
    f.unconfirmed(request_bytes(vec![source.clone()])).await;
    ack_last(&f).await;
    let before = snapshot(&f);
    let mut target = source.clone();
    target.target_timestamp = target.source_timestamp.take();
    target.current_value = Some(vec![0x21, 42]);
    let mut other = notification(AuditOperation::READ);
    other.target_timestamp = other.source_timestamp.clone();
    let data = request_bytes(vec![target.clone(), other.clone()]);
    let commits = f.store.commits.load(Ordering::Acquire);

    f.store.fail.store(true, Ordering::Release);
    let Some(Apdu::Error(error)) = f.confirmed(201, &[3], data.clone()).await else {
        panic!("expected storage error")
    };
    assert_eq!(
        (error.error_class, error.error_code),
        (ErrorClass::SERVICES, ErrorCode::OTHER)
    );
    settle().await;
    assert_eq!(snapshot(&f), before);
    assert_live(&f, &before).await;
    assert_eq!(f.requests().len(), 1);
    assert_eq!(f.store.commits.load(Ordering::Acquire), commits);

    f.store.fail.store(false, Ordering::Release);
    assert!(f.confirmed(201, &[3], data.clone()).await.is_none());
    // Preserve transient error-request suppression; a fresh runtime guard
    // demonstrates that the failed batch wrote no durable receipt.
    f.server.confirmed_request_tracker = Arc::new(ConfirmedRequestTracker::default());
    assert!(matches!(
        f.confirmed(201, &[3], data.clone()).await,
        Some(Apdu::SimpleAck(_))
    ));
    let accepted = snapshot(&f);
    assert_eq!(accepted.generation, before.generation + 1);
    assert_eq!(f.store.commits.load(Ordering::Acquire), commits + 1);
    assert_eq!(accepted.total_record_count, 2);
    assert_eq!(accepted.records.len(), 2);
    assert_eq!(accepted.records[0].sequence_number, 1);
    assert_eq!(
        accepted.records[0].record.timestamp,
        before.records[0].record.timestamp
    );
    let mut merged = source;
    merged.target_timestamp = target.target_timestamp;
    merged.current_value = target.current_value;
    assert_eq!(stored(&accepted, 0), &merged);
    assert_eq!(accepted.records[1].sequence_number, 2);
    assert_eq!(stored(&accepted, 1), &other);
    assert_eq!(accepted.completed_receipts.len(), 1);
    ack_last(&f).await;
    assert_eq!(f.requests().len(), 2);
    assert_eq!(f.requests()[1].service_request, data);
    assert_ne!(f.requests()[1].invoke_id, 201);
    assert_live(&f, &accepted).await;
    assert!(f.confirmed(201, &[3], data.clone()).await.is_none());
    assert_eq!(snapshot(&f), accepted);
    assert!(matches!(
        f.confirmed(201, &[4], data.clone()).await,
        Some(Apdu::SimpleAck(_))
    ));
    let receipt_only = snapshot(&f);
    assert_eq!(receipt_only.records, accepted.records);
    assert_eq!(receipt_only.total_record_count, 2);
    assert_eq!(receipt_only.generation, accepted.generation + 1);
    assert_eq!(receipt_only.completed_receipts.len(), 2);
    assert_eq!(f.store.commits.load(Ordering::Acquire), commits + 2);
    f.unconfirmed(data).await;
    settle().await;
    assert_eq!(snapshot(&f), receipt_only);
    assert_eq!(f.requests().len(), 2);
    f.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn audit_forwarding_window_boundary_changes_once_and_complete_match_is_silent() {
    // BACnet Time has 10 ms resolution: test +/- one representable tick.
    let boundary = DeviceConfig::default().apdu_timeout * 2 / 10;
    for distance in [boundary - 1, boundary, boundary + 1] {
        let mut f = ready().await;
        let source = notification(AuditOperation::WRITE);
        f.unconfirmed(request_bytes(vec![source.clone()])).await;
        ack_last(&f).await;
        let before = snapshot(&f);
        let mut target = source.clone();
        target.source_timestamp = None;
        target.target_timestamp = Some(BACnetTimeStamp::Time(Time {
            hour: 12,
            minute: (distance / 6000) as u8,
            second: (distance / 100 % 60) as u8,
            hundredths: (distance % 100) as u8,
        }));
        let data = request_bytes(vec![target.clone()]);
        assert!(matches!(
            f.confirmed(201, &[3], data.clone()).await,
            Some(Apdu::SimpleAck(_))
        ));
        ack_last(&f).await;
        assert_eq!(f.requests().len(), 2);
        assert_eq!(f.requests()[1].service_request, data);
        let accepted = snapshot(&f);
        let expected_count = if distance <= boundary { 1 } else { 2 };
        assert_eq!(accepted.records.len(), expected_count);
        assert_eq!(accepted.total_record_count, expected_count as u64);
        assert_eq!(accepted.records[0].sequence_number, 1);
        assert_eq!(
            accepted.records[0].record.timestamp,
            before.records[0].record.timestamp
        );
        assert_eq!(
            stored(&accepted, 0).target_timestamp.is_some(),
            distance <= boundary
        );
        assert!(f.confirmed(201, &[3], data.clone()).await.is_none());
        if distance <= boundary {
            assert!(matches!(
                f.confirmed(201, &[4], data.clone()).await,
                Some(Apdu::SimpleAck(_))
            ));
            f.unconfirmed(data).await;
            settle().await;
            assert_eq!(f.requests().len(), 2);
            assert_eq!(snapshot(&f).records, accepted.records);
        }
        assert_live(&f, &accepted).await;
        f.server.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn audit_forwarding_zero_capacity_commits_receipt_without_forward() {
    let mut f = ready_capacity(0).await;
    let data = payload(true);
    let before = snapshot(&f);
    assert!(matches!(
        f.confirmed(201, &[3], data.clone()).await,
        Some(Apdu::SimpleAck(_))
    ));
    let accepted = snapshot(&f);
    assert!(accepted.records.is_empty());
    assert_eq!(accepted.total_record_count, 1);
    assert_eq!(accepted.generation, before.generation + 1);
    assert_eq!(accepted.completed_receipts.len(), 1);
    assert!(f.confirmed(201, &[3], data.clone()).await.is_none());
    assert_eq!(snapshot(&f), accepted);
    f.unconfirmed(data).await;
    settle().await;
    assert!(f.requests().is_empty());
    let after = snapshot(&f);
    assert!(after.records.is_empty());
    assert_eq!(after.total_record_count, 2);
    assert_eq!(after.completed_receipts, accepted.completed_receipts);
    assert_live(&f, &after).await;
    f.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn audit_forwarding_mixed_timestamp_variants_create_distinct_records_and_attempts() {
    use bacnet_types::primitives::Date;
    let time = Time {
        hour: 12,
        minute: 0,
        second: 0,
        hundredths: 0,
    };
    let variants = [
        BACnetTimeStamp::Time(time),
        BACnetTimeStamp::DateTime {
            date: Date {
                year: 124,
                month: 2,
                day: 29,
                day_of_week: 4,
            },
            time,
        },
        BACnetTimeStamp::SequenceNumber(0),
    ];
    for (i, source_stamp) in variants.iter().enumerate() {
        for (j, target_stamp) in variants.iter().enumerate() {
            if i == j {
                continue;
            }
            let mut f = ready_capacity(2).await;
            let mut source = notification(AuditOperation::WRITE);
            source.source_timestamp = Some(source_stamp.clone());
            f.unconfirmed(request_bytes(vec![source.clone()])).await;
            ack_last(&f).await;
            let first = snapshot(&f).records[0].clone();
            source.source_timestamp = None;
            source.target_timestamp = Some(target_stamp.clone());
            let data = request_bytes(vec![source]);
            assert!(matches!(
                f.confirmed(201, &[3], data.clone()).await,
                Some(Apdu::SimpleAck(_))
            ));
            ack_last(&f).await;
            let accepted = snapshot(&f);
            assert_eq!(accepted.total_record_count, 2);
            assert_eq!(accepted.records.len(), 2);
            assert_eq!(accepted.records[0], first);
            assert_eq!(accepted.records[1].sequence_number, 2);
            assert_eq!(f.requests().len(), 2);
            assert_eq!(f.requests()[1].service_request, data);
            assert!(f.confirmed(201, &[3], data).await.is_none());
            settle().await;
            assert_eq!(snapshot(&f), accepted);
            assert_eq!(f.requests().len(), 2);
            f.server.stop().await.unwrap();
        }
    }
}
