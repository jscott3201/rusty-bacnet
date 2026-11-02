//! The save writer of a hosted build: a thread per object while it saves
//! (#1270), ended once the object goes quiet (#1956).
//!
//! See the [module documentation](super).

use alloc::collections::VecDeque;
use core::time::Duration;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;

use bacnet_types::error::Error;

use super::{Event, SaveTicket, SaveWait};
use crate::prelude::*;

/// How long a writer's thread waits for another save before it ends. The
/// next save starts a thread again, so a burst of saves shares one thread
/// and an object that saves rarely holds none between saves.
#[cfg(not(test))]
const IDLE_EXIT: Duration = Duration::from_secs(30);
/// None in tests: each thread ends as soon as its queue is empty, so every
/// test runs the path that starts a thread again.
#[cfg(test)]
const IDLE_EXIT: Duration = Duration::ZERO;

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

type SaveFn<S> = dyn Fn(&S) -> Result<(), Error> + Send + Sync;
type DoneFn<S> = dyn Fn(&S, &Result<(), Error>) + Send + Sync;

/// Runs one object's storage calls on a thread of its own, one at a time, in
/// the order they were queued. See the [module documentation](super).
///
/// A save starts the thread when none is running. The thread ends once its
/// queue has stayed empty for [`IDLE_EXIT`], or when the writer is dropped.
/// Dropping the writer waits for the saves already queued, so state saved
/// just before an object goes away is in storage when a new object loads
/// it.
pub(crate) struct SaveWriter<S: Send + 'static> {
    shared: Arc<Shared<S>>,
    /// The last thread started, running or ended. Asserted unwind safe, as
    /// std's `JoinHandle` is not (#1428). That is sound: only `enqueue`,
    /// which joins and replaces it, and the drop, which joins it, touch the
    /// handle, both through `&mut self`, and a panic leaves it either stored
    /// or not.
    thread: Option<AssertUnwindSafe<JoinHandle<()>>>,
    name: String,
    /// The owner is going away; see `closing`.
    closing: bool,
}

struct Shared<S> {
    queue: Mutex<Queue<S>>,
    /// The thread has a job to run, or the writer is closing.
    work: Condvar,
    /// A thread ended, for a test that waits for that.
    #[cfg(test)]
    ended: Condvar,
    /// How long the thread waits for a job before it ends.
    idle_exit: Duration,
    // A boxed closure is not `RefUnwindSafe`, so these would take that trait
    // and `UnwindSafe` from the objects that own a writer (#1428). Asserting
    // them is sound: `run_job` is their only caller and catches a panic from
    // each call, on the writer's thread or on the caller's when `enqueue`
    // could not start one. No panic from them reaches the caller's frames,
    // and one that panics fails only that save.
    save: AssertUnwindSafe<Box<SaveFn<S>>>,
    done: AssertUnwindSafe<Box<DoneFn<S>>>,
}

struct Queue<S> {
    jobs: VecDeque<Job<S>>,
    running: bool,
    closed: bool,
    /// A thread runs the queue. Only the thread clears it, and only with
    /// the queue empty, under this lock; only `enqueue` sets it, under the
    /// same lock, before it starts a thread. So while it is clear the queue
    /// is empty and no thread will touch it again.
    alive: bool,
    /// Threads started so far, for tests.
    #[cfg(test)]
    started: u64,
    /// Waits [`SaveWriter::idle`] handed out, set once the queue next
    /// empties with no job running.
    idle_waits: Vec<Arc<Event>>,
}

struct Job<S> {
    snapshot: S,
    ticket: Option<SaveTicket>,
}

