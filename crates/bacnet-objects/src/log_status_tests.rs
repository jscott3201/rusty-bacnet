use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use bacnet_encoding::constructed::{
    decode_event_log_record, decode_log_multiple_record, decode_log_record,
};
use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::{
    BACnetEventLogRecord, BACnetLogMultipleRecord, BACnetLogRecord, EventLogDatum, LogData,
    LogDatum, LogValue,
};
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, PropertyValue, Time};
use bytes::BytesMut;

use crate::clock::{ClockFrame, ClockReader};
use crate::event_log::EventLogObject;
use crate::traits::BACnetObject;
use crate::trend::{TrendLogMultipleObject, TrendLogObject};

#[path = "log_insertion_tests.rs"]
mod insertion;
#[path = "log_restore_tests.rs"]
mod restore;

const LOG_DISABLED: LogStatus = LogStatus::LOG_DISABLED;
const BUFFER_PURGED: LogStatus = LogStatus::BUFFER_PURGED;

#[derive(Clone, Copy, Debug)]
enum FamilyKind {
    Event,
    Trend,
    TrendMultiple,
}

impl FamilyKind {
    const ALL: [Self; 3] = [Self::Event, Self::Trend, Self::TrendMultiple];

    fn object(self, capacity: u32) -> Family {
        match self {
            Self::Event => Family::Event(EventLogObject::new(1, "EL-1", capacity).unwrap()),
            Self::Trend => Family::Trend(TrendLogObject::new(1, "TL-1", capacity).unwrap()),
            Self::TrendMultiple => {
                Family::TrendMultiple(TrendLogMultipleObject::new(1, "TLM-1", capacity).unwrap())
            }
        }
    }
}

enum Family {
    Event(EventLogObject),
    Trend(TrendLogObject),
    TrendMultiple(TrendLogMultipleObject),
}

impl Family {
    fn object(&self) -> &dyn BACnetObject {
        match self {
            Self::Event(object) => object,
            Self::Trend(object) => object,
            Self::TrendMultiple(object) => object,
        }
    }

    fn object_mut(&mut self) -> &mut dyn BACnetObject {
        match self {
            Self::Event(object) => object,
            Self::Trend(object) => object,
            Self::TrendMultiple(object) => object,
        }
    }

    /// The resident records, an Event Log's and a Trend Log Multiple's as
    /// Trend Log records.
    fn records(&self) -> VecDeque<BACnetLogRecord> {
        match self {
            Self::Event(object) => object.records().iter().map(single_event).collect(),
            Self::Trend(object) => object.records().clone(),
            Self::TrendMultiple(object) => object.records().iter().map(single).collect(),
        }
    }

    fn add_record(&mut self, record: BACnetLogRecord) -> Result<(), Error> {
        match self {
            Self::Event(object) => object.add_record(event(record)),
            Self::Trend(object) => object.add_record(record),
            Self::TrendMultiple(object) => object.add_record(multiple(record)),
        }
    }

    /// The family's `restore_log_buffer` (#1537).
    fn restore(&mut self, total: u32, records: Vec<BACnetLogRecord>) -> Result<(), Error> {
        match self {
            Self::Event(object) => object.restore_log_buffer(total, records.into_iter().map(event)),
            Self::Trend(object) => object.restore_log_buffer(total, records),
            Self::TrendMultiple(object) => {
                object.restore_log_buffer(total, records.into_iter().map(multiple))
            }
        }
    }

    /// The family's `record_interruption` (#1537).
    fn interrupt(&mut self, date: Date, time: Time) -> Result<(), Error> {
        match self {
            Self::Event(object) => object.record_interruption(date, time),
            Self::Trend(object) => object.record_interruption(date, time),
            Self::TrendMultiple(object) => object.record_interruption(date, time),
        }
    }

    /// The newest record as ReadRange serves it.
    fn served_last(&self) -> Vec<u8> {
        let records = self.object().log_buffer_internal().unwrap();
        let mut bytes = BytesMut::new();
        records.encode_record(records.record_count() - 1, &mut bytes);
        bytes.to_vec()
    }

    /// Submit `record` through the trend insertion hook its family serves.
    fn add_trend_record(&mut self, record: BACnetLogRecord) -> Result<(), Error> {
        match self {
            Self::TrendMultiple(object) => object.add_trend_multiple_record(multiple(record)),
            _ => self.object_mut().add_trend_record(record),
        }
    }

