//! Automatic trend logging.
//!
//! The server spawns a 1-second polling loop that checks each TrendLog object
//! whose `log_interval > 0` and logs the monitored property when the interval
//! elapses.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::RwLock;
use tracing::warn;

use bacnet_objects::database::ObjectDatabase;
use bacnet_types::constructed::{BACnetLogRecord, LogDatum};

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

/// Shared polling state — tracks last log time per TrendLog object.
/// Stored in the server struct (not a global static) for testability.
pub type TrendLogState = Arc<tokio::sync::Mutex<HashMap<ObjectIdentifier, Instant>>>;

/// Convert a `PropertyValue` to a `LogDatum`.
fn property_value_to_log_datum(pv: &PropertyValue) -> LogDatum {
    match pv {
        PropertyValue::Real(v) => LogDatum::RealValue(*v),
        PropertyValue::Unsigned(v) => LogDatum::UnsignedValue(*v),
        PropertyValue::Signed(v) => LogDatum::SignedValue(*v as i64),
        PropertyValue::Boolean(v) => LogDatum::BooleanValue(*v),
        PropertyValue::Enumerated(v) => LogDatum::EnumValue(*v),
        _ => LogDatum::NullValue,
    }
}

/// Called every second by the server's trend-log polling task.
///
/// For each TrendLog with `log_interval > 0` (polled mode), checks whether
/// enough time has elapsed since the last log entry and, if so, reads the
/// monitored property and adds a record using one actual Device-local clock
/// frame sampled before acquisition. As a local failure policy, a missing or
/// invalid frame skips the attempt without advancing its successful-log time.
pub async fn poll_trend_logs(db: &Arc<RwLock<ObjectDatabase>>, state: &TrendLogState) {
    let mut last_log = state.lock().await;
    let now = Instant::now();

    let to_poll: Vec<(ObjectIdentifier, u32, ObjectIdentifier, u32)> = {
        let db_read = db.read().await;
        let trend_oids = db_read.find_by_type(ObjectType::TREND_LOG);
        let mut result = Vec::new();
        for oid in trend_oids {
            if let Some(obj) = db_read.get(&oid) {
                let log_interval = match obj.read_property(PropertyIdentifier::LOG_INTERVAL, None) {
                    Ok(PropertyValue::Unsigned(v)) if v > 0 => v as u32,
                    _ => continue,
                };

                let logging_type = match obj.read_property(PropertyIdentifier::LOGGING_TYPE, None) {
                    Ok(PropertyValue::Enumerated(v)) => v,
                    _ => 0,
                };

                if logging_type == 1 {
                    warn!(object = %oid, "COV-based trend logging not yet implemented");
                    continue;
                }
                if logging_type == 2 {
                    warn!(object = %oid, "Triggered trend logging not yet implemented");
                    continue;
                }

                let monitored_ref =
                    match obj.read_property(PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY, None) {
                        Ok(PropertyValue::List(ref items)) if items.len() >= 2 => {
                            let target_oid = match &items[0] {
                                PropertyValue::ObjectIdentifier(o) => *o,
                                _ => continue,
                            };
                            let prop_id = match &items[1] {
                                PropertyValue::Unsigned(v) => *v as u32,
                                _ => continue,
                            };
                            (target_oid, prop_id)
                        }
                        _ => continue,
                    };

                let elapsed = last_log
                    .get(&oid)
                    .map(|t| now.duration_since(*t).as_secs() as u32)
                    .unwrap_or(u32::MAX);

                if elapsed >= log_interval {
                    result.push((oid, log_interval, monitored_ref.0, monitored_ref.1));
                }
            }
        }
        result
    };

    if to_poll.is_empty() {
        return;
    }

    let mut db_write = db.write().await;
    for (trend_oid, _interval, target_oid, prop_id) in to_poll {
        let Some(frame) = db_write
            .clock_frame()
            .filter(|frame| frame.is_valid_actual_datetime())
        else {
            continue;
        };
        let datum = if let Some(target_obj) = db_write.get(&target_oid) {
            match target_obj.read_property(PropertyIdentifier::from_raw(prop_id), None) {
                Ok(pv) => property_value_to_log_datum(&pv),
                Err(_) => LogDatum::NullValue,
            }
        } else {
            LogDatum::NullValue
        };

        let record = BACnetLogRecord {
            date: frame.local_date,
            time: frame.local_time,
            log_datum: datum,
            status_flags: None,
        };

        if let Some(trend_obj) = db_write.get_mut(&trend_oid) {
            if let Err(error) = trend_obj.add_trend_record(record) {
                warn!(object = %trend_oid, %error, "trend-log record insertion failed");
                continue;
            }
        }

        last_log.insert(trend_oid, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_objects::analog::AnalogValueObject;
    use bacnet_objects::clock::{ClockFrame, ClockReader};
    use bacnet_objects::traits::BACnetObject;
    use bacnet_objects::trend::TrendLogObject;
    use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
    use bacnet_types::primitives::{Date, Time};

    struct FixedClock;
    impl ClockReader for FixedClock {
        fn read_clock(&self) -> Option<ClockFrame> {
            Some(ClockFrame {
                local_date: Date {
                    year: 126,
                    month: 8,
                    day: 31,
                    day_of_week: 1,
                },
                local_time: Time {
                    hour: 12,
                    minute: 0,
                    second: 0,
                    hundredths: 0,
                },
                utc_offset: 0,
                daylight_savings_status: false,
            })
        }
    }

    #[test]
    fn property_value_to_datum_real() {
        let pv = PropertyValue::Real(42.5);
        match property_value_to_log_datum(&pv) {
            LogDatum::RealValue(v) => assert!((v - 42.5).abs() < f32::EPSILON),
            other => panic!("expected RealValue, got {:?}", other),
        }
    }

    #[test]
    fn property_value_to_datum_unsigned() {
        let pv = PropertyValue::Unsigned(100);
        match property_value_to_log_datum(&pv) {
            LogDatum::UnsignedValue(v) => assert_eq!(v, 100),
            other => panic!("expected UnsignedValue, got {:?}", other),
        }
    }

    #[test]
    fn property_value_to_datum_boolean() {
        let pv = PropertyValue::Boolean(true);
        match property_value_to_log_datum(&pv) {
            LogDatum::BooleanValue(v) => assert!(v),
            other => panic!("expected BooleanValue, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn poller_retries_when_mandatory_stop_status_has_no_clock() {
        let mut db = ObjectDatabase::new();
        let target = AnalogValueObject::new(1, "AV-1", 95).unwrap();
        let target_oid = target.object_identifier();
        db.add(Box::new(target)).unwrap();

        let mut trend = TrendLogObject::new(1, "TL-1", 1).unwrap();
        trend.set_log_device_object_property(Some(BACnetDeviceObjectPropertyReference {
            object_identifier: target_oid,
            property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
            property_array_index: None,
            device_identifier: None,
        }));
        trend
            .write_property(
                PropertyIdentifier::LOG_INTERVAL,
                None,
                PropertyValue::Unsigned(1),
                None,
            )
            .unwrap();
        trend
            .write_property(
                PropertyIdentifier::STOP_WHEN_FULL,
                None,
                PropertyValue::Boolean(true),
                None,
            )
            .unwrap();
        let trend_oid = trend.object_identifier();
        db.add(Box::new(trend)).unwrap();

        // Acquisition has a valid frame, but the later mandatory status
        // transition cannot obtain its own frame. Insertion failure must still
        // leave the polling deadline unchanged.
        struct AcquisitionOnlyClock(std::sync::atomic::AtomicBool);
        impl ClockReader for AcquisitionOnlyClock {
            fn read_clock(&self) -> Option<ClockFrame> {
                self.0
                    .swap(false, std::sync::atomic::Ordering::SeqCst)
                    .then(|| FixedClock.read_clock().unwrap())
            }
        }
        db.set_clock_reader(Some(Arc::new(AcquisitionOnlyClock(
            std::sync::atomic::AtomicBool::new(true),
        ))));
        let db = Arc::new(RwLock::new(db));
        let state = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        poll_trend_logs(&db, &state).await;

        let guard = db.read().await;
        let trend = guard.get(&trend_oid).unwrap();
        assert_eq!(
            trend
                .read_property(PropertyIdentifier::RECORD_COUNT, None)
                .unwrap(),
            PropertyValue::Unsigned(0)
        );
        assert_eq!(
            trend
                .read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
                .unwrap(),
            PropertyValue::Unsigned(0)
        );
        assert_eq!(
            trend
                .read_property(PropertyIdentifier::LOG_ENABLE, None)
                .unwrap(),
            PropertyValue::Boolean(true)
        );
        drop(guard);
        assert!(!state.lock().await.contains_key(&trend_oid));

        db.write()
            .await
            .set_clock_reader(Some(Arc::new(FixedClock)));
        poll_trend_logs(&db, &state).await;
        let guard = db.read().await;
        let trend = guard.get(&trend_oid).unwrap();
        assert_eq!(
            trend
                .read_property(PropertyIdentifier::RECORD_COUNT, None)
                .unwrap(),
            PropertyValue::Unsigned(1)
        );
        assert_eq!(
            trend
                .read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
                .unwrap(),
            PropertyValue::Unsigned(1)
        );
        assert_eq!(
            trend
                .read_property(PropertyIdentifier::LOG_ENABLE, None)
                .unwrap(),
            PropertyValue::Boolean(false)
        );
        assert_eq!(
            trend.log_record_identities_internal().unwrap()[0].sequence_number(),
            1
        );
        drop(guard);
        assert!(state.lock().await.contains_key(&trend_oid));
    }
}

#[cfg(test)]
#[path = "trend_log_clock_tests.rs"]
mod clock_tests;
