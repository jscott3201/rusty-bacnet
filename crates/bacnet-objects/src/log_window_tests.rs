//! The Start_Time / Stop_Time window on each log object (#1235, #1353),
//! against a clock the test sets: a record outside it is ignored, an opening
//! or closing while Enable is TRUE is logged at a write and at the next look
//! (the look the trend poller makes on each pass), and a value that names
//! no moment is refused. Trend Log Multiple's own cases are in
//! `trend::multiple_options_tests`; the poller's passes in
//! `database::trend_poll`.

use std::sync::{Arc, Mutex};

use bacnet_encoding::constructed::{
    decode_event_log_record, decode_log_multiple_record, decode_log_record,
};
use bacnet_types::bitstring::LogStatus;
use bacnet_types::calendar::SpecificDate;
use bacnet_types::constructed::{
    BACnetEventLogRecord, BACnetLogMultipleRecord, BACnetLogRecord, EventLogDatum, LogData,
    LogDatum, LogValue,
};
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{Date, PropertyValue, Time};
use bytes::BytesMut;

use crate::clock::{ClockFrame, ClockReader, UNSPECIFIED_DATETIME};
use crate::event_log::EventLogObject;
use crate::traits::BACnetObject;
use crate::trend::{TrendLogMultipleObject, TrendLogObject};

struct Clock(Mutex<Option<ClockFrame>>);

impl ClockReader for Clock {
    fn read_clock(&self) -> Option<ClockFrame> {
        *self.0.lock().unwrap()
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

fn datetime((date, time): (Date, Time)) -> PropertyValue {
    PropertyValue::List(vec![PropertyValue::Date(date), PropertyValue::Time(time)])
}

fn assert_refused(result: Result<(), Error>, code: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Remote(RemoteError::Error { class, code: c }))
            if class == ErrorClass::PROPERTY && c == code),
        "expected PROPERTY / {code:?}, got {result:?}"
    );
}

/// What a test looks for in a record: a log status, or an ordinary record.
#[derive(Debug, PartialEq)]
enum Seen {
    Status(LogStatus),
    Record,
}

const DISABLED: Seen = Seen::Status(LogStatus::LOG_DISABLED);
const ENABLED: Seen = Seen::Status(LogStatus::empty());

#[derive(Debug, Clone, Copy)]
enum Kind {
    Trend,
    Event,
    Multiple,
}

const KINDS: [Kind; 3] = [Kind::Trend, Kind::Event, Kind::Multiple];

/// A log of each kind as its own type, so its local setters can be called.
enum Object {
    Trend(TrendLogObject),
    Event(EventLogObject),
    Multiple(TrendLogMultipleObject),
}

/// One log of `kind` bound to a clock reading 09:00.
struct Log {
    kind: Kind,
    object: Object,
    clock: Arc<Clock>,
}

impl Log {
    /// A log whose window is open at both ends, as built.
    fn new(kind: Kind) -> Self {
        let object = match kind {
            Kind::Trend => Object::Trend(TrendLogObject::new(1, "TL-1", 16).unwrap()),
            Kind::Event => Object::Event(EventLogObject::new(1, "EL-1", 16).unwrap()),
            Kind::Multiple => {
                Object::Multiple(TrendLogMultipleObject::new(1, "TLM-1", 16).unwrap())
            }
        };
        let mut log = Self {
            kind,
            object,
            clock: Arc::new(Clock(Mutex::new(None))),
        };
        log.set_clock(at(9, 0));
        let clock = log.clock.clone();
        log.object_mut().bind_clock_internal(Some(clock));
        log
    }

    /// A log whose window the local setters gave, or their refusal.
    fn configured(kind: Kind, start: (Date, Time), stop: (Date, Time)) -> Result<Self, Error> {
        let mut log = Self::new(kind);
        log.set_end(P::START_TIME, start)?;
        log.set_end(P::STOP_TIME, stop)?;
        Ok(log)
    }

