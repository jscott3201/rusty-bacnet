//! Saving object state off the object database lock (#1270).
//!
//! The Audit Log, the Notification Forwarder, the Notification Class and the
//! Access Rights object keep state in storage the application provides
//! ([`AuditLogPersistence`], [`NotificationForwarderPersistence`],
//! [`NotificationClassPersistence`] and [`AccessRightsPersistence`]).
//! A storage call can be slow, since the file backends write, synchronize and
//! rename, so none of these objects makes one while its caller holds the
//! database's write guard. They all follow the pattern this module provides.
//!
//! - Under the guard, the object copies the state to keep and queues the copy
//!   on its save writer. The writer's own thread makes the storage calls, one
//!   at a time and in the order they were queued.
//! - A save whose outcome decides a request is *staged*. The object sets the
//!   state the request would leave aside and keeps serving the old one. The
//!   server drops the guard and waits for the save's [`SaveWait`] on Tokio's
//!   blocking pool, so a paused test clock does not jump while the writer
//!   thread works; then, holding
//!   the guard again, it runs the request as usual, and the object takes the
//!   saved state, or refuses the request if the save failed. So a forwarder
//!   list write that cannot be saved is refused and leaves the old list, and an
//!   Audit notification is stored, and a confirmed one acknowledged, only once
//!   its commit is durable. An application's Audit Log purge (#1238) stages
//!   the same way, though no property is written.
//! - A save no request waits for, such as a forwarder's lapse and minute
//!   saves, coalesces: a queued save the thread has not started is replaced by
//!   the newer one, so a burst costs one save of the latest state.
//!
//! # Storage leads the served state
//!
//! A staged state reaches storage before anyone can read it from the object.
//! The storage call returns on the writer's thread, and the object serves the
//! new state only when the request that staged it holds the guard again and
//! takes it (or a later stage, or the object's operation task, settles it).
//! Peers never see that gap: the request answers, and a confirmed Audit
//! notification is acknowledged, only after the object serves the saved
//! state. Code that watches storage directly does see it. A storage call
//! that has returned, such as an in-memory [`AuditLogPersistence`] whose
//! `commit` has stored a record, does not mean a read or an AuditLogQuery
//! finds the record yet; wait on the served state, such as the log's
//! Record_Count, instead.
//!
//! A request that makes several writes to one object, as a
//! WritePropertyMultiple can, stages them as one save of the state they
//! leave together. Each write takes its own step of it as the request makes
//! it, so the object serves the state the last write leaves only once every
//! write has been made, and a request that stops part way keeps the steps it
//! took while storage goes back to that served state (#1423). Under a
//! mutation authorizer, the server asks it about such writes before staging
//! them, and stages only the ones it allows (#1321).
//!
//! Some paths still wait for a save while the guard is held. They get the
//! same outcome through the same writer; the object just queues the save and
//! waits for it where it is:
//!
//! - a write nobody staged, such as application code writing through the
//!   database;
//! - an in-place change to an Audit Log, such as `add_record`, which first
//!   lets a staged commit land, waiting for it if it is still running.
//!
//! A write nobody staged takes the place of a staged write it doesn't match
//! only when the object will make it: a write the object refuses leaves the
//! staged write alone (#1424).
//!
//! Dropping an object waits for the saves it has queued. The server's
//! DeleteObject therefore drops a removed object on a blocking thread after
//! releasing the guard, and a server dropped without `stop()` in async code
//! drops its database there too (#1409); application code that removes an
//! object, or drops the last handle on the database, should do the same.
//! `bacnet_server::server::drop_database_off_runtime` lets go of a handle on
//! the server's database that way, as the server's own detached tasks do
//! (#1513).
//! An object dropped with a write still staged for a request that never
//! came back first saves the state it serves, so storage never keeps a
//! state no client was told about, and waits for that save too (#1363).
//! The server's `stop()` settles such writes once it has joined its
//! requests ([`DurableWrites::settle_forgotten_writes`]) and waits for
//! their saves, so a database dropped after a stop has nothing more to
//! save. A server dropped without `stop()` returns before those saves land.
//!
//! The writer's thread is a plain `std` thread with no Tokio runtime, so a
//! storage implementation that needs one brings its own handle. A storage
//! call that panics fails that save. An object starts the thread with its
//! first save and keeps it, parked while idle, until the object is dropped:
//! one thread per object that has saved.
//!
//! # Adding an object
//!
//! Any other object that keeps a written state in storage reuses this module
//! instead of saving under the guard:
//!
//! 1. Own a `staged::StagedSaves` over a `SaveWriter` of a snapshot of the
//!    state to keep. It stages one save of the state a request's writes
//!    leave (`staged::steps` works out each write's step), hands each write
//!    its step or the save's error, and drops a staged write its request
//!    never made, leaving the object to save its served state at once
//!    (`StagedSaves::correct`), or as the object drops. A write nobody
//!    staged saves with `StagedSaves::save_now`, once the object knows it
//!    will make it, and a save nobody waits for coalesces through
//!    `StagedSaves::submit_coalescing`.
//! 2. Implement [`DurableWrites`] for the object, including
//!    `settle_forgotten_writes` (`StagedSaves::drop_forgotten`), and return
//!    it from `BACnetObject::durable_writes_internal`.
//! 3. Add the object type, and the properties it saves, to `may_save` in the
//!    server's `durable_writes` module. The server stages only the writes
//!    listed there; any other write saves in place.
//! 4. Give a file backend a `file::ObjectFile`, which tags the file with
//!    the format and the object identifier, caps its size, and replaces it
//!    through a synchronized temporary file, a rename and `sync_parent_dir`.
//!
//! The Notification Forwarder's `saving` module is the full example: a list
//! write stages, and the operation task's saves coalesce. The Notification
//! Class's is the smallest: a Recipient_List write stages, and nothing else
//! saves. The Access Rights object's stages indexed array writes too, keyed
//! by the index as well as the value.
//!
//! [`AuditLogPersistence`]: crate::audit::AuditLogPersistence
//! [`NotificationForwarderPersistence`]: crate::notification_forwarder::NotificationForwarderPersistence
//! [`NotificationClassPersistence`]: crate::notification_class::NotificationClassPersistence
//! [`AccessRightsPersistence`]: crate::access_control::AccessRightsPersistence