    fn clear(&mut self) {
        match self {
            Self::Event(object) => object.clear(),
            Self::Trend(object) => object.clear(),
            Self::TrendMultiple(object) => object.clear(),
        }
    }

    fn write(&mut self, property: PropertyIdentifier, value: PropertyValue) -> Result<(), Error> {
        self.object_mut()
            .write_property(property, None, value, None)
    }

    fn read(&self, property: PropertyIdentifier) -> PropertyValue {
        self.object().read_property(property, None).unwrap()
    }

    fn bind_clock(&mut self, clock: Arc<dyn ClockReader>) {
        self.object_mut().bind_clock_internal(Some(clock));
    }

    fn total(&self) -> u64 {
        let PropertyValue::Unsigned(total) = self.read(PropertyIdentifier::TOTAL_RECORD_COUNT)
        else {
            panic!("expected Total_Record_Count Unsigned");
        };
        total
    }

    fn enabled(&self) -> bool {
        self.read(PropertyIdentifier::LOG_ENABLE) == PropertyValue::Boolean(true)
    }

    fn stop_when_full(&self) -> bool {
        self.read(PropertyIdentifier::STOP_WHEN_FULL) == PropertyValue::Boolean(true)
    }

    fn identities(&self) -> Vec<u64> {
        self.object()
            .log_record_identities_internal()
            .unwrap()
            .iter()
            .map(|identity| identity.sequence_number())
            .collect()
    }
}

struct TestClock {
    frame: Option<ClockFrame>,
    reads: AtomicUsize,
}

impl TestClock {
    fn valid() -> Arc<Self> {
        Arc::new(Self {
            frame: Some(valid_frame()),
            reads: AtomicUsize::new(0),
        })
    }

    fn invalid() -> Arc<Self> {
        Arc::new(Self {
            frame: Some(ClockFrame {
                local_time: Time {
                    hour: Time::UNSPECIFIED,
                    ..valid_frame().local_time
                },
                ..valid_frame()
            }),
            reads: AtomicUsize::new(0),
        })
    }

    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

impl ClockReader for TestClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.frame
    }
}

fn valid_frame() -> ClockFrame {
    ClockFrame {
        local_date: Date {
            year: 126,
            month: 8,
            day: 31,
            day_of_week: 1,
        },
        local_time: Time {
            hour: 14,
            minute: 25,
            second: 36,
            hundredths: 47,
        },
        utc_offset: 0,
        daylight_savings_status: false,
    }
}

/// A single-datum test record as a one-member Trend Log Multiple record.
fn multiple(record: BACnetLogRecord) -> BACnetLogMultipleRecord {
    let log_data = match record.log_datum {
        LogDatum::LogStatus(bits) => LogData::LogStatus(bits),
        LogDatum::UnsignedValue(value) => LogData::Values(vec![LogValue::UnsignedValue(value)]),
        other => panic!("no Trend Log Multiple form for {other:?}"),
    };
    BACnetLogMultipleRecord {
        date: record.date,
        time: record.time,
        log_data,
    }
}

/// A single-datum test record as an Event Log record, its sample carried as
/// a clock change.
fn event(record: BACnetLogRecord) -> BACnetEventLogRecord {
    let log_datum = match record.log_datum {
        LogDatum::LogStatus(bits) => EventLogDatum::LogStatus(bits),
        LogDatum::UnsignedValue(value) => EventLogDatum::TimeChange(value as f32),
        other => panic!("no Event Log form for {other:?}"),
    };
    BACnetEventLogRecord {
        date: record.date,
        time: record.time,
        log_datum,
    }
}

/// The inverse of [`event`].
fn single_event(record: &BACnetEventLogRecord) -> BACnetLogRecord {
    let log_datum = match &record.log_datum {
        EventLogDatum::LogStatus(bits) => LogDatum::LogStatus(*bits),
        EventLogDatum::TimeChange(value) => LogDatum::UnsignedValue(*value as u64),
        other => panic!("no single-datum form for {other:?}"),
    };
    BACnetLogRecord {
        date: record.date,
        time: record.time,
        log_datum,
        status_flags: None,
    }
}

