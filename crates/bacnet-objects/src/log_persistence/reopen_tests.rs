//! Reopening a log from its snapshot (#1541): the buffer, the numbering and
//! the settings come back, one LOG_INTERRUPTED record follows, a damaged
//! newer slot falls back to the older one, and BUFFER_READY carries on from
//! the saved Last_Notify_Record.

use std::path::Path;
use std::sync::Arc;

use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::{BACnetLogRecord, LogDatum};
use bacnet_types::enums::{LoggingType, ObjectType, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;
use bytes::BytesMut;

use super::tests::{
    DATE, HandClock, TempBase, at, event_record, multiple_record, reference, trend_record,
};
use super::*;
use crate::event::commit_test_proposal;
use crate::event_log::EventLogObject;
use crate::log_persistence::store::DurableLog;
use crate::traits::BACnetObject;
use crate::trend::{TrendLogMultipleObject, TrendLogObject};

#[derive(Clone, Copy, Debug)]
pub(super) enum Kind {
    Trend,
    Event,
    Multiple,
}

pub(super) const KINDS: [Kind; 3] = [Kind::Trend, Kind::Event, Kind::Multiple];

/// One log of `kind` kept at `base`, and whether it reopened.
pub(super) fn open(
    kind: Kind,
    base: &Path,
    buffer_size: u32,
) -> Result<(Box<dyn BACnetObject>, bool), Error> {
    let file = || Arc::new(FileLogPersistence::new(base).unwrap());
    Ok(match kind {
        Kind::Trend => {
            let log = TrendLogObject::with_persistence(1, "TL-1", buffer_size, file())?;
            let reopened = log.reopened();
            (Box::new(log), reopened)
        }
        Kind::Event => {
            let log = EventLogObject::with_persistence(1, "EL-1", buffer_size, file())?;
            let reopened = log.reopened();
            (Box::new(log), reopened)
        }
        Kind::Multiple => {
            let log = TrendLogMultipleObject::with_persistence(1, "TLM-1", buffer_size, file())?;
            let reopened = log.reopened();
            (Box::new(log), reopened)
        }
    })
}

pub(super) fn add(log: &mut dyn BACnetObject, hour: u8) {
    match log.object_identifier().object_type() {
        ObjectType::TREND_LOG => log.add_trend_record(trend_record(hour)),
        ObjectType::EVENT_LOG => log.add_event_log_record(event_record(hour)),
        _ => log.add_trend_multiple_record(multiple_record(hour)),
    }
    .unwrap();
}

pub(super) fn write(log: &mut dyn BACnetObject, property: P, value: PropertyValue) {
    log.write_property(property, None, value, None)
        .unwrap_or_else(|error| panic!("{property:?}: {error:?}"));
}

/// Configure what a peer could write, away from every default.
fn configure(log: &mut dyn BACnetObject) {
    let window = |hour| {
        PropertyValue::List(vec![
            PropertyValue::Date(DATE),
            PropertyValue::Time(at(hour)),
        ])
    };
    write(log, P::START_TIME, window(1));
    write(log, P::STOP_TIME, window(23));
    if log.object_identifier().object_type() == ObjectType::EVENT_LOG {
        return;
    }
    write(log, P::LOG_INTERVAL, PropertyValue::Unsigned(250));
    write(log, P::ALIGN_INTERVALS, PropertyValue::Boolean(true));
    write(log, P::INTERVAL_OFFSET, PropertyValue::Unsigned(7));
    let reference = PropertyValue::ApplicationData(encoded_reference(3));
    write(log, P::LOG_DEVICE_OBJECT_PROPERTY, reference);
}

fn encoded_reference(instance: u32) -> Vec<u8> {
    let mut buf = BytesMut::new();
    bacnet_encoding::constructed::encode_device_object_property_reference(
        &mut buf,
        &reference(instance),
    );
    buf.to_vec()
}

/// Each property a reopened log must serve as before.
const SETTINGS: [P; 13] = [
    P::BUFFER_SIZE,
    P::LOG_ENABLE,
    P::STOP_WHEN_FULL,
    P::START_TIME,
    P::STOP_TIME,
    P::LOGGING_TYPE,
    P::LOG_INTERVAL,
    P::ALIGN_INTERVALS,
    P::INTERVAL_OFFSET,
    P::LOG_DEVICE_OBJECT_PROPERTY,
    P::RECORD_COUNT,
    P::LAST_NOTIFY_RECORD,
    P::RECORDS_SINCE_NOTIFICATION,
];

fn settings(log: &dyn BACnetObject) -> Vec<Option<PropertyValue>> {
    SETTINGS
        .iter()
        .map(|&property| log.read_property(property, None).ok())
        .collect()
}

/// Every resident record as ReadRange serves it, with its sequence number.
pub(super) fn served(log: &dyn BACnetObject) -> Vec<(u64, Vec<u8>)> {
    let buffer = log.log_buffer_internal().unwrap();
    (0..buffer.record_count())
        .map(|index| {
            let mut bytes = BytesMut::new();
            buffer.encode_record(index, &mut bytes);
            (
                buffer.record_identity(index).sequence_number(),
                bytes.to_vec(),
            )
        })
        .collect()
}

pub(super) fn total(log: &dyn BACnetObject) -> u64 {
    match log.read_property(P::TOTAL_RECORD_COUNT, None).unwrap() {
        PropertyValue::Unsigned(total) => total,
        other => panic!("Total_Record_Count read {other:?}"),
    }
}

/// The octets a LOG_INTERRUPTED record stamped `DATE` at `hour` serves as:
/// the same for every family, which all put the log-status choice first
/// after the timestamp.
pub(super) fn interrupted(hour: u8) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    bacnet_encoding::constructed::encode_log_record(
        &BACnetLogRecord {
            date: DATE,
            time: at(hour),
            log_datum: LogDatum::LogStatus(LogStatus::LOG_INTERRUPTED),
            status_flags: None,
        },
        &mut bytes,
    )
    .unwrap();
    bytes.to_vec()
}

