use super::*;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::trend::TrendLogObject;
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::enums::{EngineeringUnits, PropertyIdentifier};
use bacnet_types::primitives::{Date, Time};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use std::sync::Mutex;

struct MutableClock(Mutex<Option<ClockFrame>>);

impl ClockReader for MutableClock {
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
            hour: 23,
            minute: 58,
            second: 57,
            hundredths: 63,
        },
        utc_offset: 300,
        daylight_savings_status: true,
    }
}

fn database(clock: Arc<dyn ClockReader>) -> (Arc<RwLock<ObjectDatabase>>, ObjectIdentifier) {
    let mut db = ObjectDatabase::new();
    let mut target = AnalogValueObject::new(1, "AV", EngineeringUnits::NO_UNITS).unwrap();
    target
        .write_property_from(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(42.5),
            None,
            &crate::command_source::test_origin(),
        )
        .unwrap();
    let mut trend = TrendLogObject::new(1, "Trend", 8).unwrap();
    trend
        .set_log_device_object_property(Some(BACnetDeviceObjectPropertyReference {
            object_identifier: target.object_identifier(),
            property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
            property_array_index: None,
            device_identifier: None,
        }))
        .unwrap();
    trend
        .write_property(
            PropertyIdentifier::LOG_INTERVAL,
            None,
            PropertyValue::Unsigned(60),
            None,
        )
        .unwrap();
    let oid = trend.object_identifier();
    db.add(Box::new(target)).unwrap();
    db.add(Box::new(trend)).unwrap();
    db.set_clock_reader(Some(clock));
    let origin = tokio::time::Instant::now();
    db.set_monotonic_clock_internal(Some(Arc::new(move || {
        tokio::time::Instant::now().duration_since(origin)
    })));
    (Arc::new(RwLock::new(db)), oid)
}

/// The records ReadRange serves, each as its encoded bytes.
fn served(obj: &dyn BACnetObject) -> Vec<Vec<u8>> {
    let records = obj.log_buffer_internal().unwrap();
    (0..records.record_count())
        .map(|index| {
            let mut bytes = bytes::BytesMut::new();
            records.encode_record(index, &mut bytes);
            bytes.to_vec()
        })
        .collect()
}

/// The served records, then Record_Count, Total_Record_Count and Log_Enable.
fn snapshot(db: &ObjectDatabase, oid: ObjectIdentifier) -> Vec<PropertyValue> {
    let obj = db.get(&oid).unwrap();
    let records = served(obj)
        .into_iter()
        .map(PropertyValue::ApplicationData)
        .collect();
    let mut snapshot = vec![PropertyValue::List(records)];
    snapshot.extend(
        [
            PropertyIdentifier::RECORD_COUNT,
            PropertyIdentifier::TOTAL_RECORD_COUNT,
            PropertyIdentifier::LOG_ENABLE,
        ]
        .map(|p| obj.read_property(p, None).unwrap()),
    );
    snapshot
}

#[tokio::test(start_paused = true)]
async fn ordinary_poll_uses_exact_device_local_timestamp_and_hundredths() {
    let expected = frame();
    let clock = Arc::new(MutableClock(Mutex::new(Some(expected))));
    let (db, oid) = database(clock.clone());
    poll_trend_logs(&db).await;
    let guard = db.read().await;
    let obj = guard.get(&oid).unwrap();
    let identities = obj.log_record_identities_internal().unwrap();
    assert_eq!(identities.len(), 1);
    assert_eq!(identities[0].date(), expected.local_date);
    assert_eq!(identities[0].time(), expected.local_time);
    let served = served(obj);
    let (record, _) = bacnet_encoding::constructed::decode_log_record(&served[0], 0).unwrap();
    assert_eq!(record.date, expected.local_date);
    assert_eq!(record.time, expected.local_time);
    assert_eq!(
        record.log_datum,
        bacnet_types::constructed::LogDatum::RealValue(42.5)
    );
    drop(guard);

    // The next due attempt samples the current frame rather than caching it.
    let next = ClockFrame {
        local_time: Time {
            hundredths: 91,
            ..expected.local_time
        },
        ..expected
    };
    *clock.0.lock().unwrap() = Some(next);
    tokio::time::advance(Duration::from_millis(600)).await;
    poll_trend_logs(&db).await;
    let guard = db.read().await;
    let identities = guard
        .get(&oid)
        .unwrap()
        .log_record_identities_internal()
        .unwrap();
    assert_eq!(identities.len(), 2);
    assert_eq!(identities[1].time(), next.local_time);
}

