//! The reporting settings a write sets on a durable log (#1952): each one
//! commits before the log serves it and comes back on reopening; a saved
//! one wins over its setter until `clear_written_reporting`; a staged write
//! holds the cadence back and puts storage back when its request never
//! makes it.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use bacnet_encoding::constructed::encode_object_property_reference;
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::constructed::{
    BACnetEventLogRecord, BACnetLogMultipleRecord, BACnetLogRecord, BACnetObjectPropertyReference,
};
use bacnet_types::enums::{ErrorClass, ErrorCode, NotifyType, ObjectType, PropertyIdentifier as P};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;

use super::store::DurableLog;
use super::tests::{MemoryLog, trend_record};
use super::*;
use crate::durable::{DurableWrites, PendingWrite, STAGED_WRITE_LIFETIME, SaveWait, StageStep};
use crate::event_log::EventLogObject;
use crate::traits::BACnetObject;
use crate::trend::{TrendLogMultipleObject, TrendLogObject};

fn trend_log(storage: &Arc<MemoryLog<BACnetLogRecord>>) -> TrendLogObject {
    TrendLogObject::with_persistence(1, "TL", 8, storage.clone()).unwrap()
}

fn written_reference() -> PropertyValue {
    let mut octets = BytesMut::new();
    encode_object_property_reference(
        &mut octets,
        &BACnetObjectPropertyReference::new(
            ObjectIdentifier::new(ObjectType::BINARY_VALUE, 3).unwrap(),
            P::PRESENT_VALUE.to_raw(),
        ),
    );
    PropertyValue::ApplicationData(octets.to_vec())
}

fn texts(texts: [&str; 3]) -> PropertyValue {
    PropertyValue::List(
        texts
            .iter()
            .map(|text| PropertyValue::CharacterString((*text).into()))
            .collect(),
    )
}

/// Every setting each log keeps: the property, an array index, a value no
/// new log holds, and whether only a Trend Log has it.
fn covered() -> Vec<(P, Option<u32>, PropertyValue, bool)> {
    vec![
        (
            P::NOTIFICATION_CLASS,
            None,
            PropertyValue::Unsigned(9),
            false,
        ),
        (
            P::EVENT_ENABLE,
            None,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![EventTransitionBits::TO_FAULT.to_bacnet()],
            },
            false,
        ),
        (
            P::NOTIFY_TYPE,
            None,
            PropertyValue::Enumerated(NotifyType::ALARM.to_raw()),
            false,
        ),
        (
            P::EVENT_DETECTION_ENABLE,
            None,
            PropertyValue::Boolean(false),
            false,
        ),
        (
            P::EVENT_MESSAGE_TEXTS_CONFIG,
            None,
            texts(["off", "fault", "back"]),
            false,
        ),
        (
            P::EVENT_MESSAGE_TEXTS_CONFIG,
            Some(2),
            PropertyValue::CharacterString("fault".into()),
            false,
        ),
        (
            P::EVENT_ALGORITHM_INHIBIT_REF,
            None,
            written_reference(),
            false,
        ),
        (
            P::EVENT_ALGORITHM_INHIBIT,
            None,
            PropertyValue::Boolean(true),
            false,
        ),
        (
            P::CLIENT_COV_INCREMENT,
            None,
            PropertyValue::Real(0.25),
            true,
        ),
        (
            P::COV_RESUBSCRIPTION_INTERVAL,
            None,
            PropertyValue::Unsigned(120),
            true,
        ),
    ]
}

/// One covered write on a log `open` builds over `storage`: it commits once,
/// before it answers, and the log reopened on the storage serves it.
fn written_then_reopened<R: Clone + Send + Sync + 'static>(
    storage: &Arc<MemoryLog<R>>,
    open: impl Fn() -> Box<dyn BACnetObject>,
    (property, index, value): (P, Option<u32>, PropertyValue),
) {
    let mut log = open();
    let before = log.read_property(property, None).unwrap();
    log.write_property(property, index, value, None).unwrap();
    assert_eq!(storage.commits(), 1, "{property:?} {index:?}");
    let written = log.read_property(property, None).unwrap();
    assert_ne!(written, before, "{property:?} {index:?} changed nothing");
    drop(log);
    assert_eq!(storage.commits(), 1, "{property:?}: the drop committed");
    let reopened = open();
    assert_eq!(
        reopened.read_property(property, None).unwrap(),
        written,
        "{property:?} {index:?} after reopening"
    );
}

