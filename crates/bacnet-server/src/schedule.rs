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
//!
//! A Schedule removed from the database gives up every slot it still holds
//! through the same target writes (#2256, #2264). The database queues the
//! NULLs as the object leaves, whether DeleteObject or the application's
//! own `ObjectDatabase::remove` took it out, and the server writes them
//! under the guard that takes the queue (`relinquish_for_removed`): the
//! request's for a DeleteObject, the next one the Schedule task gets for an
//! application's removal, or the first after start for one before it.
//!
//! A NULL that gives up a held slot (`ScheduleWrite::relinquish`) goes only
//! to a slot whose Value_Source still names the Schedule (#2264): one a
//! client, the application or another Schedule wrote since keeps that
//! writer's command. The check reads the target's Value_Source_Array at the
//! write's priority. A reference to a property other than Present_Value,
//! or a target that doesn't serve that element (a Channel, an object with
//! no source tracking), can't tell who holds the slot, so it gets the NULL,
//! as it did before the check. A calculated NULL, a time or default whose
//! value is NULL, is not checked: it goes to every reference.

use std::collections::HashMap;
use std::sync::Arc;

use crate::committed_cov::{BackgroundCommit, CommittedCov};
use crate::cov::CovSubscriptionTable;
use bacnet_device_core::write::local;
use bacnet_objects::clock::ClockFrame;
use bacnet_objects::command_source::CommandOrigin;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::schedule::{ScheduleTargetOutcome, ScheduleWrite};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::calendar::SpecificDate;
use bacnet_types::constructed::BACnetValueSource;
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
/// It first does the work changes of membership queued (the NULLs a removed
/// Schedule owes, retries to created objects), also without COV, so those
/// land before any Schedule re-added under a removed one's identifier
/// writes (#2264).
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
        let commit = pass(&mut db_w, schedules);
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
    let commit = pass(&mut db_w, schedules);
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
    let commit = pass(db_w, schedules);
    commit.finish(db, db_w, cov_table).await
}

/// Retry, under `db_w`, the refused references that name objects just added
/// to the database (#1440): each `(schedule, object)` pair is a Schedule
/// that held a refusal naming the object when it was added
/// (`ObjectDatabase::take_membership_work_internal`). The write each
/// Schedule builds now ([`ScheduleRunner::retry_refusals_naming`]) goes to
/// those references only, as a pass's retry would, and `commit` collects the
/// objects it changed for COV.
///
/// [`ScheduleRunner::retry_refusals_naming`]: bacnet_objects::traits::ScheduleRunner::retry_refusals_naming
pub(crate) fn retry_for_created(
    db_w: &mut ObjectDatabase,
    commit: &mut BackgroundCommit,
    retries: &[(ObjectIdentifier, ObjectIdentifier)],
) {
    for &(schedule, created) in retries {
        let Some(write) = db_w
            .get_mut(&schedule)
            .and_then(|object| object.schedule_runner())
            .and_then(|runner| runner.retry_refusals_naming(created))
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

/// Write, under `db_w`, the NULLs Schedules (and Staging objects, #2265)
/// removed from the database owe their targets (#2256, #2264): each
/// `(schedule, write)` pair is a relinquish the database queued as the
/// object left
/// (`ObjectDatabase::take_membership_work_internal`). They carry the origin
/// the Schedule's own writes did, the local Device with the Schedule as the
/// initiating object, built without the Schedule in the database
/// ([`removed_origin`]). A target that refuses one is logged as a pass's
/// failure is, and the others are still written; `commit` collects the
/// objects they changed for COV.
pub(crate) fn relinquish_for_removed(
    db_w: &mut ObjectDatabase,
    commit: &mut BackgroundCommit,
    relinquishes: &[(ObjectIdentifier, ScheduleWrite)],
) {
    for (schedule, write) in relinquishes {
        debug!(
            source = %schedule,
            refs = write.references.len(),
            "Removed Schedule or Staging object relinquishing the slots it held"
        );
        let origin = removed_origin(db_w, *schedule);
        write_targets(db_w, commit, *schedule, origin.as_ref(), write);
    }
}

/// Write, under `db_w`, the NULLs the Staging object `source` owes the slots
/// a change of its targets or priority left behind (#2289,
/// [`StagingSource::take_owed_relinquishes`]), with the origin its own
/// writes carry. Returns whether it owed any.
///
/// [`StagingSource::take_owed_relinquishes`]: bacnet_objects::traits::StagingSource::take_owed_relinquishes
pub(crate) fn relinquish_owed_by_staging(
    db_w: &mut ObjectDatabase,
    commit: &mut BackgroundCommit,
    source: ObjectIdentifier,
) -> bool {
    let Some(writes) = db_w
        .get_mut(&source)
        .and_then(|object| object.staging_source())
        .map(|staging| staging.take_owed_relinquishes())
        .filter(|writes| !writes.is_empty())
    else {
        return false;
    };
    let origin = command_origin(db_w, source);
    for write in &writes {
        debug!(
            source = %source,
            refs = write.references.len(),
            "Staging object relinquishing the slots a change of its targets left"
        );
        write_targets(db_w, commit, source, origin.as_ref(), write);
    }
    true
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
            let active = match calendar.calendar_state() {
                Some(state) => state.state_on(today),
                None => matches!(
                    calendar.read_property(PropertyIdentifier::PRESENT_VALUE, None),
                    Ok(PropertyValue::Boolean(true))
                ),
            };
            Some((oid, active))
        })
        .collect()
}