#[tokio::test(start_paused = true)]
async fn unusable_acquisition_clock_preserves_records_and_schedule_then_retries() {
    let good = frame();
    for bad in [
        None,
        Some(ClockFrame {
            local_date: Date {
                year: Date::UNSPECIFIED,
                ..good.local_date
            },
            ..good
        }),
        Some(ClockFrame {
            local_date: Date {
                month: Date::UNSPECIFIED,
                ..good.local_date
            },
            ..good
        }),
        Some(ClockFrame {
            local_date: Date {
                day: 30,
                ..good.local_date
            },
            ..good
        }),
        Some(ClockFrame {
            local_date: Date {
                day_of_week: 5,
                ..good.local_date
            },
            ..good
        }),
        Some(ClockFrame {
            local_time: Time {
                hour: Time::UNSPECIFIED,
                ..good.local_time
            },
            ..good
        }),
        Some(ClockFrame {
            local_time: Time {
                hundredths: 100,
                ..good.local_time
            },
            ..good
        }),
    ] {
        let clock = Arc::new(MutableClock(Mutex::new(Some(good))));
        let (db, oid) = database(clock.clone());
        poll_trend_logs(&db).await;
        let before = snapshot(&*db.read().await, oid);
        tokio::time::advance(Duration::from_millis(600)).await;
        *clock.0.lock().unwrap() = bad;
        poll_trend_logs(&db).await;
        assert_eq!(snapshot(&*db.read().await, oid), before, "{bad:?}");

        *clock.0.lock().unwrap() = Some(good);
        tokio::time::advance(Duration::from_millis(100)).await;
        poll_trend_logs(&db).await;
        let guard = db.read().await;
        assert_eq!(
            guard
                .get(&oid)
                .unwrap()
                .read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
                .unwrap(),
            PropertyValue::Unsigned(2)
        );
    }

    // Absence of the shared reader has the same policy as an unavailable frame.
    let (db, oid) = database(Arc::new(MutableClock(Mutex::new(Some(good)))));
    db.write().await.set_clock_reader(None);
    let before = snapshot(&*db.read().await, oid);
    poll_trend_logs(&db).await;
    assert_eq!(snapshot(&*db.read().await, oid), before);
    db.write()
        .await
        .set_clock_reader(Some(Arc::new(MutableClock(Mutex::new(Some(good))))));
    tokio::time::advance(Duration::from_millis(100)).await;
    poll_trend_logs(&db).await;
}

#[tokio::test(start_paused = true)]
async fn interval_fifty_is_half_a_second() {
    let (db, oid) = database(Arc::new(MutableClock(Mutex::new(Some(frame())))));
    db.write()
        .await
        .get_mut(&oid)
        .unwrap()
        .write_property(
            PropertyIdentifier::LOG_INTERVAL,
            None,
            PropertyValue::Unsigned(50),
            None,
        )
        .unwrap();
    poll_trend_logs(&db).await;
    tokio::time::advance(Duration::from_millis(500)).await;
    poll_trend_logs(&db).await;
    assert_eq!(
        db.read()
            .await
            .get(&oid)
            .unwrap()
            .read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
            .unwrap(),
        PropertyValue::Unsigned(2)
    );
}

#[path = "trend_poll_lifecycle_tests.rs"]
mod lifecycle;
