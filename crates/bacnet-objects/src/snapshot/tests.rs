//! The public snapshot codecs (#2047): each format round-trips through
//! `encode` and `decode` without std, refuses what it should with the kind
//! it names, and writes the octets its file backend writes.

use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventLogRecord, BACnetLogMultipleRecord,
    BACnetLogRecord, EventLogDatum, LogData, LogDatum, LogValue,
};
use bacnet_types::enums::{LoggingType, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};

use super::{SnapshotError, SnapshotErrorKind, log, write_config};
use crate::clock::UNSPECIFIED_DATETIME;
use crate::durable::crc32;
use crate::log_persistence::{LogAcquisitionSettings, LogReportingSettings, LogSnapshot};
use crate::write_config::{
    ChannelMembers, ChannelWriteConfig, TimerWriteConfig, WriteConfigSnapshot,
};

pub(super) const DATE: Date = Date {
    year: 126,
    month: 11,
    day: 2,
    day_of_week: 1,
};

pub(super) fn at(hour: u8) -> Time {
    Time {
        hour,
        minute: 5,
        second: 0,
        hundredths: 0,
    }
}

pub(super) fn oid(kind: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(kind, instance).unwrap()
}

fn reference(instance: u32) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference {
        object_identifier: oid(ObjectType::ANALOG_INPUT, instance),
        property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
        property_array_index: None,
        device_identifier: None,
    }
}

/// A snapshot of an `kind` log holding `records`, with the settings that
/// family has.
fn log_snapshot<R>(kind: ObjectType, records: Vec<R>) -> LogSnapshot<R> {
    let trend = kind != ObjectType::EVENT_LOG;
    LogSnapshot {
        object_identifier: oid(kind, 4),
        generation: 3,
        buffer_size: 8,
        log_enable: true,
        stop_when_full: false,
        total_record_count: records.len() as u32 + 10,
        records,
        last_notify_record: 9,
        records_since_notification_from: 9,
        notification_threshold: 2,
        start_time: (DATE, at(1)),
        stop_time: UNSPECIFIED_DATETIME,
        acquisition: trend.then_some(LogAcquisitionSettings {
            logging_type: LoggingType::POLLED,
            log_interval: 100,
            align_intervals: false,
            interval_offset: 0,
        }),
        log_device_object_property: if trend {
            vec![reference(1)]
        } else {
            Vec::new()
        },
        reporting: LogReportingSettings {
            notification_class: Some(7),
            event_detection_enable: Some(false),
            cov_resubscription_interval: (kind == ObjectType::TREND_LOG).then_some(300),
            ..LogReportingSettings::default()
        },
    }
}

pub(super) fn trend() -> LogSnapshot<BACnetLogRecord> {
    let record = |hour| BACnetLogRecord {
        date: DATE,
        time: at(hour),
        log_datum: LogDatum::RealValue(f32::from(hour)),
        status_flags: None,
    };
    log_snapshot(ObjectType::TREND_LOG, vec![record(1), record(2)])
}

pub(super) fn event() -> LogSnapshot<BACnetEventLogRecord> {
    let record = |hour| BACnetEventLogRecord {
        date: DATE,
        time: at(hour),
        log_datum: EventLogDatum::TimeChange(f32::from(hour)),
    };
    log_snapshot(ObjectType::EVENT_LOG, vec![record(3)])
}

fn multiple() -> LogSnapshot<BACnetLogMultipleRecord> {
    let record = BACnetLogMultipleRecord {
        date: DATE,
        time: at(4),
        log_data: LogData::Values(vec![LogValue::BooleanValue(true)]),
    };
    log_snapshot(ObjectType::TREND_LOG_MULTIPLE, vec![record])
}

fn timer() -> ObjectIdentifier {
    oid(ObjectType::TIMER, 2)
}

fn channel() -> ObjectIdentifier {
    oid(ObjectType::CHANNEL, 2)
}

fn timer_config() -> WriteConfigSnapshot {
    WriteConfigSnapshot::Timer(TimerWriteConfig {
        state_change: None,
        default_timeout: Some(90_000),
        description: None,
    })
}

/// An Analog Input's written reporting settings (#2331).
// std: only the file backend's test takes it.
#[cfg(feature = "std")]
fn reporting_config() -> WriteConfigSnapshot {
    WriteConfigSnapshot::Reporting(crate::write_config::ReportingWriteConfig {
        notification_class: Some(4),
        event_detection_enable: Some(false),
        event_message_texts_config: Some(["off".into(), String::new(), "on".into()]),
        ..Default::default()
    })
}

fn channel_config() -> WriteConfigSnapshot {
    WriteConfigSnapshot::Channel(ChannelWriteConfig {
        members: Some(ChannelMembers {
            references: vec![reference(1), reference(2)],
            execution_delay: vec![0, 250],
        }),
        channel_number: Some(12),
        control_groups: Some(vec![1, 9]),
        allow_group_delay_inhibit: Some(true),
        description: None,
    })
}