#[test]
fn a_reopened_log_serves_its_buffer_and_settings_plus_one_interruption() {
    for kind in KINDS {
        let base = TempBase::new("reopen");
        let (mut log, reopened) = open(kind, base.path(), 4).unwrap();
        assert!(!reopened, "{kind:?}");
        log.bind_clock_internal(Some(HandClock::at(8)));
        configure(log.as_mut());
        for hour in 2..8 {
            add(log.as_mut(), hour);
        }
        let before = (
            served(log.as_ref()),
            total(log.as_ref()),
            settings(log.as_ref()),
        );
        // Dropping the log commits its last state.
        drop(log);

        let (mut log, reopened) = open(kind, base.path(), 4).unwrap();
        assert!(reopened, "{kind:?}");
        assert_eq!(served(log.as_ref()), before.0, "{kind:?}");
        assert_eq!(total(log.as_ref()), before.1, "{kind:?}");
        assert_eq!(settings(log.as_ref()), before.2, "{kind:?}");

        // Joining a database with a clock appends LOG_INTERRUPTED, numbered
        // one past the saved count.
        log.bind_clock_internal(Some(HandClock::at(12)));
        let mut expected = before.0.clone();
        expected.remove(0);
        expected.push((before.1 + 1, interrupted(12)));
        assert_eq!(served(log.as_ref()), expected, "{kind:?}");
        assert_eq!(total(log.as_ref()), before.1 + 1, "{kind:?}");
        // Only once.
        log.bind_clock_internal(Some(HandClock::at(13)));
        log.refresh_log_window_internal();
        assert_eq!(total(log.as_ref()), before.1 + 1, "{kind:?}");
    }
}

