//! Audit Log commits off the database lock (#1270).

use std::future::Future;
use std::pin::pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::task::{Context, Wake, Waker};
use std::time::{Duration, Instant};

use bacnet_types::constructed::{AuditPropertyReference, BACnetAuditNotification, BACnetRecipient};
use bacnet_types::enums::{AuditOperation, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, Date, ObjectIdentifier, PropertyValue, Time};

use crate::clock::{ClockFrame, ClockReader};
use crate::durable::{DurableWrites, PendingWrite, SaveWait, StageStep};
use crate::traits::BACnetObject;

use super::{
    AuditBatchStage, AuditLogNotificationSink, AuditLogObject, AuditLogPersistence,
    AuditLogSnapshot, CompletedAuditReceipt, ConfirmedAuditNotificationOutcome, StagedAuditBatch,
};

pub(super) const WAIT: Duration = Duration::from_secs(10);

/// Storage in memory whose commits can fail, or wait until the test lets
/// each one go.
#[derive(Default)]
pub(super) struct SlowPersistence {
    snapshot: Mutex<Option<AuditLogSnapshot>>,
    pub(super) commits: AtomicUsize,
    pub(super) fail: AtomicBool,
    hold: Mutex<Option<(mpsc::Sender<u64>, mpsc::Receiver<()>)>>,
}

impl SlowPersistence {
    /// Make every later commit report its generation on the first channel
    /// and wait for a message on the second.
    pub(super) fn hold(&self) -> (mpsc::Receiver<u64>, mpsc::Sender<()>) {
        let (started, started_rx) = mpsc::channel();
        let (go, go_rx) = mpsc::channel();
        *self.hold.lock().unwrap() = Some((started, go_rx));
        (started_rx, go)
    }

    pub(super) fn committed(&self) -> AuditLogSnapshot {
        self.snapshot.lock().unwrap().clone().unwrap()
    }
}

impl AuditLogPersistence for SlowPersistence {
    fn load(&self, _expected_object: ObjectIdentifier) -> Result<Option<AuditLogSnapshot>, Error> {
        Ok(self.snapshot.lock().unwrap().clone())
    }

    fn commit(&self, snapshot: &AuditLogSnapshot) -> Result<(), Error> {
        if let Some((started, go)) = &*self.hold.lock().unwrap() {
            let _ = started.send(snapshot.generation);
            let _ = go.recv();
        }
        if self.fail.load(Ordering::SeqCst) {
            return Err(Error::Transport(std::io::Error::other(
                "injected commit failure",
            )));
        }
        *self.snapshot.lock().unwrap() = Some(snapshot.clone());
        self.commits.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

pub(super) struct FixedClock;

impl ClockReader for FixedClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(ClockFrame {
            local_date: Date {
                year: 124,
                month: 2,
                day: 29,
                day_of_week: 4,
            },
            local_time: time(30),
            utc_offset: 0,
            daylight_savings_status: false,
        })
    }
}

fn time(second: u8) -> Time {
    Time {
        hour: 12,
        minute: 0,
        second,
        hundredths: 0,
    }
}

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// A complete target report with invoke ID `invoke_id`.
pub(super) fn notification(invoke_id: u8) -> BACnetAuditNotification {
    BACnetAuditNotification {
        source_timestamp: Some(BACnetTimeStamp::Time(time(0))),
        target_timestamp: Some(BACnetTimeStamp::Time(time(0))),
        source_device: BACnetRecipient::Device(oid(ObjectType::DEVICE, 1)),
        source_object: None,
        operation: AuditOperation::WRITE,
        source_comment: None,
        target_comment: None,
        invoke_id: Some(invoke_id),
        source_user_id: None,
        source_user_role: None,
        target_device: BACnetRecipient::Device(oid(ObjectType::DEVICE, 2)),
        target_object: Some(oid(ObjectType::ANALOG_VALUE, 3)),
        target_property: Some(AuditPropertyReference {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
        }),
        target_priority: Some(8),
        target_value: Some(vec![0x21, 0x05]),
        current_value: None,
        result: None,
    }
}

pub(super) fn receipt(key: &[u8]) -> CompletedAuditReceipt {
    CompletedAuditReceipt::new(key.to_vec(), 1_725_000_000_000).unwrap()
}

