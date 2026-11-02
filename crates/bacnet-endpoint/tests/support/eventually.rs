//! Wait for a condition on paused Tokio time (#2293).
//!
//! A loop that only yields keeps the runtime busy, so paused time never
//! moves and a `timeout` or `Instant` deadline around it never fires: a
//! condition that stays false hangs the test. [`eventually`] sleeps 1 ms of
//! Tokio time between checks instead, so the deadline fires and the test
//! fails with its message. The unit tests include this file by path.

use std::time::Duration;

/// Waits until `ready` returns true, checking again after each 1 ms of
/// Tokio time. Panics with `what` once `limit` has passed without it.
pub async fn eventually(limit: Duration, what: &str, mut ready: impl AsyncFnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + limit;
    loop {
        if ready().await {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "not within {limit:?}: {what}"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}
