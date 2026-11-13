//! Writes of a log's reporting settings, saved before they're served
//! (#1952).
//!
//! A log keeps the reporting settings a write sets in its snapshot (see
//! [`crate::log_persistence`]). Unlike its records, which commit on the
//! cadence, such a write is durable before the request is answered: the
//! log commits a snapshot holding it first, on the same writer as its
//! cadence commits, and refuses the write with DEVICE / OPERATIONAL_PROBLEM
//! if that commit fails. This follows the pattern of [`crate::durable`]:
//!
//! - The server stages the request's writes ([`stage_writes`](crate::durable::DurableWrites::stage_writes)). The log
//!   works out the settings each leaves and queues one commit of its whole
//!   state with the last of them, numbered like any other commit, then goes
//!   on serving the old settings. The request awaits the commit off the
//!   database guard.
//! - Back under the guard, each write takes its step ([`write()`]) once the
//!   commit has landed, and is then made on the log as usual; a commit that
//!   failed refuses it.
//! - While a write is staged the cadence's commits hold back
//!   ([`LogStore::staged`]): a commit of the old settings landing after the
//!   staged one would leave storage behind a write already answered. They
//!   resume once the request takes or releases its write.
//! - A staged write its request never makes, or that outlives its request,
//!   is dropped; its commit may have left storage ahead of the log, so the
//!   log commits the state it serves at once. A write nobody staged commits
//!   where it is and waits for the commit.
//!
//! A written Description is kept with them (#2364): the log's copy in its
//! BUFFER_READY state is what the snapshot holds, and the log serves the
//! same text. A NULL to it changes nothing, so it is neither saved nor
//! staged.
//!
//! A step holds nothing but its write: the stage works the settings each
//! write leaves out on one working copy ([`LogReporting`]), so a request of
//! many writes holds them a constant number of times (#2431), as an
//! object's kept rows are held (#2414). The write itself is made on the
//! log's own state, so a BUFFER_READY report committed while the request
//! waited keeps its counts and history. The steps are taken only while no
//! other write of these settings came between (`LogStore::writes`), so the
//! settings the write leaves are the ones committed. What an application
//! setter changes meanwhile, the log commits at its next look, as it does
//! any change the cadence holds back.

#[cfg_attr(not(feature = "durable-writes"), allow(unused_imports))]
use core::time::Duration;

use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::{DurableLog, LogState, LogStore, SnapshotRecord};
use crate::common;
#[cfg_attr(not(feature = "durable-writes"), allow(unused_imports))]
use crate::durable::staged;
#[cfg_attr(not(feature = "durable-writes"), allow(unused_imports))]
use crate::durable::{PendingWrite, SaveWait, StageStep};
use crate::log_persistence::LogReportingSettings;
use crate::log_persistence::codec::MAX_TEXT_OCTETS;
use crate::log_reporting::BufferReadyReporting;
use crate::trend::CovSettings;
use crate::write_config::description;

/// A log's reporting settings as a write leaves them: a copy of its
/// BUFFER_READY state, and of a Trend Log's COV settings.
#[derive(Clone)]
struct LogReporting {
    buffer_ready: BufferReadyReporting,
    cov: Option<CovSettings>,
}

impl LogReporting {
    fn of(log: &impl DurableLog) -> Self {
        let (buffer_ready, cov) = log.reporting();
        Self {
            buffer_ready: buffer_ready.clone(),
            cov: cov.copied(),
        }
    }

    /// The settings `write` leaves on top of these, or the log's refusal.
    /// A durable log also refuses an Event_Message_Texts_Config string its
    /// snapshot can't keep, with RESOURCES / NO_SPACE_TO_WRITE_PROPERTY: not
    /// enough room to store the value (Clause 15.9.1.3.1). This is checked
    /// here, before anything is staged or committed, so every storage
    /// backend gives that answer, not only one that encodes the snapshot.
    fn after(&self, write: &PendingWrite, total: u32) -> Result<Self, Error> {
        let mut next = self.clone();
        make(&mut next.buffer_ready, next.cov.as_mut(), write, total)?;
        // Only texts the snapshot keeps, the written ones, count (#2331).
        let texts = next.buffer_ready.message_texts_config();
        if next.buffer_ready.written(P::EVENT_MESSAGE_TEXTS_CONFIG)
            && texts.iter().any(|text| text.len() > MAX_TEXT_OCTETS)
        {
            return Err(common::protocol_error(
                ErrorClass::RESOURCES,
                ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
            ));
        }
        Ok(next)
    }

