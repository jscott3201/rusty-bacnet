//! Releasing what a start began when the start doesn't finish (#2479).
//!
//! A start drives the objects' outputs (`ObjectDatabase::start_outputs`)
//! before steps that can still fail, or never end because the caller drops
//! the start's future: the transport's start, a BACnet/SC hub's connect.
//! A restored Load Control asks its application to shed then, so a start
//! that doesn't finish releases what it began
//! (`ObjectDatabase::release_outputs`), and the application hears `stop`.

use std::sync::Arc;

use bacnet_objects::database::ObjectDatabase;
use tokio::sync::RwLock;

/// The database from the outputs' start until the start completes. Dropped
/// before [`complete`](Self::complete), on an error return or with the
/// start's future, it releases the outputs.
pub(super) struct StartedOutputs {
    held: Option<Held>,
    /// The runtime the start runs on, for a release that must wait for the
    /// guard.
    runtime: tokio::runtime::Handle,
}

enum Held {
    /// Before the server shares the database.
    Owned(Box<ObjectDatabase>),
    /// Once it does.
    Shared(Arc<RwLock<ObjectDatabase>>),
}

impl StartedOutputs {
    /// Start `db`'s outputs and hold it until the start completes.
    pub(super) fn start(mut db: ObjectDatabase) -> Self {
        db.start_outputs();
        Self {
            held: Some(Held::Owned(Box::new(db))),
            runtime: tokio::runtime::Handle::current(),
        }
    }

    /// The database, while the server doesn't share it yet.
    pub(super) fn db(&mut self) -> &mut ObjectDatabase {
        match &mut self.held {
            Some(Held::Owned(db)) => db,
            _ => unreachable!("the database is owned until it is shared"),
        }
    }

    /// Share the database, still releasing its outputs if the start doesn't
    /// complete.
    pub(super) fn share(&mut self) -> Arc<RwLock<ObjectDatabase>> {
        let Some(Held::Owned(db)) = self.held.take() else {
            unreachable!("the database is shared once");
        };
        let db = Arc::new(RwLock::new(*db));
        self.held = Some(Held::Shared(Arc::clone(&db)));
        db
    }

    /// The start completed: nothing to release.
    pub(super) fn complete(mut self) {
        self.held = None;
    }
}

impl Drop for StartedOutputs {
    fn drop(&mut self) {
        match self.held.take() {
            None => {}
            Some(Held::Owned(mut db)) => db.release_outputs(),
            Some(Held::Shared(db)) => {
                // A free guard is taken here; otherwise a task waits for it,
                // so neither a held lock nor a blocking wait is ever met in
                // a drop.
                if let Ok(mut guard) = Arc::clone(&db).try_write_owned() {
                    guard.release_outputs();
                    drop(guard);
                    drop(super::drop_database_off_runtime(db));
                    return;
                }
                self.runtime.spawn(async move {
                    db.write().await.release_outputs();
                    drop(super::drop_database_off_runtime(db));
                });
            }
        }
    }
}

#[cfg(test)]
#[path = "start_outputs_tests.rs"]
mod tests;