#[test]
fn without_a_clock_the_first_record_lends_the_interruption_its_timestamp() {
    for kind in KINDS {
        let base = TempBase::new("no-clock");
        let (mut log, _) = open(kind, base.path(), 4).unwrap();
        add(log.as_mut(), 3);
        drop(log);

        let (mut log, _) = open(kind, base.path(), 4).unwrap();
        log.bind_clock_internal(Some(Arc::new(HandClock(std::sync::Mutex::new(None)))));
        assert_eq!(total(log.as_ref()), 1, "{kind:?}");
        add(log.as_mut(), 9);
        let records = served(log.as_ref());
        assert_eq!(records.len(), 3, "{kind:?}");
        assert_eq!(records[1], (2, interrupted(9)), "{kind:?}");
        assert_eq!(records[2].0, 3, "{kind:?}");
    }
}

#[test]
fn an_interruption_the_application_records_takes_the_owed_ones_place() {
    let base = TempBase::new("app-interruption");
    let file = || Arc::new(FileLogPersistence::new(base.path()).unwrap());
    let mut log = TrendLogObject::with_persistence(1, "TL-1", 4, file()).unwrap();
    log.add_record(trend_record(2)).unwrap();
    drop(log);
    let mut log = TrendLogObject::with_persistence(1, "TL-1", 4, file()).unwrap();
    log.record_interruption(DATE, at(5)).unwrap();
    log.bind_clock_internal(Some(HandClock::at(6)));
    assert_eq!(log.total_record_count(), 2);
}

#[test]
fn a_damaged_newer_slot_reopens_the_log_from_the_older_one() {
    for kind in KINDS {
        let base = TempBase::new("object-fallback");
        let (mut log, _) = open(kind, base.path(), 4).unwrap();
        add(log.as_mut(), 1);
        log.settle_forgotten_writes_for_test();
        let first = (served(log.as_ref()), total(log.as_ref()));
        add(log.as_mut(), 2);
        drop(log);

        let slots = FileLogPersistence::new(base.path()).unwrap().slot_paths();
        // Generation 2, the drop's commit, went to slot 0.
        let mut data = std::fs::read(&slots[0]).unwrap();
        data[30] ^= 0xFF;
        std::fs::write(&slots[0], &data).unwrap();
        let (log, reopened) = open(kind, base.path(), 4).unwrap();
        assert!(reopened, "{kind:?}");
        assert_eq!(
            (served(log.as_ref()), total(log.as_ref())),
            first,
            "{kind:?}"
        );
        drop(log);

        // With both slots damaged the log refuses to open.
        for slot in &slots {
            std::fs::write(slot, b"not a snapshot").unwrap();
        }
        let refused = open(kind, base.path(), 4).map(|_| ()).unwrap_err();
        assert!(
            matches!(refused, Error::Encoding(_)),
            "{kind:?} {refused:?}"
        );
    }
}

/// Settle the log's queued commits, as the server's stop does.
trait SettleForTest {
    fn settle_forgotten_writes_for_test(&mut self);
}

impl SettleForTest for Box<dyn BACnetObject> {
    fn settle_forgotten_writes_for_test(&mut self) {
        self.durable_writes_internal()
            .and_then(|writes| writes.settle_forgotten_writes())
            .expect("a durable log saves")
            .block();
    }
}

pub(super) fn unsigned(log: &dyn BACnetObject, property: P) -> u64 {
    match log.read_property(property, None).unwrap() {
        PropertyValue::Unsigned(value) => value,
        other => panic!("{property:?} read {other:?}"),
    }
}

