//! Automatic trend logging per ASHRAE 135-2020 Clause 12.25.
//!
//! The server spawns a 1-second polling loop that checks each TrendLog object
//! whose `log_interval > 0` and logs the monitored property when the interval
//! elapses.  COV and triggered modes log a warning and are not yet implemented.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::RwLock;
use tracing::warn;

use bacnet_objects::database::ObjectDatabase;
use bacnet_types::constructed::{BACnetLogRecord, LogDatum};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, Time};

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

/// Create a `BACnetLogRecord` with the current wall-clock time.
fn make_record(datum: LogDatum) -> BACnetLogRecord {
    let now = {
        // Use system time for the record timestamp.
        let dur = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let secs = dur.as_secs();
        // BACnet epoch is 1 Jan 1900 but we store wallclock fields directly.
        let hour = ((secs % 86400) / 3600) as u8;
        let minute = ((secs % 3600) / 60) as u8;
        let second = (secs % 60) as u8;
        (hour, minute, second)
    };
    BACnetLogRecord {
        date: Date {
            year: Date::UNSPECIFIED,
            month: Date::UNSPECIFIED,
            day: Date::UNSPECIFIED,
            day_of_week: Date::UNSPECIFIED,
        },
        time: Time {
            hour: now.0,
            minute: now.1,
            second: now.2,
            hundredths: 0,
        },
        log_datum: datum,
        status_flags: None,
    }
}

/// Called every second by the server's trend-log polling task.
///
/// For each TrendLog with `log_interval > 0` (polled mode), checks whether
/// enough time has elapsed since the last log entry and, if so, reads the
/// monitored property and adds a record.
pub async fn poll_trend_logs(db: &Arc<RwLock<ObjectDatabase>>, state: &TrendLogState) {
    let mut last_log = state.lock().await;
    let now = Instant::now();

    // Acquire a read lock to collect what we need.
    let to_poll: Vec<(ObjectIdentifier, u32, ObjectIdentifier, u32)> = {
        let db_read = db.read().await;
        let trend_oids = db_read.find_by_type(ObjectType::TREND_LOG);
        let mut result = Vec::new();
        for oid in trend_oids {
            if let Some(obj) = db_read.get(&oid) {
                // Read log_interval
                let log_interval = match obj.read_property(PropertyIdentifier::LOG_INTERVAL, None) {
                    Ok(PropertyValue::Unsigned(v)) if v > 0 => v as u32,
                    _ => continue,
                };

                // Read logging_type: 0=polled
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

                // Read log_device_object_property to find what to monitor.
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

                // Check interval elapsed
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

    // Acquire a write lock to read monitored values + add records.
    let mut db_write = db.write().await;
    for (trend_oid, _interval, target_oid, prop_id) in to_poll {
        // Read the monitored property value
        let datum = if let Some(target_obj) = db_write.get(&target_oid) {
            match target_obj.read_property(PropertyIdentifier::from_raw(prop_id), None) {
                Ok(pv) => property_value_to_log_datum(&pv),
                Err(_) => LogDatum::NullValue,
            }
        } else {
            LogDatum::NullValue
        };

        let record = make_record(datum);

        // Add the record to the TrendLog
        if let Some(trend_obj) = db_write.get_mut(&trend_oid) {
            // We write to LOG_BUFFER indirectly via the object — but the
            // TrendLogObject exposes `add_record` only on the concrete type.
            // We can use a cast through Any, but the trait doesn't expose it.
            // Instead, we'll use a lightweight approach: write a special
            // internal property. For now, we use the public API approach:
            // the TrendLogObject is behind dyn BACnetObject. We add a
            // trait method for this.
            //
            // Simplest approach: downcast not available, so we store the
            // record via write_property(RECORD_COUNT) is not right either.
            // Let's use the add_trend_record trait method.
            trend_obj.add_trend_record(record);
        }

        last_log.insert(trend_oid, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn make_record_has_valid_time() {
        let record = make_record(LogDatum::RealValue(1.0));
        // Time fields should be within valid ranges
        assert!(record.time.hour < 24);
        assert!(record.time.minute < 60);
        assert!(record.time.second < 60);
    }
}
