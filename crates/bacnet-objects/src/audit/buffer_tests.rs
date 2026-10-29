//! Buffer_Size writes and the application's purge of an Audit Log (#1238).

use std::sync::Arc;
use std::sync::atomic::Ordering;

use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::{BACnetAuditLogDatum, BACnetAuditLogQueryParameters};
use bacnet_types::enums::{
    BACnetSuccessFilter, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier,
};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use crate::durable::{DurableWrites, PendingWrite, StageStep};
use crate::traits::BACnetObject;

use super::staging_tests::{
    SlowPersistence, WAIT, block_on, log, notification, receipt, staged, staged_write,
};
use super::{
    AuditBatchStage, AuditLogNotificationSink, AuditLogObject, AuditLogSnapshot, AuditLogStorage,
    ConfirmedAuditNotificationOutcome, MAX_AUDIT_RECORDS,
};

fn write(
    log: &mut AuditLogObject,
    property: PropertyIdentifier,
    value: PropertyValue,
) -> Result<(), Error> {
    log.write_property(property, None, value, None)
}

fn resize(log: &mut AuditLogObject, size: u64) -> Result<(), Error> {
    write(
        log,
        PropertyIdentifier::BUFFER_SIZE,
        PropertyValue::Unsigned(size),
    )
}

fn enable(log: &mut AuditLogObject, on: bool) {
    write(
        log,
        PropertyIdentifier::LOG_ENABLE,
        PropertyValue::Boolean(on),
    )
    .unwrap();
}

fn read(log: &AuditLogObject, property: PropertyIdentifier) -> PropertyValue {
    log.read_property(property, None).unwrap()
}

fn assert_operational_problem(error: Error) {
    assert!(
        matches!(error, Error::Remote(RemoteError::Error { class, code })
            if class == ErrorClass::DEVICE
                && code == ErrorCode::OPERATIONAL_PROBLEM),
        "expected DEVICE / OPERATIONAL_PROBLEM, got {error:?}"
    );
}

fn assert_property_error(error: Error, expected: ErrorCode) {
    assert!(
        matches!(error, Error::Remote(RemoteError::Error { class, code })
            if class == ErrorClass::PROPERTY
                && code == expected),
        "expected PROPERTY / {expected:?}, got {error:?}"
    );
}

/// The sequence numbers of the records the log holds, oldest first: the
/// Log_Buffer that ReadRange pages.
fn held(log: &AuditLogObject) -> Vec<u64> {
    log.retained_records()
        .iter()
        .map(|record| record.sequence_number)
        .collect()
}

/// The log-status flags of the record with `sequence_number`.
fn status_of(log: &AuditLogObject, sequence_number: u64) -> LogStatus {
    let record = log
        .records()
        .iter()
        .find(|record| record.sequence_number == sequence_number)
        .expect("the record is held");
    match &record.record.datum {
        BACnetAuditLogDatum::LogStatus(status) => *status,
        other => panic!("record {sequence_number} is not a log-status record: {other:?}"),
    }
}

/// The sequence numbers an AuditLogQuery for every report on target Device 2
/// returns, newest first.
fn queried(log: &AuditLogObject) -> Vec<u64> {
    let parameters = BACnetAuditLogQueryParameters::ByTarget {
        target_device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 2).unwrap(),
        target_device_address: None,
        target_object_identifier: None,
        target_property_identifier: None,
        target_array_index: None,
        target_priority: None,
        operations: None,
        successful_actions_only: BACnetSuccessFilter::ALL,
    };
    log.query(&parameters, None, 100)
        .records
        .iter()
        .map(|record| record.sequence_number)
        .collect()
}

/// Store one target report for each invoke ID, each its own record.
fn fill(log: &mut AuditLogObject, invoke_ids: std::ops::Range<u8>) {
    for invoke_id in invoke_ids {
        log.store_notifications(&[notification(invoke_id)], 3_000)
            .unwrap();
    }
}

/// The log serves what `before` holds, and storage still holds it.
fn assert_unchanged(log: &AuditLogObject, storage: &SlowPersistence, before: &AuditLogSnapshot) {
    log.wait_for_commits();
    assert_eq!(log.generation(), before.generation);
    assert_eq!(log.buffer_size(), before.capacity);
    assert_eq!(log.log_enable(), before.log_enable);
    assert_eq!(log.total_record_count(), before.total_record_count);
    assert_eq!(
        log.records().iter().cloned().collect::<Vec<_>>(),
        before.records
    );
    assert_eq!(&storage.committed(), before);
}

