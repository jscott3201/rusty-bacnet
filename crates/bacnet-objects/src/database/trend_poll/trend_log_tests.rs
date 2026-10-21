//! How the poller acquires for a Trend Log (#1354): once per Trigger, and at
//! clock-aligned times; and how each pass records a Trend Log's or an Event
//! Log's Start_Time / Stop_Time window opening or closing with no record
//! arriving (#1353). The monotonic clock and the Device clock are both set by
//! hand.

use super::acquisition_tests::{at, set_clock, set_time};
use super::*;
use crate::event_log::EventLogObject;
use bacnet_encoding::constructed::{decode_event_log_record, decode_log_record};
use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::EventLogDatum;

fn read(db: &ObjectDatabase, oid: ObjectIdentifier, property: P) -> PropertyValue {
    db.get(&oid).unwrap().read_property(property, None).unwrap()
}

/// Each served record of `oid`, encoded.
fn served(db: &ObjectDatabase, oid: ObjectIdentifier) -> Vec<BytesMut> {
    let records = db.get(&oid).unwrap().log_buffer_internal().unwrap();
    (0..records.record_count())
        .map(|index| {
            let mut bytes = BytesMut::new();
            records.encode_record(index, &mut bytes);
            bytes
        })
        .collect()
}

fn records(db: &ObjectDatabase, oid: ObjectIdentifier) -> Vec<BACnetLogRecord> {
    served(db, oid)
        .iter()
        .map(|bytes| decode_log_record(bytes, 0).unwrap().0)
        .collect()
}

fn data(db: &ObjectDatabase, oid: ObjectIdentifier) -> Vec<LogDatum> {
    records(db, oid).into_iter().map(|r| r.log_datum).collect()
}

/// A start or stop `frame` as a written BACnetDateTime.
fn datetime(frame: ClockFrame) -> PropertyValue {
    PropertyValue::List(vec![
        PropertyValue::Date(frame.local_date),
        PropertyValue::Time(frame.local_time),
    ])
}

const DISABLED: LogStatus = LogStatus::LOG_DISABLED;
const ENABLED: LogStatus = LogStatus::empty();

/// AV-1's value as the fixture's Trend Log samples it.
const SAMPLE: LogDatum = LogDatum::RealValue(0.0);

#[test]
fn each_trigger_acquires_one_trend_log_record() {
    let (mut db, oid, time, _) = fixture(u32::MAX);
    write(&mut db, oid, P::LOGGING_TYPE, PropertyValue::Enumerated(2));
    db.poll_trend_logs();
    assert!(records(&db, oid).is_empty());

    write(&mut db, oid, P::TRIGGER, PropertyValue::Boolean(true));
    assert_eq!(read(&db, oid, P::TRIGGER), PropertyValue::Boolean(true));
    db.poll_trend_logs();
    assert_eq!(data(&db, oid), [SAMPLE]);
    assert_eq!(records(&db, oid)[0].time, frame().local_time);
    assert_eq!(read(&db, oid, P::TRIGGER), PropertyValue::Boolean(false));
    set_time(&time, Duration::from_secs(60));
    db.poll_trend_logs();
    assert_eq!(records(&db, oid).len(), 1);

    // The next Trigger is a fresh acquisition, without waiting.
    write(&mut db, oid, P::TRIGGER, PropertyValue::Boolean(true));
    db.poll_trend_logs();
    assert_eq!(data(&db, oid), [SAMPLE, SAMPLE]);
    assert!(!db.trend_poll.0.contains_key(&oid));
}

#[test]
fn a_trigger_on_a_trend_log_without_a_reference_logs_a_failure() {
    let (mut db, oid, _, _) = fixture(u32::MAX);
    // The unset form clears the reference (#1417): [0] analog-input 4194303,
    // [1] present-value.
    write(
        &mut db,
        oid,
        P::LOG_DEVICE_OBJECT_PROPERTY,
        PropertyValue::ApplicationData(vec![0x0C, 0x00, 0x3F, 0xFF, 0xFF, 0x19, 0x55]),
    );
    write(&mut db, oid, P::LOGGING_TYPE, PropertyValue::Enumerated(2));
    write(&mut db, oid, P::TRIGGER, PropertyValue::Boolean(true));
    db.poll_trend_logs();
    // The purge the reference change made, then the served Trigger.
    assert_eq!(
        data(&db, oid),
        [
            LogDatum::LogStatus(LogStatus::BUFFER_PURGED),
            LogDatum::Failure {
                error_class: ErrorClass::PROPERTY,
                error_code: ErrorCode::NO_PROPERTY_SPECIFIED,
            },
        ]
    );
    assert_eq!(read(&db, oid, P::TRIGGER), PropertyValue::Boolean(false));
    // POLLED, it stays unpolled without a reference.
    write(&mut db, oid, P::LOGGING_TYPE, PropertyValue::Enumerated(0));
    db.poll_trend_logs();
    assert_eq!(records(&db, oid).len(), 2);
}