    /// The settings a write set, as a snapshot keeps them.
    fn saved(&self) -> LogReportingSettings {
        saved(&self.buffer_ready, self.cov.as_ref())
    }

    fn unwrite(&mut self) {
        self.buffer_ready.unwrite();
        if let Some(cov) = &mut self.cov {
            cov.unwrite();
        }
    }
}

impl LogState {
    /// This state with `reporting`'s settings, and the BUFFER_READY counts
    /// they leave: Event_Detection_Enable turned on restarts them.
    fn with_reporting(mut self, reporting: &LogReporting) -> Self {
        self.reporting = reporting.saved();
        (
            self.last_notify_record,
            self.records_since_notification_from,
        ) = reporting.buffer_ready.counts();
        self
    }
}

/// The settings a write set on these parts, as a snapshot keeps them.
fn saved(buffer_ready: &BufferReadyReporting, cov: Option<&CovSettings>) -> LogReportingSettings {
    let mut saved = buffer_ready.saved();
    if let Some(cov) = cov {
        cov.save_into(&mut saved);
    }
    saved
}

/// The settings storage keeps because a write set them: all `None` without
/// storage.
pub(crate) fn written(log: &impl DurableLog) -> LogReportingSettings {
    if log.store().is_none() {
        return LogReportingSettings::default();
    }
    let (buffer_ready, cov) = log.reporting();
    saved(buffer_ready, cov)
}

/// Whether the log keeps a written `property` with its snapshot: one of the
/// reporting settings every log has, or a Trend Log's COV settings.
fn kept(log: &impl DurableLog, property: P) -> bool {
    match property {
        P::NOTIFICATION_CLASS
        | P::EVENT_ENABLE
        | P::NOTIFY_TYPE
        | P::EVENT_DETECTION_ENABLE
        | P::EVENT_MESSAGE_TEXTS_CONFIG
        | P::EVENT_ALGORITHM_INHIBIT_REF
        | P::EVENT_ALGORITHM_INHIBIT => true,
        P::CLIENT_COV_INCREMENT | P::COV_RESUBSCRIPTION_INTERVAL => log.reporting().1.is_some(),
        // Beside them, a written Description (#2364).
        P::DESCRIPTION => true,
        _ => false,
    }
}

/// Whether the log keeps `write`: one of the settings [`kept`] names, but
/// not a NULL to Description, which changes nothing.
#[cfg(feature = "durable-writes")]
fn kept_write(log: &impl DurableLog, write: &PendingWrite) -> bool {
    kept(log, write.property) && !description::null_write(write.property, &write.value)
}

/// Make `write`, of a setting [`kept`] names, on these parts. A
/// Description past 16 KiB is refused here, before anything is staged or
/// committed, as a message text is (#2364).
fn make(
    buffer_ready: &mut BufferReadyReporting,
    cov: Option<&mut CovSettings>,
    write: &PendingWrite,
    total: u32,
) -> Result<(), Error> {
    if write.property == P::DESCRIPTION {
        buffer_ready.keep_description(description::written(&write.value)?);
        return Ok(());
    }
    if let Some(cov) = cov
        && let Some(result) = cov.write(write.property, &write.value)
    {
        return result;
    }
    buffer_ready
        .write(write.property, write.array_index, &write.value, total)
        .unwrap_or_else(|| Err(common::unknown_property_error()))
}

/// Make `write` on the log itself, counting it; the log serves a written
/// Description it keeps.
fn make_on<L: DurableLog>(log: &mut L, write: &PendingWrite) -> Result<(), Error> {
    let total = log.durable_parts().1.total_record_count();
    let (buffer_ready, cov) = log.reporting_mut();
    make(buffer_ready, cov, write, total)?;
    if let PropertyValue::CharacterString(text) = &write.value
        && write.property == P::DESCRIPTION
    {
        log.description_mut().clone_from(text);
    }
    if let Some(store) = log.durable_parts().0 {
        store.writes = store.writes.wrapping_add(1);
    }
    Ok(())
}

fn operational_problem() -> Error {
    common::protocol_error(ErrorClass::DEVICE, ErrorCode::OPERATIONAL_PROBLEM)
}

/// Whether the application's setter of `property` leaves it alone: the log
/// has storage, and a write has set it, now or before a restart.
pub(crate) fn kept_over_setter(log: &impl DurableLog, property: P) -> bool {
    let (buffer_ready, cov) = log.reporting();
    log.store().is_some()
        && (buffer_ready.written(property) || cov.is_some_and(|cov| cov.written(property)))
}