#[test]
fn each_written_setting_survives_a_reopen_on_each_log() {
    for (property, index, value, trend_only) in covered() {
        let case = (property, index, value);
        let storage = MemoryLog::<BACnetLogRecord>::new();
        written_then_reopened(&storage, || Box::new(trend_log(&storage)), case.clone());
        if trend_only {
            continue;
        }
        let storage = MemoryLog::<BACnetLogMultipleRecord>::new();
        written_then_reopened(
            &storage,
            || {
                Box::new(
                    TrendLogMultipleObject::with_persistence(1, "TLM", 8, storage.clone()).unwrap(),
                )
            },
            case.clone(),
        );
        let storage = MemoryLog::<BACnetEventLogRecord>::new();
        written_then_reopened(
            &storage,
            || Box::new(EventLogObject::with_persistence(1, "EL", 8, storage.clone()).unwrap()),
            case,
        );
    }
}

fn operational_problem(result: &Result<(), Error>) -> bool {
    matches!(result, Err(Error::Remote(RemoteError::Error { class, code }))
        if *class == ErrorClass::DEVICE && *code == ErrorCode::OPERATIONAL_PROBLEM)
}

#[test]
fn a_write_whose_commit_fails_is_refused_and_the_old_value_stays() {
    for (property, index, value, _) in covered() {
        let storage = MemoryLog::<BACnetLogRecord>::new();
        let mut log = trend_log(&storage);
        let before = log.read_property(property, None).unwrap();
        storage.fail_next.store(true, Ordering::SeqCst);
        let result = log.write_property(property, index, value, None);
        assert!(operational_problem(&result), "{property:?}: {result:?}");
        assert_eq!(log.read_property(property, None).unwrap(), before);
        assert_eq!(log.written_reporting(), LogReportingSettings::default());
    }
}

/// A write the log refuses commits nothing.
#[test]
fn a_refused_write_commits_nothing() {
    let storage = MemoryLog::<BACnetLogRecord>::new();
    let mut log = trend_log(&storage);
    let refused = log.write_property(
        P::COV_RESUBSCRIPTION_INTERVAL,
        None,
        PropertyValue::Unsigned(0),
        None,
    );
    assert!(refused.is_err());
    assert!(
        log.write_property(P::NOTIFY_TYPE, None, PropertyValue::Enumerated(9), None)
            .is_err()
    );
    assert_eq!(storage.commits(), 0);
}

#[test]
fn a_written_setting_wins_over_its_setter_until_cleared() {
    let storage = MemoryLog::<BACnetLogRecord>::new();
    let mut log = trend_log(&storage);
    log.write_property(
        P::NOTIFICATION_CLASS,
        None,
        PropertyValue::Unsigned(9),
        None,
    )
    .unwrap();
    log.write_property(P::CLIENT_COV_INCREMENT, None, PropertyValue::Null, None)
        .unwrap();
    drop(log);

    // An application configuring the log at start keeps the peer's writes.
    let mut log = trend_log(&storage);
    log.set_notification_class(3);
    log.set_client_cov_increment(Some(2.0)).unwrap();
    // The argument is still checked.
    assert!(log.set_client_cov_increment(Some(-1.0)).is_err());
    // A setting no write set still takes its setter.
    log.set_notify_type(NotifyType::ALARM);
    let read = |log: &TrendLogObject, property| log.read_property(property, None).unwrap();
    assert_eq!(
        read(&log, P::NOTIFICATION_CLASS),
        PropertyValue::Unsigned(9)
    );
    assert_eq!(read(&log, P::CLIENT_COV_INCREMENT), PropertyValue::Null);
    assert_eq!(read(&log, P::NOTIFY_TYPE), PropertyValue::Enumerated(0));
    let written = log.written_reporting();
    assert_eq!(written.notification_class, Some(9));
    assert_eq!(written.client_cov_increment, Some(None));
    assert_eq!(written.notify_type, None);

    // Clearing commits the log without them; the values served stay until
    // the setters change them.
    let commits = storage.commits();
    log.clear_written_reporting().unwrap();
    assert_eq!(storage.commits(), commits + 1);
    let stored = storage.stored.lock().unwrap().clone().unwrap();
    assert_eq!(stored.reporting, LogReportingSettings::default());
    assert_eq!(
        read(&log, P::NOTIFICATION_CLASS),
        PropertyValue::Unsigned(9)
    );
    log.set_notification_class(3);
    log.set_client_cov_increment(Some(2.0)).unwrap();
    assert_eq!(
        read(&log, P::NOTIFICATION_CLASS),
        PropertyValue::Unsigned(3)
    );
    assert_eq!(
        read(&log, P::CLIENT_COV_INCREMENT),
        PropertyValue::Real(2.0)
    );
    drop(log);
    // Nothing written comes back: the application's values apply.
    let log = trend_log(&storage);
    assert_eq!(
        read(&log, P::NOTIFICATION_CLASS),
        PropertyValue::Unsigned(0)
    );
}