/// The inverse of [`multiple`].
fn single(record: &BACnetLogMultipleRecord) -> BACnetLogRecord {
    let log_datum = match &record.log_data {
        LogData::LogStatus(bits) => LogDatum::LogStatus(*bits),
        LogData::Values(values) if values.len() == 1 => values[0].clone().into(),
        other => panic!("no single-datum form for {other:?}"),
    };
    BACnetLogRecord {
        date: record.date,
        time: record.time,
        log_datum,
        status_flags: None,
    }
}

fn ordinary(hour: u8, value: u64) -> BACnetLogRecord {
    BACnetLogRecord {
        date: valid_frame().local_date,
        time: Time {
            hour,
            ..valid_frame().local_time
        },
        log_datum: LogDatum::UnsignedValue(value),
        status_flags: None,
    }
}

fn assert_status(object: &Family, status: LogStatus) {
    let records = object.records();
    let record = records.back().expect("status record");
    assert_eq!(record.date, valid_frame().local_date);
    assert_eq!(record.time, valid_frame().local_time);
    assert_eq!(record.log_datum, LogDatum::LogStatus(status));
    assert_eq!(record.status_flags, None);
    // On the wire log-disabled, bit 0, is the top bit of the octet
    // (Clause 20.2.10). Every family puts the log-status [0] choice first
    // inside its [1] field, right after the 12-octet timestamp.
    let octet = if status == LOG_DISABLED {
        0x80
    } else if status == BUFFER_PURGED {
        0x40
    } else if status == LOG_DISABLED | BUFFER_PURGED {
        0xC0
    } else if status.is_empty() {
        0x00
    } else {
        panic!("no wire vector for {status}")
    };
    let served = object.served_last();
    assert_eq!(&served[12..], &[0x1E, 0x0A, 0x05, octet, 0x1F], "{status}");
    let decoded = match object {
        Family::Event(_) => single_event(&decode_event_log_record(&served, 0).unwrap().0),
        Family::Trend(_) => decode_log_record(&served, 0).unwrap().0,
        Family::TrendMultiple(_) => single(&decode_log_multiple_record(&served, 0).unwrap().0),
    };
    assert_eq!(&decoded, record);
}

fn assert_protocol(error: Error, class: ErrorClass, code: ErrorCode) {
    match error {
        Error::Protocol {
            class: actual_class,
            code: actual_code,
        } => {
            assert_eq!(actual_class, class.to_raw() as u32);
            assert_eq!(actual_code, code.to_raw() as u32);
        }
        other => panic!("expected protocol error, got {other:?}"),
    }
}

#[test]
fn purge_is_unconditional_initial_status_and_repeats_for_every_log_family() {
    for kind in FamilyKind::ALL {
        let mut empty = kind.object(3);
        empty.bind_clock(TestClock::valid());
        empty
            .write(PropertyIdentifier::RECORD_COUNT, PropertyValue::Unsigned(0))
            .unwrap();
        assert_eq!(empty.records().len(), 1, "{kind:?}");
        assert_eq!(empty.total(), 1, "{kind:?}");
        assert_eq!(empty.identities(), vec![1], "{kind:?}");
        assert_status(&empty, BUFFER_PURGED);

        empty
            .write(PropertyIdentifier::RECORD_COUNT, PropertyValue::Unsigned(0))
            .unwrap();
        assert_eq!(empty.records().len(), 1, "{kind:?}");
        assert_eq!(empty.total(), 2, "{kind:?}");
        assert_eq!(empty.identities(), vec![2], "{kind:?}");
        assert_status(&empty, BUFFER_PURGED);

        let mut disabled = kind.object(3);
        disabled.bind_clock(TestClock::valid());
        disabled
            .write(
                PropertyIdentifier::LOG_ENABLE,
                PropertyValue::Boolean(false),
            )
            .unwrap();
        disabled
            .write(PropertyIdentifier::RECORD_COUNT, PropertyValue::Unsigned(0))
            .unwrap();
        assert_eq!(disabled.records().len(), 1, "{kind:?}");
        assert_eq!(disabled.total(), 2, "{kind:?}");
        assert_status(&disabled, LOG_DISABLED | BUFFER_PURGED);

        let mut capacity_one = kind.object(1);
        capacity_one.bind_clock(TestClock::valid());
        capacity_one
            .write(
                PropertyIdentifier::STOP_WHEN_FULL,
                PropertyValue::Boolean(true),
            )
            .unwrap();
        capacity_one
            .write(PropertyIdentifier::RECORD_COUNT, PropertyValue::Unsigned(0))
            .unwrap();
        assert!(!capacity_one.enabled(), "{kind:?}");
        assert_eq!(capacity_one.records().len(), 1, "{kind:?}");
        assert_status(&capacity_one, LOG_DISABLED | BUFFER_PURGED);
    }
}

