//! The snapshot codec and the two-slot file store (#1541), and helpers the
//! reopen and cadence tests share.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventLogRecord, BACnetLogMultipleRecord,
    BACnetLogRecord, EventLogDatum, LogData, LogDatum, LogValue,
};
use bacnet_types::enums::{LoggingType, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};

use super::*;
use crate::clock::{ClockFrame, ClockReader, UNSPECIFIED_DATETIME};
use crate::durable::crc32;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh base path in its own temporary directory, removed on drop.
pub(super) struct TempBase(PathBuf);

impl TempBase {
    pub(super) fn new(label: &str) -> Self {
        let serial = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "rusty-bacnet-log-{label}-{}-{serial}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir.join("state"))
    }

    pub(super) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempBase {
    fn drop(&mut self) {
        if let Some(parent) = self.0.parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }
}

/// Storage in memory that counts its commits.
pub(super) struct MemoryLog<R> {
    pub(super) stored: Mutex<Option<LogSnapshot<R>>>,
    pub(super) commits: AtomicU64,
    /// Fail the next commit, once.
    pub(super) fail_next: std::sync::atomic::AtomicBool,
}

impl<R> MemoryLog<R> {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            stored: Mutex::new(None),
            commits: AtomicU64::new(0),
            fail_next: std::sync::atomic::AtomicBool::new(false),
        })
    }

    pub(super) fn commits(&self) -> u64 {
        self.commits.load(Ordering::SeqCst)
    }
}

impl<R: Clone + Send + Sync> LogPersistence<R> for MemoryLog<R> {
    fn load(&self, _expected: ObjectIdentifier) -> Result<Option<LogSnapshot<R>>, Error> {
        Ok(self.stored.lock().unwrap().clone())
    }

    fn commit(&self, snapshot: &LogSnapshot<R>) -> Result<(), Error> {
        if self.fail_next.swap(false, Ordering::SeqCst) {
            return Err(Error::Encoding("injected commit failure".into()));
        }
        *self.stored.lock().unwrap() = Some(snapshot.clone());
        self.commits.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

pub(super) const DATE: Date = Date {
    year: 126,
    month: 10,
    day: 20,
    day_of_week: 2,
};

pub(super) fn at(hour: u8) -> Time {
    Time {
        hour,
        minute: 15,
        second: 0,
        hundredths: 0,
    }
}

/// A valid clock reading `DATE` at `at(hour)`.
pub(super) struct HandClock(pub(super) Mutex<Option<ClockFrame>>);

impl HandClock {
    pub(super) fn at(hour: u8) -> Arc<Self> {
        let clock = Arc::new(Self(Mutex::new(None)));
        clock.set(Some(hour));
        clock
    }

    pub(super) fn set(&self, hour: Option<u8>) {
        *self.0.lock().unwrap() = hour.map(|hour| ClockFrame {
            local_date: DATE,
            local_time: at(hour),
            utc_offset: 0,
            daylight_savings_status: false,
        });
    }
}

impl ClockReader for HandClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        *self.0.lock().unwrap()
    }
}

pub(super) fn trend_record(hour: u8) -> BACnetLogRecord {
    BACnetLogRecord {
        date: DATE,
        time: at(hour),
        log_datum: LogDatum::RealValue(f32::from(hour) + 0.5),
        status_flags: None,
    }
}

pub(super) fn event_record(hour: u8) -> BACnetEventLogRecord {
    BACnetEventLogRecord {
        date: DATE,
        time: at(hour),
        log_datum: EventLogDatum::TimeChange(f32::from(hour)),
    }
}

pub(super) fn multiple_record(hour: u8) -> BACnetLogMultipleRecord {
    BACnetLogMultipleRecord {
        date: DATE,
        time: at(hour),
        log_data: LogData::Values(vec![
            LogValue::UnsignedValue(u64::from(hour)),
            LogValue::BooleanValue(hour.is_multiple_of(2)),
        ]),
    }
}