#[test]
fn a_clear_whose_commit_fails_keeps_the_written_settings() {
    let storage = MemoryLog::<BACnetLogRecord>::new();
    let mut log = trend_log(&storage);
    log.write_property(
        P::NOTIFICATION_CLASS,
        None,
        PropertyValue::Unsigned(9),
        None,
    )
    .unwrap();
    storage.fail_next.store(true, Ordering::SeqCst);
    assert!(log.clear_written_reporting().is_err());
    assert_eq!(log.written_reporting().notification_class, Some(9));
    log.set_notification_class(3);
    assert_eq!(
        log.read_property(P::NOTIFICATION_CLASS, None).unwrap(),
        PropertyValue::Unsigned(9)
    );
}

/// The setter rule holds on every log, and only with storage.
#[test]
fn the_setter_rule_needs_storage_and_holds_on_each_log() {
    let class = |log: &dyn BACnetObject| log.read_property(P::NOTIFICATION_CLASS, None).unwrap();
    let nine = PropertyValue::Unsigned(9);
    let write = |log: &mut dyn BACnetObject| {
        log.write_property(P::NOTIFICATION_CLASS, None, nine.clone(), None)
            .unwrap();
    };

    let mut plain = TrendLogObject::new(1, "TL", 8).unwrap();
    write(&mut plain);
    plain.set_notification_class(3);
    assert_eq!(class(&plain), PropertyValue::Unsigned(3));
    plain.clear_written_reporting().unwrap();
    assert_eq!(plain.written_reporting(), LogReportingSettings::default());

    let storage = MemoryLog::<BACnetLogMultipleRecord>::new();
    let mut multiple = TrendLogMultipleObject::with_persistence(1, "TLM", 8, storage).unwrap();
    write(&mut multiple);
    multiple.set_notification_class(3);
    assert_eq!(class(&multiple), nine);
    multiple.clear_written_reporting().unwrap();
    multiple.set_notification_class(3);
    assert_eq!(class(&multiple), PropertyValue::Unsigned(3));

    let storage = MemoryLog::<BACnetEventLogRecord>::new();
    let mut event = EventLogObject::with_persistence(1, "EL", 8, storage).unwrap();
    write(&mut event);
    event.set_event_enable(EventTransitionBits::empty());
    event.set_notification_class(3);
    assert_eq!(class(&event), nine);
    event.clear_written_reporting().unwrap();
    event.set_notification_class(3);
    assert_eq!(class(&event), PropertyValue::Unsigned(3));
}

fn class_write(class: u64) -> PendingWrite {
    PendingWrite {
        property: P::NOTIFICATION_CLASS,
        array_index: None,
        value: PropertyValue::Unsigned(class),
    }
}

fn staged(step: StageStep) -> SaveWait {
    match step {
        StageStep::Staged(wait) => wait,
        other => panic!("expected a staged write, got {other:?}"),
    }
}

fn stored_class(storage: &MemoryLog<BACnetLogRecord>) -> Option<u32> {
    storage
        .stored
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|snapshot| snapshot.reporting.notification_class)
}

/// Wait for the commits the log has queued.
fn settled(log: &TrendLogObject) {
    DurableLog::store(log).unwrap().idle().block();
}

