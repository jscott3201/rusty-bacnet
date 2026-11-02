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
//!   A runtime that cannot advance parks on the real clock, not in a busy
//!   loop, and the hold's end unparks it, so it does not sleep out that
//!   park either.
//! - Once the node's future is done or dropped, wake-ups start no hold, so
//!   a stray waker cannot keep the test's runtime from shutting down.
//!
//! The transport's own waits on the dedicated thread (its start, and the
//! node's end at `stop`) take a [`Hold`] of their own, so the test's clock
//! does not run ahead while the test's runtime waits on them.
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

    /// Hold this runtime's clock until the guard drops: for a wait on the
    /// dedicated thread that the gate does not see.
    pub(super) fn hold(&self) -> Hold {
        Hold(start_hold(&self.0))
    }

    /// `future`, polled on this runtime's time.
    pub(super) fn drive<F: Future>(&self, future: F) -> OnTestTime<F> {
        let gate = Arc::new(Gate {
            runtime: Handle::clone(&self.0),
            state: Mutex::new(GateState::default()),
        });
        // Busy from the start: the task has its first poll to come.
        gate.lock().hold = Some(start_hold(&gate.runtime));
        OnTestTime {
            future: Box::pin(future),
            gate,
            waker: None,
        }
    }
}

/// Holds the test's runtime's clock while it lives ([`TestTime::hold`]).
pub(super) struct Hold(#[allow(dead_code, reason = "held for its drop")] Sender<()>);

/// Start a blocking task on `runtime` that runs until the returned sender
/// drops. The spawn itself stops the clock's auto-advance, so it is held
/// from this call on.
fn start_hold(runtime: &Handle) -> Sender<()> {
    let (release, released) = channel::<()>();
    drop(runtime.spawn_blocking(move || {
        // Returns when the sender drops.
        let _ = released.recv();
    }));
    release
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

    fn wake(&self) {
        let mut state = self.lock();
        if state.closed {
            return;
        }
        state.woken = true;
        if state.hold.is_none() {
            state.hold = Some(start_hold(&self.runtime));
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

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::task::Waker;

    use super::*;

    impl Gate {
        fn holding(&self) -> bool {
            self.lock().hold.is_some()
        }
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
    }

    /// Poll `future` once with a waker that does nothing.
    fn poll<F: Future>(future: &mut OnTestTime<F>) -> Poll<F::Output> {
        Pin::new(future).poll(&mut Context::from_waker(Waker::noop()))
    }

    /// A poll that wakes itself keeps the hold, since another poll is to
    /// come; a quiet `Pending` releases it, and so does `Ready`.
    #[test]
    fn the_hold_follows_wake_ups_during_a_poll() {
        let runtime = runtime();
        let polls = Rc::new(Cell::new(0));
        let counted = Rc::clone(&polls);
        let mut future =
            TestTime::new(runtime.handle().clone()).drive(std::future::poll_fn(move |cx| {
                counted.set(counted.get() + 1);
                match counted.get() {
                    1 => {
                        cx.waker().wake_by_ref();
                        Poll::Pending
                    }
                    2 => Poll::Pending,
                    _ => Poll::Ready(()),
                }
            }));
        let gate = Arc::clone(&future.gate);
        assert!(gate.holding(), "busy before the first poll");
        assert!(poll(&mut future).is_pending());
        assert!(gate.holding(), "woken during the poll: held");
        assert!(poll(&mut future).is_pending());
        assert!(!gate.holding(), "a quiet Pending releases");
        // A wake-up while idle holds again, and Ready releases.
        gate.wake();
        assert!(gate.holding());
        assert!(poll(&mut future).is_ready());
        assert!(!gate.holding(), "Ready leaves no hold");
        assert_eq!(polls.get(), 3);
    }

    /// Dropping the future releases a hold it had, and a waker that
    /// outlives it starts no new one.
    #[test]
    fn a_dropped_future_releases_and_later_wake_ups_hold_nothing() {
        let runtime = runtime();
        let leaked: Rc<Cell<Option<Waker>>> = Rc::new(Cell::new(None));
        let slot = Rc::clone(&leaked);
        let mut future = TestTime::new(runtime.handle().clone()).drive(std::future::poll_fn(
            move |cx: &mut Context<'_>| {
                slot.set(Some(cx.waker().clone()));
                cx.waker().wake_by_ref();
                Poll::<()>::Pending
            },
        ));
        let gate = Arc::clone(&future.gate);
        assert!(poll(&mut future).is_pending());
        assert!(gate.holding());
        drop(future);
        assert!(!gate.holding(), "closing releases the hold");
        leaked.take().expect("the future kept its waker").wake();
        assert!(!gate.holding(), "a wake-up after close holds nothing");
    }
}
