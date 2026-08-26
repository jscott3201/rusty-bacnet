//! Schedule execution engine.
//!
//! Periodically evaluates Schedule objects and writes the effective value
//! to all controlled object-property references.

use std::sync::Arc;

use bacnet_objects::clock::ClockFrame;
use bacnet_objects::database::ObjectDatabase;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use tokio::sync::RwLock;
use tracing::{debug, warn};

/// Compute the weekly-schedule index and time from one shared clock frame.
pub(crate) fn current_time_components(frame: ClockFrame) -> Option<(u8, u8, u8)> {
    let day_of_week = frame.local_date.day_of_week.checked_sub(1)?;
    (day_of_week <= 6).then_some((day_of_week, frame.local_time.hour, frame.local_time.minute))
}

/// Evaluate all Schedule objects and write to their controlled properties.
///
/// Called periodically by the server (every 60 seconds). The offset argument
/// remains for source compatibility; evaluation uses the database clock frame.
pub async fn tick_schedules(db: &Arc<RwLock<ObjectDatabase>>, _utc_offset_minutes: i16) {
    let frame = db.read().await.clock_frame();
    let Some((day_of_week, hour, minute)) = frame.and_then(current_time_components) else {
        debug!("Skipping Schedule evaluation without a valid Device clock");
        return;
    };

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