/// Rewrite a slot's checksum after an edit, so only the edit is wrong.
pub(super) fn reseal(slot: &mut [u8]) {
    let at = slot.len() - 4;
    let checksum = crc32(&slot[..at]);
    slot[at..].copy_from_slice(&checksum.to_be_bytes());
}

#[test]
fn each_log_family_round_trips() {
    let octets = log::encode(&trend()).unwrap();
    assert_eq!(octets[..8], log::MAGIC);
    assert_eq!(octets[8..10], log::VERSION.to_be_bytes());
    assert_eq!(log::decode(trend().object_identifier, &octets), Ok(trend()));
    let octets = log::encode(&event()).unwrap();
    assert_eq!(log::decode(event().object_identifier, &octets), Ok(event()));
    let octets = log::encode(&multiple()).unwrap();
    let object = multiple().object_identifier;
    assert_eq!(log::decode(object, &octets), Ok(multiple()));
}

#[test]
fn a_log_slot_refuses_with_the_kind_it_names() {
    let object = trend().object_identifier;
    let slot = log::encode(&trend()).unwrap();
    let kind = |octets: &[u8], object| {
        let error = log::decode::<BACnetLogRecord>(object, octets).unwrap_err();
        (error.kind(), error.is_damage())
    };
    assert_eq!(
        kind(&slot[..slot.len() - 1], object),
        (SnapshotErrorKind::Truncated, true)
    );
    assert_eq!(
        kind(&slot[..20], object),
        (SnapshotErrorKind::Truncated, true)
    );
    let mut flipped = slot.clone();
    flipped[30] ^= 1;
    assert_eq!(kind(&flipped, object), (SnapshotErrorKind::Damaged, true));
    let mut retagged = slot.clone();
    retagged[0] = b'X';
    assert_eq!(
        kind(&retagged, object),
        (SnapshotErrorKind::NotThisFormat, true)
    );
    let mut newer = slot.clone();
    newer[8..10].copy_from_slice(&9u16.to_be_bytes());
    reseal(&mut newer);
    assert_eq!(
        kind(&newer, object),
        (SnapshotErrorKind::UnknownVersion(9), false)
    );
    let other = oid(ObjectType::TREND_LOG, 5);
    assert_eq!(kind(&slot, other), (SnapshotErrorKind::OtherObject, false));
}

#[test]
fn a_log_snapshot_that_does_not_hold_together_never_encodes() {
    let mut zero = trend();
    zero.generation = 0;
    let error = log::encode(&zero).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::OutOfRange);
    assert!(!error.is_damage());
    let mut cov = event();
    cov.reporting.cov_resubscription_interval = Some(60);
    let error = log::encode(&cov).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::OutOfRange);
    // A snapshot of one log type passed as another's never encodes either.
    let mut retyped = trend();
    retyped.object_identifier = oid(ObjectType::TREND_LOG_MULTIPLE, 4);
    let error = log::encode(&retyped).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::OutOfRange);
}

#[test]
fn each_write_config_kind_round_trips() {
    let octets = write_config::encode(timer(), &timer_config()).unwrap();
    assert_eq!(octets[..8], write_config::MAGIC);
    assert_eq!(octets[12], write_config::KIND_TIMER);
    assert_eq!(write_config::decode(timer(), &octets), Ok(timer_config()));
    let octets = write_config::encode(channel(), &channel_config()).unwrap();
    assert_eq!(octets[12], write_config::KIND_CHANNEL);
    assert_eq!(
        write_config::decode(channel(), &octets),
        Ok(channel_config())
    );
}

#[test]
fn a_write_config_file_refuses_with_the_kind_it_names() {
    let octets = write_config::encode(channel(), &channel_config()).unwrap();
    let kind = |object, octets: &[u8]| write_config::decode(object, octets).unwrap_err().kind();
    for cut in 13..octets.len() {
        assert_eq!(
            kind(channel(), &octets[..cut]),
            SnapshotErrorKind::Truncated,
            "cut at {cut}"
        );
    }
    assert_eq!(
        kind(channel(), &octets[..5]),
        SnapshotErrorKind::NotThisFormat
    );
    assert_eq!(
        kind(oid(ObjectType::CHANNEL, 3), &octets),
        SnapshotErrorKind::OtherObject
    );
    let mut unknown = octets.clone();
    unknown[12] = 9;
    assert_eq!(kind(channel(), &unknown), SnapshotErrorKind::UnknownKind(9));
    // A Timer's kind in a Channel's file: a kind this build knows, for the
    // wrong object type.
    let mut timer_kind = octets.clone();
    timer_kind[12] = write_config::KIND_TIMER;
    assert_eq!(kind(channel(), &timer_kind), SnapshotErrorKind::OtherObject);
    let mut trailing = octets.clone();
    trailing.push(0);
    assert_eq!(kind(channel(), &trailing), SnapshotErrorKind::Damaged);
    let mut huge = octets;
    huge.resize(write_config::MAX_LEN + 1, 0);
    assert_eq!(kind(channel(), &huge), SnapshotErrorKind::TooLarge);
    // Saved for a Timer, a Channel's configuration is refused unwritten.
    let error = write_config::encode(timer(), &channel_config()).unwrap_err();
    assert_eq!(error.kind(), SnapshotErrorKind::OutOfRange);
    assert!(!error.is_damage());
}