    /// Set Start_Time or Stop_Time through the object's local setter.
    fn set_end(&mut self, property: P, (date, time): (Date, Time)) -> Result<(), Error> {
        let start = property == P::START_TIME;
        match &mut self.object {
            Object::Trend(log) if start => log.set_start_time(date, time),
            Object::Trend(log) => log.set_stop_time(date, time),
            Object::Event(log) if start => log.set_start_time(date, time),
            Object::Event(log) => log.set_stop_time(date, time),
            Object::Multiple(log) if start => log.set_start_time(date, time),
            Object::Multiple(log) => log.set_stop_time(date, time),
        }
    }

    fn object(&self) -> &dyn BACnetObject {
        match &self.object {
            Object::Trend(log) => log,
            Object::Event(log) => log,
            Object::Multiple(log) => log,
        }
    }

    fn object_mut(&mut self) -> &mut dyn BACnetObject {
        match &mut self.object {
            Object::Trend(log) => log,
            Object::Event(log) => log,
            Object::Multiple(log) => log,
        }
    }

    fn set_clock(&self, (local_date, local_time): (Date, Time)) {
        *self.clock.0.lock().unwrap() = Some(ClockFrame {
            local_date,
            local_time,
            utc_offset: 0,
            daylight_savings_status: false,
        });
    }

    /// Take the clock away, so no look can tell the time.
    fn clear_clock(&self) {
        *self.clock.0.lock().unwrap() = None;
    }

    fn write(&mut self, property: P, value: PropertyValue) -> Result<(), Error> {
        self.object_mut()
            .write_property(property, None, value, None)
    }

    fn read(&self, property: P) -> PropertyValue {
        self.object().read_property(property, None).unwrap()
    }

    /// The look the poller's pass makes; whether it logged a change.
    fn pass(&mut self) -> bool {
        self.object_mut().refresh_log_window_internal()
    }

    /// Offer an ordinary record taken at `(date, time)`.
    fn add(&mut self, (date, time): (Date, Time)) {
        let kind = self.kind;
        let object = self.object_mut();
        match kind {
            Kind::Trend => object.add_trend_record(BACnetLogRecord {
                date,
                time,
                log_datum: LogDatum::RealValue(1.5),
                status_flags: None,
            }),
            Kind::Event => object.add_event_log_record(BACnetEventLogRecord {
                date,
                time,
                log_datum: EventLogDatum::TimeChange(1.5),
            }),
            Kind::Multiple => object.add_trend_multiple_record(BACnetLogMultipleRecord {
                date,
                time,
                log_data: LogData::Values(vec![LogValue::RealValue(1.5)]),
            }),
        }
        .unwrap();
    }

    /// The resident records, as ReadRange serves them.
    fn seen(&self) -> Vec<Seen> {
        let records = self.object().log_buffer_internal().unwrap();
        (0..records.record_count())
            .map(|index| {
                let mut bytes = BytesMut::new();
                records.encode_record(index, &mut bytes);
                let status = match self.kind {
                    Kind::Trend => match decode_log_record(&bytes, 0).unwrap().0.log_datum {
                        LogDatum::LogStatus(status) => Some(status),
                        _ => None,
                    },
                    Kind::Event => match decode_event_log_record(&bytes, 0).unwrap().0.log_datum {
                        EventLogDatum::LogStatus(status) => Some(status),
                        _ => None,
                    },
                    Kind::Multiple => {
                        match decode_log_multiple_record(&bytes, 0).unwrap().0.log_data {
                            LogData::LogStatus(status) => Some(status),
                            _ => None,
                        }
                    }
                };
                status.map_or(Seen::Record, Seen::Status)
            })
            .collect()
    }
}

