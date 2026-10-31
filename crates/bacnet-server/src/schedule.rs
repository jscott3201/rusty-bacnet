//! Schedule execution engine.
//!
//! Periodically evaluates Schedule objects (Clause 12.24.4) against one Device
//! clock frame and writes the effective value to every controlled
//! object-property reference at the Schedule's Priority_For_Writing. A write
//! that commits to a Schedule runs the same evaluation for that Schedule at
//! once, since a change to what it holds can change its value (#1057).
//!
//! Each pass first sends what a Schedule owes apart from its calculation,
//! through the same target writes: the NULLs that relinquish slots a change
//! of its references or priority left behind (#1088), then a Present_Value a
//! client wrote while it was out of service (Clause 12.24.14, #1055). Those
//! need no clock. Then it calculates; that needs the clock, and a Schedule
//! out of service skips it.
//!
//! After each write the pass tells the Schedule how every target took it, so
//! a target that refuses the schedule's datatype faults it (Clause 12.24.13,
//! #1086), as does a reference naming a missing object or property or an
//! array index the property can't take (#1433). While such a refusal stands,
//! a pass with nothing else to send for that Schedule offers its value again
//! to the refused references alone (#1436): the 60-second tick, or the pass a
//! committed write to the Schedule runs, of any property. So the fault clears
//! within one tick of its cause going away: the array grown, say. A missing
//! object doesn't wait for the tick: once it is created, by CreateObject or
//! the application's own `ObjectDatabase::add`, the server retries the
//! references naming it at once (`retry_for_created`, #1440) and fans COV
//! out for what that writes. A retry still refused logs at debug, since it
//! repeats every pass; one that fails otherwise ends the refusal and warns
//! once.
//!
//! A NULL relinquishes the Schedule's slot in a commandable target. On a
//! target property that isn't commandable and has no NULL in its datatype,
//! it is the no-op WriteProperty would make of it (`bacnet_device_core::write::relinquish`,
//! #1416): the property keeps its value and the target counts as having
//! taken the write. Every target here is local; a Schedule's references
//! name objects of its own device only.

use std::collections::HashMap;
use std::sync::Arc;

use crate::committed_cov::{BackgroundCommit, CommittedCov};
use crate::cov::CovSubscriptionTable;
use bacnet_objects::clock::ClockFrame;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::schedule::{ScheduleTargetOutcome, ScheduleWrite};
use bacnet_types::calendar::SpecificDate;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, Time};
use tokio::sync::RwLock;
use tracing::{debug, warn};

/// The local date and time a clock frame gives schedule evaluation, or `None`
/// when the frame's date is not a real day or its time is not specific.
pub(crate) fn schedule_instant(frame: ClockFrame) -> Option<(SpecificDate, Time)> {
    let today = SpecificDate::from_date(&frame.local_date)?;
    frame
        .local_time
        .is_specific()
        .then_some((today, frame.local_time))
}

/// Evaluate all Schedule objects and write to their controlled properties.
///
/// A running server evaluates schedules itself every 60 seconds and fans COV
/// out for the objects they write; this entry point evaluates without COV.
///
/// A write to a Command object's Present_Value starts its list, and one to a
/// Channel object's Present_Value starts passing the value to its members.
/// This call runs either before returning, post delays and Execution_Delay
/// included: each write is made through bacnet-device-core's
/// `write::write_property`, with the Command or Channel as the initiating
/// object, and a write that reaches another Command or Channel starts that
/// run too (#1178, #1151). A
/// run that would start an object already above it is ended as failed rather
/// than started. Dropping the future first ends each unfinished run as
/// unsuccessful, so no Command is left in process and no Channel IN_PROGRESS.
pub async fn tick_schedules(db: &Arc<RwLock<ObjectDatabase>>) {
    let runs = {
        let mut db_w = db.write().await;
        let schedules = db_w.find_by_type(ObjectType::SCHEDULE);
        let commit = evaluate(&mut db_w, schedules);
        commit.take_command_runs(db, &mut db_w)
    };
    crate::command_lists::run_unattached(db, runs).await;
}

/// Evaluate schedules for the live server, returning the COV fanout owed for
/// the objects they wrote, and any Command runs those writes started, once
/// the database guard is dropped.
pub(crate) async fn tick_schedules_committed(
    db: &Arc<RwLock<ObjectDatabase>>,
    cov_table: &RwLock<CovSubscriptionTable>,
) -> CommittedCov {
    let mut db_w = db.write().await;
    let schedules = db_w.find_by_type(ObjectType::SCHEDULE);
    let commit = evaluate(&mut db_w, schedules);
    commit.finish(db, &mut db_w, cov_table).await
}