/// Take back the settings a reopened log saved, before its state is noted
/// as the one storage holds, and serve a saved Description.
pub(crate) fn restore<L: DurableLog>(log: &mut L, saved: &LogReportingSettings) {
    let (buffer_ready, cov) = log.reporting_mut();
    buffer_ready.restore(saved);
    if let Some(cov) = cov {
        cov.restore(saved);
    }
    if let Some(text) = &saved.description {
        log.description_mut().clone_from(text);
    }
}

/// After a storage call that may have dropped a staged write: if its commit
/// may have left storage ahead of the log, commit the state it serves. The
/// commit lands after the dropped one, since commits run in order.
fn correct<L: DurableLog>(log: &mut L) {
    let due = log
        .durable_parts()
        .0
        .is_some_and(|store| store.saves.take_correction());
    if due {
        let state = log.log_state();
        if let (Some(store), buffer) = log.durable_parts() {
            store.queue(state, buffer.records());
        }
    }
}

/// A write of `property` on `log`, when the log has storage and keeps the
/// setting; `None` otherwise, for the log to make as usual. The caller has
/// checked the array index.
///
/// The write's staged step is taken if it is the next one: the write is
/// then made, or refused with DEVICE / OPERATIONAL_PROBLEM if its commit
/// failed. Otherwise the write is worked out, which refuses it as the log
/// would, and a snapshot holding it committed in place first, with the
/// same refusal if that fails.
pub(crate) fn write<L: DurableLog>(
    log: &mut L,
    property: P,
    array_index: Option<u32>,
    value: &PropertyValue,
) -> Option<Result<(), Error>> {
    if log.store().is_none() || !kept(log, property) || description::null_write(property, value) {
        return None;
    }
    let write = PendingWrite {
        property,
        array_index,
        value: value.clone(),
    };
    Some(write_kept(log, &write))
}

fn write_kept<L: DurableLog>(log: &mut L, write: &PendingWrite) -> Result<(), Error> {
    let store = log.durable_parts().0.expect("checked by the caller");
    let base = store.writes;
    let claimed = store
        .saves
        .claim(write.property, write.array_index, &write.value, base);
    match claimed {
        Some(Ok(_)) => {
            let made = make_on(log, write);
            if made.is_err() {
                // Not expected, as the step was worked out on these same
                // settings; but storage holds it, so it goes back.
                if let Some(store) = log.durable_parts().0 {
                    store.drop_staged();
                }
                log.commit_if_changed();
            }
            correct(log);
            return made;
        }
        Some(Err(_)) => {
            correct(log);
            return Err(operational_problem());
        }
        None => {}
    }
    // A write the log refuses leaves another request's staged write alone.
    let state = log.log_state();
    let next = LogReporting::of(log).after(write, state.total_record_count)?;
    let state = state.with_reporting(&next);
    give_way(log);
    let (store, buffer) = log.durable_parts();
    let store = store.expect("checked by the caller");
    store
        .commit_now(state, buffer.records())
        .map_err(|_| operational_problem())?;
    make_on(log, write)
}

/// A write nobody staged, or a clear, is about to be made: another
/// request's staged write gives way, and storage goes back to the state the
/// log serves first, should the commit that follows fail.
fn give_way<L: DurableLog>(log: &mut L) {
    if let Some(store) = log.durable_parts().0 {
        store.saves.drop_staged();
    }
    correct(log);
}

impl<R: SnapshotRecord> LogStore<R> {
    /// Commit `state` with `records` and wait for the commit, for a write
    /// nobody staged or a clear; storage's error when it fails.
    fn commit_now(
        &mut self,
        state: LogState,
        records: &alloc::collections::VecDeque<R>,
    ) -> Result<(), Error> {
        let snapshot = state.snapshot(self.oid, records);
        self.saves.save_now(crate::sync::Mutex::new(snapshot))?;
        self.note_queued(state);
        Ok(())
    }
}