#[test]
fn stop_before_full_omits_triggering_ordinary_and_clock_failure_is_atomic() {
    for kind in FamilyKind::ALL {
        let mut one = kind.object(1);
        one.bind_clock(TestClock::valid());
        one.write(
            PropertyIdentifier::STOP_WHEN_FULL,
            PropertyValue::Boolean(true),
        )
        .unwrap();
        one.add_record(ordinary(1, 10)).unwrap();
        assert!(!one.enabled(), "{kind:?}");
        assert_eq!(one.total(), 1, "{kind:?}");
        assert_eq!(one.records().len(), 1, "{kind:?}");
        assert_status(&one, LOG_DISABLED);
        one.add_record(ordinary(2, 20)).unwrap();
        assert_eq!(one.total(), 1, "{kind:?}");

        let mut many = kind.object(3);
        many.bind_clock(TestClock::valid());
        many.write(
            PropertyIdentifier::STOP_WHEN_FULL,
            PropertyValue::Boolean(true),
        )
        .unwrap();
        many.add_record(ordinary(1, 10)).unwrap();
        many.add_record(ordinary(2, 20)).unwrap();
        many.add_record(ordinary(3, 30)).unwrap();
        assert_eq!(many.total(), 3, "{kind:?}");
        assert_eq!(many.records().len(), 3, "{kind:?}");
        assert_eq!(many.records()[0].log_datum, LogDatum::UnsignedValue(10));
        assert_eq!(many.records()[1].log_datum, LogDatum::UnsignedValue(20));
        assert_status(&many, LOG_DISABLED);

        for clock in [None, Some(TestClock::invalid())] {
            let mut atomic = kind.object(2);
            if let Some(clock) = clock {
                atomic.bind_clock(clock);
            }
            atomic
                .write(
                    PropertyIdentifier::STOP_WHEN_FULL,
                    PropertyValue::Boolean(true),
                )
                .unwrap();
            atomic.add_record(ordinary(1, 10)).unwrap();
            let before_records = atomic.records();
            let before_total = atomic.total();
            let before_identities = atomic.identities();
            assert_protocol(
                atomic.add_record(ordinary(2, 20)).unwrap_err(),
                ErrorClass::DEVICE,
                ErrorCode::OPERATIONAL_PROBLEM,
            );
            assert_eq!(atomic.records(), before_records, "{kind:?}");
            assert_eq!(atomic.total(), before_total, "{kind:?}");
            assert_eq!(atomic.identities(), before_identities, "{kind:?}");
            assert!(atomic.enabled(), "{kind:?}");
        }
    }
}