/// A staged write commits before the log serves it; the cadence holds back
/// while it is staged, and the write itself commits nothing more.
#[test]
fn a_staged_write_commits_first_and_the_cadence_waits_for_it() {
    let storage = MemoryLog::<BACnetLogRecord>::new();
    let mut log = trend_log(&storage);
    log.set_commit_cadence(LogCommitCadence {
        records: 1,
        interval: core::time::Duration::from_secs(1),
    });
    let wait = staged(log.stage_writes(&[class_write(9)]));
    wait.block();
    assert_eq!(storage.commits(), 1);
    assert_eq!(stored_class(&storage), Some(9));
    assert_eq!(
        log.read_property(P::NOTIFICATION_CLASS, None).unwrap(),
        PropertyValue::Unsigned(0)
    );
    assert!(log.has_staged_write());
    // A record due a commit by count waits for the staged write, and so
    // does a change at the end of the commit interval.
    log.add_record(trend_record(1)).unwrap();
    for seconds in [10, 20] {
        crate::traits::Timed::advance_monotonic_time(
            &mut log,
            core::time::Duration::from_secs(seconds),
        );
    }
    settled(&log);
    assert_eq!(storage.commits(), 1);

    log.write_property(
        P::NOTIFICATION_CLASS,
        None,
        PropertyValue::Unsigned(9),
        None,
    )
    .unwrap();
    log.release_staged_write(&wait);
    settled(&log);
    assert_eq!(storage.commits(), 1);
    assert!(!log.has_staged_write());
    // The cadence goes on, keeping the written setting.
    log.add_record(trend_record(2)).unwrap();
    settled(&log);
    assert_eq!(storage.commits(), 2);
    let stored = storage.stored.lock().unwrap().clone().unwrap();
    assert_eq!(stored.total_record_count, 2);
    assert_eq!(stored.reporting.notification_class, Some(9));
}

/// What the application sets between a stage and its commit, the log
/// commits at its next look, as any change the cadence holds back
/// (#2431): the staged commit holds the old Notification_Threshold, and
/// the interval's commit after the release holds the new one beside the
/// written class.
#[test]
fn a_setter_between_a_stage_and_its_commit_commits_at_the_next_look() {
    let storage = MemoryLog::<BACnetLogRecord>::new();
    let mut log = trend_log(&storage);
    log.set_commit_cadence(LogCommitCadence {
        records: 0,
        interval: core::time::Duration::from_secs(1),
    });
    let wait = staged(log.stage_writes(&[class_write(9)]));
    wait.block();
    log.set_notification_threshold(7);
    log.write_property(
        P::NOTIFICATION_CLASS,
        None,
        PropertyValue::Unsigned(9),
        None,
    )
    .unwrap();
    log.release_staged_write(&wait);
    settled(&log);
    let stored = || storage.stored.lock().unwrap().clone().unwrap();
    assert_eq!(storage.commits(), 1);
    assert_eq!(stored().notification_threshold, 0);
    for seconds in [10, 20] {
        crate::traits::Timed::advance_monotonic_time(
            &mut log,
            core::time::Duration::from_secs(seconds),
        );
    }
    settled(&log);
    assert_eq!(storage.commits(), 2);
    assert_eq!(stored().notification_threshold, 7);
    assert_eq!(stored().reporting.notification_class, Some(9));
}

/// A WritePropertyMultiple's settings commit once, together.
#[test]
fn a_requests_writes_to_one_log_commit_once() {
    let storage = MemoryLog::<BACnetLogRecord>::new();
    let mut log = trend_log(&storage);
    let alarm = PendingWrite {
        property: P::NOTIFY_TYPE,
        array_index: None,
        value: PropertyValue::Enumerated(NotifyType::ALARM.to_raw()),
    };
    let wait = staged(log.stage_writes(&[class_write(9), alarm.clone()]));
    wait.block();
    for write in [class_write(9), alarm] {
        log.write_property(write.property, None, write.value, None)
            .unwrap();
    }
    log.release_staged_write(&wait);
    settled(&log);
    assert_eq!(storage.commits(), 1);
    let stored = storage.stored.lock().unwrap().clone().unwrap();
    assert_eq!(stored.reporting.notification_class, Some(9));
    assert_eq!(stored.reporting.notify_type, Some(NotifyType::ALARM));
}