fn reopen(storage: &Arc<SlowPersistence>, configured: u32) -> AuditLogObject {
    let mut log = AuditLogObject::new(1, "audit", configured, Arc::clone(storage) as _).unwrap();
    log.bind_clock_internal(Some(Arc::new(super::staging_tests::FixedClock)));
    log
}

#[test]
fn buffer_size_takes_a_write_only_while_logging_is_off() {
    let (mut log, storage) = log();
    fill(&mut log, 1..4);
    let before = storage.committed();
    // While logging is on every write is refused, the current size too.
    for value in [
        PropertyValue::Unsigned(10),
        PropertyValue::Unsigned(5),
        PropertyValue::Boolean(true),
    ] {
        assert_property_error(
            write(&mut log, PropertyIdentifier::BUFFER_SIZE, value).unwrap_err(),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
    }
    assert_unchanged(&log, &storage, &before);

    enable(&mut log, false);
    let before = storage.committed();
    for (value, code) in [
        (PropertyValue::Boolean(true), ErrorCode::INVALID_DATA_TYPE),
        (PropertyValue::Real(5.0), ErrorCode::INVALID_DATA_TYPE),
        (
            PropertyValue::Unsigned(u64::from(MAX_AUDIT_RECORDS) + 1),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        // Clause 12.64.9 sets 2^32-1 aside for a size bounded only by memory.
        (
            PropertyValue::Unsigned(u64::from(u32::MAX)),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            PropertyValue::Unsigned(u64::from(u32::MAX) + 1),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
    ] {
        assert_property_error(
            write(&mut log, PropertyIdentifier::BUFFER_SIZE, value).unwrap_err(),
            code,
        );
    }
    // Writing the current size commits nothing.
    resize(&mut log, 10).unwrap();
    assert_unchanged(&log, &storage, &before);

    // A larger buffer keeps every record, and storage has it first.
    resize(&mut log, u64::from(MAX_AUDIT_RECORDS)).unwrap();
    assert_eq!(
        read(&log, PropertyIdentifier::BUFFER_SIZE),
        PropertyValue::Unsigned(u64::from(MAX_AUDIT_RECORDS))
    );
    assert_eq!(held(&log), [1, 2, 3, 4]);
    assert_eq!(storage.committed().capacity, MAX_AUDIT_RECORDS);
    assert_eq!(storage.committed().generation, before.generation + 1);

    enable(&mut log, true);
    assert_property_error(
        resize(&mut log, 4).unwrap_err(),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    assert_eq!(log.buffer_size(), MAX_AUDIT_RECORDS);
}

#[test]
fn a_shrink_below_the_record_count_keeps_the_newest_records() {
    let (mut log, storage) = log();
    fill(&mut log, 1..6);
    enable(&mut log, false);
    assert_eq!(held(&log), [1, 2, 3, 4, 5, 6]);

    resize(&mut log, 2).unwrap();
    assert_eq!(held(&log), [5, 6]);
    assert_eq!(status_of(&log, 6), LogStatus::LOG_DISABLED);
    assert_eq!(
        read(&log, PropertyIdentifier::RECORD_COUNT),
        PropertyValue::Unsigned(2)
    );
    assert_eq!(
        read(&log, PropertyIdentifier::TOTAL_RECORD_COUNT),
        PropertyValue::Unsigned(6)
    );
    // AuditLogQuery scans the same ring: only the newest report is left.
    assert_eq!(queried(&log), [5]);
    let committed = storage.committed();
    assert_eq!(committed.capacity, 2);
    assert_eq!(committed.total_record_count, 6);
    assert_eq!(
        committed
            .records
            .iter()
            .map(|record| record.sequence_number)
            .collect::<Vec<_>>(),
        [5, 6]
    );

    // Reopened, the log keeps the written size over the configured one.
    drop(log);
    let mut log = reopen(&storage, 10);
    assert_eq!(log.buffer_size(), 2);
    assert_eq!(held(&log), [5, 6]);

    // Growing again makes room but brings nothing back.
    resize(&mut log, 4).unwrap();
    enable(&mut log, true);
    assert_eq!(held(&log), [5, 6, 7]);

    // Down to zero only the count is left, and a log grown from zero holds
    // no records until the next one arrives; storage loads it so.
    enable(&mut log, false);
    resize(&mut log, 0).unwrap();
    assert!(held(&log).is_empty());
    resize(&mut log, 3).unwrap();
    assert!(held(&log).is_empty());
    assert_eq!(log.total_record_count(), 8);
    drop(log);
    let mut log = reopen(&storage, 10);
    assert_eq!(log.buffer_size(), 3);
    assert!(held(&log).is_empty());
    assert_eq!(log.total_record_count(), 8);
    enable(&mut log, true);
    assert_eq!(held(&log), [9]);
}

#[test]
fn a_purge_leaves_one_buffer_purged_record_and_nothing_to_query() {
    let (mut log, storage) = log();
    let batch =
        staged(log.stage_notification_batch(&[notification(1)], 3_000, Some(receipt(b"kept"))));
    block_on(batch.saved());
    log.finish_notification_batch(batch).unwrap();
    fill(&mut log, 2..4);
    assert_eq!(queried(&log), [3, 2, 1]);

    // A peer cannot purge it: Record_Count stays read-only (Clause 12.64.11).
    let before = storage.committed();
    assert_property_error(
        write(
            &mut log,
            PropertyIdentifier::RECORD_COUNT,
            PropertyValue::Unsigned(0),
        )
        .unwrap_err(),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    assert_unchanged(&log, &storage, &before);

    assert_eq!(log.purge().unwrap(), 4);
    assert_eq!(held(&log), [4]);
    assert_eq!(status_of(&log, 4), LogStatus::BUFFER_PURGED);
    assert_eq!(
        read(&log, PropertyIdentifier::RECORD_COUNT),
        PropertyValue::Unsigned(1)
    );
    assert_eq!(
        read(&log, PropertyIdentifier::TOTAL_RECORD_COUNT),
        PropertyValue::Unsigned(4)
    );
    // AuditLogQuery returns notifications only, and none is left.
    assert!(queried(&log).is_empty());
    let committed = storage.committed();
    assert_eq!(
        committed.records,
        log.records().iter().cloned().collect::<Vec<_>>()
    );
    assert_eq!(committed.total_record_count, 4);
    assert_eq!(committed.generation, before.generation + 1);

    // The completed receipts survive: the confirmed batch sent again is still a
    // duplicate, and a new report follows the purge record.
    assert!(matches!(
        log.stage_notification_batch(&[notification(1)], 3_000, Some(receipt(b"kept"))),
        Ok(AuditBatchStage::Done(
            ConfirmedAuditNotificationOutcome::Duplicate,
            false
        ))
    ));
    fill(&mut log, 9..10);
    assert_eq!(held(&log), [4, 5]);
    assert_eq!(queried(&log), [5]);

    // Purged while logging is off, the record says that too.
    enable(&mut log, false);
    assert_eq!(log.purge().unwrap(), 7);
    assert_eq!(held(&log), [7]);
    assert_eq!(
        status_of(&log, 7),
        LogStatus::BUFFER_PURGED | LogStatus::LOG_DISABLED
    );
}

#[test]
fn a_purge_without_a_valid_clock_changes_nothing() {
    let storage = Arc::new(SlowPersistence::default());
    let mut log = AuditLogObject::new(1, "audit", 10, Arc::clone(&storage) as _).unwrap();
    let before = storage.committed();
    assert_operational_problem(log.purge().unwrap_err());
    assert!(matches!(log.stage_purge(), StageStep::Skip));
    assert_unchanged(&log, &storage, &before);
}

#[test]
fn a_failing_store_refuses_a_resize_or_purge_and_leaves_the_log_unchanged() {
    let (mut log, storage) = log();
    fill(&mut log, 1..4);
    enable(&mut log, false);
    let before = storage.committed();
    storage.fail.store(true, Ordering::SeqCst);

    // In place, as application code holding the guard makes them. Each is
    // refused as a change the device cannot make now.
    assert_operational_problem(resize(&mut log, 1).unwrap_err());
    assert_unchanged(&log, &storage, &before);
    assert_operational_problem(log.purge().unwrap_err());
    assert_unchanged(&log, &storage, &before);
    assert_operational_problem(
        write(
            &mut log,
            PropertyIdentifier::LOG_ENABLE,
            PropertyValue::Boolean(true),
        )
        .unwrap_err(),
    );
    assert_unchanged(&log, &storage, &before);

    // Staged, as the server makes them.
    let wait = staged_write(log.stage_write(
        PropertyIdentifier::BUFFER_SIZE,
        None,
        &PropertyValue::Unsigned(1),
    ));
    block_on(wait.clone());
    assert_operational_problem(resize(&mut log, 1).unwrap_err());
    log.release_staged_write(&wait);
    assert_unchanged(&log, &storage, &before);
    let wait = staged_write(log.stage_purge());
    block_on(wait.clone());
    assert_operational_problem(log.commit_purge().unwrap_err());
    log.release_staged_write(&wait);
    assert_unchanged(&log, &storage, &before);

    // With storage back the same changes go through.
    storage.fail.store(false, Ordering::SeqCst);
    resize(&mut log, 1).unwrap();
    assert_eq!(held(&log), [4]);
    assert_eq!(log.purge().unwrap(), 5);
    assert_eq!(held(&log), [5]);
}

#[test]
fn a_staged_resize_or_purge_commits_while_the_log_serves_its_committed_state() {
    let (mut log, storage) = log();
    fill(&mut log, 1..4);
    enable(&mut log, false);
    let generation = log.generation();
    let commits = storage.commits.load(Ordering::SeqCst);
    let (started, go) = storage.hold();

    let wait = staged_write(log.stage_write(
        PropertyIdentifier::BUFFER_SIZE,
        None,
        &PropertyValue::Unsigned(2),
    ));
    assert_eq!(started.recv_timeout(WAIT).unwrap(), generation + 1);
    assert_eq!(
        read(&log, PropertyIdentifier::BUFFER_SIZE),
        PropertyValue::Unsigned(10)
    );
    assert_eq!(held(&log), [1, 2, 3, 4]);
    go.send(()).unwrap();
    block_on(wait.clone());
    resize(&mut log, 2).unwrap();
    log.release_staged_write(&wait);
    assert_eq!(log.buffer_size(), 2);
    assert_eq!(held(&log), [3, 4]);

    let wait = staged_write(log.stage_purge());
    assert_eq!(started.recv_timeout(WAIT).unwrap(), generation + 2);
    assert_eq!(held(&log), [3, 4]);
    go.send(()).unwrap();
    block_on(wait.clone());
    log.commit_purge().unwrap();
    log.release_staged_write(&wait);
    assert_eq!(held(&log), [5]);
    assert_eq!(
        status_of(&log, 5),
        LogStatus::BUFFER_PURGED | LogStatus::LOG_DISABLED
    );
    assert_eq!(log.generation(), generation + 2);
    // Each request took its staged commit and committed nothing more.
    assert_eq!(storage.commits.load(Ordering::SeqCst), commits + 2);
}

#[test]
fn a_purge_staged_behind_a_batch_lands_after_it() {
    let (mut log, storage) = log();
    let (started, go) = storage.hold();
    let batch =
        staged(log.stage_notification_batch(&[notification(1)], 3_000, Some(receipt(b"first"))));
    started.recv_timeout(WAIT).unwrap();
    // The purge waits for the batch's commit, then builds on the batch.
    let StageStep::Busy(wait) = log.stage_purge() else {
        panic!("the purge should wait while the batch commits");
    };
    assert!(!wait.is_ready());
    go.send(()).unwrap();
    block_on(wait);
    let purge = staged_write(log.stage_purge());
    started.recv_timeout(WAIT).unwrap();
    go.send(()).unwrap();
    block_on(purge.clone());
    log.commit_purge().unwrap();
    log.release_staged_write(&purge);

    // The batch's request still gets its outcome, and its record went with
    // the purge.
    assert_eq!(
        log.finish_notification_batch(batch).unwrap(),
        (ConfirmedAuditNotificationOutcome::Stored, true)
    );
    assert_eq!(held(&log), [2]);
    assert_eq!(status_of(&log, 2), LogStatus::BUFFER_PURGED);
    assert!(queried(&log).is_empty());
    assert_eq!(storage.committed().total_record_count, 2);
}

#[test]
fn a_batch_that_arrives_during_a_purge_lands_after_it() {
    let (mut log, storage) = log();
    fill(&mut log, 1..2);
    let (started, go) = storage.hold();
    let purge = staged_write(log.stage_purge());
    started.recv_timeout(WAIT).unwrap();
    // The batch waits until the purge's request takes the purge, even once
    // the purge's commit has run.
    let AuditBatchStage::Busy(wait) = log
        .stage_notification_batch(&[notification(2)], 3_000, None)
        .unwrap()
    else {
        panic!("the batch should wait while the purge is staged");
    };
    go.send(()).unwrap();
    block_on(purge.clone());
    assert!(!wait.is_ready());
    log.commit_purge().unwrap();
    log.release_staged_write(&purge);
    block_on(wait);

    let batch = staged(log.stage_notification_batch(&[notification(2)], 3_000, None));
    started.recv_timeout(WAIT).unwrap();
    go.send(()).unwrap();
    block_on(batch.saved());
    assert_eq!(
        log.finish_notification_batch(batch).unwrap(),
        (ConfirmedAuditNotificationOutcome::Stored, true)
    );
    assert_eq!(held(&log), [2, 3]);
    assert_eq!(status_of(&log, 2), LogStatus::BUFFER_PURGED);
    assert_eq!(queried(&log), [3]);
}

#[test]
fn staging_skips_a_resize_the_write_would_refuse_or_not_need() {
    let (mut log, storage) = log();
    let skipped = |step| matches!(step, StageStep::Skip);
    let size = |size| PropertyValue::Unsigned(size);
    assert!(skipped(log.stage_write(
        PropertyIdentifier::BUFFER_SIZE,
        None,
        &size(5)
    )));
    enable(&mut log, false);
    let commits = storage.commits.load(Ordering::SeqCst);
    for (property, index, value) in [
        (PropertyIdentifier::BUFFER_SIZE, None, size(10)),
        (
            PropertyIdentifier::BUFFER_SIZE,
            None,
            PropertyValue::Boolean(true),
        ),
        (
            PropertyIdentifier::BUFFER_SIZE,
            None,
            size(u64::from(MAX_AUDIT_RECORDS) + 1),
        ),
        (PropertyIdentifier::BUFFER_SIZE, Some(1), size(5)),
        (PropertyIdentifier::RECORD_COUNT, None, size(0)),
    ] {
        assert!(
            skipped(log.stage_write(property, index, &value)),
            "{property:?} {index:?} {value:?}"
        );
    }
    log.wait_for_commits();
    assert_eq!(storage.commits.load(Ordering::SeqCst), commits);
}

#[test]
fn an_object_without_a_log_refuses_a_purge() {
    let mut forwarder =
        crate::notification_forwarder::NotificationForwarderObject::new(1, "NF").unwrap();
    let writes = forwarder.durable_writes_internal().unwrap();
    assert!(matches!(writes.stage_purge(), StageStep::Skip));
    let error = writes.commit_purge().unwrap_err();
    assert!(
        matches!(error, Error::Remote(RemoteError::Error { class, code })
            if class == ErrorClass::OBJECT
                && code == ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED),
        "{error:?}"
    );
}

fn pending(property: PropertyIdentifier, value: PropertyValue) -> PendingWrite {
    PendingWrite {
        property,
        array_index: None,
        value,
    }
}

/// A WritePropertyMultiple that turns logging off, writes Description and
/// then resizes.
fn off_then_resize(size: u64) -> [PendingWrite; 3] {
    [
        pending(
            PropertyIdentifier::LOG_ENABLE,
            PropertyValue::Boolean(false),
        ),
        pending(
            PropertyIdentifier::DESCRIPTION,
            PropertyValue::CharacterString("resized".into()),
        ),
        pending(
            PropertyIdentifier::BUFFER_SIZE,
            PropertyValue::Unsigned(size),
        ),
    ]
}

#[test]
fn a_request_folds_its_log_enable_and_buffer_size_writes_into_one_commit() {
    let (mut log, storage) = log();
    fill(&mut log, 1..4);
    let generation = log.generation();
    let commits = storage.commits.load(Ordering::SeqCst);
    let (started, go) = storage.hold();
    let wait = staged_write(log.stage_writes(&off_then_resize(2)));
    assert_eq!(started.recv_timeout(WAIT).unwrap(), generation + 1);
    // One commit holds both changes, while the log serves what it had.
    assert!(log.log_enable());
    assert_eq!(log.buffer_size(), 10);
    assert_eq!(held(&log), [1, 2, 3]);
    go.send(()).unwrap();
    block_on(wait.clone());
    let committed = storage.committed();
    assert!(!committed.log_enable);
    assert_eq!(committed.capacity, 2);

    // The request makes its writes in order, each taking its own step.
    enable(&mut log, false);
    assert_eq!(log.buffer_size(), 10);
    assert_eq!(held(&log), [1, 2, 3, 4]);
    write(
        &mut log,
        PropertyIdentifier::DESCRIPTION,
        PropertyValue::CharacterString("resized".into()),
    )
    .unwrap();
    resize(&mut log, 2).unwrap();
    log.release_staged_write(&wait);
    log.wait_for_commits();
    assert_eq!(held(&log), [3, 4]);
    assert_eq!(log.generation(), generation + 1);
    assert_eq!(storage.commits.load(Ordering::SeqCst), commits + 1);
    assert_eq!(storage.committed(), committed);
}

#[test]
fn a_request_that_stops_between_its_staged_writes_puts_the_served_log_in_storage() {
    let (mut log, storage) = log();
    fill(&mut log, 1..4);
    let wait = staged_write(log.stage_writes(&off_then_resize(2)));
    block_on(wait.clone());
    assert_eq!(storage.committed().capacity, 2);
    // Logging goes off, then the request fails before its resize.
    enable(&mut log, false);
    log.release_staged_write(&wait);
    log.wait_for_commits();
    let committed = storage.committed();
    assert!(!committed.log_enable);
    assert_eq!(committed.capacity, 10);
    assert_eq!(
        committed.records,
        log.records().iter().cloned().collect::<Vec<_>>()
    );
    drop(log);
    let log = reopen(&storage, 10);
    assert_eq!(log.buffer_size(), 10);
    assert!(!log.log_enable());
    assert_eq!(held(&log), [1, 2, 3, 4]);
}

#[test]
fn a_request_that_resizes_before_turning_logging_off_stages_nothing() {
    let (mut log, storage) = log();
    let commits = storage.commits.load(Ordering::SeqCst);
    // The resize is refused while logging is on, so the request ends there.
    let writes = [
        pending(PropertyIdentifier::BUFFER_SIZE, PropertyValue::Unsigned(2)),
        pending(
            PropertyIdentifier::LOG_ENABLE,
            PropertyValue::Boolean(false),
        ),
    ];
    assert!(matches!(log.stage_writes(&writes), StageStep::Skip));
    log.wait_for_commits();
    assert_eq!(storage.commits.load(Ordering::SeqCst), commits);
}

#[test]
fn a_request_goes_on_past_a_null_the_log_leaves_unchanged() {
    let null = |property| pending(property, PropertyValue::Null);
    // The server leaves a NULL to either property as it is (#1396), so the
    // writes after one still stage. With logging off, a NULL resize is no
    // refusal either.
    let (mut log, storage) = log();
    let writes = [
        null(PropertyIdentifier::LOG_ENABLE),
        pending(
            PropertyIdentifier::LOG_ENABLE,
            PropertyValue::Boolean(false),
        ),
        null(PropertyIdentifier::BUFFER_SIZE),
        pending(PropertyIdentifier::BUFFER_SIZE, PropertyValue::Unsigned(2)),
    ];
    let wait = staged_write(log.stage_writes(&writes));
    block_on(wait.clone());
    let committed = storage.committed();
    assert!(!committed.log_enable);
    assert_eq!(committed.capacity, 2);
    log.release_staged_write(&wait);
    log.wait_for_commits();

    // With logging on, a resize is refused whatever its value, a NULL too,
    // so the request ends there.
    let (mut log, storage) = self::log();
    let commits = storage.commits.load(Ordering::SeqCst);
    let writes = [
        null(PropertyIdentifier::BUFFER_SIZE),
        pending(
            PropertyIdentifier::LOG_ENABLE,
            PropertyValue::Boolean(false),
        ),
    ];
    assert!(matches!(log.stage_writes(&writes), StageStep::Skip));
    log.wait_for_commits();
    assert_eq!(storage.commits.load(Ordering::SeqCst), commits);
}
