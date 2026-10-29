//! What a reopened log does with its saved state (#1541 review): the
//! configured Buffer_Size wins in both directions, a full and stopped log
//! keeps its records over the LOG_INTERRUPTED marker, Notification_Threshold
//! comes back, and a Buffer_Size a slot can't hold is refused.

use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::{
    BACnetEventLogRecord, BACnetLogMultipleRecord, BACnetLogRecord, LogDatum,
};
use bacnet_types::enums::PropertyIdentifier as P;
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;
use bytes::BytesMut;

use super::codec::{SnapshotRecord, max_buffer_size};
use super::reopen_tests::{KINDS, add, interrupted, open, served, total, unsigned, write};
use super::tests::{DATE, HandClock, MemoryLog, TempBase, at};
use crate::event_log::EventLogObject;
use crate::trend::{TrendLogMultipleObject, TrendLogObject};

fn sequence_numbers(records: &[(u64, Vec<u8>)]) -> Vec<u64> {
    records.iter().map(|(number, _)| *number).collect()
}

#[test]
fn a_larger_configured_buffer_size_keeps_every_record() {
    for kind in KINDS {
        let base = TempBase::new("grow");
        let (mut log, _) = open(kind, base.path(), 4).unwrap();
        for hour in 1..5 {
            add(log.as_mut(), hour);
        }
        let before = served(log.as_ref());
        drop(log);

        let (log, reopened) = open(kind, base.path(), 8).unwrap();
        assert!(reopened, "{kind:?}");
        assert_eq!(served(log.as_ref()), before, "{kind:?}");
        assert_eq!(unsigned(log.as_ref(), P::BUFFER_SIZE), 8, "{kind:?}");
        // The new size is committed, though nothing else changed.
        drop(log);
        let (log, _) = open(kind, base.path(), 8).unwrap();
        assert_eq!(unsigned(log.as_ref(), P::BUFFER_SIZE), 8, "{kind:?}");
        assert_eq!(served(log.as_ref()), before, "{kind:?}");
    }
}

#[test]
fn a_smaller_configured_buffer_size_keeps_the_newest_records() {
    for kind in KINDS {
        let base = TempBase::new("shrink");
        let (mut log, _) = open(kind, base.path(), 6).unwrap();
        log.bind_clock_internal(Some(HandClock::at(8)));
        write(
            log.as_mut(),
            P::NOTIFICATION_THRESHOLD,
            PropertyValue::Unsigned(50),
        );
        for hour in 1..6 {
            add(log.as_mut(), hour);
        }
        let before = served(log.as_ref());
        let since = unsigned(log.as_ref(), P::RECORDS_SINCE_NOTIFICATION);
        drop(log);

        let (log, _) = open(kind, base.path(), 3).unwrap();
        assert_eq!(served(log.as_ref()), before[2..].to_vec(), "{kind:?}");
        assert_eq!(sequence_numbers(&served(log.as_ref())), vec![3, 4, 5]);
        assert_eq!(total(log.as_ref()), 5, "{kind:?}");
        assert_eq!(unsigned(log.as_ref(), P::BUFFER_SIZE), 3, "{kind:?}");
        // Counted from Total_Record_Count, which the trim leaves alone.
        assert_eq!(
            unsigned(log.as_ref(), P::RECORDS_SINCE_NOTIFICATION),
            since,
            "{kind:?}"
        );
    }
}

#[test]
fn a_smaller_buffer_size_leaves_a_stop_when_full_log_its_last_slot() {
    for kind in KINDS {
        let base = TempBase::new("shrink-swf");
        let (mut log, _) = open(kind, base.path(), 6).unwrap();
        write(
            log.as_mut(),
            P::STOP_WHEN_FULL,
            PropertyValue::Boolean(true),
        );
        for hour in 1..4 {
            add(log.as_mut(), hour);
        }
        drop(log);

        let (mut log, _) = open(kind, base.path(), 3).unwrap();
        // One slot short of full, so the log still collects...
        assert_eq!(sequence_numbers(&served(log.as_ref())), vec![2, 3]);
        assert_eq!(
            log.read_property(P::LOG_ENABLE, None).unwrap(),
            PropertyValue::Boolean(true),
            "{kind:?}"
        );
        // ...until the next record, here the marker, fills it and stops it.
        log.bind_clock_internal(Some(HandClock::at(9)));
        assert_eq!(served(log.as_ref()).len(), 3, "{kind:?}");
        assert_eq!(
            log.read_property(P::LOG_ENABLE, None).unwrap(),
            PropertyValue::Boolean(false),
            "{kind:?}"
        );
    }
}

