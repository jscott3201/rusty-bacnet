//! When a durable log commits (#1541): after the cadence's record count,
//! after its interval when the state changed, and once more at stop or drop.
//! Time is the hand-set monotonic `now` the server's pass would give.

use std::sync::Arc;
use std::time::Duration;

use bacnet_types::constructed::{BACnetEventLogRecord, BACnetLogRecord};
use bacnet_types::enums::PropertyIdentifier as P;
use bacnet_types::primitives::PropertyValue;

use super::tests::{HandClock, MemoryLog, event_record, trend_record};
use super::*;
use crate::event_log::EventLogObject;
use crate::log_persistence::store::DurableLog;
use crate::traits::BACnetObject;
use crate::trend::{TrendLogMultipleObject, TrendLogObject};

const SECOND: Duration = Duration::from_secs(1);

fn trend_log(cadence: LogCommitCadence) -> (TrendLogObject, Arc<MemoryLog<BACnetLogRecord>>) {
    let storage = MemoryLog::new();
    let mut log = TrendLogObject::with_persistence(1, "TL-1", 500, storage.clone()).unwrap();
    log.set_commit_cadence(cadence);
    log.bind_clock_internal(Some(HandClock::at(10)));
    (log, storage)
}

/// Wait for every commit the log has queued.
fn settled(log: &mut impl DurableLog) {
    log.durable_parts().0.expect("storage").idle().block();
}

fn stored_total(storage: &MemoryLog<BACnetLogRecord>) -> Option<u32> {
    storage
        .stored
        .lock()
        .unwrap()
        .as_ref()
        .map(|snapshot| snapshot.total_record_count)
}

#[test]
fn every_n_records_commit_without_waiting_for_the_interval() {
    let (mut log, storage) = trend_log(LogCommitCadence {
        records: 3,
        interval: Duration::ZERO,
    });
    for (hour, commits) in [(1, 0), (2, 0), (3, 1), (4, 1), (5, 1), (6, 2)] {
        log.add_record(trend_record(hour)).unwrap();
        settled(&mut log);
        assert_eq!(storage.commits(), commits, "after record {hour}");
    }
    assert_eq!(stored_total(&storage), Some(6));
    // Status records count too: a purge's BUFFER_PURGED is the 7th.
    log.write_property(P::RECORD_COUNT, None, PropertyValue::Unsigned(0), None)
        .unwrap();
    log.add_record(trend_record(7)).unwrap();
    log.add_record(trend_record(8)).unwrap();
    settled(&mut log);
    assert_eq!(storage.commits(), 3);
    assert_eq!(stored_total(&storage), Some(9));
}

#[test]
fn the_interval_commits_a_changed_log_and_skips_an_unchanged_one() {
    let (mut log, storage) = trend_log(LogCommitCadence {
        records: 0,
        interval: 60 * SECOND,
    });
    // The first look starts the interval.
    log.advance_monotonic_time_internal(Duration::ZERO);
    log.add_record(trend_record(1)).unwrap();
    log.advance_monotonic_time_internal(59 * SECOND);
    settled(&mut log);
    assert_eq!(storage.commits(), 0);
    log.advance_monotonic_time_internal(60 * SECOND);
    settled(&mut log);
    assert_eq!(storage.commits(), 1);
    assert_eq!(stored_total(&storage), Some(1));

    // Nothing changed: no commit however long it waits.
    for seconds in [61, 125, 300] {
        log.advance_monotonic_time_internal(seconds * SECOND);
    }
    settled(&mut log);
    assert_eq!(storage.commits(), 1);

    // A change after a quiet spell goes at the next look; a peer's write of
    // a setting is a change too.
    log.write_property(P::LOG_INTERVAL, None, PropertyValue::Unsigned(500), None)
        .unwrap();
    log.advance_monotonic_time_internal(301 * SECOND);
    settled(&mut log);
    assert_eq!(storage.commits(), 2);
    assert_eq!(
        storage
            .stored
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .acquisition
            .unwrap()
            .log_interval,
        500
    );
}

#[test]
fn a_count_commit_restarts_the_interval() {
    let (mut log, storage) = trend_log(LogCommitCadence {
        records: 2,
        interval: 60 * SECOND,
    });
    log.advance_monotonic_time_internal(Duration::ZERO);
    log.add_record(trend_record(1)).unwrap();
    log.advance_monotonic_time_internal(50 * SECOND);
    log.add_record(trend_record(2)).unwrap();
    settled(&mut log);
    assert_eq!(storage.commits(), 1);
    // The next look, at 55 s, restarts the interval there.
    log.advance_monotonic_time_internal(55 * SECOND);
    log.add_record(trend_record(3)).unwrap();
    log.advance_monotonic_time_internal(110 * SECOND);
    settled(&mut log);
    assert_eq!(storage.commits(), 1);
    log.advance_monotonic_time_internal(115 * SECOND);
    settled(&mut log);
    assert_eq!(storage.commits(), 2);
    assert_eq!(stored_total(&storage), Some(3));
}

