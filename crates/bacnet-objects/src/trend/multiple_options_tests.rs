//! Trend Log Multiple's optional behaviour (#1235): Logging_Type limited to
//! POLLED and TRIGGERED, Trigger, the clock-alignment rows and the
//! Start_Time / Stop_Time window, each against a clock the test sets.

use super::*;
use crate::clock::{ClockFrame, ClockReader};
use bacnet_types::bitstring::LogStatus;
use bacnet_types::calendar::SpecificDate;
use bacnet_types::constructed::{BACnetLogMultipleRecord, LogData, LogValue};
use bacnet_types::primitives::{Date, Time};
use std::sync::Mutex;

struct Clock(Mutex<ClockFrame>);

impl ClockReader for Clock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(*self.0.lock().unwrap())
    }
}

impl Clock {
    fn set(&self, now: (Date, Time)) {
        *self.0.lock().unwrap() = frame(now);
    }
}

/// `hour`:`minute` on 2026-10-03, its weekday filled in.
fn at(hour: u8, minute: u8) -> (Date, Time) {
    (
        SpecificDate::new(2026, 10, 3).unwrap().to_date(),
        Time {
            hour,
            minute,
            second: 0,
            hundredths: 0,
        },
    )
}

fn frame((local_date, local_time): (Date, Time)) -> ClockFrame {
    ClockFrame {
        local_date,
        local_time,
        utc_offset: 0,
        daylight_savings_status: false,
    }
}

fn datetime((date, time): (Date, Time)) -> PropertyValue {
    PropertyValue::List(vec![PropertyValue::Date(date), PropertyValue::Time(time)])
}

/// A log of `capacity` bound to a clock reading 09:00.
fn clocked(capacity: u32) -> (TrendLogMultipleObject, Arc<Clock>) {
    let clock = Arc::new(Clock(Mutex::new(frame(at(9, 0)))));
    let mut log = TrendLogMultipleObject::new(1, "TLM-1", capacity).unwrap();
    log.bind_clock_internal(Some(clock.clone()));
    (log, clock)
}

fn sample(now: (Date, Time)) -> BACnetLogMultipleRecord {
    BACnetLogMultipleRecord {
        date: now.0,
        time: now.1,
        log_data: LogData::Values(vec![LogValue::RealValue(1.5)]),
    }
}

fn data(log: &TrendLogMultipleObject) -> Vec<LogData> {
    log.records().iter().map(|r| r.log_data.clone()).collect()
}

fn read(log: &TrendLogMultipleObject, property: PropertyIdentifier) -> PropertyValue {
    log.read_property(property, None).unwrap()
}

fn write(
    log: &mut TrendLogMultipleObject,
    property: PropertyIdentifier,
    value: PropertyValue,
) -> Result<(), Error> {
    log.write_property(property, None, value, None)
}

fn assert_refused(result: Result<(), Error>, class: ErrorClass, code: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Protocol { class: c, code: e })
            if c == class && e == code),
        "expected {class:?} / {code:?}, got {result:?}"
    );
}

const DISABLED: LogData = LogData::LogStatus(LogStatus::LOG_DISABLED);
const ENABLED: LogData = LogData::LogStatus(LogStatus::empty());

