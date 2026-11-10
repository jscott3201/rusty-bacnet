//! How the poller acquires for a Trend Log Multiple (#1235): once per
//! Trigger, and only inside the Start_Time / Stop_Time window, whose changes
//! each pass records. The monotonic clock and the Device clock are both set
//! by hand. Clock-aligned polling is in `alignment_tests`.

use super::multiple_tests::{ao, av, clocked_database, member, multiple, records};
use super::*;
use crate::trend::TrendLogMultipleObject;
use bacnet_types::bitstring::LogStatus;
use bacnet_types::enums::LoggingType;

/// The fixture's day at `hour`:`minute`:`second`.`hundredths`.
pub(super) fn at(hour: u8, minute: u8, second: u8, hundredths: u8) -> ClockFrame {
    ClockFrame {
        local_time: Time {
            hour,
            minute,
            second,
            hundredths,
        },
        ..frame()
    }
}

pub(super) fn set_clock(clock: &WallClock, now: Option<ClockFrame>) {
    *clock.0.lock().unwrap() = now;
}

pub(super) fn set_time(time: &Mutex<Duration>, now: Duration) {
    *time.lock().unwrap() = now;
}

fn read(db: &ObjectDatabase, oid: ObjectIdentifier, property: P) -> PropertyValue {
    db.get(&oid).unwrap().read_property(property, None).unwrap()
}

fn write_trigger(db: &mut ObjectDatabase, oid: ObjectIdentifier) {
    write(db, oid, P::TRIGGER, PropertyValue::Boolean(true));
}

fn triggered(members: Vec<BACnetDeviceObjectPropertyReference>) -> TrendLogMultipleObject {
    let mut log = multiple(0, 8, members);
    log.set_logging_type(LoggingType::TRIGGERED).unwrap();
    log
}

fn data(db: &ObjectDatabase, oid: ObjectIdentifier) -> Vec<LogData> {
    records(db, oid).into_iter().map(|r| r.log_data).collect()
}

pub(super) fn times(db: &ObjectDatabase, oid: ObjectIdentifier) -> Vec<Time> {
    records(db, oid).into_iter().map(|r| r.time).collect()
}

const DISABLED: LogData = LogData::LogStatus(LogStatus::LOG_DISABLED);
const ENABLED: LogData = LogData::LogStatus(LogStatus::empty());

pub(super) fn sample() -> LogData {
    LogData::Values(vec![LogValue::RealValue(42.5)])
}

#[test]
fn each_trigger_acquires_one_record_and_reads_false_again() {
    let (mut db, time, _) = clocked_database();
    let log = triggered(vec![
        member(av(), P::PRESENT_VALUE, None, None),
        member(ao(), P::PRESENT_VALUE, None, None),
    ]);
    let oid = log.object_identifier();
    db.add(Box::new(log)).unwrap();
    db.poll_trend_logs();
    assert!(records(&db, oid).is_empty());

    write_trigger(&mut db, oid);
    assert_eq!(read(&db, oid, P::TRIGGER), PropertyValue::Boolean(true));
    db.poll_trend_logs();
    assert_eq!(
        data(&db, oid),
        [LogData::Values(vec![
            LogValue::RealValue(42.5),
            LogValue::RealValue(61.5),
        ])]
    );
    assert_eq!(read(&db, oid, P::TRIGGER), PropertyValue::Boolean(false));
    set_time(&time, Duration::from_secs(60));
    db.poll_trend_logs();
    assert_eq!(records(&db, oid).len(), 1);

    // The next Trigger is a fresh acquisition, without waiting.
    write_trigger(&mut db, oid);
    db.poll_trend_logs();
    assert_eq!(records(&db, oid).len(), 2);
    assert!(!db.trend_poll.0.contains_key(&oid));
}

