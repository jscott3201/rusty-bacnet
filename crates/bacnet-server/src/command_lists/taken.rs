//! Runs taken from their objects and not yet handed to whatever makes them
//! (#1324).
//!
//! A run is taken under the guard that committed the Present_Value write
//! that queued it, but its owner (a task in the server's request set, or
//! [`run_unattached`](super::run_unattached)'s queue) takes it up only once
//! the writer's COV, event and Schedule passes are done. A future cancelled
//! in between, a `write_local` under a timeout for one, would drop the run
//! with nothing to end it and leave its object busy. [`TakenRuns`] holds the
//! runs across that stretch and ends any it still holds when dropped.

use std::fmt;
use std::sync::Arc;

use bacnet_objects::command::CommandRun;
use bacnet_objects::database::ObjectDatabase;
use bacnet_types::primitives::ObjectIdentifier;
use tokio::sync::RwLock;
use tracing::debug;

use super::{Unfinished, when_free};

/// Runs taken from their objects, owned until each is handed on.
///
/// Dropping it ends every run it still holds as if none of the run's writes
/// had been made: a Command's In_Process back to FALSE with each command
/// unsuccessful, a Channel's Write_Status FAILED with Reliability
/// PROCESS_ERROR. That happens at once when the database is free and
/// otherwise as soon as it is, from a task, since `Drop` can't wait. Nothing
/// is reported to COV subscribers from there.
///
/// A run leaves through [`Self::hand_over`] or by being dropped here, never
/// both, so it ends once. Ending is also generation-checked: an end aimed at
/// a run that already ended, or at an object that has since started another
/// run, changes nothing.
#[derive(Default)]
#[must_use = "runs dropped unstarted end as unsuccessful"]
pub(crate) struct TakenRuns {
    runs: Vec<CommandRun>,
    /// The database the runs' objects live in, where a drop ends them. Set
    /// whenever `runs` holds anything.
    database: Option<Arc<RwLock<ObjectDatabase>>>,
}

impl TakenRuns {
    /// Take the runs that Present_Value writes queued on Command and Channel
    /// objects among `oids`, under `db`, the guard on `database` that
    /// committed those writes.
    pub(crate) fn take(
        database: &Arc<RwLock<ObjectDatabase>>,
        db: &mut ObjectDatabase,
        oids: &[ObjectIdentifier],
    ) -> Self {
        let runs = super::take_queued(db, oids);
        // Most writes queue nothing, and nothing then needs the database.
        let database = (!runs.is_empty()).then(|| Arc::clone(database));
        Self { runs, database }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &CommandRun> {
        self.runs.iter()
    }

    pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = &mut CommandRun> {
        self.runs.iter_mut()
    }

    /// Hold `other`'s runs too, after these.
    pub(crate) fn extend(&mut self, mut other: Self) {
        if other.runs.is_empty() {
            return;
        }
        debug_assert!(
            self.database
                .as_ref()
                .zip(other.database.as_ref())
                .is_none_or(|(ours, theirs)| Arc::ptr_eq(ours, theirs))
        );
        if self.database.is_none() {
            self.database = other.database.take();
        }
        self.runs.append(&mut other.runs);
    }

    /// Move the runs `split` picks into a set of their own, keeping the rest
    /// here in order.
    pub(crate) fn split_off(&mut self, mut split: impl FnMut(&CommandRun) -> bool) -> Self {
        let (picked, kept) = std::mem::take(&mut self.runs)
            .into_iter()
            .partition(|run| split(run));
        self.runs = kept;
        Self {
            runs: picked,
            database: self.database.clone(),
        }
    }

    /// Hand each run, in order, to `owner`, which owns it from then on and
    /// has to end it. A run is held here until `owner` takes it, so a panic
    /// part way leaves the rest to be ended by the drop.
    pub(crate) fn hand_over(mut self, mut owner: impl FnMut(CommandRun)) {
        self.runs.reverse();
        while let Some(run) = self.runs.pop() {
            owner(run);
        }
    }

    /// End every run here under `db` as if none of its writes had been made.
    /// Returns the objects whose runs those were.
    pub(crate) fn end(mut self, db: &mut ObjectDatabase) -> Vec<ObjectIdentifier> {
        self.runs
            .drain(..)
            .map(|run| {
                Unfinished::start(&run).end(db);
                run.source
            })
            .collect()
    }
}

impl Drop for TakenRuns {
    fn drop(&mut self) {
        let Some(database) = self.database.take() else {
            // Runs only arrive through `take`, which brings their database.
            debug_assert!(self.runs.is_empty(), "taken runs without a database");
            return;
        };
        if self.runs.is_empty() {
            return;
        }
        let left: Vec<_> = self
            .runs
            .drain(..)
            .map(|run| {
                debug!(source = %run.source, "Ending a run let go of before it started");
                Unfinished::start(&run)
            })
            .collect();
        when_free(&database, move |db| {
            for left in left {
                left.end(db);
            }
        });
    }
}

impl fmt::Debug for TakenRuns {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(&self.runs).finish()
    }
}

#[cfg(test)]
#[path = "taken_tests.rs"]
mod tests;
