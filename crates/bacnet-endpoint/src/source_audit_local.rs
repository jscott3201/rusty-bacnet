//! The session's local operations on its database: the recipient write and
//! the reset report (#2759).
//!
//! Neither waits for the database's guard in its caller's task: Tokio's
//! lock hands its guard to a queued waiter whether or not that waiter is
//! polled again, so a caller parked in a `select!`, `join!` or
//! `buffer_unordered` could own the database, with every request the node
//! serves waiting behind it. The caller takes the guard only with
//! `try_write`, which queues for nothing, and with the database free runs
//! the operation in that poll, as it always did. With the database busy, a
//! task on the session's pool waits for the guard and runs the same
//! synchronous operation under it, and the caller waits only for its
//! answer, as an audited request does (#2755).
//!
//! A caller that goes before its task holds the guard leaves the operation
//! undone; one that goes after leaves it done. There is one task per call
//! that found the database busy, and it ends once it has run the operation
//! or its caller has gone. The stop that closes the pool ends it, the
//! operation undone; a call borrows the session, so its caller has gone by
//! then.
use super::*;
use tokio::sync::RwLockWriteGuard;

impl SourceAudit {
    /// Run `op` under the database's write guard, never waiting for the
    /// guard in the caller's task (see the module documentation). `None`
    /// when the operation was not run: the session's pool is gone or
    /// closed, or `op` panicked in the task.
    ///
    /// It stays apart from `bacnet_server::caller_waits::for_caller`
    /// (#2781): that helper raises a panic again in the caller where this
    /// answers `None`, and would bound `op` by `Sync` too.
    pub(crate) async fn under_write_guard<R: Send + 'static>(
        self: &Arc<Self>,
        op: impl FnOnce(&Arc<Self>, &mut ObjectDatabase) -> R + Send + 'static,
    ) -> Option<R> {
        if let Ok(mut db) = self.db.try_write() {
            return Some(op(self, &mut db));
        }
        let owner = self.notifications.upgrade()?;
        let (mut reply, answer) = oneshot::channel();
        let source = Arc::clone(self);
        // A rejected spawn drops the task, and with it the reply.
        owner.spawn(async move {
            let Some(mut db) = source.write_for(&mut reply).await else {
                return;
            };
            let result = op(&source, &mut db);
            drop(db);
            let _ = reply.send(result);
        });
        drop(owner);
        answer.await.ok()
    }

    /// The database's write guard, for a task the session runs for a caller
    /// that waits on `reply`, or `None` once that caller has gone, before
    /// the guard came or as it came. Only such a task, which the session
    /// polls, waits here: an audited request's (#2755) and a local
    /// operation's (#2759).
    pub(super) async fn write_for<T>(
        &self,
        reply: &mut oneshot::Sender<T>,
    ) -> Option<RwLockWriteGuard<'_, ObjectDatabase>> {
        let db = tokio::select! {
            biased;
            db = self.db.write() => db,
            () = reply.closed() => return None,
        };
        (!reply.is_closed()).then_some(db)
    }
}