#[test]
fn a_trigger_waits_for_a_clock_and_stays_true_until_served() {
    let (mut db, time, clock) = clocked_database();
    let log = triggered(vec![member(av(), P::PRESENT_VALUE, None, None)]);
    let oid = log.object_identifier();
    db.add(Box::new(log)).unwrap();
    set_clock(&clock, None);
    write_trigger(&mut db, oid);
    db.poll_trend_logs();
    set_time(&time, RECONCILE - Duration::from_millis(1));
    set_clock(&clock, Some(frame()));
    db.poll_trend_logs();
    assert!(records(&db, oid).is_empty());
    assert_eq!(read(&db, oid, P::TRIGGER), PropertyValue::Boolean(true));
    // The failed attempt is retried after the usual 100 ms.
    set_time(&time, RECONCILE);
    db.poll_trend_logs();
    assert_eq!(data(&db, oid), [sample()]);
    assert_eq!(read(&db, oid, P::TRIGGER), PropertyValue::Boolean(false));
}

#[test]
fn a_trigger_is_served_without_members_and_while_disabled() {
    // No members: a record of no values, so Trigger doesn't stay TRUE.
    let (mut db, _, _) = clocked_database();
    let log = triggered(Vec::new());
    let oid = log.object_identifier();
    db.add(Box::new(log)).unwrap();
    write_trigger(&mut db, oid);
    db.poll_trend_logs();
    assert_eq!(data(&db, oid), [LogData::Values(Vec::new())]);
    assert_eq!(read(&db, oid, P::TRIGGER), PropertyValue::Boolean(false));

    // Disabled: nothing logged, but the Trigger is still served.
    let (mut db, _, _) = clocked_database();
    let log = triggered(vec![member(av(), P::PRESENT_VALUE, None, None)]);
    let oid = log.object_identifier();
    db.add(Box::new(log)).unwrap();
    write(&mut db, oid, P::ENABLE, PropertyValue::Boolean(false));
    write_trigger(&mut db, oid);
    db.poll_trend_logs();
    assert_eq!(data(&db, oid), [DISABLED]);
    assert_eq!(read(&db, oid, P::TRIGGER), PropertyValue::Boolean(false));
}

#[test]
fn each_pass_records_the_window_opening_and_closing() {
    let (mut db, time, clock) = clocked_database();
    // Every second, from 12:00:01 until 12:00:02, set before the first look.
    let mut log = multiple(100, 8, vec![member(av(), P::PRESENT_VALUE, None, None)]);
    let start = at(12, 0, 1, 0);
    let stop = at(12, 0, 2, 0);
    log.set_start_time(start.local_date, start.local_time)
        .unwrap();
    log.set_stop_time(stop.local_date, stop.local_time).unwrap();
    let oid = log.object_identifier();
    db.add(Box::new(log)).unwrap();
    db.poll_trend_logs();
    assert!(records(&db, oid).is_empty());
    set_time(&time, Duration::from_secs(1));
    set_clock(&clock, Some(at(12, 0, 1, 37)));
    db.poll_trend_logs();
    assert_eq!(data(&db, oid), [ENABLED, sample()]);
    set_time(&time, Duration::from_secs(2));
    set_clock(&clock, Some(at(12, 0, 2, 37)));
    db.poll_trend_logs();
    assert_eq!(data(&db, oid), [ENABLED, sample(), DISABLED]);

    // A TRIGGERED log acquires nothing, yet the pass still records the
    // window opening for it.
    let (mut db, _, clock) = clocked_database();
    let log = triggered(vec![member(av(), P::PRESENT_VALUE, None, None)]);
    let oid = log.object_identifier();
    db.add(Box::new(log)).unwrap();
    let opens = at(12, 30, 0, 0);
    write(
        &mut db,
        oid,
        P::START_TIME,
        PropertyValue::List(vec![
            PropertyValue::Date(opens.local_date),
            PropertyValue::Time(opens.local_time),
        ]),
    );
    assert_eq!(data(&db, oid), [DISABLED]);
    set_clock(&clock, Some(opens));
    db.poll_trend_logs();
    assert_eq!(data(&db, oid), [DISABLED, ENABLED]);
    assert_eq!(times(&db, oid)[1], opens.local_time);
}
