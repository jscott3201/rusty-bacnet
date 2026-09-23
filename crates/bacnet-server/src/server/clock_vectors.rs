use super::*;

#[test]
fn civil_frames_match_independent_calendar_vectors() {
    // Unix hundredths and ISO weekdays independently checked with Python's
    // datetime calendar, not this module's inverse conversion.
    for (hundredths, year, month, day, weekday) in [
        (-220_898_880_000, 0, 1, 1, 1), // 1900-01-01
        (0, 70, 1, 1, 4),               // Unix epoch
        (501_120_000, 70, 2, 28, 6),
        (509_760_000, 70, 3, 1, 7),
        (94_659_840_000, 99, 12, 31, 5),
        (94_668_480_000, 100, 1, 1, 6),
        (95_178_240_000, 100, 2, 29, 2), // 2000 is a leap century
        (95_186_880_000, 100, 3, 1, 3),
        (410_745_600_000, 200, 2, 28, 7),
        (410_754_240_000, 200, 3, 1, 1), // 2100 is not a leap year
        (583_796_160_000, 254, 12, 31, 2),
    ] {
        let frame = frame_from_utc_hundredths(hundredths, ClockConfig::default()).unwrap();
        assert_eq!(
            frame.local_date,
            Date {
                year,
                month,
                day,
                day_of_week: weekday
            }
        );
        assert_eq!(
            frame.local_time,
            Time {
                hour: 0,
                minute: 0,
                second: 0,
                hundredths: 0
            }
        );
        assert!(frame.is_valid_actual_datetime());
    }
}

#[test]
fn civil_frames_enforce_concrete_year_limits_after_local_adjustment() {
    const FIRST: i128 = -220_898_880_000;
    const AFTER_LAST: i128 = 583_804_800_000;
    assert!(frame_from_utc_hundredths(FIRST - 1, ClockConfig::default()).is_none());
    assert!(frame_from_utc_hundredths(AFTER_LAST, ClockConfig::default()).is_none());
    let last = frame_from_utc_hundredths(AFTER_LAST - 1, ClockConfig::default()).unwrap();
    assert_eq!(
        last.local_date,
        Date {
            year: 254,
            month: 12,
            day: 31,
            day_of_week: 2
        }
    );
    assert_eq!(
        last.local_time,
        Time {
            hour: 23,
            minute: 59,
            second: 59,
            hundredths: 99
        }
    );
    // Range checks apply to Device local time, including offset and DST.
    assert!(frame_from_utc_hundredths(FIRST, ClockConfig::new(60, false).unwrap()).is_none());
    assert!(
        frame_from_utc_hundredths(AFTER_LAST - 1, ClockConfig::new(0, true).unwrap()).is_none()
    );
    let repaired = frame_from_utc_hundredths(FIRST, ClockConfig::new(60, true).unwrap()).unwrap();
    assert_eq!(repaired.local_date.year, 0);
    assert!(repaired.is_valid_actual_datetime());
}

#[tokio::test]
async fn ordinary_trend_poll_uses_synchronized_offset_and_dst_frame() {
    use bacnet_objects::analog::AnalogValueObject;
    use bacnet_objects::traits::BACnetObject;
    use bacnet_objects::trend::TrendLogObject;
    use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
    use bacnet_types::enums::PropertyIdentifier;
    use bacnet_types::primitives::PropertyValue;
    use std::collections::HashMap;
    use std::sync::Arc;

    struct CapturingClock {
        clock: ServerClock,
        captured: Mutex<Option<ClockFrame>>,
    }
    impl ClockReader for CapturingClock {
        fn read_clock(&self) -> Option<ClockFrame> {
            let frame = self.clock.read_clock();
            *self.captured.lock().unwrap() = frame;
            frame
        }
    }
    let clock = Arc::new(CapturingClock {
        clock: ServerClock::new(ClockConfig::new(300, true).unwrap()),
        captured: Mutex::new(None),
    });
    clock
        .clock
        .synchronize(
            Date {
                year: 124,
                month: 7,
                day: 4,
                day_of_week: 4,
            },
            Time {
                hour: 13,
                minute: 15,
                second: 0,
                hundredths: 47,
            },
            true,
        )
        .unwrap();
    let mut db = ObjectDatabase::new();
    let target = AnalogValueObject::new(1, "AV", 95).unwrap();
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
    db.set_clock_reader(Some(clock.clone()));
    let db = Arc::new(tokio::sync::RwLock::new(db));
    let state = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    crate::trend_log::poll_trend_logs(&db, &state).await;
    let frame = clock.captured.lock().unwrap().unwrap();
    assert_eq!(
        frame.local_date,
        Date {
            year: 124,
            month: 7,
            day: 4,
            day_of_week: 4
        }
    );
    assert_eq!((frame.local_time.hour, frame.local_time.minute), (9, 15));
    assert_eq!(
        (frame.utc_offset, frame.daylight_savings_status),
        (300, true)
    );
    let guard = db.read().await;
    let identities = guard
        .get(&oid)
        .unwrap()
        .log_record_identities_internal()
        .unwrap();
    assert_eq!(identities.len(), 1);
    assert_eq!(identities[0].date(), frame.local_date);
    assert_eq!(identities[0].time(), frame.local_time);
}