#[test]
fn records_outside_the_window_are_ignored_and_each_change_is_logged() {
    for kind in KINDS {
        let mut log = Log::new(kind);
        // Opening at 10:00 shuts the window at 09:00: logged at the write.
        log.write(P::START_TIME, datetime(at(10, 0))).unwrap();
        log.write(P::STOP_TIME, datetime(at(11, 0))).unwrap();
        assert_eq!(log.read(P::START_TIME), datetime(at(10, 0)), "{kind:?}");
        assert_eq!(log.read(P::STOP_TIME), datetime(at(11, 0)), "{kind:?}");
        assert_eq!(log.seen(), [DISABLED], "{kind:?}");
        // A record outside the window is ignored, and Enable stays TRUE.
        log.add(at(9, 30));
        assert_eq!(log.seen(), [DISABLED], "{kind:?}");
        assert_eq!(log.read(P::LOG_ENABLE), PropertyValue::Boolean(true));

        // Start_Time reached: the next pass logs the opening, once.
        log.set_clock(at(10, 0));
        assert!(log.pass(), "{kind:?}");
        assert!(!log.pass(), "{kind:?}");
        log.add(at(10, 30));
        assert_eq!(log.seen(), [DISABLED, ENABLED, Seen::Record], "{kind:?}");

        // Stop_Time reached: the next pass logs the closing, and a record
        // taken from then on is ignored.
        log.set_clock(at(11, 0));
        assert!(log.pass(), "{kind:?}");
        log.add(at(11, 0));
        assert_eq!(log.seen()[3..], [DISABLED], "{kind:?}");

        // A write that opens the window logs it at once, and one that shuts
        // it again too.
        log.write(P::STOP_TIME, datetime(UNSPECIFIED_DATETIME))
            .unwrap();
        log.add(at(11, 0));
        assert_eq!(log.seen()[4..], [ENABLED, Seen::Record], "{kind:?}");
        log.write(P::START_TIME, datetime(at(12, 0))).unwrap();
        assert_eq!(log.seen()[6..], [DISABLED], "{kind:?}");
        assert!(!log.pass(), "{kind:?}");
    }
}

#[test]
fn while_enable_is_false_the_window_moves_without_a_record() {
    for kind in KINDS {
        let mut log = Log::new(kind);
        log.write(P::START_TIME, datetime(at(10, 0))).unwrap();
        log.write(P::LOG_ENABLE, PropertyValue::Boolean(false))
            .unwrap();
        // While shut, the Enable change keeps logging off: no record.
        assert_eq!(log.seen(), [DISABLED], "{kind:?}");
        log.set_clock(at(10, 0));
        assert!(!log.pass(), "{kind:?}");
        assert_eq!(log.seen(), [DISABLED], "{kind:?}");
        // Enable written TRUE inside the window logs a clear status.
        log.write(P::LOG_ENABLE, PropertyValue::Boolean(true))
            .unwrap();
        assert_eq!(log.seen(), [DISABLED, ENABLED], "{kind:?}");
        // A purge outside the window carries LOG_DISABLED.
        log.write(P::STOP_TIME, datetime(at(10, 0))).unwrap();
        log.write(P::RECORD_COUNT, PropertyValue::Unsigned(0))
            .unwrap();
        assert_eq!(
            log.seen(),
            [Seen::Status(
                LogStatus::BUFFER_PURGED | LogStatus::LOG_DISABLED
            )],
            "{kind:?}"
        );
    }
}