/// The error is plain data: it is copied and compared, and formats with
/// its field and reason, without the codec allocating.
#[test]
fn an_error_names_its_field_and_reason() {
    let error = SnapshotError::truncated("is truncated").in_field("Buffer_Size");
    let copy = error;
    assert_eq!(copy, error);
    assert_eq!(error.field(), Some("Buffer_Size"));
    assert_eq!(error.to_string(), "Buffer_Size is truncated");
    let version = SnapshotError::new(SnapshotErrorKind::UnknownVersion(4), "");
    assert_eq!(version.to_string(), "schema version 4 is unsupported");
}

/// The file backends write what the public codecs return and load what
/// they decode, octet for octet.
// std: the file backends.
#[cfg(feature = "std")]
pub(super) mod file_backends {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::log_persistence::{FileLogPersistence, LogPersistence};
    use crate::write_config::{FileWriteConfigPersistence, WriteConfigPersistence};

    /// A directory of its own, removed on drop.
    pub(in crate::snapshot) struct Scratch(PathBuf);

    impl Scratch {
        pub(in crate::snapshot) fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let directory = std::env::temp_dir().join(format!(
                "rusty-bacnet-snapshot-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&directory).unwrap();
            Self(directory)
        }

        pub(in crate::snapshot) fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// `snapshot` committed through the file backend: the slot it wrote
    /// (the generation's parity names it) is what `log::encode` returns,
    /// `log::decode` reads it, and a slot `log::encode` wrote loads back.
    fn same_log_octets<R>(snapshot: LogSnapshot<R>)
    where
        R: log::LogRecord + PartialEq + core::fmt::Debug,
        FileLogPersistence: LogPersistence<R>,
    {
        let scratch = Scratch::new();
        let store = FileLogPersistence::new(scratch.path("log")).unwrap();
        store.commit(&snapshot).unwrap();
        let slot = store.slot_paths()[(snapshot.generation % 2) as usize].clone();
        let written = std::fs::read(&slot).unwrap();
        assert_eq!(written, log::encode(&snapshot).unwrap());
        let object = snapshot.object_identifier;
        assert_eq!(log::decode::<R>(object, &written).as_ref(), Ok(&snapshot));

        let mut next = snapshot;
        next.generation += 1;
        let other = store.slot_paths()[(next.generation % 2) as usize].clone();
        std::fs::write(&other, log::encode(&next).unwrap()).unwrap();
        assert_eq!(store.load(object).unwrap(), Some(next));
    }

    #[test]
    fn the_log_file_backend_writes_and_reads_the_public_octets() {
        same_log_octets(trend());
        same_log_octets(event());
        same_log_octets(multiple());
    }

    #[test]
    fn the_write_config_file_backend_writes_and_reads_the_public_octets() {
        let analog = oid(ObjectType::ANALOG_INPUT, 2);
        for (object, config) in [
            (timer(), timer_config()),
            (channel(), channel_config()),
            (analog, reporting_config()),
        ] {
            let scratch = Scratch::new();
            let path = scratch.path("config");
            let store = FileWriteConfigPersistence::new(&path).unwrap();
            store.save(object, &config).unwrap();
            let written = std::fs::read(&path).unwrap();
            assert_eq!(written, write_config::encode(object, &config).unwrap());
            assert_eq!(write_config::decode(object, &written).as_ref(), Ok(&config));

            let changed = match config {
                WriteConfigSnapshot::Timer(mut config) => {
                    config.default_timeout = Some(5_000);
                    WriteConfigSnapshot::Timer(config)
                }
                WriteConfigSnapshot::Channel(mut config) => {
                    config.channel_number = Some(13);
                    WriteConfigSnapshot::Channel(config)
                }
                WriteConfigSnapshot::Reporting(mut config) => {
                    config.notification_class = Some(13);
                    WriteConfigSnapshot::Reporting(config)
                }
            };
            std::fs::write(&path, write_config::encode(object, &changed).unwrap()).unwrap();
            assert_eq!(store.load(object).unwrap(), Some(changed));
        }
    }
}
