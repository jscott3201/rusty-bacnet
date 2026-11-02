//! The work adding and removing objects leaves for the server (#1341,
//! #1440).
//!
//! `ObjectDatabase::add` and `remove` judge again the Pulse Converters whose
//! Input_Reference names the object, under the caller's guard, and queue
//! those whose Reliability changed; `add` also queues each Schedule holding
//! a refused reference to the new object, and a Schedule's removal (or
//! replacement by `add`) queues the NULLs that give up the slots it held. The rest is the server's: the
//! Schedule's retry to the new object ([`crate::schedule::retry_for_created`]),
//! the removed Schedule's relinquishes
//! ([`crate::schedule::relinquish_for_removed`], #2256, #2264) and the COV
//! for what changed. A CreateObject or DeleteObject takes the
//! queue under its own guard and adds the fanout, and any Command run a
//! retry starts, to the request's, and an application's own `add` or
//! `remove` wakes the waker [`install_waker`] puts on the database, which
//! the server's Schedule task waits on to take the queue under the next
//! guard it gets ([`settle_committed`]). Every Schedule pass also drains the
//! queue ([`drain`]) before it evaluates, so a removed Schedule's NULLs
//! always land before a Schedule re-added under its identifier writes:
//! their Value_Source check can't tell the two apart (#2264). Lock order is
//! the server's: the database guard, then the COV table.

use std::sync::Arc;

use bacnet_objects::database::ObjectDatabase;
use tokio::sync::{Notify, RwLock};

use crate::committed_cov::{BackgroundCommit, CommittedCov};
use crate::cov::CovSubscriptionTable;

/// Take the work queued on `db_w`, the guard on `db`, and return the COV
/// fanout it owes, to send once that guard is dropped.
pub(crate) async fn settle(
    db: &Arc<RwLock<ObjectDatabase>>,
    db_w: &mut ObjectDatabase,
    cov_table: &RwLock<CovSubscriptionTable>,
) -> CommittedCov {
    let mut commit = BackgroundCommit::new();
    if !drain(db_w, &mut commit) {
        return CommittedCov::default();
    }
    commit.finish(db, db_w, cov_table).await
}

/// Take the work queued on `db_w` and do it into `commit`: the removed
/// Schedules' relinquishes, then the retries to created objects, with the
/// objects they changed and those the queue names collected for COV.
/// Returns whether there was any.
pub(crate) fn drain(db_w: &mut ObjectDatabase, commit: &mut BackgroundCommit) -> bool {
    let work = db_w.take_membership_work_internal();
    if work.is_empty() {
        return false;
    }
    for oid in work.changed {
        commit.changed(oid);
    }
    crate::schedule::relinquish_for_removed(db_w, commit, &work.relinquishes);
    crate::schedule::retry_for_created(db_w, commit, &work.schedule_retries);
    true
}

/// [`settle`] under a guard of its own, for the server's background task.
pub(crate) async fn settle_committed(
    db: &Arc<RwLock<ObjectDatabase>>,
    cov_table: &RwLock<CovSubscriptionTable>,
) -> CommittedCov {
    let mut db_w = db.write().await;
    settle(db, &mut db_w, cov_table).await
}

/// Install a waker on `db` that wakes the returned notify whenever a change
/// of membership queues work, armed once so work queued before the server
/// started is taken too.
pub(crate) fn install_waker(db: &mut ObjectDatabase) -> Arc<Notify> {
    let notify = Arc::new(Notify::new());
    let waker = Arc::clone(&notify);
    db.set_membership_waker_internal(Some(Arc::new(move || waker.notify_one())));
    notify.notify_one();
    notify
}