use std::collections::VecDeque;
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::task::{Context, Poll, Waker};
use std::thread::JoinHandle;
use std::time::Instant;

use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

pub(crate) mod file;
pub(crate) mod staged;

/// How long a staged write may wait for its request once its save has run.
/// It counts from the end of the save, so a slow save never uses it up. Past
/// it the request is taken to be gone, and the object drops the staged state
/// at the next stage or operation task call.
#[cfg(not(test))]
pub(crate) const STAGED_WRITE_LIFETIME: std::time::Duration = std::time::Duration::from_secs(10);
/// Short in tests, so one can watch a forgotten staged write go.
#[cfg(test)]
pub(crate) const STAGED_WRITE_LIFETIME: std::time::Duration = std::time::Duration::from_millis(300);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Something that happens once, which any number of threads and tasks can
/// wait for.
#[derive(Default)]
pub(crate) struct Event {
    state: Mutex<EventState>,
    happened: Condvar,
}

#[derive(Default)]
struct EventState {
    done: bool,
    wakers: Vec<Waker>,
}

impl Event {
    /// Mark the event as happened and wake every waiter.
    pub(crate) fn set(&self) {
        let wakers = {
            let mut state = lock(&self.state);
            state.done = true;
            std::mem::take(&mut state.wakers)
        };
        self.happened.notify_all();
        for waker in wakers {
            waker.wake();
        }
    }

    pub(crate) fn is_set(&self) -> bool {
        lock(&self.state).done
    }

    fn wait_blocking(&self) {
        let mut state = lock(&self.state);
        while !state.done {
            state = self
                .happened
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

/// A future that resolves once a save has run, or once a staged write that
/// held an object has gone, whichever it was handed out for.
///
/// It holds no lock and borrows nothing, so it can be awaited after the
/// database guard is dropped.
#[must_use = "a SaveWait does nothing unless awaited"]
#[derive(Clone)]
pub struct SaveWait {
    event: Arc<Event>,
}

impl SaveWait {
    pub(crate) fn new(event: Arc<Event>) -> Self {
        Self { event }
    }

    /// Whether the wait is already over.
    pub fn is_ready(&self) -> bool {
        self.event.is_set()
    }

    /// Block this thread until the wait is over, for a caller that waits
    /// off its async runtime, such as on Tokio's blocking pool.
    pub fn block(&self) {
        self.event.wait_blocking();
    }
}

impl std::fmt::Debug for SaveWait {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaveWait")
            .field("ready", &self.is_ready())
            .finish()
    }
}

impl Future for SaveWait {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut state = lock(&self.event.state);
        if state.done {
            return Poll::Ready(());
        }
        if !state.wakers.iter().any(|waker| waker.will_wake(cx.waker())) {
            state.wakers.push(cx.waker().clone());
        }
        Poll::Pending
    }
}

/// The outcome of one queued save, filled in by the writer's thread.
#[derive(Clone, Default)]
pub(crate) struct SaveTicket {
    slot: Arc<TicketSlot>,
}

#[derive(Default)]
struct TicketSlot {
    outcome: Mutex<Outcome>,
    /// When the save finished; set before `done`.
    finished_at: OnceLock<Instant>,
    done: Arc<Event>,
}

#[derive(Default)]
enum Outcome {
    #[default]
    Pending,
    Done(Result<(), Error>),
    /// Someone took the result; whether the save succeeded.
    Taken(bool),
}

impl SaveTicket {
    fn complete(&self, result: Result<(), Error>) {
        *lock(&self.slot.outcome) = Outcome::Done(result);
        let _ = self.slot.finished_at.set(Instant::now());
        self.slot.done.set();
    }