/// Evaluate the Schedules among `written`, objects a write just committed to,
/// under `db_w`, the guard on `db` that committed it; returns the COV fanout
/// owed for the objects they wrote, and any Command runs those writes
/// started, once that guard is dropped.
///
/// This is the pass [`tick_schedules_committed`] runs, limited to those
/// Schedules, so the new contents take effect without waiting for the next
/// tick. Lock order: the caller's database guard, then the COV table.
pub(crate) async fn reevaluate_written(
    db: &Arc<RwLock<ObjectDatabase>>,
    db_w: &mut ObjectDatabase,
    written: &[ObjectIdentifier],
    cov_table: &RwLock<CovSubscriptionTable>,
) -> CommittedCov {
    let schedules: Vec<_> = written
        .iter()
        .copied()
        .filter(|oid| oid.object_type() == ObjectType::SCHEDULE)
        .collect();
    if schedules.is_empty() {
        return CommittedCov::default();
    }
    let commit = evaluate(db_w, schedules);
    commit.finish(db, db_w, cov_table).await
}

/// Retry, under `db_w`, the refused references that name objects just added
/// to the database (#1440): each `(schedule, object)` pair is a Schedule
/// that held a refusal naming the object when it was added
/// (`ObjectDatabase::take_membership_work_internal`). The write each
/// Schedule builds now ([`BACnetObject::retry_refusals_naming`]) goes to
/// those references only, as a pass's retry would, and `commit` collects the
/// objects it changed for COV.
///
/// [`BACnetObject::retry_refusals_naming`]: bacnet_objects::traits::BACnetObject::retry_refusals_naming
pub(crate) fn retry_for_created(
    db_w: &mut ObjectDatabase,
    commit: &mut BackgroundCommit,
    retries: &[(ObjectIdentifier, ObjectIdentifier)],
) {
    for &(schedule, created) in retries {
        let Some(write) = db_w
            .get(&schedule)
            .and_then(|object| object.retry_refusals_naming(created))
        else {
            continue;
        };
        debug!(
            schedule = %schedule,
            created = %created,
            "Schedule retrying the references a created object makes writable"
        );
        deliver(db_w, commit, schedule, &write);
    }
}

/// Whether each Calendar is TRUE on `today`, resolved once per pass so every
/// Schedule that references one sees the same answer.
fn calendar_states(db: &ObjectDatabase, today: SpecificDate) -> HashMap<ObjectIdentifier, bool> {
    db.find_by_type(ObjectType::CALENDAR)
        .into_iter()
        .filter_map(|oid| {
            let calendar = db.get(&oid)?;
            // A Calendar that does not evaluate a date list itself reports
            // its state through Present_Value.
            let active = calendar.calendar_state_internal(today).unwrap_or_else(|| {
                matches!(
                    calendar.read_property(PropertyIdentifier::PRESENT_VALUE, None),
                    Ok(PropertyValue::Boolean(true))
                )
            });
            Some((oid, active))
        })
        .collect()
}

fn evaluate(db_w: &mut ObjectDatabase, schedules: Vec<ObjectIdentifier>) -> BackgroundCommit {
    let mut commit = BackgroundCommit::new();
    let instant = db_w.clock_frame().and_then(schedule_instant);
    if instant.is_none() {
        debug!("Skipping the Schedule calculation without a valid Device clock");
    }
    let calendars = instant
        .map(|(today, _)| calendar_states(db_w, today))
        .unwrap_or_default();
    let calendar_active = |oid: ObjectIdentifier| calendars.get(&oid).copied().unwrap_or(false);

    let mut writes = Vec::new();
    for oid in schedules {
        let Some(obj) = db_w.get_mut(&oid) else {
            continue;
        };
        // Owed writes go first, so a calculated value written in the same
        // pass, after a return to service, lands last.
        for write in obj.take_owed_schedule_writes() {
            debug!(
                schedule = %oid,
                refs = write.references.len(),
                "Schedule owes a write outside its calculation, writing to controlled properties"
            );
            writes.push((oid, write));
        }
        let Some((today, now)) = instant else {
            continue;
        };
        if let Some(write) = obj.tick_schedule(today, now, &calendar_active) {
            if write.retry {
                debug!(
                    schedule = %oid,
                    refs = write.references.len(),
                    "Schedule retrying the controlled properties that refused its value"
                );
            } else {
                debug!(
                    schedule = %oid,
                    refs = write.references.len(),
                    "Schedule value changed, writing to controlled properties"
                );
            }
            writes.push((oid, write));
        }
    }

    for (initiator, write) in writes {
        deliver(db_w, &mut commit, initiator, &write);
    }
    commit
}

