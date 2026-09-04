use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use bacnet_encoding::npdu::NpduAddress;
use bacnet_types::enums::AuditOperation;
use bacnet_types::MacAddr;

use super::super::*;
use super::{
    confirmed_request, count, database, dispatch, dispatch_confirmed, notification, oid,
    request_bytes, MemoryPersistence,
};

#[tokio::test]
async fn durable_duplicate_is_silent_after_reopen_with_fresh_tracker() {
    let persistence = Arc::new(MemoryPersistence::default());
    let sink = oid(ObjectType::AUDIT_LOG, 7);
    let db = database(Arc::clone(&persistence), 7);
    let authorizations = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&authorizations);
    let config = ServerConfig {
        audit_notification_sink: Some(sink),
        audit_notification_authorizer: Some(Arc::new(move |_| {
            observed.fetch_add(1, Ordering::AcqRel);
            true
        })),
        ..ServerConfig::default()
    };
    let bytes = request_bytes(vec![notification(AuditOperation::WRITE)]);

    let response = dispatch(
        &db,
        &config,
        &Arc::new(ConfirmedRequestTracker::default()),
        9,
        &[0x10],
        None,
        bytes.clone(),
    )
    .await
    .unwrap();
    assert!(matches!(response, Apdu::SimpleAck(_)));
    let durable_before = persistence.snapshot.lock().unwrap().clone().unwrap();
    assert_eq!(count(&db, sink).await, (1, 1));
    drop(db);

    let reopened = database(Arc::clone(&persistence), 7);
    assert!(dispatch(
        &reopened,
        &config,
        &Arc::new(ConfirmedRequestTracker::default()),
        9,
        &[0x10],
        None,
        bytes,
    )
    .await
    .is_err());

    assert_eq!(authorizations.load(Ordering::Acquire), 1);
    assert_eq!(count(&reopened, sink).await, (1, 1));
    assert_eq!(
        persistence.snapshot.lock().unwrap().as_ref(),
        Some(&durable_before)
    );
}

#[tokio::test]
async fn changed_exact_identity_fields_are_admitted_after_reopen() {
    let persistence = Arc::new(MemoryPersistence::default());
    let sink = oid(ObjectType::AUDIT_LOG, 7);
    let db = database(Arc::clone(&persistence), 7);
    let authorizations = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&authorizations);
    let config = ServerConfig {
        audit_notification_sink: Some(sink),
        audit_notification_authorizer: Some(Arc::new(move |_| {
            observed.fetch_add(1, Ordering::AcqRel);
            true
        })),
        ..ServerConfig::default()
    };
    let payload = request_bytes(vec![notification(AuditOperation::WRITE)]);
    let baseline = confirmed_request(20, payload.clone());
    assert!(matches!(
        dispatch_confirmed(
            &db,
            &config,
            &Arc::new(ConfirmedRequestTracker::default()),
            &[1],
            None,
            baseline.clone(),
        )
        .await
        .unwrap(),
        Apdu::SimpleAck(_)
    ));
    drop(db);
    let reopened = database(Arc::clone(&persistence), 7);

    let mut variants = vec![
        (vec![2], None, baseline.clone()),
        (
            vec![1],
            Some(NpduAddress {
                network: 10,
                mac_address: MacAddr::from_slice(&[3]),
            }),
            baseline.clone(),
        ),
    ];
    let mut changed = baseline.clone();
    changed.invoke_id += 1;
    variants.push((vec![1], None, changed));
    let mut changed = baseline.clone();
    changed.segmented = true;
    variants.push((vec![1], None, changed));
    let mut changed = baseline.clone();
    changed.more_follows = true;
    variants.push((vec![1], None, changed));
    let mut changed = baseline.clone();
    changed.segmented_response_accepted = true;
    variants.push((vec![1], None, changed));
    let mut changed = baseline.clone();
    changed.max_segments = Some(4);
    variants.push((vec![1], None, changed));
    let mut changed = baseline.clone();
    changed.max_apdu_length = 480;
    variants.push((vec![1], None, changed));
    let mut changed = baseline.clone();
    changed.sequence_number = Some(1);
    variants.push((vec![1], None, changed));
    let mut changed = baseline.clone();
    changed.proposed_window_size = Some(2);
    variants.push((vec![1], None, changed));
    let mut changed = baseline;
    changed.service_request = request_bytes(vec![notification(AuditOperation::READ)]);
    variants.push((vec![1], None, changed));

    let expected_authorizations = variants.len() + 1;
    for (source_mac, source_network, request) in variants {
        let response = dispatch_confirmed(
            &reopened,
            &config,
            &Arc::new(ConfirmedRequestTracker::default()),
            &source_mac,
            source_network,
            request,
        )
        .await
        .unwrap();
        assert!(matches!(response, Apdu::SimpleAck(_)));
    }
    assert_eq!(
        authorizations.load(Ordering::Acquire),
        expected_authorizations
    );
    assert_eq!(
        persistence
            .snapshot
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .completed_receipts
            .len(),
        expected_authorizations
    );
}