    /// Whether the save has run.
    pub(crate) fn is_done(&self) -> bool {
        self.slot.done.is_set()
    }

    /// A future that resolves once the save has run.
    pub(crate) fn wait(&self) -> SaveWait {
        SaveWait::new(Arc::clone(&self.slot.done))
    }

    /// Whether `wait` is the one [`wait`](Self::wait) hands out for this
    /// save, so a request releasing a staged write can show it staged it.
    pub(crate) fn issued(&self, wait: &SaveWait) -> bool {
        Arc::ptr_eq(&self.slot.done, &wait.event)
    }

    /// Whether, at `now`, a staged write waiting on this save has outlived
    /// [`STAGED_WRITE_LIFETIME`]. The lifetime starts when the save finishes;
    /// a save still running never outlives it.
    pub(crate) fn outlived_at(&self, now: Instant) -> bool {
        self.slot.finished_at.get().is_some_and(|finished| {
            now.saturating_duration_since(*finished) >= STAGED_WRITE_LIFETIME
        })
    }

    /// [`outlived_at`](Self::outlived_at) now.
    pub(crate) fn outlived(&self) -> bool {
        self.outlived_at(Instant::now())
    }

    /// Whether the save succeeded, once it has run.
    pub(crate) fn succeeded(&self) -> Option<bool> {
        match &*lock(&self.slot.outcome) {
            Outcome::Pending => None,
            Outcome::Done(result) => Some(result.is_ok()),
            Outcome::Taken(ok) => Some(*ok),
        }
    }