/// Write one Schedule write to each of its targets, then report how each took
/// it to the Schedule.
fn deliver(
    db_w: &mut ObjectDatabase,
    commit: &mut BackgroundCommit,
    initiator: ObjectIdentifier,
    write: &ScheduleWrite,
) {
    let origin =
        crate::command_source::resolve_local(db_w, crate::LocalCommandSource::Object(initiator))
            .ok();
    let null = bacnet_device_core::write::relinquish::is_null_value(&write.value);
    // Clause 12.24.4: a failed member does not stop the others.
    let mut outcomes = Vec::with_capacity(write.references.len());
    for reference in &write.references {
        let target_oid = reference.object_identifier;
        let prop_id = reference.property_identifier;
        let property = PropertyIdentifier::from_raw(prop_id);
        commit.before_change(db_w, target_oid);
        // A missing object is a reference the Schedule can't write (#1433).
        let Some(target_obj) = db_w.get_mut(&target_oid) else {
            outcomes.push(ScheduleTargetOutcome::ReferenceRefused);
            continue;
        };
        // WriteProperty's index gate comes first (#1426), so an index on a
        // property that isn't an array fails the target without the object
        // seeing the value, a NULL included. Its refusals, like the object's
        // UNKNOWN_PROPERTY or INVALID_ARRAY_INDEX, fault the Schedule (#1433).
        let result = bacnet_device_core::write::check_write_array_index(
            &*target_obj,
            property,
            reference.property_array_index,
        )
        .and_then(|()| {
            crate::command_source::write_target(
                target_obj,
                property,
                reference.property_array_index,
                write.value.clone(),
                Some(write.priority),
                origin.as_ref(),
            )
        });
        let outcome = ScheduleTargetOutcome::of(&result);
        match &result {
            Ok(()) => commit.changed(target_oid),
            // A NULL on a target property that isn't commandable and can't
            // hold one leaves it as it is, and the target took the write
            // (`bacnet_device_core::write::relinquish`); nothing changed, so no COV follows.
            Err(error)
                if null
                    && bacnet_device_core::write::relinquish::leaves_unchanged(
                        &*target_obj,
                        property,
                        reference.property_array_index,
                        error,
                    ) =>
            {
                outcomes.push(ScheduleTargetOutcome::Accepted);
                continue;
            }
            // A retry still refused repeats every pass while the refusal
            // stands (#1436). One failing otherwise ends the refusal, so it
            // warns once, as a first write failing that way does.
            Err(e)
                if write.retry
                    && matches!(
                        outcome,
                        ScheduleTargetOutcome::DatatypeRefused
                            | ScheduleTargetOutcome::ReferenceRefused
                    ) =>
            {
                debug!(
                    target = %target_oid,
                    property = prop_id,
                    error = %e,
                    "Schedule retry still refused by controlled property"
                )
            }
            Err(e) => warn!(
                target = %target_oid,
                property = prop_id,
                error = %e,
                "Schedule failed to write to controlled property"
            ),
        }
        outcomes.push(outcome);
    }
    let reliability_changed = db_w
        .get_mut(&initiator)
        .is_some_and(|schedule| schedule.complete_schedule_write(write, &outcomes));
    if reliability_changed {
        commit.changed(initiator);
    }
}

#[cfg(test)]
#[path = "schedule_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "schedule_command_tests.rs"]
mod command_tests;

#[cfg(test)]
#[path = "schedule_channel_tests.rs"]
mod channel_tests;

#[cfg(test)]
#[path = "schedule_relinquish_tests.rs"]
mod relinquish_tests;

#[cfg(test)]
#[path = "schedule_reference_reliability_tests.rs"]
mod reference_reliability_tests;

#[cfg(test)]
#[path = "schedule_reference_retry_tests.rs"]
mod reference_retry_tests;