#[tokio::test]
async fn concurrent_exact_admission_authorizes_and_commits_once_without_locking_callback() {
    let persistence = Arc::new(MemoryPersistence::default());
    let sink = oid(ObjectType::AUDIT_LOG, 7);
    let db = database(Arc::clone(&persistence), 7);
    let authorizations = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&authorizations);
    let callback_db = Arc::clone(&db);
    let config = ServerConfig {
        audit_notification_sink: Some(sink),
        audit_notification_authorizer: Some(Arc::new(move |_| {
            let guard = callback_db
                .try_write()
                .expect("database lock must not be held across authorization");
            drop(guard);
            observed.fetch_add(1, Ordering::AcqRel);
            true
        })),
        ..ServerConfig::default()
    };
    let tracker = Arc::new(ConfirmedRequestTracker::default());
    let bytes = request_bytes(vec![notification(AuditOperation::WRITE)]);

    let (left, right) = tokio::join!(
        dispatch(&db, &config, &tracker, 11, &[1], None, bytes.clone()),
        dispatch(&db, &config, &tracker, 11, &[1], None, bytes),
    );
    assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
    assert_eq!(authorizations.load(Ordering::Acquire), 1);
    assert_eq!(count(&db, sink).await, (1, 1));
    assert_eq!(
        persistence
            .snapshot
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .completed_receipts
            .len(),
        1
    );
}

#[tokio::test]
async fn failed_confirmed_commit_writes_no_receipt_and_fresh_retry_succeeds() {
    let persistence = Arc::new(MemoryPersistence::default());
    let sink = oid(ObjectType::AUDIT_LOG, 7);
    let db = database(Arc::clone(&persistence), 7);
    let config = ServerConfig {
        audit_notification_sink: Some(sink),
        audit_notification_authorizer: Some(Arc::new(|_| true)),
        ..ServerConfig::default()
    };
    let bytes = request_bytes(vec![notification(AuditOperation::WRITE)]);
    let before = persistence.snapshot.lock().unwrap().clone().unwrap();
    persistence.fail.store(true, Ordering::Release);

    let response = dispatch(
        &db,
        &config,
        &Arc::new(ConfirmedRequestTracker::default()),
        12,
        &[1],
        None,
        bytes.clone(),
    )
    .await
    .unwrap();
    assert!(matches!(response, Apdu::Error(_)));
    assert_eq!(persistence.snapshot.lock().unwrap().as_ref(), Some(&before));
    assert_eq!(count(&db, sink).await, (0, 0));

    persistence.fail.store(false, Ordering::Release);
    let response = dispatch(
        &db,
        &config,
        &Arc::new(ConfirmedRequestTracker::default()),
        12,
        &[1],
        None,
        bytes,
    )
    .await
    .unwrap();
    assert!(matches!(response, Apdu::SimpleAck(_)));
    assert_eq!(count(&db, sink).await, (1, 1));
    assert_eq!(
        persistence
            .snapshot
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .completed_receipts
            .len(),
        1
    );
}