#[test]
fn logging_type_takes_polled_or_triggered_and_refuses_cov() {
    let mut log = TrendLogMultipleObject::new(1, "TLM-1", 8).unwrap();
    // COV logging isn't allowed (Clause 12.30.12); neither is a value
    // outside BACnetLoggingType. Through the wire and the setter alike.
    for raw in [LoggingType::COV.to_raw(), 3, 255] {
        assert_refused(
            write(
                &mut log,
                PropertyIdentifier::LOGGING_TYPE,
                PropertyValue::Enumerated(raw),
            ),
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_refused(
            log.set_logging_type(LoggingType::from_raw(raw)),
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    assert_refused(
        write(
            &mut log,
            PropertyIdentifier::LOGGING_TYPE,
            PropertyValue::Unsigned(2),
        ),
        ErrorClass::PROPERTY,
        ErrorCode::INVALID_DATA_TYPE,
    );
    assert_eq!(
        read(&log, PropertyIdentifier::LOGGING_TYPE),
        PropertyValue::Enumerated(0)
    );
    assert_eq!(
        read(&log, PropertyIdentifier::LOG_INTERVAL),
        PropertyValue::Unsigned(0)
    );

    // TRIGGERED zeroes Log_Interval, which is then read-only.
    log.set_log_interval(500).unwrap();
    write(
        &mut log,
        PropertyIdentifier::LOGGING_TYPE,
        PropertyValue::Enumerated(2),
    )
    .unwrap();
    assert_eq!(
        read(&log, PropertyIdentifier::LOG_INTERVAL),
        PropertyValue::Unsigned(0)
    );
    assert!(!log.is_writable_property(PropertyIdentifier::LOG_INTERVAL));
    for value in [PropertyValue::Unsigned(500), PropertyValue::Null] {
        assert_refused(
            write(&mut log, PropertyIdentifier::LOG_INTERVAL, value),
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        );
    }
    assert_refused(
        log.set_log_interval(500),
        ErrorClass::PROPERTY,
        ErrorCode::WRITE_ACCESS_DENIED,
    );

    // POLLED with a zero interval takes the default; a nonzero one stays.
    log.set_logging_type(LoggingType::POLLED).unwrap();
    assert!(log.is_writable_property(PropertyIdentifier::LOG_INTERVAL));
    assert_eq!(
        read(&log, PropertyIdentifier::LOG_INTERVAL),
        PropertyValue::Unsigned(DEFAULT_LOG_INTERVAL.into())
    );
    log.set_log_interval(250).unwrap();
    write(
        &mut log,
        PropertyIdentifier::LOGGING_TYPE,
        PropertyValue::Enumerated(0),
    )
    .unwrap();
    assert_eq!(
        read(&log, PropertyIdentifier::LOG_INTERVAL),
        PropertyValue::Unsigned(250)
    );
}

#[test]
fn trigger_asks_a_triggered_log_for_one_record() {
    let (mut log, _) = clocked(8);
    // Only a TRIGGERED log takes Trigger TRUE (Clause 12.30.16).
    assert_refused(
        write(
            &mut log,
            PropertyIdentifier::TRIGGER,
            PropertyValue::Boolean(true),
        ),
        ErrorClass::PROPERTY,
        ErrorCode::NOT_CONFIGURED_FOR_TRIGGERED_LOGGING,
    );
    assert_refused(
        log.trigger(),
        ErrorClass::PROPERTY,
        ErrorCode::NOT_CONFIGURED_FOR_TRIGGERED_LOGGING,
    );
    write(
        &mut log,
        PropertyIdentifier::TRIGGER,
        PropertyValue::Boolean(false),
    )
    .unwrap();
    assert_eq!(
        read(&log, PropertyIdentifier::TRIGGER),
        PropertyValue::Boolean(false)
    );

    log.set_logging_type(LoggingType::TRIGGERED).unwrap();
    write(
        &mut log,
        PropertyIdentifier::TRIGGER,
        PropertyValue::Boolean(true),
    )
    .unwrap();
    // FALSE doesn't take back an acquisition already asked for.
    write(
        &mut log,
        PropertyIdentifier::TRIGGER,
        PropertyValue::Boolean(false),
    )
    .unwrap();
    assert_eq!(
        read(&log, PropertyIdentifier::TRIGGER),
        PropertyValue::Boolean(true)
    );
    assert_refused(
        write(
            &mut log,
            PropertyIdentifier::TRIGGER,
            PropertyValue::Unsigned(1),
        ),
        ErrorClass::PROPERTY,
        ErrorCode::INVALID_DATA_TYPE,
    );
    // The record it asked for serves it.
    log.add_record(sample(at(9, 0))).unwrap();
    assert_eq!(
        read(&log, PropertyIdentifier::TRIGGER),
        PropertyValue::Boolean(false)
    );

    // Leaving TRIGGERED drops a Trigger not yet served.
    log.trigger().unwrap();
    log.set_logging_type(LoggingType::POLLED).unwrap();
    assert_eq!(
        read(&log, PropertyIdentifier::TRIGGER),
        PropertyValue::Boolean(false)
    );
}

#[test]
fn alignment_rows_read_back_what_is_written() {
    let mut log = TrendLogMultipleObject::new(1, "TLM-1", 8).unwrap();
    assert_eq!(
        read(&log, PropertyIdentifier::ALIGN_INTERVALS),
        PropertyValue::Boolean(false)
    );
    assert_eq!(
        read(&log, PropertyIdentifier::INTERVAL_OFFSET),
        PropertyValue::Unsigned(0)
    );
    write(
        &mut log,
        PropertyIdentifier::ALIGN_INTERVALS,
        PropertyValue::Boolean(true),
    )
    .unwrap();
    write(
        &mut log,
        PropertyIdentifier::INTERVAL_OFFSET,
        PropertyValue::Unsigned(31),
    )
    .unwrap();
    assert_eq!(
        read(&log, PropertyIdentifier::ALIGN_INTERVALS),
        PropertyValue::Boolean(true)
    );
    assert_eq!(
        read(&log, PropertyIdentifier::INTERVAL_OFFSET),
        PropertyValue::Unsigned(31)
    );
    log.set_align_intervals(false);
    log.set_interval_offset(7);
    assert_eq!(
        read(&log, PropertyIdentifier::ALIGN_INTERVALS),
        PropertyValue::Boolean(false)
    );
    assert_eq!(
        read(&log, PropertyIdentifier::INTERVAL_OFFSET),
        PropertyValue::Unsigned(7)
    );
    for (property, value, code) in [
        (
            PropertyIdentifier::ALIGN_INTERVALS,
            PropertyValue::Unsigned(1),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (
            PropertyIdentifier::INTERVAL_OFFSET,
            PropertyValue::Boolean(true),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (
            PropertyIdentifier::INTERVAL_OFFSET,
            PropertyValue::Unsigned(u64::from(u32::MAX) + 1),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
    ] {
        assert_refused(write(&mut log, property, value), ErrorClass::PROPERTY, code);
    }
    assert_eq!(
        read(&log, PropertyIdentifier::INTERVAL_OFFSET),
        PropertyValue::Unsigned(7)
    );
}

#[test]
fn the_window_holds_back_records_and_logs_each_change() {
    let (mut log, clock) = clocked(16);
    // Opening at 10:00 closes the window at 09:00, which is logged at once.
    write(
        &mut log,
        PropertyIdentifier::START_TIME,
        datetime(at(10, 0)),
    )
    .unwrap();
    write(&mut log, PropertyIdentifier::STOP_TIME, datetime(at(11, 0))).unwrap();
    assert_eq!(
        read(&log, PropertyIdentifier::START_TIME),
        datetime(at(10, 0))
    );
    assert_eq!(data(&log), [DISABLED]);
    // A record outside the window is ignored, and Enable stays TRUE.
    log.add_record(sample(at(9, 0))).unwrap();
    assert_eq!(data(&log), [DISABLED]);
    assert_eq!(
        read(&log, PropertyIdentifier::LOG_ENABLE),
        PropertyValue::Boolean(true)
    );

    // Start_Time reached: logging resumes and says so.
    clock.set(at(10, 0));
    assert!(log.refresh_log_window_internal());
    assert!(!log.refresh_log_window_internal());
    log.add_record(sample(at(10, 0))).unwrap();
    assert_eq!(data(&log), [DISABLED, ENABLED, sample(at(10, 0)).log_data]);

    // Stop_Time reached: the next record finds the window shut first.
    clock.set(at(11, 0));
    log.add_record(sample(at(11, 0))).unwrap();
    assert_eq!(data(&log)[3..], [DISABLED]);
    assert_eq!(log.records().len(), 4);

    // Outside the window Enable changes leave logging off, so nothing is
    // logged for them; a purge there carries LOG_DISABLED.
    write(
        &mut log,
        PropertyIdentifier::LOG_ENABLE,
        PropertyValue::Boolean(false),
    )
    .unwrap();
    assert_eq!(
        read(&log, PropertyIdentifier::LOG_ENABLE),
        PropertyValue::Boolean(false)
    );
    write(
        &mut log,
        PropertyIdentifier::LOG_ENABLE,
        PropertyValue::Boolean(true),
    )
    .unwrap();
    assert_eq!(log.records().len(), 4);
    write(
        &mut log,
        PropertyIdentifier::RECORD_COUNT,
        PropertyValue::Unsigned(0),
    )
    .unwrap();
    assert_eq!(
        data(&log),
        [LogData::LogStatus(
            LogStatus::BUFFER_PURGED | LogStatus::LOG_DISABLED
        )]
    );

    // While Enable is FALSE the window moves without a record; Enable
    // written TRUE inside it then logs a clear status.
    write(
        &mut log,
        PropertyIdentifier::LOG_ENABLE,
        PropertyValue::Boolean(false),
    )
    .unwrap();
    clock.set(at(10, 30));
    assert!(!log.refresh_log_window_internal());
    write(
        &mut log,
        PropertyIdentifier::LOG_ENABLE,
        PropertyValue::Boolean(true),
    )
    .unwrap();
    assert_eq!(data(&log)[1..], [ENABLED]);

    // An unspecified Stop_Time leaves the window open after 11:00.
    clock.set(at(12, 0));
    write(
        &mut log,
        PropertyIdentifier::STOP_TIME,
        datetime(crate::clock::UNSPECIFIED_DATETIME),
    )
    .unwrap();
    assert!(!log.refresh_log_window_internal());
    assert_eq!(log.records().len(), 2);
}

#[test]
fn each_record_is_judged_by_its_own_timestamp() {
    // The clock already reads 10:00 when a record taken at 09:59 arrives:
    // it is outside the window, and the opening it hasn't seen yet is
    // logged with the next record, at that record's time.
    let (mut log, clock) = clocked(16);
    write(
        &mut log,
        PropertyIdentifier::START_TIME,
        datetime(at(10, 0)),
    )
    .unwrap();
    clock.set(at(10, 0));
    log.add_record(sample(at(9, 59))).unwrap();
    assert_eq!(data(&log), [DISABLED]);
    log.add_record(sample(at(10, 0))).unwrap();
    assert_eq!(data(&log), [DISABLED, ENABLED, sample(at(10, 0)).log_data]);
    assert_eq!(log.records()[1].time, at(10, 0).1);

    // The other way round: the clock still reads 10:59 when a record taken
    // at 11:00 arrives; the closing is logged at 11:00 and the record kept
    // out.
    write(&mut log, PropertyIdentifier::STOP_TIME, datetime(at(11, 0))).unwrap();
    clock.set(at(10, 59));
    log.add_record(sample(at(11, 0))).unwrap();
    assert_eq!(data(&log)[3..], [DISABLED]);
    assert_eq!(log.records()[3].time, at(11, 0).1);
}

#[test]
fn window_writes_refuse_what_is_not_an_actual_moment() {
    let (mut log, _) = clocked(16);
    let (date, time) = at(10, 0);
    for value in [
        datetime((
            Date {
                year: Date::UNSPECIFIED,
                ..date
            },
            time,
        )),
        datetime((
            date,
            Time {
                minute: Time::UNSPECIFIED,
                ..time
            },
        )),
    ] {
        for property in [
            PropertyIdentifier::START_TIME,
            PropertyIdentifier::STOP_TIME,
        ] {
            assert_refused(
                write(&mut log, property, value.clone()),
                ErrorClass::PROPERTY,
                ErrorCode::VALUE_OUT_OF_RANGE,
            );
        }
    }
    assert_refused(
        write(
            &mut log,
            PropertyIdentifier::START_TIME,
            PropertyValue::Date(date),
        ),
        ErrorClass::PROPERTY,
        ErrorCode::INVALID_DATA_TYPE,
    );
    assert_refused(
        log.set_stop_time(date, Time { hour: 24, ..time }),
        ErrorClass::PROPERTY,
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_eq!(
        read(&log, PropertyIdentifier::START_TIME),
        datetime(crate::clock::UNSPECIFIED_DATETIME)
    );
    assert!(log.records().is_empty());
}

#[test]
fn a_local_window_is_configuration_and_its_opening_is_logged() {
    let (mut log, clock) = clocked(16);
    // Set before the first look: noting the closed window logs nothing.
    let (date, time) = at(10, 0);
    log.set_start_time(date, time).unwrap();
    assert!(!log.refresh_log_window_internal());
    log.add_record(sample(at(9, 0))).unwrap();
    assert!(log.records().is_empty());
    clock.set(at(10, 0));
    assert!(log.refresh_log_window_internal());
    assert_eq!(data(&log), [ENABLED]);
}

#[test]
fn an_opening_that_would_fill_a_stop_when_full_log_disables_it() {
    let (mut log, clock) = clocked(2);
    write(
        &mut log,
        PropertyIdentifier::STOP_WHEN_FULL,
        PropertyValue::Boolean(true),
    )
    .unwrap();
    write(
        &mut log,
        PropertyIdentifier::START_TIME,
        datetime(at(10, 0)),
    )
    .unwrap();
    assert_eq!(data(&log), [DISABLED]);
    // The clear status would fill the buffer, so Enable goes FALSE as an
    // Enable write would make it.
    clock.set(at(10, 0));
    assert!(log.refresh_log_window_internal());
    assert_eq!(data(&log), [DISABLED, DISABLED]);
    assert_eq!(
        read(&log, PropertyIdentifier::LOG_ENABLE),
        PropertyValue::Boolean(false)
    );
}
