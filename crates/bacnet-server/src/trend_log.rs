//! Automatic trend logging with database-owned scheduling.

use bacnet_objects::database::ObjectDatabase;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// Select, acquire and append under one exclusive database guard.
/// The returned wait includes the database's bounded configuration reconciliation
/// and failure retry policy. No polling ownership escapes the database.
pub async fn poll_trend_logs(db: &Arc<RwLock<ObjectDatabase>>) -> Duration {
    db.write().await.poll_trend_logs()
}

pub(crate) async fn run(db: Arc<RwLock<ObjectDatabase>>) {
    loop {
        let delay = poll_trend_logs(&db).await;
        tokio::time::sleep(delay).await;
    }
}

#[cfg(test)]
#[path = "trend_log_clock_tests.rs"]
mod clock_tests;
