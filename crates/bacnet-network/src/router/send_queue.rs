//! One port's bounded backlog. All four priority lanes share one capacity;
//! taking a send releases its slot before the transport starts that send.
//! Higher network priority wins, with arrival order within each lane.
//!
//! Admission, selection and closure share a short synchronous lock. No lock
//! crosses an await, and no queued send is evicted. The sole receiver uses
//! `Notify::notify_one`'s retained permit, so admission or the last sender's
//! drop between checking the queue and awaiting cannot lose a wakeup.

use bacnet_npdu::NetworkPriority;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::Notify;
use tokio::sync::mpsc::error::{TryRecvError, TrySendError};

use bacnet_router_core::router::Backlog;

use super::SendRequest;
use super::engine::EngineGuard;

pub(super) const CAPACITY: usize = 256;

struct State {
    lanes: [VecDeque<SendRequest>; 4],
    len: usize,
    capacity: usize,
    senders: usize,
    receiver: bool,
    /// The engine counts the port as congested, so it wants to hear the
    /// backlog as it drains too (#1697).
    watched: bool,
}

struct Shared {
    state: Mutex<State>,
    ready: Notify,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn backlog(&self) -> Backlog {
        let state = self.lock();
        backlog(&state)
    }

    fn set_watched(&self, watched: bool) {
        self.lock().watched = watched;
    }
}

/// The queue's backlog as the engine counts it. A queue holds far fewer
/// than `u32::MAX` sends.
fn backlog(state: &State) -> Backlog {
    let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    Backlog::new(count(state.len), count(state.capacity))
}

pub(super) struct Sender(Arc<Shared>);

pub(super) struct Receiver(Arc<Shared>);

pub(super) fn channel(capacity: usize) -> (Sender, Receiver) {
    assert!(capacity > 0);
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            lanes: std::array::from_fn(|_| VecDeque::new()),
            len: 0,
            capacity,
            senders: 1,
            receiver: true,
            watched: false,
        }),
        ready: Notify::new(),
    });
    (Sender(Arc::clone(&shared)), Receiver(shared))
}

impl Sender {
    /// Admit at the engine's network priority.
    /// Closure wins over fullness, as for the previous bounded channel.
    /// Refused work is dropped; callers need only the refusal kind.
    pub(super) fn try_send(
        &self,
        priority: NetworkPriority,
        request: SendRequest,
    ) -> Result<(), TrySendError<()>> {
        {
            let mut state = self.0.lock();
            if !state.receiver {
                return Err(TrySendError::Closed(()));
            }
            if state.len == state.capacity {
                return Err(TrySendError::Full(()));
            }
            state.lanes[usize::from(priority.to_raw())].push_back(request);
            state.len += 1;
        }
        self.0.ready.notify_one();
        Ok(())
    }

    /// The backlog now, to report to the engine.
    pub(super) fn backlog(&self) -> Backlog {
        self.0.backlog()
    }

    /// Whether the receiver reports the backlog as it drains.
    pub(super) fn set_watched(&self, watched: bool) {
        self.0.set_watched(watched);
    }
}

impl Clone for Sender {
    fn clone(&self) -> Self {
        self.0.lock().senders += 1;
        Self(Arc::clone(&self.0))
    }
}

impl Drop for Sender {
    fn drop(&mut self) {
        let last = {
            let mut state = self.0.lock();
            state.senders -= 1;
            state.senders == 0
        };
        if last {
            self.0.ready.notify_one();
        }
    }
}

impl Receiver {
    pub(super) fn try_recv(&mut self) -> Result<SendRequest, TryRecvError> {
        let mut state = self.0.lock();
        for lane in state.lanes.iter_mut().rev() {
            if let Some(request) = lane.pop_front() {
                state.len -= 1;
                return Ok(request);
            }
        }
        Err(if state.senders == 0 {
            TryRecvError::Disconnected
        } else {
            TryRecvError::Empty
        })
    }

    /// Whether the engine wants to hear the backlog as the queue drains:
    /// a cheap look, before the engine's lock is worth taking (#1697).
    pub(super) fn is_watched(&self) -> bool {
        self.0.lock().watched
    }

    /// The backlog now, when the engine still wants to hear it. It takes
    /// the engine's guard so it can only be read under the engine's lock:
    /// read before it, a drain on another task could refill the queue
    /// between the read and the report, and the engine would hear a stale
    /// low backlog, announcing a full port available. Under the lock, every
    /// admission is reported after this one, so the engine always hears
    /// the latest. Engine, then queue: the order `drain` takes them in.
    pub(super) fn watched_backlog(&self, _engine: &EngineGuard<'_>) -> Option<Backlog> {
        let state = self.0.lock();
        state.watched.then(|| backlog(&state))
    }

    /// Whether to keep reporting the backlog as it drains.
    pub(super) fn set_watched(&self, watched: bool) {
        self.0.set_watched(watched);
    }

    /// Cancellation while waiting consumes no send. Only this receiver can
    /// wait, and its next call checks the queue even after a notification.
    pub(super) async fn recv(&mut self) -> Option<SendRequest> {
        // Like Tokio's bounded receiver, cooperate even if the queue and
        // transport stay ready. Yield before selecting a send, not after
        // removing one that a higher-priority arrival could overtake.
        tokio::task::consume_budget().await;
        loop {
            match self.try_recv() {
                Ok(request) => return Some(request),
                Err(TryRecvError::Disconnected) => return None,
                Err(TryRecvError::Empty) => self.0.ready.notified().await,
            }
        }
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        let discarded = {
            let mut state = self.0.lock();
            state.receiver = false;
            state.len = 0;
            std::mem::take(&mut state.lanes)
        };
        // Release payloads even while senders still own the shared state.
        drop(discarded);
    }
}

#[cfg(test)]
#[path = "send_queue_tests.rs"]
mod tests;