#[test]
fn enable_and_stop_when_full_transitions_emit_exactly_one_status() {
    for kind in FamilyKind::ALL {
        let mut flag_only = kind.object(3);
        flag_only.bind_clock(TestClock::valid());
        flag_only
            .write(
                PropertyIdentifier::STOP_WHEN_FULL,
                PropertyValue::Boolean(true),
            )
            .unwrap();
        assert_eq!(flag_only.total(), 0, "{kind:?}");
        flag_only
            .write(
                PropertyIdentifier::STOP_WHEN_FULL,
                PropertyValue::Boolean(true),
            )
            .unwrap();
        flag_only
            .write(
                PropertyIdentifier::STOP_WHEN_FULL,
                PropertyValue::Boolean(false),
            )
            .unwrap();
        assert_eq!(flag_only.total(), 0, "{kind:?}");

        let mut transitions = kind.object(3);
        transitions.bind_clock(TestClock::valid());
        transitions
            .write(
                PropertyIdentifier::LOG_ENABLE,
                PropertyValue::Boolean(false),
            )
            .unwrap();
        assert_eq!(transitions.total(), 1, "{kind:?}");
        assert_status(&transitions, LOG_DISABLED);
        transitions
            .write(
                PropertyIdentifier::LOG_ENABLE,
                PropertyValue::Boolean(false),
            )
            .unwrap();
        assert_eq!(transitions.total(), 1, "{kind:?}");
        transitions
            .write(PropertyIdentifier::LOG_ENABLE, PropertyValue::Boolean(true))
            .unwrap();
        assert!(transitions.enabled(), "{kind:?}");
        assert_eq!(transitions.total(), 2, "{kind:?}");
        assert_status(&transitions, LogStatus::empty());

        transitions
            .write(
                PropertyIdentifier::LOG_ENABLE,
                PropertyValue::Boolean(false),
            )
            .unwrap();
        transitions.clear();
        transitions.add_record(ordinary(1, 10)).unwrap();
        assert!(transitions.records().is_empty(), "{kind:?}");

        let mut fills = kind.object(3);
        fills.bind_clock(TestClock::valid());
        fills.add_record(ordinary(1, 10)).unwrap();
        fills
            .write(
                PropertyIdentifier::LOG_ENABLE,
                PropertyValue::Boolean(false),
            )
            .unwrap();
        fills
            .write(
                PropertyIdentifier::STOP_WHEN_FULL,
                PropertyValue::Boolean(true),
            )
            .unwrap();
        fills
            .write(PropertyIdentifier::LOG_ENABLE, PropertyValue::Boolean(true))
            .unwrap();
        assert!(!fills.enabled(), "{kind:?}");
        assert_eq!(fills.total(), 3, "{kind:?}");
        assert_status(&fills, LOG_DISABLED);

        let mut full = kind.object(2);
        let clock = TestClock::valid();
        full.bind_clock(clock.clone());
        full.add_record(ordinary(1, 10)).unwrap();
        full.add_record(ordinary(2, 20)).unwrap();
        full.write(
            PropertyIdentifier::STOP_WHEN_FULL,
            PropertyValue::Boolean(true),
        )
        .unwrap();
        assert!(!full.enabled(), "{kind:?}");
        assert!(full.stop_when_full(), "{kind:?}");
        assert_eq!(full.records().len(), 2, "{kind:?}");
        assert_eq!(full.total(), 3, "{kind:?}");
        assert_eq!(full.identities(), vec![2, 3], "{kind:?}");
        assert_status(&full, LOG_DISABLED);
        let reads = clock.reads();
        let error = full
            .write(PropertyIdentifier::LOG_ENABLE, PropertyValue::Boolean(true))
            .unwrap_err();
        assert_protocol(error, ErrorClass::OBJECT, ErrorCode::LOG_BUFFER_FULL);
        assert_eq!(clock.reads(), reads, "{kind:?}");
        assert_eq!(full.total(), 3, "{kind:?}");
    }
}

#[test]
fn zero_capacity_counts_without_residents_and_enforces_full_enable_gate() {
    for kind in FamilyKind::ALL {
        let mut object = kind.object(0);
        let clock = TestClock::valid();
        object.bind_clock(clock.clone());
        object.add_record(ordinary(1, 10)).unwrap();
        object.add_record(ordinary(2, 20)).unwrap();
        assert!(object.records().is_empty(), "{kind:?}");
        assert_eq!(object.total(), 2, "{kind:?}");
        assert!(object.identities().is_empty(), "{kind:?}");

        object
            .write(PropertyIdentifier::RECORD_COUNT, PropertyValue::Unsigned(0))
            .unwrap();
        assert!(object.records().is_empty(), "{kind:?}");
        assert_eq!(object.total(), 3, "{kind:?}");

        object
            .write(
                PropertyIdentifier::STOP_WHEN_FULL,
                PropertyValue::Boolean(true),
            )
            .unwrap();
        assert!(!object.enabled(), "{kind:?}");
        assert!(object.records().is_empty(), "{kind:?}");
        assert_eq!(object.total(), 4, "{kind:?}");
        let reads = clock.reads();
        let error = object
            .write(PropertyIdentifier::LOG_ENABLE, PropertyValue::Boolean(true))
            .unwrap_err();
        assert_protocol(error, ErrorClass::OBJECT, ErrorCode::LOG_BUFFER_FULL);
        assert_eq!(clock.reads(), reads, "{kind:?}");
        assert_eq!(object.total(), 4, "{kind:?}");
    }
}