pub(super) fn log() -> (AuditLogObject, Arc<SlowPersistence>) {
    let storage = Arc::new(SlowPersistence::default());
    let mut log = AuditLogObject::new(1, "audit", 10, storage.clone()).unwrap();
    log.bind_clock_internal(Some(Arc::new(FixedClock)));
    (log, storage)
}

pub(super) fn staged(stage: Result<AuditBatchStage, Error>) -> StagedAuditBatch {
    match stage.unwrap() {
        AuditBatchStage::Staged(staged) => staged,
        other => panic!("expected a staged batch, got {other:?}"),
    }
}

struct ThreadWaker(std::thread::Thread);

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

pub(super) fn block_on(future: impl Future<Output = ()>) {
    let waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut future = pin!(future);
    let deadline = Instant::now() + WAIT;
    while future.as_mut().poll(&mut cx).is_pending() {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "the future did not resolve in time");
        std::thread::park_timeout(left);
    }
}

#[test]
fn a_staged_batch_commits_while_the_log_serves_its_committed_state() {
    let (mut log, storage) = log();
    let generation = log.generation();
    let (started, go) = storage.hold();
    let batch =
        staged(log.stage_notification_batch(&[notification(1)], 3_000, Some(receipt(b"first"))));

    // The commit runs on the writer thread and is held there. The log is
    // free meanwhile and serves what it had committed.
    assert_eq!(started.recv_timeout(WAIT).unwrap(), generation + 1);
    assert!(!batch.saved().is_ready());
    assert!(log.records().is_empty());
    assert_eq!(log.generation(), generation);
    assert!(
        !log.has_completed_confirmed_receipt(b"first", 1_725_000_000_000)
            .unwrap()
    );
    assert_eq!(
        log.read_property(PropertyIdentifier::RECORD_COUNT, None)
            .unwrap(),
        PropertyValue::Unsigned(0)
    );

    go.send(()).unwrap();
    block_on(batch.saved());
    // The records and the receipt are durable before the log takes them.
    let committed = storage.committed();
    assert_eq!(committed.generation, generation + 1);
    assert_eq!(committed.records.len(), 1);
    assert_eq!(committed.completed_receipts.len(), 1);
    assert_eq!(
        log.finish_notification_batch(batch).unwrap(),
        (ConfirmedAuditNotificationOutcome::Stored, true)
    );
    assert_eq!(log.records().len(), 1);
    assert_eq!(log.generation(), generation + 1);
    assert!(
        log.has_completed_confirmed_receipt(b"first", 1_725_000_000_000)
            .unwrap()
    );
}

#[test]
fn a_staged_batch_whose_commit_fails_leaves_the_log_as_it_was() {
    let (mut log, storage) = log();
    let generation = log.generation();
    storage.fail.store(true, Ordering::SeqCst);
    let batch =
        staged(log.stage_notification_batch(&[notification(1)], 3_000, Some(receipt(b"lost"))));
    block_on(batch.saved());
    assert!(log.finish_notification_batch(batch).is_err());
    assert!(log.records().is_empty());
    assert_eq!(log.generation(), generation);
    assert!(
        !log.has_completed_confirmed_receipt(b"lost", 1_725_000_000_000)
            .unwrap()
    );
    // The retransmission commits once storage is back.
    storage.fail.store(false, Ordering::SeqCst);
    let retry =
        staged(log.stage_notification_batch(&[notification(1)], 3_000, Some(receipt(b"lost"))));
    block_on(retry.saved());
    assert_eq!(
        log.finish_notification_batch(retry).unwrap(),
        (ConfirmedAuditNotificationOutcome::Stored, true)
    );
    assert_eq!(log.generation(), generation + 1);
}

