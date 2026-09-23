use super::*;
use crate::analog::AnalogValueObject;
use crate::clock::{ClockFrame, ClockReader};
use crate::trend::TrendLogObject;
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, Time};
use std::sync::{Arc, Mutex};

struct WallClock(Mutex<Option<ClockFrame>>);
impl ClockReader for WallClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        *self.0.lock().unwrap()
    }
}
fn frame() -> ClockFrame {
    ClockFrame {
        local_date: Date {
            year: 124,
            month: 2,
            day: 29,
            day_of_week: 4,
        },
        local_time: Time {
            hour: 12,
            minute: 0,
            second: 0,
            hundredths: 37,
        },
        utc_offset: 0,
        daylight_savings_status: false,
    }
}
fn target() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap()
}
fn trend(interval: u32, capacity: u32) -> TrendLogObject {
    let mut trend = TrendLogObject::new(1, "Trend", capacity).unwrap();
    trend.set_log_device_object_property(Some(BACnetDeviceObjectPropertyReference {
        object_identifier: target(),
        property_identifier: P::PRESENT_VALUE.to_raw(),
        property_array_index: None,
        device_identifier: None,
    }));
    trend
        .write_property(
            P::LOG_INTERVAL,
            None,
            PropertyValue::Unsigned(u64::from(interval)),
            None,
        )
        .unwrap();
    trend
}
fn fixture(
    interval: u32,
) -> (
    ObjectDatabase,
    ObjectIdentifier,
    Arc<Mutex<Duration>>,
    Arc<WallClock>,
) {
    let time = Arc::new(Mutex::new(Duration::ZERO));
    let clock = Arc::new(WallClock(Mutex::new(Some(frame()))));
    let mut db = ObjectDatabase::new();
    let source = time.clone();
    db.set_monotonic_clock_internal(Some(Arc::new(move || *source.lock().unwrap())));
    db.set_clock_reader(Some(clock.clone()));
    db.add(Box::new(AnalogValueObject::new(1, "AV", 95).unwrap()))
        .unwrap();
    let object = trend(interval, 16);
    let oid = object.object_identifier();
    db.add(Box::new(object)).unwrap();
    (db, oid, time, clock)
}
fn count(db: &ObjectDatabase, oid: ObjectIdentifier) -> u64 {
    let PropertyValue::Unsigned(count) = db
        .get(&oid)
        .unwrap()
        .read_property(P::TOTAL_RECORD_COUNT, None)
        .unwrap()
    else {
        panic!()
    };
    count
}
fn write(db: &mut ObjectDatabase, oid: ObjectIdentifier, p: P, value: PropertyValue) {
    db.get_mut(&oid)
        .unwrap()
        .write_property(p, None, value, None)
        .unwrap();
}

#[test]
fn hundredth_intervals_are_exact_and_late_wakes_do_not_catch_up() {
    for raw in [1, 50, 100, 150, u32::MAX] {
        let (mut db, oid, time, _) = fixture(raw);
        let interval = Duration::from_millis(u64::from(raw) * 10);
        assert_eq!(db.poll_trend_logs(), interval.min(RECONCILE));
        assert_eq!(count(&db, oid), 1);
        *time.lock().unwrap() = interval - Duration::from_nanos(1);
        assert_eq!(db.poll_trend_logs(), Duration::from_nanos(1));
        assert_eq!(count(&db, oid), 1);
        *time.lock().unwrap() = interval;
        db.poll_trend_logs();
        assert_eq!(count(&db, oid), 2);
        *time.lock().unwrap() = interval * 10;
        db.poll_trend_logs();
        assert_eq!(count(&db, oid), 3);
        db.poll_trend_logs();
        assert_eq!(count(&db, oid), 3);
        assert_eq!(db.trend_poll.0[&oid].last_success, Some(interval * 10));
    }
}

#[test]
fn full_width_time_uses_saturating_elapsed_without_overflow_or_spin() {
    let (mut db, oid, time, _) = fixture(u32::MAX);
    *time.lock().unwrap() = Duration::MAX;
    assert_eq!(db.poll_trend_logs(), RECONCILE);
    assert_eq!(db.poll_trend_logs(), RECONCILE);
    assert_eq!(count(&db, oid), 1);
    *time.lock().unwrap() = Duration::ZERO;
    assert_eq!(db.poll_trend_logs(), RECONCILE);
    assert_eq!(count(&db, oid), 1);
}