#[test]
fn status_writes_require_a_valid_clock_before_mutation() {
    for kind in FamilyKind::ALL {
        for clock in [None, Some(TestClock::invalid())] {
            let mut object = kind.object(2);
            object.add_record(ordinary(1, 10)).unwrap();
            if let Some(clock) = clock.clone() {
                object.bind_clock(clock);
            }
            let before = object.records();
            let before_total = object.total();
            let before_enable = object.enabled();
            let before_stop = object.stop_when_full();
            let error = object
                .write(PropertyIdentifier::RECORD_COUNT, PropertyValue::Unsigned(0))
                .unwrap_err();
            assert_protocol(error, ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM);
            assert_eq!(object.records(), before, "{kind:?}");
            assert_eq!(object.total(), before_total, "{kind:?}");
            assert_eq!(object.enabled(), before_enable, "{kind:?}");
            assert_eq!(object.stop_when_full(), before_stop, "{kind:?}");

            let mut enable = kind.object(2);
            if let Some(clock) = clock.clone() {
                enable.bind_clock(clock);
            }
            let error = enable
                .write(
                    PropertyIdentifier::LOG_ENABLE,
                    PropertyValue::Boolean(false),
                )
                .unwrap_err();
            assert_protocol(error, ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM);
            assert!(enable.enabled(), "{kind:?}");
            assert_eq!(enable.total(), 0, "{kind:?}");

            let mut stop = kind.object(2);
            if let Some(clock) = clock.clone() {
                stop.bind_clock(clock);
            }
            stop.add_record(ordinary(1, 10)).unwrap();
            stop.add_record(ordinary(2, 20)).unwrap();
            let before = stop.records();
            let error = stop
                .write(
                    PropertyIdentifier::STOP_WHEN_FULL,
                    PropertyValue::Boolean(true),
                )
                .unwrap_err();
            assert_protocol(error, ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM);
            assert_eq!(stop.records(), before, "{kind:?}");
            assert_eq!(stop.total(), 2, "{kind:?}");
            assert!(stop.enabled(), "{kind:?}");
            assert!(!stop.stop_when_full(), "{kind:?}");
        }
    }
}

#[test]
fn log_family_writability_matches_runtime_routes() {
    let cases = [
        (
            FamilyKind::Event,
            vec![
                PropertyIdentifier::LOG_ENABLE,
                PropertyIdentifier::STOP_WHEN_FULL,
                PropertyIdentifier::RECORD_COUNT,
                PropertyIdentifier::DESCRIPTION,
            ],
        ),
        (
            FamilyKind::Trend,
            vec![
                PropertyIdentifier::LOG_ENABLE,
                PropertyIdentifier::LOG_INTERVAL,
                PropertyIdentifier::STOP_WHEN_FULL,
                PropertyIdentifier::RECORD_COUNT,
                PropertyIdentifier::DESCRIPTION,
            ],
        ),
        (
            FamilyKind::TrendMultiple,
            vec![
                PropertyIdentifier::LOG_ENABLE,
                PropertyIdentifier::LOG_INTERVAL,
                PropertyIdentifier::STOP_WHEN_FULL,
                PropertyIdentifier::RECORD_COUNT,
                PropertyIdentifier::DESCRIPTION,
            ],
        ),
    ];
    for (kind, writable) in cases {
        let object = kind.object(3);
        for property in writable {
            assert!(
                object.object().is_writable_property(property),
                "{kind:?} {property:?}"
            );
        }
        for property in [
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::BUFFER_SIZE,
            PropertyIdentifier::LOG_BUFFER,
            PropertyIdentifier::TOTAL_RECORD_COUNT,
            PropertyIdentifier::RELIABILITY,
        ] {
            assert!(
                !object.object().is_writable_property(property),
                "{kind:?} {property:?}"
            );
        }
        // Tables 12-29, 12-35 and 12-31 define no Out_Of_Service (#985,
        // #1064), and Table 12-31 has no Log_Interval either.
        assert!(!object
            .object()
            .is_writable_property(PropertyIdentifier::OUT_OF_SERVICE));
        if matches!(kind, FamilyKind::Event) {
            assert!(!object
                .object()
                .is_writable_property(PropertyIdentifier::LOG_INTERVAL));
        }
    }
}