pub(super) fn reference(instance: u32) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference {
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
        property_array_index: None,
        device_identifier: None,
    }
}

fn trend_snapshot(generation: u64) -> LogSnapshot<BACnetLogRecord> {
    LogSnapshot {
        object_identifier: ObjectIdentifier::new(ObjectType::TREND_LOG, 3).unwrap(),
        generation,
        buffer_size: 4,
        log_enable: true,
        stop_when_full: false,
        total_record_count: 7,
        records: (1..4).map(trend_record).collect(),
        last_notify_record: 5,
        records_since_notification_from: 6,
        notification_threshold: 4,
        start_time: (DATE, at(1)),
        stop_time: UNSPECIFIED_DATETIME,
        acquisition: Some(LogAcquisitionSettings {
            logging_type: LoggingType::POLLED,
            log_interval: 500,
            align_intervals: true,
            interval_offset: 20,
        }),
        log_device_object_property: vec![reference(1)],
    }
}

fn event_snapshot(generation: u64) -> LogSnapshot<BACnetEventLogRecord> {
    LogSnapshot {
        object_identifier: ObjectIdentifier::new(ObjectType::EVENT_LOG, 3).unwrap(),
        generation,
        buffer_size: 4,
        log_enable: false,
        stop_when_full: true,
        total_record_count: 2,
        records: (1..3).map(event_record).collect(),
        last_notify_record: 0,
        records_since_notification_from: 0,
        notification_threshold: 0,
        start_time: UNSPECIFIED_DATETIME,
        stop_time: (DATE, at(23)),
        acquisition: None,
        log_device_object_property: Vec::new(),
    }
}

fn multiple_snapshot(generation: u64) -> LogSnapshot<BACnetLogMultipleRecord> {
    LogSnapshot {
        object_identifier: ObjectIdentifier::new(ObjectType::TREND_LOG_MULTIPLE, 3).unwrap(),
        generation,
        buffer_size: 3,
        log_enable: true,
        stop_when_full: false,
        // Wrapped: the three records are 2^32 - 1, 1 and 2.
        total_record_count: 2,
        records: (1..4).map(multiple_record).collect(),
        last_notify_record: u32::MAX,
        records_since_notification_from: 1,
        notification_threshold: 0,
        start_time: UNSPECIFIED_DATETIME,
        stop_time: UNSPECIFIED_DATETIME,
        acquisition: Some(LogAcquisitionSettings {
            logging_type: LoggingType::TRIGGERED,
            log_interval: 0,
            align_intervals: false,
            interval_offset: 0,
        }),
        log_device_object_property: vec![reference(1), reference(2)],
    }
}

fn round_trip<R>(snapshot: impl Fn(u64) -> LogSnapshot<R>)
where
    R: std::fmt::Debug + PartialEq,
    FileLogPersistence: LogPersistence<R>,
{
    let base = TempBase::new("round-trip");
    let store = FileLogPersistence::new(base.path()).unwrap();
    let oid = snapshot(1).object_identifier;
    assert_eq!(store.load(oid).unwrap(), None);
    store.commit(&snapshot(1)).unwrap();
    assert_eq!(store.load(oid).unwrap(), Some(snapshot(1)));
    store.commit(&snapshot(2)).unwrap();
    assert_eq!(store.load(oid).unwrap(), Some(snapshot(2)));
    // Generations alternate between the two slots.
    let [slot0, slot1] = store.slot_paths();
    assert!(slot0.exists() && slot1.exists());
}

#[test]
fn each_family_round_trips_through_the_file_store() {
    round_trip(trend_snapshot);
    round_trip(event_snapshot);
    round_trip(multiple_snapshot);
}

/// Flip one payload octet and fix nothing else, so the checksum fails.
fn damage(path: &Path) {
    let mut data = std::fs::read(path).unwrap();
    data[30] ^= 0x55;
    std::fs::write(path, data).unwrap();
}