#[test]
fn lifecycle_and_configuration_changes_reset_owned_schedule() {
    let (mut db, oid, _, _) = fixture(u32::MAX);
    db.poll_trend_logs();
    assert_eq!(count(&db, oid), 1);
    let removed = db.remove(&oid).unwrap().unwrap();
    assert!(!db.trend_poll.0.contains_key(&oid));
    db.add(removed).unwrap();
    db.poll_trend_logs();
    assert_eq!(count(&db, oid), 2);
    db.add(Box::new(trend(u32::MAX, 16))).unwrap();
    db.poll_trend_logs();
    assert_eq!(count(&db, oid), 1);
    write(&mut db, oid, P::LOG_INTERVAL, PropertyValue::Unsigned(50));
    db.poll_trend_logs();
    assert_eq!(count(&db, oid), 2);
    write(&mut db, oid, P::LOG_INTERVAL, PropertyValue::Unsigned(0));
    db.poll_trend_logs();
    assert!(!db.trend_poll.0.contains_key(&oid));
    write(&mut db, oid, P::LOG_INTERVAL, PropertyValue::Unsigned(50));
    db.poll_trend_logs();
    assert_eq!(count(&db, oid), 3);
}

#[test]
fn structural_callback_retires_schedule_on_error_and_unwind() {
    let (mut db, oid, _, _) = fixture(u32::MAX);
    for panic_after in [false, true] {
        db.poll_trend_logs();
        assert!(db.trend_poll.0.contains_key(&oid));
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            db.with_object_adapter(&oid, |slot| -> Result<(), Error> {
                *slot = Box::new(trend(u32::MAX, 16));
                if panic_after {
                    panic!("after structural mutation")
                }
                Err(Error::Encoding("after structural mutation".into()))
            })
        }));
        assert_eq!(outcome.is_err(), panic_after);
        assert!(!db.trend_poll.0.contains_key(&oid));
        assert_eq!(db.find_by_name("Trend").unwrap().object_identifier(), oid);
        assert_eq!(db.find_by_type(ObjectType::TREND_LOG), vec![oid]);
        db.poll_trend_logs();
        assert_eq!(count(&db, oid), 1);
    }
}

#[test]
fn invalid_clock_retries_after_backoff_without_advancing_last_success() {
    let (mut db, oid, time, clock) = fixture(1);
    db.poll_trend_logs();
    *time.lock().unwrap() = Duration::from_millis(10);
    *clock.0.lock().unwrap() = None;
    assert_eq!(db.poll_trend_logs(), RECONCILE);
    assert_eq!(count(&db, oid), 1);
    assert_eq!(db.trend_poll.0[&oid].last_success, Some(Duration::ZERO));
    *clock.0.lock().unwrap() = Some(frame());
    *time.lock().unwrap() = Duration::from_millis(109);
    db.poll_trend_logs();
    assert_eq!(count(&db, oid), 1);
    *time.lock().unwrap() = Duration::from_millis(110);
    db.poll_trend_logs();
    assert_eq!(count(&db, oid), 2);
}

