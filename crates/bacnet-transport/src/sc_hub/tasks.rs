//! Hub-owned tasks. Only the supervisor polls joins; weak spawners cannot
//! keep the task set alive through the futures it owns.

use std::future::{poll_fn, Future};
use std::sync::{Arc, Mutex, Weak};
use std::task::{Poll, Waker};
use tokio::sync::watch;
use tokio::task::JoinSet;

#[derive(Clone)]
pub(super) struct Tasks {
    state: Arc<Mutex<State>>,
    shutdown: watch::Sender<bool>,
}

struct State {
    set: JoinSet<()>,
    sealed: bool,
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
                empty_waiter: None,
            })),
            shutdown: watch::channel(false).0,
        }
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

    pub fn request_shutdown(&self) {
        self.state.lock().unwrap().sealed = true;
        self.shutdown.send_replace(true);
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