#[test]
fn a_second_batch_waits_for_the_first_commit_then_builds_on_it() {
    let (mut log, storage) = log();
    let generation = log.generation();
    let (started, go) = storage.hold();
    let first = staged(log.stage_notification_batch(&[notification(1)], 3_000, None));
    started.recv_timeout(WAIT).unwrap();
    let AuditBatchStage::Busy(wait) = log
        .stage_notification_batch(&[notification(2)], 3_000, None)
        .unwrap()
    else {
        panic!("the log should be busy while the first commit runs");
    };
    go.send(()).unwrap();
    block_on(wait);
    // The first commit ran but its request has not come back: the second
    // takes the first over and stages on top of it.
    let second = staged(log.stage_notification_batch(&[notification(2)], 3_000, None));
    assert_eq!(log.records().len(), 1);
    go.send(()).unwrap();
    block_on(second.saved());
    assert_eq!(
        log.finish_notification_batch(second).unwrap(),
        (ConfirmedAuditNotificationOutcome::Stored, true)
    );
    assert_eq!(
        log.finish_notification_batch(first).unwrap(),
        (ConfirmedAuditNotificationOutcome::Stored, true)
    );
    assert_eq!(log.records().len(), 2);
    assert_eq!(log.generation(), generation + 2);
    assert_eq!(storage.committed().generation, generation + 2);
}

#[test]
fn an_in_place_change_lets_a_staged_batch_land_first() {
    let (mut log, storage) = log();
    let generation = log.generation();
    let (started, go) = storage.hold();
    let batch =
        staged(log.stage_notification_batch(&[notification(1)], 3_000, Some(receipt(b"staged"))));
    started.recv_timeout(WAIT).unwrap();
    std::thread::scope(|scope| {
        // An application record waits for the staged commit, then commits
        // the next generation in place.
        let adding = scope.spawn(|| {
            log.add_record(super::log_enable_record(
                (
                    Date {
                        year: 124,
                        month: 2,
                        day: 29,
                        day_of_week: 4,
                    },
                    time(40),
                ),
                true,
            ))
        });
        go.send(()).unwrap();
        assert_eq!(started.recv_timeout(WAIT).unwrap(), generation + 2);
        go.send(()).unwrap();
        assert_eq!(adding.join().unwrap().unwrap(), Some(2));
    });
    assert_eq!(
        log.finish_notification_batch(batch).unwrap(),
        (ConfirmedAuditNotificationOutcome::Stored, true)
    );
    assert_eq!(log.generation(), generation + 2);
    assert_eq!(storage.committed().records.len(), 2);
    assert_eq!(storage.committed().completed_receipts.len(), 1);
}

#[test]
fn a_duplicate_or_empty_change_stages_nothing() {
    let (mut log, storage) = log();
    let batch =
        staged(log.stage_notification_batch(&[notification(1)], 3_000, Some(receipt(b"once"))));
    block_on(batch.saved());
    log.finish_notification_batch(batch).unwrap();
    let commits = storage.commits.load(Ordering::SeqCst);
    assert!(matches!(
        log.stage_notification_batch(&[notification(1)], 3_000, Some(receipt(b"once"))),
        Ok(AuditBatchStage::Done(
            ConfirmedAuditNotificationOutcome::Duplicate,
            false
        ))
    ));
    // An unconfirmed copy of a complete record changes nothing.
    assert!(matches!(
        log.stage_notification_batch(&[notification(1)], 3_000, None),
        Ok(AuditBatchStage::Done(
            ConfirmedAuditNotificationOutcome::Stored,
            false
        ))
    ));
    log.wait_for_commits();
    assert_eq!(storage.commits.load(Ordering::SeqCst), commits);
}

pub(super) fn staged_write(step: StageStep) -> SaveWait {
    match step {
        StageStep::Staged(wait) => wait,
        other => panic!("expected a staged write, got {other:?}"),
    }
}

#[test]
fn a_staged_log_enable_write_commits_off_the_lock_and_is_taken_by_the_write() {
    let (mut log, storage) = log();
    let generation = log.generation();
    let (started, go) = storage.hold();
    let wait = staged_write(log.stage_write(
        PropertyIdentifier::LOG_ENABLE,
        None,
        &PropertyValue::Boolean(false),
    ));
    started.recv_timeout(WAIT).unwrap();
    assert!(log.log_enable());
    assert_eq!(
        log.read_property(PropertyIdentifier::LOG_ENABLE, None)
            .unwrap(),
        PropertyValue::Boolean(true)
    );
    go.send(()).unwrap();
    block_on(wait.clone());
    log.write_property(
        PropertyIdentifier::LOG_ENABLE,
        None,
        PropertyValue::Boolean(false),
        None,
    )
    .unwrap();
    log.release_staged_write(&wait);
    assert!(!log.log_enable());
    assert_eq!(log.generation(), generation + 1);
    assert_eq!(storage.commits.load(Ordering::SeqCst), 2);
    assert!(!storage.committed().log_enable);
}

