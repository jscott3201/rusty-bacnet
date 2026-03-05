//! Schedule execution engine (Clause 12.24).
//!
//! Periodically evaluates Schedule objects and writes the effective value
//! to all controlled object-property references.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use bacnet_objects::database::ObjectDatabase;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use tokio::sync::RwLock;
use tracing::{debug, warn};

/// Compute (day_of_week, hour, minute) from the current UTC time.
///
/// day_of_week: 0=Monday .. 6=Sunday (matching BACnet weekly_schedule index).
fn current_time_components() -> (u8, u8, u8) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Unix epoch (1970-01-01) was a Thursday (day 3 in 0=Mon convention).
    let day_of_week = ((secs / 86400 + 3) % 7) as u8;
    let time_of_day = secs % 86400;
    let hour = (time_of_day / 3600) as u8;
    let minute = ((time_of_day % 3600) / 60) as u8;
    (day_of_week, hour, minute)
}

/// Evaluate all Schedule objects and write to their controlled properties.
///
/// Called periodically by the server (every 60 seconds). Uses the current
/// system time to determine the active schedule value.
pub async fn tick_schedules(db: &Arc<RwLock<ObjectDatabase>>) {
    let (day_of_week, hour, minute) = current_time_components();

    // Phase 1: evaluate schedules and collect writes to make.
    let mut writes = Vec::new();
    {
        let mut db_w = db.write().await;
        let schedule_oids = db_w.find_by_type(ObjectType::SCHEDULE);
        for oid in schedule_oids {
            if let Some(obj) = db_w.get_mut(&oid) {
                if let Some((value, refs)) = obj.tick_schedule(day_of_week, hour, minute) {
                    debug!(
                        schedule = %oid,
                        refs = refs.len(),
                        "Schedule value changed, writing to controlled properties"
                    );
                    for (target_oid, prop_id) in refs {
                        writes.push((target_oid, prop_id, value.clone()));
                    }
                }
            }
        }

        // Phase 2: write to controlled properties (same lock scope).
        for (target_oid, prop_id, value) in writes {
            if let Some(target_obj) = db_w.get_mut(&target_oid) {
                let prop = PropertyIdentifier::from_raw(prop_id);
                if let Err(e) = target_obj.write_property(prop, None, value, None) {
                    warn!(
                        target = %target_oid,
                        property = prop_id,
                        error = %e,
                        "Schedule failed to write to controlled property"
                    );
                }
            }
        }
    }
}
