//! The save writer of a hosted build: one thread per object (#1270).
//!
//! See the [module documentation](super).

use alloc::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;

use bacnet_types::error::Error;

use super::{Event, SaveTicket, SaveWait};
use crate::prelude::*;

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

type SaveFn<S> = dyn Fn(&S) -> Result<(), Error> + Send + Sync;
type DoneFn<S> = dyn Fn(&S, &Result<(), Error>) + Send + Sync;

/// Runs one object's storage calls on a thread of its own, one at a time, in
/// the order they were queued. See the [module documentation](super).
///
/// The thread starts with the first save and ends when the writer is
/// dropped. Dropping the writer waits for the saves already queued, so state
/// saved just before an object goes away is in storage when a new object
/// loads it.
pub(crate) struct SaveWriter<S: Send + 'static> {
    shared: Arc<Shared<S>>,
    /// Asserted unwind safe, as std's `JoinHandle` is not (#1428). That is
    /// sound: only `enqueue`, which stores it, and the drop, which joins it,
    /// touch the handle, both through `&mut self`, and a panic leaves it
    /// either stored or not.
    thread: Option<AssertUnwindSafe<JoinHandle<()>>>,
    name: String,
}

struct Shared<S> {
    queue: Mutex<Queue<S>>,
    /// The thread has a job to run, or the writer is closing.
    work: Condvar,
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
        Self {
            shared: Arc::new(Shared {
                queue: Mutex::new(Queue {
                    jobs: VecDeque::new(),
                    running: false,
                    closed: false,
                    idle_waits: Vec::new(),
                }),
                work: Condvar::new(),
                save: AssertUnwindSafe(Box::new(save)),
                done: AssertUnwindSafe(Box::new(done)),
            }),
            thread: None,
            name,
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

    fn enqueue(&mut self, job: Job<S>) {
        if self.thread.is_none() {
            let shared = Arc::clone(&self.shared);
            match std::thread::Builder::new()
                .name(self.name.clone())
                .spawn(move || run(&shared))
            {
                Ok(handle) => self.thread = Some(AssertUnwindSafe(handle)),
                Err(error) => {
                    // Without a thread the save runs here, as it did before
                    // saves left the caller's thread.
                    log_warn!(%error, writer = %self.name, "Could not start a save thread");
                    run_job(&self.shared, job);
                    return;
                }
            }
        }
        lock(&self.shared.queue).jobs.push_back(job);
        self.shared.work.notify_one();
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
                    return;
                }
                queue = shared
                    .work
                    .wait(queue)
                    .unwrap_or_else(PoisonError::into_inner);
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