#[test]
fn stop_commits_once_more_and_waits_for_it() {
    let (mut log, storage) = trend_log(LogCommitCadence::default());
    log.add_record(trend_record(1)).unwrap();
    let wait = log
        .durable_writes_internal()
        .expect("a durable log settles at stop")
        .settle_forgotten_writes()
        .expect("a wait for its commit");
    wait.block();
    assert_eq!(storage.commits(), 1);
    assert_eq!(stored_total(&storage), Some(1));
    // Nothing changed since: the next stop, or the drop, commits nothing.
    log.durable_writes_internal()
        .unwrap()
        .settle_forgotten_writes()
        .unwrap()
        .block();
    drop(log);
    assert_eq!(storage.commits(), 1);
}

#[test]
fn dropping_a_changed_log_commits_it() {
    let storage = MemoryLog::<BACnetEventLogRecord>::new();
    let mut log = EventLogObject::with_persistence(1, "EL-1", 8, storage.clone()).unwrap();
    log.add_record(event_record(1)).unwrap();
    assert_eq!(log.commit_cadence(), Some(LogCommitCadence::default()));
    drop(log);
    assert_eq!(storage.commits(), 1);
    let stored = storage.stored.lock().unwrap();
    assert_eq!(stored.as_ref().unwrap().records, vec![event_record(1)]);
    assert_eq!(stored.as_ref().unwrap().generation, 1);
}

#[test]
fn a_log_without_storage_commits_nothing_and_settles_nothing() {
    let mut trend = TrendLogObject::new(1, "TL-1", 4).unwrap();
    let mut event = EventLogObject::new(1, "EL-1", 4).unwrap();
    let mut multiple = TrendLogMultipleObject::new(1, "TLM-1", 4).unwrap();
    for log in [
        &mut trend as &mut dyn BACnetObject,
        &mut event,
        &mut multiple,
    ] {
        assert!(log.durable_writes_internal().is_none());
        assert!(!log.advance_monotonic_time_internal(1000 * SECOND));
    }
    assert_eq!(trend.commit_cadence(), None);
    trend.set_commit_cadence(LogCommitCadence::default());
    assert!(!trend.reopened());
}

#[test]
fn a_failed_commit_is_retried_at_the_next_interval_pass_without_a_new_change() {
    let (mut log, storage) = trend_log(LogCommitCadence {
        records: 0,
        interval: 60 * SECOND,
    });
    log.advance_monotonic_time_internal(Duration::ZERO);
    log.add_record(trend_record(1)).unwrap();
    storage
        .fail_next
        .store(true, std::sync::atomic::Ordering::SeqCst);
    log.advance_monotonic_time_internal(60 * SECOND);
    settled(&mut log);
    assert_eq!(storage.commits(), 0);
    assert_eq!(stored_total(&storage), None);
    // The look after the failed commit restarts the interval; nothing has
    // changed since, but the state never landed, so the next pass commits.
    log.advance_monotonic_time_internal(61 * SECOND);
    log.advance_monotonic_time_internal(120 * SECOND);
    settled(&mut log);
    assert_eq!(storage.commits(), 0);
    log.advance_monotonic_time_internal(121 * SECOND);
    settled(&mut log);
    assert_eq!(storage.commits(), 1);
    assert_eq!(stored_total(&storage), Some(1));
    // Landed: no further commit without a change.
    log.advance_monotonic_time_internal(122 * SECOND);
    log.advance_monotonic_time_internal(300 * SECOND);
    settled(&mut log);
    assert_eq!(storage.commits(), 1);
}

#[test]
fn stop_retries_a_failed_commit_even_with_no_new_change() {
    let (mut log, storage) = trend_log(LogCommitCadence {
        records: 1,
        interval: Duration::ZERO,
    });
    storage
        .fail_next
        .store(true, std::sync::atomic::Ordering::SeqCst);
    log.add_record(trend_record(1)).unwrap();
    settled(&mut log);
    assert_eq!(storage.commits(), 0);
    log.durable_writes_internal()
        .unwrap()
        .settle_forgotten_writes()
        .unwrap()
        .block();
    assert_eq!(storage.commits(), 1);
    assert_eq!(stored_total(&storage), Some(1));
}
