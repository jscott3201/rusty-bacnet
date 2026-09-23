use super::*;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::trend::TrendLogObject;
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::primitives::{Date, Time};
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
    let mut target = AnalogValueObject::new(1, "AV", 95).unwrap();
    target
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(42.5),
            None,
        )
        .unwrap();
    let mut trend = TrendLogObject::new(1, "Trend", 8).unwrap();
    trend.set_log_device_object_property(Some(BACnetDeviceObjectPropertyReference {
        object_identifier: target.object_identifier(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
        property_array_index: None,
        device_identifier: None,
    }));
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
    (Arc::new(RwLock::new(db)), oid)
}

fn snapshot(db: &ObjectDatabase, oid: ObjectIdentifier) -> Vec<PropertyValue> {
    let obj = db.get(&oid).unwrap();
    [
        PropertyIdentifier::LOG_BUFFER,
        PropertyIdentifier::RECORD_COUNT,
        PropertyIdentifier::TOTAL_RECORD_COUNT,
        PropertyIdentifier::LOG_ENABLE,
    ]
    .map(|p| obj.read_property(p, None).unwrap())
    .to_vec()
}

#[tokio::test]
async fn ordinary_poll_uses_exact_device_local_timestamp_and_hundredths() {
    let expected = frame();
    let clock = Arc::new(MutableClock(Mutex::new(Some(expected))));
    let (db, oid) = database(clock.clone());
    let state = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    poll_trend_logs(&db, &state).await;
    let guard = db.read().await;
    let obj = guard.get(&oid).unwrap();
    let identities = obj.log_record_identities_internal().unwrap();
    assert_eq!(identities.len(), 1);
    assert_eq!(identities[0].date(), expected.local_date);
    assert_eq!(identities[0].time(), expected.local_time);
    let PropertyValue::List(records) = obj
        .read_property(PropertyIdentifier::LOG_BUFFER, None)
        .unwrap()
    else {
        panic!("expected log buffer")
    };
    let PropertyValue::List(record) = &records[0] else {
        panic!("expected record")
    };
    assert_eq!(record[0], PropertyValue::Date(expected.local_date));
    assert_eq!(record[1], PropertyValue::Time(expected.local_time));
    assert_eq!(record[2], PropertyValue::Real(42.5));
    drop(guard);
    assert!(state.lock().await.contains_key(&oid));

    // The next due attempt samples the current frame rather than caching it.
    let next = ClockFrame {
        local_time: Time {
            hundredths: 91,
            ..expected.local_time
        },
        ..expected
    };
    *clock.0.lock().unwrap() = Some(next);
    state
        .lock()
        .await
        .insert(oid, Instant::now() - std::time::Duration::from_secs(61));
    poll_trend_logs(&db, &state).await;
    let guard = db.read().await;
    let identities = guard
        .get(&oid)
        .unwrap()
        .log_record_identities_internal()
        .unwrap();
    assert_eq!(identities.len(), 2);
    assert_eq!(identities[1].time(), next.local_time);
}

#[tokio::test]
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
        let state = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        poll_trend_logs(&db, &state).await;
        let before = snapshot(&*db.read().await, oid);
        let last_success = Instant::now() - std::time::Duration::from_secs(61);
        state.lock().await.insert(oid, last_success);
        *clock.0.lock().unwrap() = bad;
        poll_trend_logs(&db, &state).await;
        assert_eq!(snapshot(&*db.read().await, oid), before, "{bad:?}");
        assert_eq!(state.lock().await.get(&oid), Some(&last_success), "{bad:?}");

        *clock.0.lock().unwrap() = Some(good);
        poll_trend_logs(&db, &state).await;
        let guard = db.read().await;
        assert_eq!(
            guard
                .get(&oid)
                .unwrap()
                .read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
                .unwrap(),
            PropertyValue::Unsigned(2)
        );
        assert!(state.lock().await[&oid] > last_success);
    }

    // Absence of the shared reader has the same policy as an unavailable frame.
    let (db, oid) = database(Arc::new(MutableClock(Mutex::new(Some(good)))));
    db.write().await.set_clock_reader(None);
    let state = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    let before = snapshot(&*db.read().await, oid);
    poll_trend_logs(&db, &state).await;
    assert_eq!(snapshot(&*db.read().await, oid), before);
    assert!(state.lock().await.is_empty());
    db.write()
        .await
        .set_clock_reader(Some(Arc::new(MutableClock(Mutex::new(Some(good))))));
    poll_trend_logs(&db, &state).await;
    assert!(state.lock().await.contains_key(&oid));
}
