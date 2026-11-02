//! Test time for the dedicated MS/TP thread (#2284), behind the
//! `test-support` feature: [`MstpThreadConfig::with_test_time`](super::MstpThreadConfig::with_test_time).
//!
//! In [`MstpExecutionMode::DedicatedThread`](super::MstpExecutionMode::DedicatedThread)
//! the node runs on a runtime of its own, so its deadlines (T_no_token, the
//! reply window, poll timeouts) are on the wall clock, and a test whose own
//! thread is starved on a loaded host sees them pass early. With test time,
//! the node still runs on its own thread and runtime, but every Tokio time
//! call it makes reads, and every timer it sets is registered with, the
//! runtime the test named: the node's future is polled with that runtime's
//! handle entered. The core's clock comes with it unchanged, since
//! `runtime_clock::tokio_clock` reads Tokio time wherever it is called.
//!
//! # Waiting for the next deadline
//!
//! The test's runtime is a paused `current_thread` one. Tokio advances its
//! clock to the next timer only when that runtime has nothing to run, so
//! the node's deadlines come no sooner than they would if the node ran on
//! the test's runtime, provided the runtime also waits while the node has
//! work. That is the [`Gate`]'s job. A paused runtime does not auto-advance
//! while a blocking task it spawned is running (`tokio::time::pause` documents
//! this), so the gate keeps one blocking "hold" task running on it for as long
//! as the node is busy:
//!
//! - The node is busy from its spawn until a poll returns `Pending` with no
//!   wake-up during that poll. Then the hold is released.
//! - Every wake-up of the node goes through the gate first. If no hold is
//!   running, it starts one before it wakes the node's task. A wake-up from
//!   the test's runtime (a peer's frame, an answer, a fired timer) so holds
//!   that runtime's clock before the runtime can next go idle.
//! - A wake-up during a poll marks the gate, so the poll that returns
//!   `Pending` keeps the hold: the node is about to be polled again. Each
//!   wake-up and release takes the gate's lock, so none is lost between the
//!   poll's check and the release.
//! - The hold's thread ends some time after its release. Until then the
//!   clock stays put, which can only make a deadline later, never earlier.
//!   A runtime that cannot advance parks on the real clock, so a late
//!   release costs real time, not a busy loop.
//! - Once the node's future is done or dropped, wake-ups start no hold, so
//!   a stray waker cannot keep the test's runtime from shutting down.
//!
//! Anything else the node spawns on Tokio while polled (blocking serial
//! drains, say) goes to the test's runtime too; the loopback ports tests
//! use spawn nothing.

use std::future::Future;
use std::pin::Pin;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll, Wake, Waker};

use tokio::runtime::Handle;

/// The runtime whose time a dedicated node runs on; see the module docs.
///
/// Two are equal when they name the same runtime handle (one clone of it),
/// so [`MstpThreadConfig`](super::MstpThreadConfig) keeps its `Eq`.
#[derive(Debug, Clone)]
pub(super) struct TestTime(Arc<Handle>);

impl PartialEq for TestTime {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for TestTime {}

impl TestTime {
    pub(super) fn new(runtime: Handle) -> Self {
        Self(Arc::new(runtime))
    }

    /// `future`, polled on this runtime's time.
    pub(super) fn drive<F: Future>(&self, future: F) -> OnTestTime<F> {
        let gate = Arc::new(Gate {
            runtime: Handle::clone(&self.0),
            state: Mutex::new(GateState::default()),
        });
        // Busy from the start: the task has its first poll to come.
        gate.lock().hold = Some(gate.start_hold());
        OnTestTime {
            future: Box::pin(future),
            gate,
            waker: None,
        }
    }
}

/// A future polled with the test's runtime entered, behind a [`Gate`].
pub(super) struct OnTestTime<F> {
    future: Pin<Box<F>>,
    gate: Arc<Gate>,
    /// The task's waker and its gated wrapper, rebuilt when the task's
    /// changes.
    waker: Option<(Waker, Waker)>,
}

impl<F: Future> Future for OnTestTime<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<F::Output> {
        let this = self.get_mut();
        let waker = match &this.waker {
            Some((task, gated)) if task.will_wake(cx.waker()) => gated.clone(),
            _ => {
                let gated = Waker::from(Arc::new(GatedWaker {
                    gate: Arc::clone(&this.gate),
                    task: cx.waker().clone(),
                }));
                this.waker = Some((cx.waker().clone(), gated.clone()));
                gated
            }
        };
        this.gate.lock().woken = false;
        let polled = {
            let _entered = this.gate.runtime.enter();
            this.future.as_mut().poll(&mut Context::from_waker(&waker))
        };
        match polled {
            Poll::Ready(output) => {
                this.gate.close();
                Poll::Ready(output)
            }
            Poll::Pending => {
                let released = {
                    let mut state = this.gate.lock();
                    if state.woken { None } else { state.hold.take() }
                };
                drop(released);
                Poll::Pending
            }
        }
    }
}

impl<F> Drop for OnTestTime<F> {
    fn drop(&mut self) {
        self.gate.close();
    }
}

/// Holds the test's runtime's clock while the node is busy; see the module
/// docs.
struct Gate {
    runtime: Handle,
    state: Mutex<GateState>,
}

#[derive(Default)]
struct GateState {
    /// Dropping it ends the running hold task.
    hold: Option<Sender<()>>,
    /// A wake-up came since the current poll began.
    woken: bool,
    /// The future is done or dropped.
    closed: bool,
}

impl Gate {
    fn lock(&self) -> std::sync::MutexGuard<'_, GateState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Start a blocking task on the test's runtime that runs until the
    /// returned sender drops. The spawn itself stops the clock's
    /// auto-advance, so it is held from this call on.
    fn start_hold(&self) -> Sender<()> {
        let (release, released) = channel::<()>();
        drop(self.runtime.spawn_blocking(move || {
            // Returns when the sender drops.
            let _ = released.recv();
        }));
        release
    }

    fn wake(&self) {
        let mut state = self.lock();
        if state.closed {
            return;
        }
        state.woken = true;
        if state.hold.is_none() {
            state.hold = Some(self.start_hold());
        }
    }

    fn close(&self) {
        let released = {
            let mut state = self.lock();
            state.closed = true;
            state.hold.take()
        };
        drop(released);
    }
}

/// The task's waker, which holds the gate before it wakes the task.
struct GatedWaker {
    gate: Arc<Gate>,
    task: Waker,
}

impl Wake for GatedWaker {
    fn wake(self: Arc<Self>) {
        self.gate.wake();
        self.task.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.gate.wake();
        self.task.wake_by_ref();
    }
}