#[test]
fn buffer_ready_resumes_from_the_saved_last_notify_record() {
    for kind in KINDS {
        let base = TempBase::new("buffer-ready");
        let (mut log, _) = open(kind, base.path(), 10).unwrap();
        log.bind_clock_internal(Some(HandClock::at(8)));
        write(
            log.as_mut(),
            P::NOTIFICATION_THRESHOLD,
            PropertyValue::Unsigned(3),
        );
        for hour in 1..4 {
            add(log.as_mut(), hour);
        }
        let outcome = log.tick_intrinsic_reporting().expect("a report is due");
        commit_test_proposal(log.as_mut(), outcome);
        add(log.as_mut(), 4);
        assert_eq!(unsigned(log.as_ref(), P::LAST_NOTIFY_RECORD), 3, "{kind:?}");
        assert_eq!(unsigned(log.as_ref(), P::RECORDS_SINCE_NOTIFICATION), 1);
        drop(log);

        let (mut log, _) = open(kind, base.path(), 10).unwrap();
        // Notification_Threshold is the application's to set on each start.
        write(
            log.as_mut(),
            P::NOTIFICATION_THRESHOLD,
            PropertyValue::Unsigned(3),
        );
        log.bind_clock_internal(Some(HandClock::at(9)));
        // The saved Last_Notify_Record, and LOG_INTERRUPTED counted after it.
        assert_eq!(unsigned(log.as_ref(), P::LAST_NOTIFY_RECORD), 3, "{kind:?}");
        assert_eq!(unsigned(log.as_ref(), P::RECORDS_SINCE_NOTIFICATION), 2);
        assert_eq!(log.tick_intrinsic_reporting(), None, "{kind:?}");
        add(log.as_mut(), 10);
        let outcome = log
            .tick_intrinsic_reporting()
            .expect("the third record since");
        commit_test_proposal(log.as_mut(), outcome);
        assert_eq!(
            log.buffer_ready_report_internal(),
            Some(crate::log_reporting::BufferReadyReport {
                previous_notification: 3,
                current_notification: 6,
            }),
            "{kind:?}"
        );
    }
}

#[test]
fn reopened_settings_survive_where_a_restore_would_restart_them() {
    // A Trend Log Multiple's whole reference array and TRIGGERED mode.
    let base = TempBase::new("tlm-settings");
    let file = || Arc::new(FileLogPersistence::new(base.path()).unwrap());
    let mut log = TrendLogMultipleObject::with_persistence(1, "TLM-1", 5, file()).unwrap();
    for instance in 1..4 {
        log.add_property_reference(reference(instance)).unwrap();
    }
    log.set_logging_type(LoggingType::TRIGGERED).unwrap();
    drop(log);
    let log = TrendLogMultipleObject::with_persistence(1, "TLM-1", 5, file()).unwrap();
    assert!(log.reopened());
    assert_eq!(
        log.read_property(P::LOG_DEVICE_OBJECT_PROPERTY, Some(0))
            .unwrap(),
        PropertyValue::Unsigned(3)
    );
    assert_eq!(
        log.read_property(P::LOGGING_TYPE, None).unwrap(),
        PropertyValue::Enumerated(LoggingType::TRIGGERED.to_raw())
    );
    assert_eq!(log.storage_generation(), Some(1));
}

#[test]
fn a_reopened_log_continues_the_generation_it_loaded() {
    let base = TempBase::new("generation");
    let file = || Arc::new(FileLogPersistence::new(base.path()).unwrap());
    let mut log = EventLogObject::with_persistence(1, "EL-1", 4, file()).unwrap();
    assert_eq!(log.storage_generation(), Some(0));
    log.add_record(event_record(1)).unwrap();
    log.commit_final().unwrap().block();
    assert_eq!(log.storage_generation(), Some(1));
    drop(log);
    let mut log = EventLogObject::with_persistence(1, "EL-1", 4, file()).unwrap();
    assert_eq!(log.storage_generation(), Some(1));
    log.add_record(event_record(2)).unwrap();
    log.commit_final().unwrap().block();
    assert_eq!(log.storage_generation(), Some(2));
    assert!(
        EventLogObject::new(2, "EL-2", 4)
            .unwrap()
            .storage_generation()
            .is_none()
    );
}