#[test]
fn a_damaged_newer_slot_falls_back_to_the_older_one() {
    let base = TempBase::new("fallback");
    let store = FileLogPersistence::new(base.path()).unwrap();
    let oid = trend_snapshot(1).object_identifier;
    store.commit(&trend_snapshot(1)).unwrap();
    store.commit(&trend_snapshot(2)).unwrap();
    let [slot0, slot1] = store.slot_paths();
    // Generation 2 is in slot 0.
    damage(&slot0);
    assert_eq!(store.load(oid).unwrap(), Some(trend_snapshot(1)));
    // A truncated slot is damage too.
    std::fs::write(&slot0, b"RBLOGS01").unwrap();
    assert_eq!(store.load(oid).unwrap(), Some(trend_snapshot(1)));
    // With both damaged there is nothing to load.
    damage(&slot1);
    let error = LogPersistence::<BACnetLogRecord>::load(&store, oid).unwrap_err();
    assert!(matches!(error, Error::Encoding(_)), "{error:?}");
}

/// Rewrite a slot's checksum after editing it, so only the edit is wrong.
fn reseal(data: &mut [u8]) {
    let at = data.len() - 4;
    let checksum = crc32(&data[..at]);
    data[at..].copy_from_slice(&checksum.to_be_bytes());
}

#[test]
fn a_slot_that_decodes_but_does_not_hold_together_falls_back() {
    let base = TempBase::new("invalid");
    let store = FileLogPersistence::new(base.path()).unwrap();
    let oid = trend_snapshot(1).object_identifier;
    store.commit(&trend_snapshot(1)).unwrap();
    store.commit(&trend_snapshot(2)).unwrap();
    let [slot0, _] = store.slot_paths();
    // Buffer_Size, the payload's first field, cut to 1 below the three
    // records it holds; the checksum is made to match.
    let mut data = std::fs::read(&slot0).unwrap();
    data[26..30].copy_from_slice(&1u32.to_be_bytes());
    reseal(&mut data);
    std::fs::write(&slot0, data).unwrap();
    assert_eq!(store.load(oid).unwrap(), Some(trend_snapshot(1)));
}

#[test]
fn another_objects_or_an_unknown_schemas_slot_is_refused_without_falling_back() {
    let base = TempBase::new("refused");
    let store = FileLogPersistence::new(base.path()).unwrap();
    let oid = trend_snapshot(1).object_identifier;
    store.commit(&trend_snapshot(1)).unwrap();
    let mut other = trend_snapshot(2);
    other.object_identifier = ObjectIdentifier::new(ObjectType::TREND_LOG, 4).unwrap();
    store.commit(&other).unwrap();
    let error = LogPersistence::<BACnetLogRecord>::load(&store, oid).unwrap_err();
    assert!(
        matches!(&error, Error::Encoding(message) if message.contains("another object")),
        "{error:?}"
    );

    store.commit(&trend_snapshot(2)).unwrap();
    let [slot0, _] = store.slot_paths();
    let mut data = std::fs::read(&slot0).unwrap();
    data[8..10].copy_from_slice(&9u16.to_be_bytes());
    reseal(&mut data);
    std::fs::write(&slot0, data).unwrap();
    let error = LogPersistence::<BACnetLogRecord>::load(&store, oid).unwrap_err();
    assert!(
        matches!(&error, Error::Encoding(message) if message.contains("schema version 9")),
        "{error:?}"
    );
}

#[test]
fn slots_disagreeing_at_one_generation_are_refused() {
    let base = TempBase::new("diverge");
    let store = FileLogPersistence::new(base.path()).unwrap();
    let oid = trend_snapshot(1).object_identifier;
    store.commit(&trend_snapshot(2)).unwrap();
    let [slot0, slot1] = store.slot_paths();
    let mut twin = trend_snapshot(2);
    twin.log_enable = false;
    let other = FileLogPersistence::new(base.path().with_extension("twin")).unwrap();
    other.commit(&twin).unwrap();
    std::fs::copy(&other.slot_paths()[0], &slot1).unwrap();
    assert!(slot0.exists());
    let error = LogPersistence::<BACnetLogRecord>::load(&store, oid).unwrap_err();
    assert!(
        matches!(&error, Error::Encoding(message) if message.contains("one generation")),
        "{error:?}"
    );
}

