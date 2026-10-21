//! Trend Log's acquisition rows (#1354): Logging_Type's POLLED, TRIGGERED
//! and COV (#1480), Log_Interval's mode rules and switches (Clause 12.25.9),
//! the two COV rows, Trigger and the clock-alignment rows. The window is in
//! `crate::log_window_tests`, shared with the Event Log.

use super::*;
use crate::clock::ClockFrame;
use bacnet_types::constructed::LogDatum;
use bacnet_types::primitives::{Date, Time};

fn read(log: &TrendLogObject, property: PropertyIdentifier) -> PropertyValue {
    log.read_property(property, None).unwrap()
}

fn write(
    log: &mut TrendLogObject,
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

fn interval(log: &TrendLogObject) -> PropertyValue {
    read(log, PropertyIdentifier::LOG_INTERVAL)
}

fn record() -> BACnetLogRecord {
    BACnetLogRecord {
        date: Date {
            year: 126,
            month: 10,
            day: 4,
            day_of_week: 7,
        },
        time: Time {
            hour: 9,
            minute: 0,
            second: 0,
            hundredths: 0,
        },
        log_datum: LogDatum::RealValue(1.5),
        status_flags: None,
    }
}

#[test]
fn logging_type_takes_polled_triggered_and_cov() {
    let mut log = TrendLogObject::new(1, "TL-1", 8).unwrap();
    log.set_log_interval(500).unwrap();
    // A value outside BACnetLoggingType is one the object doesn't support,
    // so it gets the Clause 12.25.26 error, through the wire and the setter
    // alike.
    for raw in [3, 255] {
        assert_refused(
            write(
                &mut log,
                PropertyIdentifier::LOGGING_TYPE,
                PropertyValue::Enumerated(raw),
            ),
            ErrorClass::PROPERTY,
            ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
        );
        assert_refused(
            log.set_logging_type(LoggingType::from_raw(raw)),
            ErrorClass::PROPERTY,
            ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
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
    assert_eq!(logging_type(&log), LoggingType::POLLED);
    assert_eq!(interval(&log), PropertyValue::Unsigned(500));

    // COV, by the wire and by the setter, zeroes Log_Interval (Clause
    // 12.25.26), which stays writable (Table 12-29 footnote 3).
    write(
        &mut log,
        PropertyIdentifier::LOGGING_TYPE,
        PropertyValue::Enumerated(LoggingType::COV.to_raw()),
    )
    .unwrap();
    assert_eq!(logging_type(&log), LoggingType::COV);
    assert_eq!(interval(&log), PropertyValue::Unsigned(0));
    assert!(log.is_writable_property(PropertyIdentifier::LOG_INTERVAL));
    log.set_logging_type(LoggingType::POLLED).unwrap();
    log.set_log_interval(500).unwrap();
    log.set_logging_type(LoggingType::COV).unwrap();
    assert_eq!(logging_type(&log), LoggingType::COV);
    assert_eq!(interval(&log), PropertyValue::Unsigned(0));
    // A Trigger is for a TRIGGERED log only.
    assert_refused(
        log.trigger(),
        ErrorClass::PROPERTY,
        ErrorCode::NOT_CONFIGURED_FOR_TRIGGERED_LOGGING,
    );

    // TRIGGERED zeroes Log_Interval, which is then read-only.
    write(
        &mut log,
        PropertyIdentifier::LOGGING_TYPE,
        PropertyValue::Enumerated(2),
    )
    .unwrap();
    assert_eq!(interval(&log), PropertyValue::Unsigned(0));
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
        interval(&log),
        PropertyValue::Unsigned(DEFAULT_LOG_INTERVAL.into())
    );
    log.set_log_interval(250).unwrap();
    log.set_logging_type(LoggingType::POLLED).unwrap();
    assert_eq!(interval(&log), PropertyValue::Unsigned(250));
}

fn logging_type(log: &TrendLogObject) -> LoggingType {
    let PropertyValue::Enumerated(raw) = read(log, PropertyIdentifier::LOGGING_TYPE) else {
        panic!("Logging_Type is an enumeration");
    };
    LoggingType::from_raw(raw)
}

#[test]
fn log_interval_switches_between_polled_and_cov() {
    let mut log = TrendLogObject::new(1, "TL-1", 8).unwrap();
    // A new log is POLLED at zero: writing zero again changes nothing and
    // asks for no switch.
    write(
        &mut log,
        PropertyIdentifier::LOG_INTERVAL,
        PropertyValue::Unsigned(0),
    )
    .unwrap();
    log.set_log_interval(0).unwrap();
    assert_eq!(logging_type(&log), LoggingType::POLLED);
    write(
        &mut log,
        PropertyIdentifier::LOG_INTERVAL,
        PropertyValue::Unsigned(300),
    )
    .unwrap();
    // From nonzero to zero switches a POLLED log to COV (Clause 12.25.9),
    // by the wire and by the setter.
    write(
        &mut log,
        PropertyIdentifier::LOG_INTERVAL,
        PropertyValue::Unsigned(0),
    )
    .unwrap();
    assert_eq!(logging_type(&log), LoggingType::COV);
    assert_eq!(interval(&log), PropertyValue::Unsigned(0));
    // Zero again leaves it COV.
    log.set_log_interval(0).unwrap();
    assert_eq!(logging_type(&log), LoggingType::COV);
    // A nonzero interval switches a COV log back to POLLED at it.
    write(
        &mut log,
        PropertyIdentifier::LOG_INTERVAL,
        PropertyValue::Unsigned(700),
    )
    .unwrap();
    assert_eq!(logging_type(&log), LoggingType::POLLED);
    assert_eq!(interval(&log), PropertyValue::Unsigned(700));
    log.set_log_interval(0).unwrap();
    assert_eq!(logging_type(&log), LoggingType::COV);
    log.set_log_interval(40).unwrap();
    assert_eq!(logging_type(&log), LoggingType::POLLED);
    assert_eq!(interval(&log), PropertyValue::Unsigned(40));
    // Any other interval is taken, and so are the usual refusals.
    log.set_log_interval(1).unwrap();
    assert_eq!(interval(&log), PropertyValue::Unsigned(1));
    for (value, code) in [
        (
            PropertyValue::Unsigned(u64::from(u32::MAX) + 1),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (PropertyValue::Boolean(true), ErrorCode::INVALID_DATA_TYPE),
    ] {
        assert_refused(
            write(&mut log, PropertyIdentifier::LOG_INTERVAL, value),
            ErrorClass::PROPERTY,
            code,
        );
    }
    assert_eq!(interval(&log), PropertyValue::Unsigned(1));

    // A Trend Log Multiple has no such switch: zero leaves it idle and
    // POLLED, and COV is out of its range (Clause 12.30.12).
    let mut multiple = TrendLogMultipleObject::new(1, "TLM-1", 8).unwrap();
    multiple.set_log_interval(300).unwrap();
    multiple.set_log_interval(0).unwrap();
    assert_eq!(
        multiple
            .read_property(PropertyIdentifier::LOG_INTERVAL, None)
            .unwrap(),
        PropertyValue::Unsigned(0)
    );
    assert_eq!(
        multiple
            .read_property(PropertyIdentifier::LOGGING_TYPE, None)
            .unwrap(),
        PropertyValue::Enumerated(0)
    );
    assert_refused(
        multiple.set_logging_type(LoggingType::COV),
        ErrorClass::PROPERTY,
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
}

#[test]
fn the_cov_rows_take_their_datatypes_and_ranges() {
    let mut log = TrendLogObject::new(1, "TL-1", 8).unwrap();
    let increment = PropertyIdentifier::CLIENT_COV_INCREMENT;
    let resubscription = PropertyIdentifier::COV_RESUBSCRIPTION_INTERVAL;
    // NULL, the referenced property's own criteria, by default.
    assert_eq!(read(&log, increment), PropertyValue::Null);
    assert_eq!(
        read(&log, resubscription),
        PropertyValue::Unsigned(DEFAULT_COV_RESUBSCRIPTION_INTERVAL.into())
    );
    write(&mut log, increment, PropertyValue::Real(0.5)).unwrap();
    assert_eq!(read(&log, increment), PropertyValue::Real(0.5));
    write(&mut log, increment, PropertyValue::Null).unwrap();
    assert_eq!(read(&log, increment), PropertyValue::Null);
    log.set_client_cov_increment(Some(2.0)).unwrap();
    assert_eq!(read(&log, increment), PropertyValue::Real(2.0));
    for value in [-1.0, f32::NAN, f32::INFINITY] {
        assert_refused(
            write(&mut log, increment, PropertyValue::Real(value)),
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_refused(
            log.set_client_cov_increment(Some(value)),
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    for value in [PropertyValue::Unsigned(1), PropertyValue::Double(1.0)] {
        assert_refused(
            write(&mut log, increment, value),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_TYPE,
        );
    }
    assert_eq!(read(&log, increment), PropertyValue::Real(2.0));

    write(&mut log, resubscription, PropertyValue::Unsigned(60)).unwrap();
    assert_eq!(read(&log, resubscription), PropertyValue::Unsigned(60));
    log.set_cov_resubscription_interval(90).unwrap();
    assert_eq!(read(&log, resubscription), PropertyValue::Unsigned(90));
    // Clause 12.25.10 keeps it nonzero.
    for (value, code) in [
        (PropertyValue::Unsigned(0), ErrorCode::VALUE_OUT_OF_RANGE),
        (
            PropertyValue::Unsigned(u64::from(u32::MAX) + 1),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (PropertyValue::Null, ErrorCode::INVALID_DATA_TYPE),
    ] {
        assert_refused(
            write(&mut log, resubscription, value),
            ErrorClass::PROPERTY,
            code,
        );
    }
    assert_refused(
        log.set_cov_resubscription_interval(0),
        ErrorClass::PROPERTY,
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_eq!(read(&log, resubscription), PropertyValue::Unsigned(90));
    // Neither row is on a Trend Log Multiple (Table 12-35).
    let multiple = TrendLogMultipleObject::new(1, "TLM-1", 8).unwrap();
    for property in [increment, resubscription] {
        assert!(multiple.read_property(property, None).is_err());
    }
}

#[test]
fn trigger_asks_a_triggered_log_for_one_record() {
    let mut log = TrendLogObject::new(1, "TL-1", 8).unwrap();
    // Only a TRIGGERED log takes Trigger TRUE (Clause 12.25.29).
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
    log.add_record(record()).unwrap();
    assert_eq!(
        read(&log, PropertyIdentifier::TRIGGER),
        PropertyValue::Boolean(false)
    );
    assert_eq!(log.records().len(), 1);

    // Leaving TRIGGERED drops a Trigger not yet served.
    log.trigger().unwrap();
    log.set_logging_type(LoggingType::POLLED).unwrap();
    assert_eq!(
        read(&log, PropertyIdentifier::TRIGGER),
        PropertyValue::Boolean(false)
    );
}

#[test]
fn a_record_the_log_ignores_still_serves_the_trigger() {
    struct Nine;
    impl ClockReader for Nine {
        fn read_clock(&self) -> Option<ClockFrame> {
            Some(ClockFrame {
                local_date: record().date,
                local_time: record().time,
                utc_offset: 0,
                daylight_savings_status: false,
            })
        }
    }
    let mut log = TrendLogObject::new(1, "TL-1", 8).unwrap();
    log.bind_clock_internal(Some(Arc::new(Nine)));
    log.set_logging_type(LoggingType::TRIGGERED).unwrap();
    let trigger = |log: &TrendLogObject| read(log, PropertyIdentifier::TRIGGER);

    // Enable FALSE: the record is ignored, but the acquisition was made.
    write(
        &mut log,
        PropertyIdentifier::LOG_ENABLE,
        PropertyValue::Boolean(false),
    )
    .unwrap();
    log.trigger().unwrap();
    log.add_record(record()).unwrap();
    assert_eq!(trigger(&log), PropertyValue::Boolean(false));
    assert!(log
        .records()
        .iter()
        .all(|r| matches!(r.log_datum, LogDatum::LogStatus(_))));

    // Outside the window, the same.
    write(
        &mut log,
        PropertyIdentifier::LOG_ENABLE,
        PropertyValue::Boolean(true),
    )
    .unwrap();
    let (date, time) = (record().date, record().time);
    log.set_start_time(date, Time { hour: 10, ..time }).unwrap();
    log.trigger().unwrap();
    log.add_record(record()).unwrap();
    assert_eq!(trigger(&log), PropertyValue::Boolean(false));
    assert!(log
        .records()
        .iter()
        .all(|r| matches!(r.log_datum, LogDatum::LogStatus(_))));
}

#[test]
fn alignment_rows_read_back_what_is_written() {
    let mut log = TrendLogObject::new(1, "TL-1", 8).unwrap();
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
    for (property, value) in [
        (
            PropertyIdentifier::ALIGN_INTERVALS,
            PropertyValue::Unsigned(1),
        ),
        (
            PropertyIdentifier::INTERVAL_OFFSET,
            PropertyValue::Boolean(true),
        ),
    ] {
        assert_refused(
            write(&mut log, property, value),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_TYPE,
        );
    }
}