impl<S: Send + 'static> SaveWriter<S> {
    /// A writer whose thread, named `name`, saves each snapshot with `save`
    /// and then reports the result to `done`.
    pub(crate) fn new(
        name: String,
        save: impl Fn(&S) -> Result<(), Error> + Send + Sync + 'static,
        done: impl Fn(&S, &Result<(), Error>) + Send + Sync + 'static,
    ) -> Self {
        Self::with_idle_exit(name, IDLE_EXIT, save, done)
    }

    /// [`new`](Self::new), with a thread that ends after `idle_exit` with
    /// nothing to save.
    fn with_idle_exit(
        name: String,
        idle_exit: Duration,
        save: impl Fn(&S) -> Result<(), Error> + Send + Sync + 'static,
        done: impl Fn(&S, &Result<(), Error>) + Send + Sync + 'static,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                queue: Mutex::new(Queue {
                    jobs: VecDeque::new(),
                    running: false,
                    closed: false,
                    alive: false,
                    #[cfg(test)]
                    started: 0,
                    idle_waits: Vec::new(),
                }),
                work: Condvar::new(),
                #[cfg(test)]
                ended: Condvar::new(),
                idle_exit,
                save: AssertUnwindSafe(Box::new(save)),
                done: AssertUnwindSafe(Box::new(done)),
            }),
            thread: None,
            name,
            closing: false,
        }
    }

    /// Queue a save whose outcome the caller wants.
    pub(crate) fn submit(&mut self, snapshot: S) -> SaveTicket {
        let ticket = SaveTicket::default();
        self.enqueue(Job {
            snapshot,
            ticket: Some(ticket.clone()),
        });
        ticket
    }

    /// Queue a save nobody waits for. A save of this kind still queued and
    /// not started is replaced, so only the latest state is saved.
    pub(crate) fn submit_coalescing(&mut self, snapshot: S) {
        {
            let mut queue = lock(&self.shared.queue);
            if let Some(last) = queue.jobs.back_mut().filter(|job| job.ticket.is_none()) {
                last.snapshot = snapshot;
                return;
            }
        }
        self.enqueue(Job {
            snapshot,
            ticket: None,
        });
    }

    /// A wait that ends once every save queued so far has run.
    pub(crate) fn idle(&self) -> SaveWait {
        let event = Arc::new(Event::default());
        let mut queue = lock(&self.shared.queue);
        if queue.running || !queue.jobs.is_empty() {
            queue.idle_waits.push(Arc::clone(&event));
        } else {
            drop(queue);
            event.set();
        }
        SaveWait::new(event)
    }

    /// Block until every queued save has run.
    pub(crate) fn wait_idle(&self) {
        self.idle().block();
    }

    /// The object is going away while storage may hold a staged state it
    /// never served: save `served`, the state it does serve, and wait for
    /// the save. The writer's own drop, which follows, would wait for it
    /// anyway, so this blocks no longer than that drop does. A save that
    /// fails is logged twice, by the done hook and here: storage then keeps
    /// the staged state, and the next start serves it.
    pub(crate) fn put_back(&mut self, served: S) {
        if let Err(error) = self.submit(served).take_outcome() {
            log_warn!(
                writer = %self.name,
                %error,
                "Could not put storage back to the served state as the object closed; \
                 the next start serves the state staged for a request that never finished"
            );
        }
    }

    /// The owner is going away, and its drop will wait for every save it
    /// queues. A save queued from now on while no thread is running runs
    /// here, on the caller's thread, instead of on a thread started only to
    /// be joined (#2233). With no thread running the queue is empty, so the
    /// order holds; with one running, saves still go to it.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn closing(&mut self) {
        self.closing = true;
    }

    fn enqueue(&mut self, job: Job<S>) {
        {
            let mut queue = lock(&self.shared.queue);
            if queue.alive {
                queue.jobs.push_back(job);
                drop(queue);
                self.shared.work.notify_one();
                return;
            }
        }
        // No thread runs the queue, so it is empty: none has started, or the
        // last one ended idle.
        if self.closing {
            run_job(&self.shared, job);
            return;
        }
        // A thread that ended has nothing left but to return; join it, so a
        // writer never has two.
        if let Some(AssertUnwindSafe(ended)) = self.thread.take() {
            let _ = ended.join();
        }
        {
            let mut queue = lock(&self.shared.queue);
            queue.alive = true;
            #[cfg(test)]
            {
                queue.started += 1;
            }
            queue.jobs.push_back(job);
        }
        let shared = Arc::clone(&self.shared);
        match std::thread::Builder::new()
            .name(self.name.clone())
            .spawn(move || run(&shared))
        {
            Ok(handle) => self.thread = Some(AssertUnwindSafe(handle)),
            Err(error) => {
                // Without a thread the save runs here, as it did before
                // saves left the caller's thread. The queue holds only it.
                log_warn!(%error, writer = %self.name, "Could not start a save thread");
                let job = {
                    let mut queue = lock(&self.shared.queue);
                    queue.alive = false;
                    queue.jobs.pop_front()
                };
                if let Some(job) = job {
                    run_job(&self.shared, job);
                }
            }
        }
    }
}