#[test]
fn a_staged_log_enable_write_its_request_dropped_leaves_storage_with_the_served_log() {
    let (mut log, storage) = log();
    let generation = log.generation();
    let wait = staged_write(log.stage_write(
        PropertyIdentifier::LOG_ENABLE,
        None,
        &PropertyValue::Boolean(false),
    ));
    block_on(wait.clone());
    assert!(!storage.committed().log_enable);
    // The request failed before its write reached the log.
    log.release_staged_write(&wait);
    log.wait_for_commits();
    let committed = storage.committed();
    assert!(committed.log_enable);
    assert!(committed.records.is_empty());
    assert!(log.log_enable());
    // A reload serves what the log served, and the next commit follows on.
    drop(log);
    let mut reopened = AuditLogObject::new(1, "audit", 10, storage.clone()).unwrap();
    reopened.bind_clock_internal(Some(Arc::new(FixedClock)));
    assert!(reopened.log_enable());
    assert_eq!(reopened.generation(), generation + 1);
    reopened
        .write_property(
            PropertyIdentifier::LOG_ENABLE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();
    assert_eq!(storage.committed().generation, generation + 2);
}

#[test]
fn a_finished_batch_its_request_forgot_lands_from_the_operation_task() {
    let (mut log, _storage) = log();
    let batch = staged(log.stage_notification_batch(&[notification(1)], 3_000, None));
    block_on(batch.saved());
    assert!(log.records().is_empty());
    log.advance_monotonic_time_internal(Duration::ZERO);
    assert_eq!(log.records().len(), 1);
    // A request that comes back late still gets its outcome.
    assert_eq!(
        log.finish_notification_batch(batch).unwrap(),
        (ConfirmedAuditNotificationOutcome::Stored, true)
    );
}

#[test]
fn staging_skips_a_write_with_nothing_to_commit() {
    let (mut log, storage) = log();
    let skipped = |step| matches!(step, StageStep::Skip);
    assert!(skipped(log.stage_write(
        PropertyIdentifier::LOG_ENABLE,
        None,
        &PropertyValue::Boolean(true),
    )));
    assert!(skipped(log.stage_write(
        PropertyIdentifier::LOG_ENABLE,
        None,
        &PropertyValue::Unsigned(0),
    )));
    assert!(skipped(log.stage_write(
        PropertyIdentifier::DESCRIPTION,
        None,
        &PropertyValue::CharacterString("x".into()),
    )));
    log.wait_for_commits();
    assert_eq!(storage.commits.load(Ordering::SeqCst), 1);
}

// A log can go while changes are still staged for a request that never came
// back, as when the server stops mid-request and the database is dropped
// (#1363). Storage then goes back to the log as it was served.

fn reopen(storage: &Arc<SlowPersistence>) -> AuditLogObject {
    AuditLogObject::new(1, "audit", 10, storage.clone()).unwrap()
}

fn stage_log_disable(log: &mut AuditLogObject) -> SaveWait {
    staged_write(log.stage_write(
        PropertyIdentifier::LOG_ENABLE,
        None,
        &PropertyValue::Boolean(false),
    ))
}

#[test]
fn a_log_dropped_with_a_staged_change_puts_storage_back_to_the_served_log() {
    let (mut log, storage) = log();
    let generation = log.generation();
    let wait = stage_log_disable(&mut log);
    block_on(wait);
    assert!(!storage.committed().log_enable);
    // The log goes before any lifetime check could drop the change.
    drop(log);
    let committed = storage.committed();
    assert!(committed.log_enable);
    assert!(committed.records.is_empty());
    // The first commit, the staged one and the put-back.
    assert_eq!(storage.commits.load(Ordering::SeqCst), 3);
    let reopened = reopen(&storage);
    assert!(reopened.log_enable());
    assert_eq!(reopened.generation(), generation + 1);
}

#[test]
fn a_log_dropped_after_its_staged_change_was_made_keeps_it() {
    let (mut log, storage) = log();
    let wait = stage_log_disable(&mut log);
    block_on(wait.clone());
    log.write_property(
        PropertyIdentifier::LOG_ENABLE,
        None,
        PropertyValue::Boolean(false),
        None,
    )
    .unwrap();
    log.release_staged_write(&wait);
    drop(log);
    assert_eq!(storage.commits.load(Ordering::SeqCst), 2);
    assert!(!reopen(&storage).log_enable());
}

#[test]
fn a_log_dropped_with_a_staged_change_whose_commit_failed_leaves_storage_alone() {
    let (mut log, storage) = log();
    storage.fail.store(true, Ordering::SeqCst);
    let wait = stage_log_disable(&mut log);
    block_on(wait);
    // Storage works again, so a commit at the drop would land and count.
    storage.fail.store(false, Ordering::SeqCst);
    drop(log);
    assert_eq!(storage.commits.load(Ordering::SeqCst), 1);
    assert!(reopen(&storage).log_enable());
}

#[test]
fn a_log_dropped_part_way_through_staged_changes_keeps_the_steps_taken() {
    let (mut log, storage) = log();
    let generation = log.generation();
    let wait = staged_write(log.stage_writes(&[
        PendingWrite {
            property: PropertyIdentifier::LOG_ENABLE,
            array_index: None,
            value: PropertyValue::Boolean(false),
        },
        PendingWrite {
            property: PropertyIdentifier::BUFFER_SIZE,
            array_index: None,
            value: PropertyValue::Unsigned(5),
        },
    ]));
    block_on(wait);
    assert_eq!(storage.committed().capacity, 5);
    // The request makes its first write, then goes before its second.
    log.write_property(
        PropertyIdentifier::LOG_ENABLE,
        None,
        PropertyValue::Boolean(false),
        None,
    )
    .unwrap();
    drop(log);
    let reopened = reopen(&storage);
    assert!(!reopened.log_enable());
    assert_eq!(reopened.buffer_size(), 10);
    assert_eq!(reopened.generation(), generation + 2);
}

#[test]
fn a_log_dropped_while_its_staged_commit_runs_puts_storage_back_after_it() {
    let (mut log, storage) = log();
    let (started, go) = storage.hold();
    let _forgotten = stage_log_disable(&mut log);
    started.recv_timeout(WAIT).unwrap();
    // The drop waits for the commits it queues, so it runs on a thread of
    // its own while the staged commit is held.
    let dropping = std::thread::spawn(move || drop(log));
    drop(go);
    dropping.join().unwrap();
    assert!(storage.committed().log_enable);
    assert_eq!(storage.commits.load(Ordering::SeqCst), 3);
}

#[test]
fn a_log_dropped_with_a_committed_batch_keeps_it_as_settling_would() {
    let (mut log, storage) = log();
    let batch = staged(log.stage_notification_batch(&[notification(1)], 3_000, None));
    block_on(batch.saved());
    assert!(log.records().is_empty());
    // Settling takes a batch whose commit ran; storage already holds it,
    // so the drop leaves it there.
    drop(log);
    assert_eq!(storage.commits.load(Ordering::SeqCst), 2);
    assert_eq!(reopen(&storage).records().len(), 1);
}

#[test]
fn settling_forgotten_writes_puts_storage_back_and_takes_a_batch_once_committed() {
    let (mut log, storage) = log();
    let wait = stage_log_disable(&mut log);
    block_on(wait);
    // What the server's stop() does once it has joined every request.
    block_on(log.settle_forgotten_writes().expect("the log commits"));
    assert!(storage.committed().log_enable);
    assert!(log.log_enable());
    // A batch whose commit still runs is left to land; settling again once
    // it has landed takes it.
    let (started, go) = storage.hold();
    let _batch = staged(log.stage_notification_batch(&[notification(1)], 3_000, None));
    started.recv_timeout(WAIT).unwrap();
    let landing = log.settle_forgotten_writes().unwrap();
    assert!(log.records().is_empty());
    drop(go);
    block_on(landing);
    assert_eq!(storage.committed().records.len(), 1);
    assert!(log.records().is_empty());
    block_on(log.settle_forgotten_writes().unwrap());
    assert_eq!(log.records().len(), 1);
}