/// The Schedule pass for `schedules` under `db_w`: first the work the
/// membership queue holds (`crate::membership::drain`), so a removed
/// Schedule's NULLs land before a Schedule re-added under its identifier
/// writes (#2264), then [`evaluate`]. Kept out of the async callers so their
/// futures hold only the commit it returns.
fn pass(db_w: &mut ObjectDatabase, schedules: Vec<ObjectIdentifier>) -> BackgroundCommit {
    let mut commit = BackgroundCommit::new();
    crate::membership::drain(db_w, &mut commit);
    evaluate(db_w, &mut commit, schedules);
    commit
}

/// Run the pass for `schedules` under `db_w`, collecting what it changed in
/// `commit`.
fn evaluate(
    db_w: &mut ObjectDatabase,
    commit: &mut BackgroundCommit,
    schedules: Vec<ObjectIdentifier>,
) {
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
        let Some(runner) = db_w
            .get_mut(&oid)
            .and_then(|object| object.schedule_runner())
        else {
            continue;
        };
        // Owed writes go first, so a calculated value written in the same
        // pass, after a return to service, lands last.
        for write in runner.take_owed_schedule_writes() {
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
        if let Some(write) = runner.tick_schedule(today, now, &calendar_active) {
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
        deliver(db_w, commit, initiator, &write);
    }
}

/// Write one Schedule write to each of its targets, then report how each took
/// it to the Schedule.
fn deliver(
    db_w: &mut ObjectDatabase,
    commit: &mut BackgroundCommit,
    initiator: ObjectIdentifier,
    write: &ScheduleWrite,
) {
    let origin = command_origin(db_w, initiator);
    let outcomes = write_targets(db_w, commit, initiator, origin.as_ref(), write);
    let reliability_changed = db_w
        .get_mut(&initiator)
        .and_then(|schedule| schedule.schedule_runner())
        .is_some_and(|runner| runner.complete_schedule_write(write, &outcomes));
    if reliability_changed {
        commit.changed(initiator);
    }
}

/// The origin a Schedule's writes carry: the local Device, with the Schedule
/// as the initiating object, while both are in the database.
fn command_origin(db: &ObjectDatabase, schedule: ObjectIdentifier) -> Option<CommandOrigin> {
    local::resolve_source(db, local::LocalCommandSource::Object(schedule)).ok()
}

/// [`command_origin`] for a Schedule already out of the database: the local
/// Device, while its object is here, with `schedule` as the initiating
/// object, the origin its writes carried while it was in.
fn removed_origin(db: &ObjectDatabase, schedule: ObjectIdentifier) -> Option<CommandOrigin> {
    match local::resolve_source(db, local::LocalCommandSource::ServerDevice) {
        Ok(CommandOrigin::Local { owner_device, .. }) => Some(CommandOrigin::Local {
            owner_device,
            initiating_object: Some(schedule),
        }),
        _ => None,
    }
}

/// Whether the slot at `priority` of `reference` on `target` may still be
/// `schedule`'s (#2264): `false` only when the target's Value_Source_Array
/// element for that priority names another writer, or none. A reference to
/// a property other than Present_Value, or a target that doesn't serve the
/// element, can't tell, and counts as the Schedule's.
fn still_held(
    target: &dyn BACnetObject,
    reference: &bacnet_types::constructed::BACnetObjectPropertyReference,
    priority: u8,
    schedule: ObjectIdentifier,
    local_device: Option<ObjectIdentifier>,
) -> bool {
    if reference.property_identifier != PropertyIdentifier::PRESENT_VALUE.to_raw()
        || reference.property_array_index.is_some()
    {
        return true;
    }
    let Ok(PropertyValue::ApplicationData(bytes)) = target.read_property(
        PropertyIdentifier::VALUE_SOURCE_ARRAY,
        Some(u32::from(priority)),
    ) else {
        return true;
    };
    let Ok((source, _)) = bacnet_encoding::constructed::decode_value_source(&bytes, 0) else {
        return true;
    };
    match source {
        BACnetValueSource::Object(source) => {
            source.object_identifier == schedule
                && source
                    .device_identifier
                    .is_none_or(|device| Some(device) == local_device)
        }
        _ => false,
    }
}

/// Write `write` to each of its targets under `db_w`, collecting the changed
/// ones in `commit`, and return how each took it.
fn write_targets(
    db_w: &mut ObjectDatabase,
    commit: &mut BackgroundCommit,
    schedule: ObjectIdentifier,
    origin: Option<&CommandOrigin>,
    write: &ScheduleWrite,
) -> Vec<ScheduleTargetOutcome> {
    let null = bacnet_device_core::write::relinquish::is_null_value(&write.value);
    let local_device = db_w.local_device().identifier();
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
        // A slot another writer took since is theirs now: the relinquish
        // leaves it, and the target counts as having taken it (#2264).
        if write.relinquish
            && !still_held(
                &*target_obj,
                reference,
                write.priority,
                schedule,
                local_device,
            )
        {
            debug!(
                schedule = %schedule,
                target = %target_oid,
                priority = write.priority,
                "Schedule relinquish skipped: another writer holds the slot"
            );
            outcomes.push(ScheduleTargetOutcome::Accepted);
            continue;
        }
        // WriteProperty's index gate comes first (#1426), so an index on a
        // property that isn't an array fails the target without the object
        // seeing the value, a NULL included. Its refusals, like the object's
        // UNKNOWN_PROPERTY or INVALID_ARRAY_INDEX, fault the Schedule (#1433).
        let result = bacnet_objects::property_metadata::check_write_array_index(
            &*target_obj,
            property,
            reference.property_array_index,
        )
        .and_then(|()| {
            local::write_target(
                target_obj,
                property,
                reference.property_array_index,
                write.value.clone(),
                Some(write.priority),
                origin,
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
            // The source may be a removed Staging object's relinquish too
            // (#2265), so the log names it.
            Err(e) => warn!(
                source = %schedule,
                source_type = ?schedule.object_type(),
                target = %target_oid,
                property = prop_id,
                error = %e,
                "Schedule or Staging object failed to write to controlled property"
            ),
        }
        outcomes.push(outcome);
    }
    outcomes
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