#[cfg(test)]
impl<S: Send + 'static> SaveWriter<S> {
    /// [`new`](Self::new) with a thread that waits `idle_exit` for another
    /// save before it ends.
    pub(crate) fn idling_for(
        idle_exit: Duration,
        save: impl Fn(&S) -> Result<(), Error> + Send + Sync + 'static,
    ) -> Self {
        Self::with_idle_exit("test-writer".into(), idle_exit, save, |_, _| {})
    }

    /// Whether a thread runs the queue now.
    pub(crate) fn thread_running(&self) -> bool {
        lock(&self.shared.queue).alive
    }

    /// How many threads the writer has started.
    pub(crate) fn threads_started(&self) -> u64 {
        lock(&self.shared.queue).started
    }

    /// Wait until no thread runs the queue, for at most ten seconds, and
    /// say whether none does. A thread that waits out its idle period ends
    /// well inside that.
    pub(crate) fn thread_ended(&self) -> bool {
        let queue = lock(&self.shared.queue);
        let (queue, _) = self
            .shared
            .ended
            .wait_timeout_while(queue, Duration::from_secs(10), |queue| queue.alive)
            .unwrap_or_else(PoisonError::into_inner);
        !queue.alive
    }
}

impl<S: Send + 'static> Drop for SaveWriter<S> {
    fn drop(&mut self) {
        lock(&self.shared.queue).closed = true;
        self.shared.work.notify_one();
        if let Some(AssertUnwindSafe(thread)) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The thread: run jobs until the queue has stayed empty for the idle
/// period, or the writer closes with nothing left to run.
fn run<S>(shared: &Shared<S>) {
    loop {
        let job = {
            let mut queue = lock(&shared.queue);
            loop {
                if let Some(job) = queue.jobs.pop_front() {
                    queue.running = true;
                    break job;
                }
                if queue.closed {
                    end(shared, &mut queue);
                    return;
                }
                let (waited, timeout) = shared
                    .work
                    .wait_timeout_while(queue, shared.idle_exit, |queue| {
                        queue.jobs.is_empty() && !queue.closed
                    })
                    .unwrap_or_else(PoisonError::into_inner);
                queue = waited;
                // Ending is decided under the lock that `enqueue` checks, so a
                // save queued before this sees the thread and wakes it, and
                // one queued after sees none and starts another.
                if timeout.timed_out() && queue.jobs.is_empty() && !queue.closed {
                    end(shared, &mut queue);
                    return;
                }
            }
        };
        run_job(shared, job);
        let idle = {
            let mut queue = lock(&shared.queue);
            queue.running = false;
            if queue.jobs.is_empty() {
                core::mem::take(&mut queue.idle_waits)
            } else {
                Vec::new()
            }
        };
        for event in idle {
            event.set();
        }
    }
}

/// The thread ends with the queue empty: say so, under the queue's lock.
fn end<S>(shared: &Shared<S>, queue: &mut Queue<S>) {
    queue.alive = false;
    #[cfg(test)]
    shared.ended.notify_all();
    #[cfg(not(test))]
    let _ = shared;
}

/// Make one storage call. A storage implementation that panics fails the save
/// instead of ending the thread, so every ticket still completes. The
/// snapshot is dropped before the ticket completes, so a caller that shares
/// it through an `Arc` holds the only reference once the save has run.
fn run_job<S>(shared: &Shared<S>, job: Job<S>) {
    let Job { snapshot, ticket } = job;
    let result = catch_unwind(AssertUnwindSafe(|| (shared.save.0)(&snapshot)))
        .unwrap_or_else(|_| Err(Error::Encoding("persistence panicked during a save".into())));
    let _ = catch_unwind(AssertUnwindSafe(|| (shared.done.0)(&snapshot, &result)));
    drop(snapshot);
    if let Some(ticket) = ticket {
        ticket.complete(result);
    }
}