#[test]
fn mandatory_status_failure_retries_and_accepted_noops_keep_cadence() {
    let (mut db, oid, time, _) = fixture(1);
    let mut object = trend(1, 1);
    object
        .write_property(P::STOP_WHEN_FULL, None, PropertyValue::Boolean(true), None)
        .unwrap();
    db.add(Box::new(object)).unwrap();
    struct OneFrame(std::sync::atomic::AtomicBool);
    impl ClockReader for OneFrame {
        fn read_clock(&self) -> Option<ClockFrame> {
            self.0
                .swap(false, std::sync::atomic::Ordering::SeqCst)
                .then(frame)
        }
    }
    db.set_clock_reader(Some(Arc::new(OneFrame(
        std::sync::atomic::AtomicBool::new(true),
    ))));
    assert_eq!(db.poll_trend_logs(), RECONCILE);
    assert_eq!(count(&db, oid), 0);
    assert_eq!(db.trend_poll.0[&oid].last_success, None);
    db.set_clock_reader(Some(Arc::new(WallClock(Mutex::new(Some(frame()))))));
    *time.lock().unwrap() = RECONCILE;
    db.poll_trend_logs();
    assert_eq!(count(&db, oid), 1); // Mandatory LOG_DISABLED, not ordinary sample.
    *time.lock().unwrap() += Duration::from_millis(10);
    db.poll_trend_logs();
    assert_eq!(count(&db, oid), 1); // Accepted disabled no-op advances schedule.
    assert_eq!(
        db.trend_poll.0[&oid].last_success,
        Some(Duration::from_millis(110))
    );
    db.add(Box::new(trend(1, 0))).unwrap();
    db.poll_trend_logs();
    assert_eq!(count(&db, oid), 1); // Accepted count-only success.
    assert_eq!(
        db.get(&oid)
            .unwrap()
            .read_property(P::RECORD_COUNT, None)
            .unwrap(),
        PropertyValue::Unsigned(0)
    );
}

struct ConfigurableTrend {
    inner: Box<dyn BACnetObject>,
    reference: Arc<Mutex<PropertyValue>>,
    mode: Arc<std::sync::atomic::AtomicU32>,
}
impl BACnetObject for ConfigurableTrend {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.inner.object_identifier()
    }
    fn object_name(&self) -> &str {
        self.inner.object_name()
    }
    fn property_list(&self) -> std::borrow::Cow<'static, [P]> {
        self.inner.property_list()
    }
    fn read_property(&self, property: P, index: Option<u32>) -> Result<PropertyValue, Error> {
        if property == P::LOG_DEVICE_OBJECT_PROPERTY {
            return Ok(self.reference.lock().unwrap().clone());
        }
        if property == P::LOGGING_TYPE {
            return Ok(PropertyValue::Enumerated(
                self.mode.load(std::sync::atomic::Ordering::SeqCst),
            ));
        }
        self.inner.read_property(property, index)
    }
    fn write_property(
        &mut self,
        p: P,
        i: Option<u32>,
        v: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.inner.write_property(p, i, v, priority)
    }
    fn bind_clock_internal(&mut self, clock: Option<Arc<dyn ClockReader>>) {
        self.inner.bind_clock_internal(clock);
    }
    fn add_trend_record(&mut self, record: BACnetLogRecord) -> Result<(), Error> {
        self.inner.add_trend_record(record)
    }
}

#[test]
fn full_reference_and_logging_mode_changes_retire_previous_selection() {
    use std::sync::atomic::{AtomicU32, Ordering};
    let (mut db, oid, _, _) = fixture(u32::MAX);
    let reference = Arc::new(Mutex::new(
        db.get(&oid)
            .unwrap()
            .read_property(P::LOG_DEVICE_OBJECT_PROPERTY, None)
            .unwrap(),
    ));
    let mode = Arc::new(AtomicU32::new(0));
    let inner = db.remove(&oid).unwrap().unwrap();
    db.add(Box::new(ConfigurableTrend {
        inner,
        reference: reference.clone(),
        mode: mode.clone(),
    }))
    .unwrap();
    db.poll_trend_logs();
    assert_eq!(count(&db, oid), 1);
    for (number, fields) in [
        vec![
            PropertyValue::ObjectIdentifier(target()),
            PropertyValue::Unsigned(P::DESCRIPTION.to_raw().into()),
            PropertyValue::Null,
            PropertyValue::Null,
        ],
        vec![
            PropertyValue::ObjectIdentifier(target()),
            PropertyValue::Unsigned(P::DESCRIPTION.to_raw().into()),
            PropertyValue::Unsigned(3),
            PropertyValue::Null,
        ],
        vec![
            PropertyValue::ObjectIdentifier(
                ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 2).unwrap(),
            ),
            PropertyValue::Unsigned(P::PRESENT_VALUE.to_raw().into()),
        ],
    ]
    .into_iter()
    .enumerate()
    {
        *reference.lock().unwrap() = PropertyValue::List(fields);
        db.poll_trend_logs();
        assert_eq!(count(&db, oid), number as u64 + 2);
    }
    // Missing target/read failure stays Null; indexed/remote execution is not added.
    let PropertyValue::List(records) = db
        .get(&oid)
        .unwrap()
        .read_property(P::LOG_BUFFER, None)
        .unwrap()
    else {
        panic!()
    };
    assert!(
        matches!(records.last(), Some(PropertyValue::List(fields)) if fields[2] == PropertyValue::Null)
    );
    for disabled_mode in [1, 2] {
        mode.store(disabled_mode, Ordering::SeqCst);
        db.poll_trend_logs();
        assert!(!db.trend_poll.0.contains_key(&oid));
        assert_eq!(count(&db, oid), 4 + u64::from(disabled_mode - 1));
        mode.store(0, Ordering::SeqCst);
        db.poll_trend_logs();
    }
    assert_eq!(count(&db, oid), 6);
}