#[test]
fn an_aligned_trend_log_acquires_on_each_boundary() {
    // Every minute, 1.50 s past it; the Device clock reads 12:00:00.37.
    let (mut db, oid, time, clock) = fixture(6_000);
    write(
        &mut db,
        oid,
        P::ALIGN_INTERVALS,
        PropertyValue::Boolean(true),
    );
    write(
        &mut db,
        oid,
        P::INTERVAL_OFFSET,
        PropertyValue::Unsigned(150),
    );
    db.poll_trend_logs();
    assert!(records(&db, oid).is_empty());
    assert_eq!(
        db.trend_poll.0[&oid].remaining(Duration::ZERO),
        Duration::from_millis(1_130)
    );

    set_time(&time, Duration::from_millis(1_130));
    set_clock(&clock, Some(at(12, 0, 1, 50)));
    db.poll_trend_logs();
    assert_eq!(data(&db, oid), [SAMPLE]);
    assert_eq!(records(&db, oid)[0].time, at(12, 0, 1, 50).local_time);
    let next = db.trend_poll.0[&oid].remaining(Duration::from_millis(1_130));
    assert_eq!(next, Duration::from_secs(60));

    // A hundredth early, nothing; on the boundary, the next record.
    set_time(&time, Duration::from_millis(61_120));
    set_clock(&clock, Some(at(12, 1, 1, 49)));
    db.poll_trend_logs();
    assert_eq!(records(&db, oid).len(), 1);
    set_time(&time, Duration::from_millis(61_130));
    set_clock(&clock, Some(at(12, 1, 1, 50)));
    db.poll_trend_logs();
    assert_eq!(
        records(&db, oid).iter().map(|r| r.time).collect::<Vec<_>>(),
        [at(12, 0, 1, 50).local_time, at(12, 1, 1, 50).local_time]
    );
}

#[test]
fn each_pass_records_a_trend_log_window_opening_and_closing() {
    // TRIGGERED, so no record is acquired: only the pass logs the changes.
    let (mut db, oid, _, clock) = fixture(u32::MAX);
    write(&mut db, oid, P::LOGGING_TYPE, PropertyValue::Enumerated(2));
    write(&mut db, oid, P::START_TIME, datetime(at(12, 0, 1, 0)));
    write(&mut db, oid, P::STOP_TIME, datetime(at(12, 0, 2, 0)));
    assert_eq!(data(&db, oid), [LogDatum::LogStatus(DISABLED)]);
    set_clock(&clock, Some(at(12, 0, 1, 37)));
    db.poll_trend_logs();
    set_clock(&clock, Some(at(12, 0, 2, 37)));
    db.poll_trend_logs();
    db.poll_trend_logs();
    assert_eq!(
        data(&db, oid),
        [DISABLED, ENABLED, DISABLED].map(LogDatum::LogStatus)
    );
    // Each stamped with the pass that saw it.
    assert_eq!(records(&db, oid)[1].time, at(12, 0, 1, 37).local_time);
    assert_eq!(records(&db, oid)[2].time, at(12, 0, 2, 37).local_time);
}

#[test]
fn each_pass_records_an_event_log_window_opening_and_closing() {
    let (mut db, _, _, clock) = fixture(u32::MAX);
    let log = EventLogObject::new(1, "EL-1", 8).unwrap();
    let oid = log.object_identifier();
    db.add(Box::new(log)).unwrap();
    let statuses = |db: &ObjectDatabase| -> Vec<(LogStatus, Time)> {
        served(db, oid)
            .iter()
            .map(|bytes| {
                let record = decode_event_log_record(bytes, 0).unwrap().0;
                let EventLogDatum::LogStatus(status) = record.log_datum else {
                    panic!("{record:?}")
                };
                (status, record.time)
            })
            .collect()
    };
    write(&mut db, oid, P::START_TIME, datetime(at(12, 0, 1, 0)));
    write(&mut db, oid, P::STOP_TIME, datetime(at(12, 0, 2, 0)));
    assert_eq!(statuses(&db), [(DISABLED, frame().local_time)]);
    // No record arrives; the pass alone sees the window open, then shut.
    set_clock(&clock, Some(at(12, 0, 1, 37)));
    db.poll_trend_logs();
    // The pass looks at the window even without a monotonic clock.
    db.set_monotonic_clock_internal(None);
    set_clock(&clock, Some(at(12, 0, 2, 37)));
    assert_eq!(db.poll_trend_logs(), RECONCILE);
    db.poll_trend_logs();
    assert_eq!(
        statuses(&db),
        [
            (DISABLED, frame().local_time),
            (ENABLED, at(12, 0, 1, 37).local_time),
            (DISABLED, at(12, 0, 2, 37).local_time),
        ]
    );
}
