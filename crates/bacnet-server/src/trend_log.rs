//! Automatic trend logging with database-owned scheduling.

use crate::caller_waits::{HeldDatabase, for_caller, on_runtime};
use bacnet_objects::database::ObjectDatabase;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// Select, acquire and append under one exclusive database guard.
/// The returned wait includes the database's bounded configuration reconciliation
/// and failure retry policy. No polling ownership escapes the database.
///
/// # Cancellation
///
/// It never waits for the database in its caller's task (#2781). With the
/// database busy, a task on the runtime it is polled on waits for the
/// guard, keeping the call's place in the queue, polls the logs under it
/// and hands back the wait, so a call left unpolled holds up nothing else
/// that uses the database, a running server's requests included. A call
/// dropped part way is as
/// [`write_local`](crate::server::BACnetServer::write_local)'s Cancellation
/// section describes for a local call (#2772): dropped before that task
/// holds the guard, it polls nothing; dropped after, it may still have
/// polled. Outside a runtime it waits in its caller's task.
pub async fn poll_trend_logs(db: &Arc<RwLock<ObjectDatabase>>) -> Duration {
    let take = HeldDatabase::new(Arc::clone(db)).write_owned();
    match for_caller(on_runtime, take, |mut db| db.poll_trend_logs()).await {
        Some(delay) => delay,
        // Its runtime dropped the task before it took the guard: poll here.
        None => db.write().await.poll_trend_logs(),
    }
}

/// The server's own trend task. The runtime polls it, so it waits for the
/// database itself, with no task of its own per busy poll.
pub(crate) async fn run(db: Arc<RwLock<ObjectDatabase>>) {
    loop {
        let delay = db.write().await.poll_trend_logs();
        tokio::time::sleep(delay).await;
    }
}

#[cfg(test)]
#[path = "trend_log_clock_tests.rs"]
mod clock_tests;