#[test]
fn slow_acquisition_anchors_success_to_completion_and_yields_when_other_logs_are_due() {
    let (mut db, oid, time, _) = fixture(1);
    struct SlowClock(Arc<Mutex<Duration>>);
    impl ClockReader for SlowClock {
        fn read_clock(&self) -> Option<ClockFrame> {
            *self.0.lock().unwrap() += Duration::from_millis(100);
            Some(frame())
        }
    }
    db.set_clock_reader(Some(Arc::new(SlowClock(time.clone()))));
    assert_eq!(db.poll_trend_logs(), Duration::from_millis(10));
    assert_eq!(
        db.trend_poll.0[&oid].last_success,
        Some(Duration::from_millis(100))
    );
    assert_eq!(count(&db, oid), 1);
    assert_eq!(db.poll_trend_logs(), Duration::from_millis(10));
    assert_eq!(count(&db, oid), 1);
    let mut second = TrendLogObject::new(2, "Second", 8).unwrap();
    second.set_log_device_object_property(Some(BACnetDeviceObjectPropertyReference {
        object_identifier: target(),
        property_identifier: P::PRESENT_VALUE.to_raw(),
        property_array_index: None,
        device_identifier: None,
    }));
    second
        .write_property(P::LOG_INTERVAL, None, PropertyValue::Unsigned(1), None)
        .unwrap();
    db.add(Box::new(second)).unwrap();
    *time.lock().unwrap() = Duration::from_millis(110);
    assert_eq!(db.poll_trend_logs(), Duration::from_millis(1));
}

#[test]
fn scalar_projection_preserves_supported_datums_and_unsupported_null() {
    for (value, expected) in [
        (PropertyValue::Real(42.5), LogDatum::RealValue(42.5)),
        (PropertyValue::Unsigned(100), LogDatum::UnsignedValue(100)),
        (PropertyValue::Signed(-12), LogDatum::SignedValue(-12)),
        (PropertyValue::Boolean(true), LogDatum::BooleanValue(true)),
        (PropertyValue::Enumerated(7), LogDatum::EnumValue(7)),
        (PropertyValue::Null, LogDatum::NullValue),
    ] {
        assert_eq!(property_value_to_log_datum(&value), expected);
    }
}

#[test]
fn idle_reconciliation_and_max_interval_failure_backoff_are_bounded() {
    let (mut db, oid, time, clock) = fixture(u32::MAX);
    *clock.0.lock().unwrap() = None;
    assert_eq!(db.poll_trend_logs(), RECONCILE);
    assert_eq!(count(&db, oid), 0);
    *clock.0.lock().unwrap() = Some(frame());
    *time.lock().unwrap() = Duration::from_millis(99);
    db.poll_trend_logs();
    assert_eq!(count(&db, oid), 0);
    *time.lock().unwrap() = Duration::from_millis(100);
    db.poll_trend_logs();
    assert_eq!(count(&db, oid), 1);
    db.set_monotonic_clock_internal(Some(Arc::new(|| Duration::ZERO)));
    db.poll_trend_logs();
    assert_eq!(count(&db, oid), 2);
    db.remove(&oid).unwrap();
    assert!(db.trend_poll.0.is_empty());
    assert_eq!(db.poll_trend_logs(), RECONCILE);
}