#[test]
fn snapshots_that_do_not_hold_together_are_never_written() {
    let base = TempBase::new("refuse-commit");
    let store = FileLogPersistence::new(base.path()).unwrap();
    let refused = |snapshot: &LogSnapshot<BACnetLogRecord>| {
        assert!(store.commit(snapshot).is_err(), "{snapshot:?}");
    };
    let mut bad = trend_snapshot(0);
    refused(&bad);
    bad = trend_snapshot(1);
    bad.buffer_size = 2;
    refused(&bad);
    bad = trend_snapshot(1);
    bad.total_record_count = 0;
    refused(&bad);
    bad = trend_snapshot(1);
    bad.buffer_size = 3;
    bad.stop_when_full = true;
    refused(&bad);
    bad = trend_snapshot(1);
    bad.start_time.1.minute = Time::UNSPECIFIED;
    refused(&bad);
    bad = trend_snapshot(1);
    bad.acquisition = None;
    refused(&bad);
    bad = trend_snapshot(1);
    bad.log_device_object_property.push(reference(2));
    refused(&bad);
    bad = trend_snapshot(1);
    bad.acquisition.as_mut().unwrap().logging_type = LoggingType::TRIGGERED;
    refused(&bad);
    bad = trend_snapshot(1);
    bad.object_identifier = ObjectIdentifier::new(ObjectType::EVENT_LOG, 3).unwrap();
    refused(&bad);
    // A Trend Log Multiple never logs by COV, and an Event Log has no
    // acquisition settings.
    let mut multiple = multiple_snapshot(1);
    multiple.acquisition.as_mut().unwrap().logging_type = LoggingType::COV;
    assert!(store.commit(&multiple).is_err());
    let mut event = event_snapshot(1);
    event.acquisition = trend_snapshot(1).acquisition;
    assert!(store.commit(&event).is_err());
    assert!(store.slot_paths().iter().all(|path| !path.exists()));
}

#[test]
fn an_empty_base_path_is_refused() {
    assert!(matches!(
        FileLogPersistence::new(""),
        Err(Error::OutOfRange(_))
    ));
}

#[test]
fn the_default_cadence_is_one_hundred_records_or_a_minute() {
    let cadence = LogCommitCadence::default();
    assert_eq!(cadence.records, 100);
    assert_eq!(cadence.interval, std::time::Duration::from_secs(60));
}

#[test]
fn a_crafted_record_count_is_refused_before_anything_is_reserved() {
    let base = TempBase::new("crafted-count");
    let store = FileLogPersistence::new(base.path()).unwrap();
    let oid = event_snapshot(1).object_identifier;
    store.commit(&event_snapshot(1)).unwrap();
    let [_, slot1] = store.slot_paths();
    let mut data = std::fs::read(&slot1).unwrap();
    // An Event Log's payload: 37 octets of fixed fields, the acquisition
    // presence octet, an empty reference list, then the record count.
    let at = super::codec::HEADER_LEN + 37 + 1 + 4;
    assert_eq!(data[at..at + 4], 2u32.to_be_bytes());
    // Past Buffer_Size (4), though the payload could still frame that
    // many length prefixes.
    data[at..at + 4].copy_from_slice(&5u32.to_be_bytes());
    reseal(&mut data);
    std::fs::write(&slot1, data).unwrap();
    let error = LogPersistence::<BACnetEventLogRecord>::load(&store, oid).unwrap_err();
    assert!(
        matches!(&error, Error::Encoding(message) if message.contains("count is too large")),
        "{error:?}"
    );
}
