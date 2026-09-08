use std::future::{poll_fn, Future};
use std::sync::Mutex;
use tokio::task::{JoinError, JoinSet};

/// Owns only the top-level inbound request handlers, not independent timers,
/// notification workers or segmented-response workers started by services.
#[derive(Default)]
pub(super) struct RequestTasks(Mutex<State>);

#[derive(Default)]
struct State {
    closed: bool,
    tasks: JoinSet<()>,
}

impl RequestTasks {
    pub(super) fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) {
        let mut state = self.0.lock().unwrap();
        // Admission and registration are one synchronous critical section.
        if !state.closed {
            state.tasks.spawn(task);
        }
    }

    pub(super) fn close(&self) {
        let mut state = self.0.lock().unwrap();
        state.closed = true;
        state.tasks.abort_all();
    }

    pub(super) fn is_empty(&self) -> bool {
        self.0.lock().unwrap().tasks.is_empty()
    }

    /// One join consumer at a time: dispatch while running, stop after dispatch
    /// is joined. No handle or mutex guard is held across an await, so cancelling
    /// stop leaves the remaining joins in this server-owned set.
    pub(super) async fn join_next(&self) -> Option<Result<(), JoinError>> {
        poll_fn(|cx| self.0.lock().unwrap().tasks.poll_join_next(cx)).await
    }

    pub(super) fn observe(result: Option<Result<(), JoinError>>) {
        if let Some(Err(error)) = result {
            if !error.is_cancelled() {
                tracing::warn!(%error, "Inbound request handler failed");
            }
        }
    }
}
