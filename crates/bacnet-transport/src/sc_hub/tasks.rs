//! Hub-owned tasks. Only the supervisor polls joins; weak spawners cannot
//! keep the task set alive through the futures it owns.

use std::future::{poll_fn, Future};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::task::{Poll, Waker};
use tokio::sync::watch;
use tokio::task::JoinSet;

use super::graceful::{GracefulCtx, ScHubGracefulTimeouts, ScHubShutdownOutcome};

#[derive(Clone)]
pub(super) struct Tasks {
    state: Arc<Mutex<State>>,
    shutdown: watch::Sender<bool>,
    graceful: watch::Sender<bool>,
    graceful_failed: Arc<AtomicBool>,
    graceful_timeouts: ScHubGracefulTimeouts,
    outcome: Arc<Mutex<Option<ScHubShutdownOutcome>>>,
    #[cfg(test)]
    pub(super) probe_scans: Arc<std::sync::atomic::AtomicU64>,
    pub(super) timing: super::timing::HubTiming,
    pub(super) broadcast: Arc<super::broadcast_rate::HubBudget>,
}

struct State {
    set: JoinSet<()>,
    sealed: bool,
    graceful_kind: bool,
    empty_waiter: Option<Waker>,
}

#[derive(Clone)]
pub(super) struct Spawner(Weak<Mutex<State>>);

impl Tasks {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                set: JoinSet::new(),
                sealed: false,
                graceful_kind: false,
                empty_waiter: None,
            })),
            shutdown: watch::channel(false).0,
            graceful: watch::channel(false).0,
            graceful_failed: Arc::new(AtomicBool::new(false)),
            graceful_timeouts: ScHubGracefulTimeouts::default(),
            outcome: Arc::new(Mutex::new(None)),
            #[cfg(test)]
            probe_scans: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            timing: super::timing::HubTiming::new(super::ScHubProbePolicy::default()),
            broadcast: Arc::new(super::broadcast_rate::HubBudget::default()),
        }
    }

    pub fn with_probe_policy(mut self, policy: super::ScHubProbePolicy) -> Self {
        self.timing = super::timing::HubTiming::new(policy);
        self
    }

    pub fn with_broadcast_budget(mut self, budget: Arc<super::broadcast_rate::HubBudget>) -> Self {
        self.broadcast = budget;
        self
    }

    pub fn with_graceful_timeouts(mut self, timeouts: ScHubGracefulTimeouts) -> Self {
        self.graceful_timeouts = timeouts;
        self
    }

    pub fn abort_on_exit(&self) -> AbortOnExit {
        AbortOnExit(self.clone())
    }

    pub fn spawner(&self) -> Spawner {
        Spawner(Arc::downgrade(&self.state))
    }

    pub fn subscribe(&self) -> watch::Receiver<bool> {
        self.shutdown.subscribe()
    }

    pub fn subscribe_graceful(&self) -> watch::Receiver<bool> {
        self.graceful.subscribe()
    }

    pub fn request_shutdown(&self) {
        self.state.lock().unwrap().sealed = true;
        self.shutdown.send_replace(true);
    }

    /// Seal for graceful shutdown (first seal wins the drain kind).
    ///
    /// Sets the graceful kind only when nothing sealed before, so a prior
    /// forceful [`Self::request_shutdown`] keeps the run forced. Always
    /// seals admission via the shutdown channel; connection tasks observe
    /// the separate graceful channel to start the Disconnect exchange.
    pub fn request_graceful(&self) {
        let first = {
            let mut state = self.state.lock().unwrap();
            if state.sealed {
                false
            } else {
                state.sealed = true;
                state.graceful_kind = true;
                true
            }
        };
        self.shutdown.send_replace(true);
        if first {
            self.graceful.send_replace(true);
        }
    }

    /// Drain kind chosen by the first seal: graceful only when the first
    /// seal was [`Self::request_graceful`].
    pub fn graceful_kind(&self) -> bool {
        self.state.lock().unwrap().graceful_kind
    }

    pub fn graceful_ctx(&self) -> GracefulCtx {
        GracefulCtx::new(
            self.graceful.clone(),
            self.graceful_failed.clone(),
            self.graceful_timeouts,
        )
    }

    pub fn graceful_failed(&self) -> bool {
        self.graceful_failed.load(Ordering::Acquire)
    }

    pub fn set_outcome(&self, outcome: ScHubShutdownOutcome) {
        *self.outcome.lock().unwrap() = Some(outcome);
    }

    pub fn get_outcome(&self) -> Option<ScHubShutdownOutcome> {
        *self.outcome.lock().unwrap()
    }

    /// Reap completed tasks while accepting connections. Empty sets must sleep
    /// until a spawn, rather than making the supervisor spin.
    pub async fn reap(&self) {
        poll_fn(|cx| {
            let mut state = self.state.lock().unwrap();
            match state.set.poll_join_next(cx) {
                Poll::Ready(None) => {
                    state.empty_waiter = Some(cx.waker().clone());
                    Poll::Pending
                }
                Poll::Ready(Some(_)) => Poll::Ready(()),
                Poll::Pending => Poll::Pending,
            }
        })
        .await;
    }

    pub async fn drain(&self) {
        self.request_shutdown();
        let mut set = {
            let mut state = self.state.lock().unwrap();
            state.empty_waiter = None;
            std::mem::take(&mut state.set)
        };
        // No task can be added after sealing. Joining outside the lock lets
        // cancelled workers drop their captures without holding shared state.
        set.shutdown().await;
    }

    /// Wait for supervised tasks to finish naturally within `overall`.
    ///
    /// Returns true when every task completed without abort. On expiry,
    /// falls back to forceful abort (like [`Self::drain`]) and returns
    /// false so the caller reports forced cleanup, never protocol success
    /// from an abort.
    pub async fn drain_gracefully(&self, overall: std::time::Duration) -> bool {
        let mut set = {
            let mut state = self.state.lock().unwrap();
            state.empty_waiter = None;
            std::mem::take(&mut state.set)
        };
        let completed =
            tokio::time::timeout(overall, async { while set.join_next().await.is_some() {} })
                .await
                .is_ok();
        if !completed {
            set.shutdown().await;
        }
        completed
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.state.lock().unwrap().set.len()
    }
}

impl Spawner {
    /// Synchronous registration has no queue capacity wait. Rejected futures
    /// are dropped after releasing the lock, including unpolled admissions.
    pub fn spawn(&self, future: impl Future<Output = ()> + Send + 'static) -> bool {
        let Some(shared) = self.0.upgrade() else {
            return false;
        };
        let mut state = shared.lock().unwrap();
        if state.sealed {
            return false;
        }
        state.set.spawn(future);
        let waiter = state.empty_waiter.take();
        drop(state);
        if let Some(waiter) = waiter {
            waiter.wake();
        }
        true
    }
}

// Exceptional supervisor destruction cannot leave running workers merely
// because ScHub still holds a strong Tasks reference. Normal shutdown drains
// joins explicitly; this destructor only requests cooperative cancellation.
pub(super) struct AbortOnExit(Tasks);

impl Drop for AbortOnExit {
    fn drop(&mut self) {
        self.0.request_shutdown();
        self.0.state.lock().unwrap().set.abort_all();
    }
}