#[test]
fn window_values_that_name_no_moment_are_value_out_of_range() {
    let (date, time) = at(10, 0);
    let partly = [
        (
            Date {
                year: Date::UNSPECIFIED,
                ..date
            },
            time,
        ),
        (Date { day: 32, ..date }, time),
        (
            date,
            Time {
                minute: Time::UNSPECIFIED,
                ..time
            },
        ),
        (date, Time { hour: 24, ..time }),
        (date, UNSPECIFIED_DATETIME.1),
        (UNSPECIFIED_DATETIME.0, time),
    ];
    for kind in KINDS {
        let mut log = Log::new(kind);
        for value in partly {
            for property in [P::START_TIME, P::STOP_TIME] {
                assert_refused(
                    log.write(property, datetime(value)),
                    ErrorCode::VALUE_OUT_OF_RANGE,
                );
            }
            // The local setters refuse it too.
            for (start, stop) in [(value, UNSPECIFIED_DATETIME), (UNSPECIFIED_DATETIME, value)] {
                assert_refused(
                    Log::configured(kind, start, stop).map(|_| ()),
                    ErrorCode::VALUE_OUT_OF_RANGE,
                );
            }
        }
        assert_refused(
            log.write(P::START_TIME, PropertyValue::Date(date)),
            ErrorCode::INVALID_DATA_TYPE,
        );
        // Nothing changed and nothing was logged.
        for property in [P::START_TIME, P::STOP_TIME] {
            assert_eq!(
                log.read(property),
                datetime(UNSPECIFIED_DATETIME),
                "{kind:?}"
            );
        }
        assert!(log.seen().is_empty(), "{kind:?}");
        // Unspecified seconds and hundredths count as zero.
        let loose = Time {
            second: Time::UNSPECIFIED,
            hundredths: Time::UNSPECIFIED,
            ..time
        };
        log.write(P::START_TIME, datetime((date, loose))).unwrap();
        assert_eq!(log.read(P::START_TIME), datetime((date, loose)));
        assert_eq!(log.seen(), [DISABLED], "{kind:?}");
    }
}

#[test]
fn a_local_window_is_configuration_and_its_opening_is_logged() {
    for kind in KINDS {
        // Set before the first look: noting the shut window logs nothing.
        let mut log = Log::configured(kind, at(10, 0), at(11, 0)).unwrap();
        assert!(!log.pass(), "{kind:?}");
        log.add(at(9, 0));
        assert!(log.seen().is_empty(), "{kind:?}");
        log.set_clock(at(10, 0));
        assert!(log.pass(), "{kind:?}");
        log.set_clock(at(11, 0));
        assert!(log.pass(), "{kind:?}");
        assert_eq!(log.seen(), [ENABLED, DISABLED], "{kind:?}");
    }
}

#[test]
fn a_window_opened_at_both_ends_logs_its_opening_at_the_write() {
    for kind in KINDS {
        // With a clock, the write that leaves both ends open logs it.
        let mut log = Log::new(kind);
        log.write(P::START_TIME, datetime(at(10, 0))).unwrap();
        log.write(P::START_TIME, datetime(UNSPECIFIED_DATETIME))
            .unwrap();
        assert_eq!(log.seen(), [DISABLED, ENABLED], "{kind:?}");
        assert!(!log.pass(), "{kind:?}");
    }
}

#[test]
fn a_window_opened_at_both_ends_without_a_clock_logs_it_at_the_next_pass() {
    for kind in KINDS {
        // No valid clock at the write: the first pass with one logs it.
        let mut log = Log::new(kind);
        log.write(P::START_TIME, datetime(at(10, 0))).unwrap();
        log.clear_clock();
        log.write(P::START_TIME, datetime(UNSPECIFIED_DATETIME))
            .unwrap();
        assert!(!log.pass(), "{kind:?}");
        assert_eq!(log.seen(), [DISABLED], "{kind:?}");
        log.set_clock(at(9, 30));
        assert!(log.pass(), "{kind:?}");
        assert_eq!(log.seen(), [DISABLED, ENABLED], "{kind:?}");
    }
}

#[test]
fn a_client_write_after_a_local_setter_logs_the_change() {
    for kind in KINDS {
        // The setter notes the window open at 09:00 without a record, so the
        // client's write that shuts it is logged with no pass between.
        let mut log = Log::new(kind);
        log.set_end(P::STOP_TIME, at(10, 0)).unwrap();
        assert!(log.seen().is_empty(), "{kind:?}");
        log.write(P::STOP_TIME, datetime(at(8, 0))).unwrap();
        assert_eq!(log.seen(), [DISABLED], "{kind:?}");
    }
}