    /// Wait for the save, blocking the thread if it has not run yet, and
    /// take its result. The result goes to one taker; a later call gets an
    /// error that says so.
    pub(crate) fn take_outcome(&self) -> Result<(), Error> {
        self.slot.done.wait_blocking();
        let mut outcome = lock(&self.slot.outcome);
        match std::mem::take(&mut *outcome) {
            Outcome::Done(result) => {
                *outcome = Outcome::Taken(result.is_ok());
                result
            }
            Outcome::Taken(ok) => {
                *outcome = Outcome::Taken(ok);
                if ok {
                    Ok(())
                } else {
                    Err(Error::Encoding(
                        "save failed; its error was reported".into(),
                    ))
                }
            }
            Outcome::Pending => unreachable!("the event is set only after the outcome"),
        }
    }
}

type SaveFn<S> = dyn Fn(&S) -> Result<(), Error> + Send + Sync;
type DoneFn<S> = dyn Fn(&S, &Result<(), Error>) + Send + Sync;

/// Runs one object's storage calls on a thread of its own, one at a time, in
/// the order they were queued. See the [module documentation](self).
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
            tracing::warn!(
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
                    tracing::warn!(%error, writer = %self.name, "Could not start a save thread");
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
                std::mem::take(&mut queue.idle_waits)
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

/// Make a rename or a newly created file in `path`'s directory durable by
/// synchronizing the directory.
///
/// Call it once the file is in place. By then the save has landed: memory
/// takes the saved state, so storage and memory agree, and this never fails
/// the save. A filesystem that cannot synchronize a directory (`EINVAL`,
/// `ENOTSUP`, `EOPNOTSUPP` or `EBADF`, as some network and FUSE filesystems
/// answer) is passed over quietly, as PostgreSQL does. Any other error is
/// logged as a warning: the save stands, but a power loss right after it
/// may bring back the previous file.
///
/// On Unix this opens the directory and calls `fsync` on it. Windows cannot
/// open a directory through `std::fs`, so there the call does nothing: NTFS
/// journals the rename, but a power loss right after it may still bring back
/// the previous file.
pub(crate) fn sync_parent_dir(path: &Path) {
    #[cfg(unix)]
    {
        let parent = match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent,
            _ => Path::new("."),
        };
        let result = std::fs::File::open(parent).and_then(|directory| directory.sync_all());
        if let Err(error) = result {
            if directory_sync_unsupported(&error) {
                tracing::debug!(directory = %parent.display(), %error, "Directory sync not supported here");
            } else {
                tracing::warn!(
                    directory = %parent.display(),
                    %error,
                    "Could not synchronize the directory after a save; the save stands"
                );
            }
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Whether a directory sync failed only because the filesystem cannot
/// synchronize a directory.
#[cfg(unix)]
fn directory_sync_unsupported(error: &std::io::Error) -> bool {
    // ENOTSUP and EOPNOTSUPP are one value on Linux, two on macOS.
    error.raw_os_error().is_some_and(|code| {
        [libc::EINVAL, libc::ENOTSUP, libc::EOPNOTSUPP, libc::EBADF].contains(&code)
    })
}

/// What [`DurableWrites::stage_write`] gave its caller.
#[doc(hidden)]
#[derive(Debug)]
pub enum StageStep {
    /// The object queued a save of the state the write would leave, and keeps
    /// that state aside. Await the wait without the database guard, then make
    /// the write as usual: the object takes the saved state, or refuses the
    /// write if the save failed. Keep a clone of the wait to release the
    /// staged write with.
    Staged(SaveWait),
    /// Another request's staged write holds the object. Await the wait, then
    /// stage again.
    Busy(SaveWait),
    /// Nothing to stage: the object saves nothing for this write, or will
    /// refuse it anyway. Make the write as usual.
    Skip,
}

/// One write a request makes to an object, as
/// [`DurableWrites::stage_writes`] receives it.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq)]
pub struct PendingWrite {
    /// The property written.
    pub property: PropertyIdentifier,
    /// The array index written, if any.
    pub array_index: Option<u32>,
    /// The value written.
    pub value: PropertyValue,
}

/// Writes whose new state an object saves before serving it, staged so the
/// save runs while the database guard is dropped (#1270).
///
/// The bundled server stages the writes each request makes to such an
/// object: it calls [`stage_writes`](Self::stage_writes) under the guard,
/// awaits the save after dropping it, makes the writes under the guard
/// again, and in that same critical section calls
/// [`release_staged_write`](Self::release_staged_write). A write made
/// without staging still works: the object then saves in place.
#[doc(hidden)]
pub trait DurableWrites {
    /// Stage a write of `value` to `property` (at `array_index`).
    fn stage_write(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: &PropertyValue,
    ) -> StageStep;

    /// Stage the writes one request makes to this object, in the order it
    /// makes them; a WritePropertyMultiple can make several. The default
    /// stages the first write the object takes through
    /// [`stage_write`](Self::stage_write), and the request's later writes to
    /// the object save in place. Every bundled object overrides it to fold
    /// the writes into one save (#1423): each write then takes its own step
    /// of that save as the request makes it.
    fn stage_writes(&mut self, writes: &[PendingWrite]) -> StageStep {
        for write in writes {
            match self.stage_write(write.property, write.array_index, &write.value) {
                StageStep::Skip => continue,
                step => return step,
            }
        }
        StageStep::Skip
    }

    /// Stage a purge of the object's records, which the application asks
    /// for through the server (an Audit Log's, #1238). The server goes on as
    /// for a staged write, calling [`commit_purge`](Self::commit_purge) where
    /// it would make the write. The default has nothing to purge.
    fn stage_purge(&mut self) -> StageStep {
        StageStep::Skip
    }

    /// Purge the object's records: take the purge staged on the state the
    /// object serves, if there is one, or purge in place. The default
    /// refuses with OBJECT / OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.
    fn commit_purge(&mut self) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::OBJECT.to_raw() as u32,
            code: ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.to_raw() as u32,
        })
    }

    /// The request that staged a write is done, whether or not the write
    /// reached the object. `staged` is the wait [`StageStep::Staged`] gave
    /// it; a staged write some other request made since is left alone. A
    /// staged state the write never took is dropped, and a save of the state
    /// the object serves is queued at once, so storage follows the object
    /// again.
    fn release_staged_write(&mut self, staged: &SaveWait);

    /// Whether the object holds something staged for a request that the
    /// request has still to take or release. The default stages nothing.
    fn has_staged_write(&self) -> bool {
        false
    }

    /// No request is left to take or release what is staged, as once the
    /// server's `stop()` has joined its requests (#1363). Settle it: a
    /// staged write goes as a release would drop it, so storage goes back
    /// to the served state. Returns a wait that ends once every save the
    /// object has queued so far has run, to await off the guard, or `None`
    /// when the object keeps nothing in storage. The default has nothing
    /// staged.
    fn settle_forgotten_writes(&mut self) -> Option<SaveWait> {
        None
    }
}

#[cfg(test)]
#[path = "durable/tests.rs"]
mod tests;