#[test]
fn a_staged_write_whose_commit_fails_is_refused() {
    let storage = MemoryLog::<BACnetLogRecord>::new();
    let mut log = trend_log(&storage);
    storage.fail_next.store(true, Ordering::SeqCst);
    let wait = staged(log.stage_writes(&[class_write(9)]));
    wait.block();
    let result = log.write_property(
        P::NOTIFICATION_CLASS,
        None,
        PropertyValue::Unsigned(9),
        None,
    );
    assert!(operational_problem(&result), "{result:?}");
    assert_eq!(
        log.read_property(P::NOTIFICATION_CLASS, None).unwrap(),
        PropertyValue::Unsigned(0)
    );
    log.release_staged_write(&wait);
    assert_eq!(log.written_reporting(), LogReportingSettings::default());
}

/// Another request finds the log busy while a write is staged.
#[test]
fn a_second_request_waits_for_a_staged_write() {
    let storage = MemoryLog::<BACnetLogRecord>::new();
    let mut log = trend_log(&storage);
    let wait = staged(log.stage_writes(&[class_write(9)]));
    wait.block();
    assert!(matches!(
        log.stage_writes(&[class_write(4)]),
        StageStep::Busy(_)
    ));
    log.release_staged_write(&wait);
    assert!(matches!(
        log.stage_writes(&[class_write(4)]),
        StageStep::Staged(_)
    ));
}

/// A staged write its request never makes leaves storage ahead of the log
/// until the release, the expiry or the drop puts it back.
#[test]
fn a_staged_write_never_made_puts_storage_back() {
    // Released by its request.
    let storage = MemoryLog::<BACnetLogRecord>::new();
    let mut log = trend_log(&storage);
    let wait = staged(log.stage_writes(&[class_write(9)]));
    wait.block();
    assert_eq!(stored_class(&storage), Some(9));
    log.release_staged_write(&wait);
    settled(&log);
    assert_eq!(stored_class(&storage), None);
    assert_eq!(storage.commits(), 2);

    // Outlived on the operation task's clock.
    let storage = MemoryLog::<BACnetLogRecord>::new();
    let mut log = trend_log(&storage);
    staged(log.stage_writes(&[class_write(9)])).block();
    let start = core::time::Duration::from_secs(5);
    log.expire_staged(start);
    assert!(log.has_staged_write());
    log.expire_staged(start + STAGED_WRITE_LIFETIME);
    assert!(!log.has_staged_write());
    settled(&log);
    assert_eq!(stored_class(&storage), None);

    // Dropped with the log.
    let storage = MemoryLog::<BACnetLogRecord>::new();
    let mut log = trend_log(&storage);
    staged(log.stage_writes(&[class_write(9)])).block();
    drop(log);
    assert_eq!(stored_class(&storage), None);

    // Settled as the server stops.
    let storage = MemoryLog::<BACnetLogRecord>::new();
    let mut log = trend_log(&storage);
    staged(log.stage_writes(&[class_write(9)])).block();
    log.settle_forgotten_writes().unwrap().block();
    assert_eq!(stored_class(&storage), None);
    assert!(!log.has_staged_write());
}

/// Writes that aren't the log's to keep stage nothing, and a log without
/// storage stages nothing at all.
#[test]
fn only_kept_settings_stage() {
    let storage = MemoryLog::<BACnetLogRecord>::new();
    let mut log = trend_log(&storage);
    let threshold = PendingWrite {
        property: P::NOTIFICATION_THRESHOLD,
        array_index: None,
        value: PropertyValue::Unsigned(3),
    };
    assert!(matches!(log.stage_writes(&[threshold]), StageStep::Skip));
    // A refused write stages nothing either.
    let refused = PendingWrite {
        property: P::NOTIFICATION_CLASS,
        array_index: Some(1),
        value: PropertyValue::Unsigned(3),
    };
    assert!(matches!(log.stage_writes(&[refused]), StageStep::Skip));
    assert_eq!(storage.commits(), 0);
    let storage = MemoryLog::<BACnetLogMultipleRecord>::new();
    let mut multiple = TrendLogMultipleObject::with_persistence(1, "TLM", 8, storage).unwrap();
    let increment = PendingWrite {
        property: P::CLIENT_COV_INCREMENT,
        array_index: None,
        value: PropertyValue::Real(1.0),
    };
    assert!(matches!(
        multiple.stage_writes(&[increment]),
        StageStep::Skip
    ));
}

