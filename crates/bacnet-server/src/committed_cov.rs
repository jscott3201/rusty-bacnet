//! COV fanout owed by a background commit (#889).
//!
//! Background tasks (periodic intrinsic reporting, fault detection, schedule
//! writes) mutate objects under the database guard and fan COV out after
//! dropping it, as a network write does. Ordinary objects are re-evaluated as a
//! whole; Life Safety objects report exactly the properties the pass changed.
//! A Schedule writing a Command object's Present_Value also leaves a Command
//! run to start once the guard is dropped (#1150).
//!
//! Each object a pass changed commits the intrinsic transition its new state
//! causes under the pass's guard too, before the timestamped capture, as a
//! write does (#2535): no reader, COV report or timestamped row sees it with
//! a new value or Reliability and the Event_State from before.

use crate::command_lists::TakenRuns;
use crate::cov::CovSubscriptionTable;
use bacnet_device_core::cov::life_safety::{
    LifeSafetyCovChange, LifeSafetyCovSnapshots, is_life_safety_object,
};
use bacnet_objects::database::ObjectDatabase;
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;

/// What [`BackgroundCommit::finish`] calls for each object the pass changed,
/// under its guard: the per-write intrinsic evaluation, which commits the
/// transition the object's new state causes and pushes its event
/// notifications onto their lanes (#2535). True when it committed one.
pub(crate) type CommitTransition<'a> =
    &'a mut (dyn FnMut(&mut ObjectDatabase, ObjectIdentifier) -> bool + Send);

/// Objects one background pass changed, collected under its database guard.
pub(crate) struct BackgroundCommit {
    life_safety: LifeSafetyCovSnapshots,
    changed: Vec<ObjectIdentifier>,
    marked: HashSet<ObjectIdentifier>,
}

impl BackgroundCommit {
    /// A pass that calls [`Self::before_change`] ahead of each mutation.
    pub(crate) fn new() -> Self {
        Self {
            life_safety: LifeSafetyCovSnapshots::default(),
            changed: Vec::new(),
            marked: HashSet::new(),
        }
    }

    /// A pass whose mutations can't be observed one by one: snapshot every
    /// Life Safety object up front.
    pub(crate) fn snapshot_all(db: &ObjectDatabase) -> Self {
        let life_safety = db
            .find_by_type(ObjectType::LIFE_SAFETY_POINT)
            .into_iter()
            .chain(db.find_by_type(ObjectType::LIFE_SAFETY_ZONE));
        Self {
            life_safety: LifeSafetyCovSnapshots::capture_oids(db, life_safety),
            changed: Vec::new(),
            marked: HashSet::new(),
        }
    }

    /// Keep a Life Safety object's state from before the pass mutates it.
    pub(crate) fn before_change(&mut self, db: &ObjectDatabase, oid: ObjectIdentifier) {
        self.life_safety.capture(db, oid);
    }

    /// Owe a fanout for `oid`. Marking an object that did not actually change
    /// is harmless, since the COV criteria then report nothing.
    pub(crate) fn changed(&mut self, oid: ObjectIdentifier) {
        if self.marked.insert(oid) {
            self.changed.push(oid);
        }
    }

    /// Take the Command runs the pass's writes queued, under its guard `db` on
    /// `database`. The caller owns them (#1178).
    pub(crate) fn take_command_runs(
        &self,
        database: &Arc<RwLock<ObjectDatabase>>,
        db: &mut ObjectDatabase,
    ) -> TakenRuns {
        TakenRuns::take(database, db, &self.changed)
    }

    /// Finish under the same guard, `db` on `database`: commit, through
    /// `transition`, the intrinsic transition each changed object's new state
    /// causes, in the order the pass changed them, then do the rest of
    /// [`Self::finish_committed`]. A transition with a Time_Delay is only
    /// seeded here, due its delay after now, and the intrinsic sweep fires it.
    pub(crate) async fn finish(
        self,
        database: &Arc<RwLock<ObjectDatabase>>,
        db: &mut ObjectDatabase,
        cov_table: &RwLock<CovSubscriptionTable>,
        transition: CommitTransition<'_>,
    ) -> CommittedCov {
        for oid in &self.changed {
            transition(db, *oid);
        }
        self.finish_committed(database, db, cov_table).await
    }

    /// Finish under the same guard, `db` on `database`, a pass whose objects'
    /// transitions are committed already (the intrinsic sweep's): record
    /// timestamped COV-multiple history at commit time, after them, so a
    /// change and its transition are one row (#2535), take the Command runs
    /// the pass queued, then split the fanout owed once the guard is dropped.
    pub(crate) async fn finish_committed(
        self,
        database: &Arc<RwLock<ObjectDatabase>>,
        db: &mut ObjectDatabase,
        cov_table: &RwLock<CovSubscriptionTable>,
    ) -> CommittedCov {
        if self.changed.is_empty() {
            return CommittedCov::default();
        }
        let command_runs = self.take_command_runs(database, db);
        let (life_safety, coarse): (Vec<_>, Vec<_>) = self
            .changed
            .into_iter()
            .partition(|oid| is_life_safety_object(*oid));
        let life_safety = self.life_safety.changes(db, &life_safety);
        let captures: Vec<_> = {
            let table = cov_table.read().await;
            coarse
                .iter()
                .map(|oid| table.timed_capture(*oid))
                .chain(std::iter::once(table.timed_capture_exact(&life_safety)))
                .collect()
        };
        for capture in captures {
            capture.run(db);
        }
        CommittedCov {
            life_safety,
            coarse,
            command_runs,
        }
    }
}

/// COV fanout, and Command runs to start, owed after a background commit.
#[derive(Debug, Default)]
pub(crate) struct CommittedCov {
    pub(crate) coarse: Vec<ObjectIdentifier>,
    pub(crate) life_safety: Vec<LifeSafetyCovChange>,
    /// Runs the pass's Present_Value writes queued on Command and Channel
    /// objects. A pass that can write one (the Schedule's) starts them; any
    /// left unstarted end unsuccessful when this is dropped.
    pub(crate) command_runs: TakenRuns,
}

impl CommittedCov {
    pub(crate) fn is_empty(&self) -> bool {
        self.coarse.is_empty() && self.life_safety.is_empty() && self.command_runs.is_empty()
    }

    /// Add this fanout and these runs to a request's own, leaving out objects
    /// the request already fans out for.
    pub(crate) fn merge_into(
        self,
        coarse: &mut Vec<ObjectIdentifier>,
        life_safety: &mut Vec<LifeSafetyCovChange>,
        command_runs: &mut TakenRuns,
    ) {
        for oid in self.coarse {
            if !coarse.contains(&oid) {
                coarse.push(oid);
            }
        }
        life_safety.extend(self.life_safety);
        command_runs.extend(self.command_runs);
    }
}