#[test]
fn a_full_stopped_log_keeps_its_records_over_the_marker() {
    for kind in KINDS {
        let base = TempBase::new("full-stopped");
        let (mut log, _) = open(kind, base.path(), 3).unwrap();
        log.bind_clock_internal(Some(HandClock::at(8)));
        write(
            log.as_mut(),
            P::STOP_WHEN_FULL,
            PropertyValue::Boolean(true),
        );
        // The third record would fill the buffer: the log records
        // LOG_DISABLED in that slot and stops.
        for hour in 1..4 {
            add(log.as_mut(), hour);
        }
        let before = (served(log.as_ref()), total(log.as_ref()));
        assert_eq!(before.0.len(), 3, "{kind:?}");
        drop(log);

        let (mut log, _) = open(kind, base.path(), 3).unwrap();
        log.bind_clock_internal(Some(HandClock::at(12)));
        log.refresh_log_window_internal();
        assert_eq!(
            (served(log.as_ref()), total(log.as_ref())),
            before,
            "{kind:?}"
        );
    }
}

/// A LOG_INTERRUPTED record that also carries LOG_DISABLED.
fn interrupted_disabled(hour: u8) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    bacnet_encoding::constructed::encode_log_record(
        &BACnetLogRecord {
            date: DATE,
            time: at(hour),
            log_datum: LogDatum::LogStatus(LogStatus::LOG_INTERRUPTED | LogStatus::LOG_DISABLED),
            status_flags: None,
        },
        &mut bytes,
    )
    .unwrap();
    bytes.to_vec()
}

#[test]
fn a_stopped_log_with_room_takes_the_marker_marked_disabled() {
    for kind in KINDS {
        let base = TempBase::new("stopped-room");
        let (mut log, _) = open(kind, base.path(), 5).unwrap();
        log.bind_clock_internal(Some(HandClock::at(8)));
        add(log.as_mut(), 1);
        write(log.as_mut(), P::LOG_ENABLE, PropertyValue::Boolean(false));
        drop(log);

        let (mut log, _) = open(kind, base.path(), 5).unwrap();
        log.bind_clock_internal(Some(HandClock::at(12)));
        let records = served(log.as_ref());
        assert_eq!(records.len(), 3, "{kind:?}");
        assert_eq!(records[2], (3, interrupted_disabled(12)), "{kind:?}");
        assert_ne!(records[2].1, interrupted(12));
    }
}

#[test]
fn notification_threshold_comes_back() {
    for kind in KINDS {
        let base = TempBase::new("threshold");
        let (mut log, _) = open(kind, base.path(), 5).unwrap();
        write(
            log.as_mut(),
            P::NOTIFICATION_THRESHOLD,
            PropertyValue::Unsigned(7),
        );
        drop(log);
        let (log, _) = open(kind, base.path(), 5).unwrap();
        assert_eq!(
            unsigned(log.as_ref(), P::NOTIFICATION_THRESHOLD),
            7,
            "{kind:?}"
        );
    }
}

fn refused_size<R: SnapshotRecord>(open: impl Fn(u32) -> Result<(), Error>) {
    let max = max_buffer_size::<R>();
    assert!(open(max).is_ok());
    let error = open(max + 1).unwrap_err();
    assert!(
        matches!(&error, Error::OutOfRange(message) if message.contains("at most")),
        "{error:?}"
    );
}

#[test]
fn a_buffer_size_a_slot_cannot_hold_is_refused() {
    refused_size::<BACnetLogRecord>(|size| {
        TrendLogObject::with_persistence(1, "TL-1", size, MemoryLog::new()).map(drop)
    });
    refused_size::<BACnetEventLogRecord>(|size| {
        EventLogObject::with_persistence(1, "EL-1", size, MemoryLog::new()).map(drop)
    });
    refused_size::<BACnetLogMultipleRecord>(|size| {
        TrendLogMultipleObject::with_persistence(1, "TLM-1", size, MemoryLog::new()).map(drop)
    });
    // Each family's limit, as rust-api.md documents it.
    assert_eq!(max_buffer_size::<BACnetLogRecord>(), 828_302);
    assert_eq!(max_buffer_size::<BACnetEventLogRecord>(), 178_437);
    assert_eq!(max_buffer_size::<BACnetLogMultipleRecord>(), 15_849);
}