/// Event_Detection_Enable written FALSE and then TRUE again restarts the
/// BUFFER_READY counts in the commit, as on the log.
#[test]
fn detection_turned_back_on_commits_the_restarted_counts() {
    let storage = MemoryLog::<BACnetLogRecord>::new();
    let mut log = trend_log(&storage);
    log.write_property(
        P::EVENT_DETECTION_ENABLE,
        None,
        PropertyValue::Boolean(false),
        None,
    )
    .unwrap();
    for hour in 1..4 {
        log.add_record(trend_record(hour)).unwrap();
    }
    let on = PendingWrite {
        property: P::EVENT_DETECTION_ENABLE,
        array_index: None,
        value: PropertyValue::Boolean(true),
    };
    staged(log.stage_writes(&[on])).block();
    let stored = storage.stored.lock().unwrap().clone().unwrap();
    assert_eq!(stored.reporting.event_detection_enable, Some(true));
    assert_eq!(
        (
            stored.last_notify_record,
            stored.records_since_notification_from
        ),
        (3, 3)
    );
}

/// A write nobody staged supersedes another request's staged write: storage
/// first goes back to the served settings, then takes the new one, and the
/// staged request's own write later commits in place.
#[test]
fn a_write_nobody_staged_supersedes_a_staged_one() {
    let storage = MemoryLog::<BACnetLogRecord>::new();
    let mut log = trend_log(&storage);
    let wait = staged(log.stage_writes(&[class_write(9)]));
    wait.block();
    log.write_property(
        P::NOTIFICATION_CLASS,
        None,
        PropertyValue::Unsigned(4),
        None,
    )
    .unwrap();
    // The staged commit, the one putting storage back, and the new write's.
    assert_eq!(storage.commits(), 3);
    assert_eq!(stored_class(&storage), Some(4));
    assert!(!log.has_staged_write());
    log.write_property(
        P::NOTIFICATION_CLASS,
        None,
        PropertyValue::Unsigned(9),
        None,
    )
    .unwrap();
    log.release_staged_write(&wait);
    settled(&log);
    assert_eq!(storage.commits(), 4);
    assert_eq!(stored_class(&storage), Some(9));
    assert_eq!(
        log.read_property(P::NOTIFICATION_CLASS, None).unwrap(),
        PropertyValue::Unsigned(9)
    );
}

fn no_space(result: &Result<(), Error>) -> bool {
    matches!(result, Err(Error::Remote(RemoteError::Error { class, code }))
        if *class == ErrorClass::RESOURCES && *code == ErrorCode::NO_SPACE_TO_WRITE_PROPERTY)
}

/// A message text past what a snapshot keeps is refused as it is written,
/// before anything stages or commits, with storage that never encodes a
/// snapshot as with the file backend; the log then still reopens.
#[test]
fn an_over_long_message_text_is_refused_with_any_storage() {
    let long = "x".repeat(16 * 1024 + 1);
    let fits = "y".repeat(16 * 1024);
    let texts_property = P::EVENT_MESSAGE_TEXTS_CONFIG;
    let storage = MemoryLog::<BACnetLogRecord>::new();
    let mut log = trend_log(&storage);
    let whole = texts([long.as_str(), "", ""]);
    assert!(no_space(&log.write_property(
        texts_property,
        None,
        whole,
        None
    )));
    let element = PropertyValue::CharacterString(long.clone());
    assert!(no_space(&log.write_property(
        texts_property,
        Some(2),
        element.clone(),
        None
    )));
    let staged_write = PendingWrite {
        property: texts_property,
        array_index: Some(2),
        value: element,
    };
    assert!(matches!(log.stage_writes(&[staged_write]), StageStep::Skip));
    assert_eq!(storage.commits(), 0);
    let fitting = PropertyValue::CharacterString(fits);
    log.write_property(texts_property, Some(1), fitting.clone(), None)
        .unwrap();
    drop(log);
    let reopened = trend_log(&storage);
    assert_eq!(
        reopened.read_property(texts_property, Some(1)).unwrap(),
        fitting
    );

    // std: the file backend gives the same answer.
    #[cfg(feature = "std")]
    {
        let base = super::tests::TempBase::new("long-text");
        let files = Arc::new(FileLogPersistence::new(base.path()).unwrap());
        let mut log = EventLogObject::with_persistence(1, "EL", 8, files).unwrap();
        let element = PropertyValue::CharacterString(long);
        assert!(no_space(&log.write_property(
            texts_property,
            Some(3),
            element,
            None
        )));
    }
}