/// Stage the writes a request makes to `log`
/// ([`DurableWrites::stage_writes`](crate::durable::DurableWrites::stage_writes)):
/// one commit of the log's state with the settings they leave together.
#[cfg(feature = "durable-writes")]
pub(crate) fn stage_writes<L: DurableLog>(log: &mut L, writes: &[PendingWrite]) -> StageStep {
    if log.store().is_none() || !writes.iter().any(|write| kept_write(log, write)) {
        return StageStep::Skip;
    }
    let busy = log.durable_parts().0.and_then(|store| store.saves.busy());
    correct(log);
    if let Some(wait) = busy {
        return StageStep::Busy(wait);
    }
    let state = log.log_state();
    let current = LogReporting::of(log);
    let total = state.total_record_count;
    let view: &L = log;
    // One working copy of the settings the steps so far leave; a step keeps
    // only its write (#2431).
    let mut working: Option<LogReporting> = None;
    let steps = staged::steps(writes, |_, write| {
        if !kept_write(view, write) {
            return Ok(None);
        }
        crate::property_metadata::check_write_array_index(view, write.property, write.array_index)?;
        working = Some(working.as_ref().unwrap_or(&current).after(write, total)?);
        Ok(Some(()))
    });
    // A step was taken exactly when the working copy was set.
    let Some(last) = working else {
        return StageStep::Skip;
    };
    let state = state.with_reporting(&last);
    let (store, buffer) = log.durable_parts();
    let store = store.expect("checked above");
    let snapshot = state.snapshot(store.oid, buffer.records());
    let step = store
        .saves
        .stage_snapshot(steps, store.writes, crate::sync::Mutex::new(snapshot));
    if matches!(step, StageStep::Staged(_)) {
        store.note_queued(state);
    }
    step
}

/// See [`DurableWrites::release_staged_write`](crate::durable::DurableWrites::release_staged_write).
#[cfg(feature = "durable-writes")]
pub(crate) fn release<L: DurableLog>(log: &mut L, staged: &SaveWait) {
    if let Some(store) = log.durable_parts().0 {
        store.saves.release(staged);
    }
    correct(log);
}

/// See [`DurableWrites::expire_staged`](crate::durable::DurableWrites::expire_staged).
#[cfg(feature = "durable-writes")]
pub(crate) fn expire<L: DurableLog>(log: &mut L, now: Duration) {
    if let Some(store) = log.durable_parts().0 {
        store.saves.expire(now);
    }
    correct(log);
}

/// Drop every reporting setting a write saved: commit the log without them
/// and wait for the commit, then let the setters apply again. The settings
/// served stay until a setter or a write changes them. A commit that fails
/// keeps them written and returns storage's error. Does nothing without
/// storage; with nothing written, only marks it so.
pub(crate) fn clear<L: DurableLog>(log: &mut L) -> Result<(), Error> {
    if log.store().is_none() {
        return Ok(());
    }
    let state = log.log_state();
    if state.reporting != LogReportingSettings::default() {
        let mut cleared = LogReporting::of(log);
        cleared.unwrite();
        let state = state.with_reporting(&cleared);
        // A staged step worked out from the written settings gives way.
        give_way(log);
        let (store, buffer) = log.durable_parts();
        let store = store.expect("checked above");
        store.commit_now(state, buffer.records())?;
        store.writes = store.writes.wrapping_add(1);
    }
    let (buffer_ready, cov) = log.reporting_mut();
    buffer_ready.unwrite();
    if let Some(cov) = cov {
        cov.unwrite();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::error::RemoteError;

    /// A long message text the snapshot doesn't keep, as one unwritten,
    /// refuses no other write; once a write sets the texts it is refused
    /// (#2331).
    #[test]
    fn only_kept_texts_meet_the_cap() {
        let long = "x".repeat(MAX_TEXT_OCTETS + 1);
        let mut buffer_ready = BufferReadyReporting::default();
        buffer_ready.restore(&LogReportingSettings {
            event_message_texts_config: Some([long, String::new(), String::new()]),
            ..LogReportingSettings::default()
        });
        buffer_ready.unwrite();
        let reporting = LogReporting {
            buffer_ready,
            cov: None,
        };
        let write = |property, array_index, value| PendingWrite {
            property,
            array_index,
            value,
        };
        let class = write(P::NOTIFICATION_CLASS, None, PropertyValue::Unsigned(9));
        assert!(reporting.after(&class, 0).is_ok());
        let text = write(
            P::EVENT_MESSAGE_TEXTS_CONFIG,
            Some(3),
            PropertyValue::CharacterString("back".into()),
        );
        assert!(matches!(
            reporting.after(&text, 0),
            Err(Error::Remote(RemoteError::Error { class, code }))
                if class == ErrorClass::RESOURCES && code == ErrorCode::NO_SPACE_TO_WRITE_PROPERTY
        ));
    }
}
