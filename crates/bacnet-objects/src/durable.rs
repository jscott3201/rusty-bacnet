//! Saving object state off the object database lock (#1270).
//!
//! The Audit Log, the Notification Forwarder, the Notification Class and the
//! Access Rights object keep state in storage the application provides
//! ([`AuditLogPersistence`], [`NotificationForwarderPersistence`],
//! [`NotificationClassPersistence`] and [`AccessRightsPersistence`]).
//! A storage call can be slow, since the file backends write, synchronize and
//! rename, so none of these objects makes one while its caller holds the
//! database's write guard. They all follow the pattern this module provides.
//! The Trend Log, Trend Log Multiple and Event Log commit to their
//! [`LogPersistence`] through the same writer, on a cadence rather than per
//! request (see [`crate::log_persistence`]), so they never stage.
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
//! Without std there are no threads: each save runs on the caller's thread
//! before the call that queued it returns, so every wait is already over
//! (#1615). The staging above works the same way; only the storage call now
//! runs under the caller's guard.
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
//! [`LogPersistence`]: crate::log_persistence::LogPersistence

use alloc::sync::Arc;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};
#[cfg(feature = "std")]
use std::path::Path;
#[cfg(feature = "std")]
use std::sync::OnceLock;
#[cfg(feature = "std")]
use std::time::Instant;

use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use crate::prelude::*;
use crate::sync::Mutex;

#[cfg(feature = "std")]
pub(crate) mod file;
// Without std there are no threads, so each save runs inline, on the
// caller's (#1615).
#[cfg(not(feature = "std"))]
mod inline;
pub(crate) mod staged;
#[cfg(feature = "std")]
mod thread;

#[cfg(not(feature = "std"))]
pub(crate) use inline::SaveWriter;
#[cfg(feature = "std")]
pub(crate) use thread::SaveWriter;

/// How long a staged write may wait for its request once its save has run.
/// It counts from the end of the save, so a slow save never uses it up. Past
/// it the request is taken to be gone, and the object drops the staged state
/// at the next stage or operation task call.
#[cfg(not(test))]
pub(crate) const STAGED_WRITE_LIFETIME: core::time::Duration = core::time::Duration::from_secs(10);
/// Short in tests, so one can watch a forgotten staged write go.
#[cfg(test)]
pub(crate) const STAGED_WRITE_LIFETIME: core::time::Duration =
    core::time::Duration::from_millis(300);

/// Something that happens once, which any number of threads and tasks can
/// wait for.
#[derive(Default)]
pub(crate) struct Event {
    state: Mutex<EventState>,
    #[cfg(feature = "std")]
    happened: std::sync::Condvar,
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
            let mut state = self.state.lock();
            state.done = true;
            core::mem::take(&mut state.wakers)
        };
        #[cfg(feature = "std")]
        self.happened.notify_all();
        for waker in wakers {
            waker.wake();
        }
    }

    pub(crate) fn is_set(&self) -> bool {
        self.state.lock().done
    }

    #[cfg(feature = "std")]
    fn wait_blocking(&self) {
        let mut state = self.state.lock();
        while !state.done {
            state = self
                .happened
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    /// Without std nothing can park the thread, so this spins. A save runs
    /// on the caller's thread there, so a save's event is set before anyone
    /// can wait on it.
    #[cfg(not(feature = "std"))]
    fn wait_blocking(&self) {
        while !self.is_set() {
            core::hint::spin_loop();
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
    /// A second wait this one also ends after, when an object that keeps
    /// two kinds of state in storage settles both ([`and`](Self::and)).
    then: Option<Arc<SaveWait>>,
}

impl SaveWait {
    pub(crate) fn new(event: Arc<Event>) -> Self {
        Self { event, then: None }
    }

    /// A wait that ends once both this wait and `other` have.
    pub(crate) fn and(self, other: SaveWait) -> SaveWait {
        let then = match self.then {
            Some(then) => SaveWait::clone(&then).and(other),
            None => other,
        };
        SaveWait {
            event: self.event,
            then: Some(Arc::new(then)),
        }
    }

    /// Whether the wait is already over.
    pub fn is_ready(&self) -> bool {
        self.event.is_set() && self.then.as_ref().is_none_or(|then| then.is_ready())
    }

    /// Block this thread until the wait is over, for a caller that waits
    /// off its async runtime, such as on Tokio's blocking pool.
    ///
    /// Without std, await the wait instead; never block on one that isn't
    /// over. A save's wait is over by the time anyone holds it, since the
    /// save ran on the caller's thread, but [`StageStep::Busy`] hands out a
    /// wait for another request's staged write, which only that request can
    /// end. Nothing can park the thread, so `block` would spin, and on a
    /// single-threaded executor it would spin forever. A debug build panics
    /// instead; a release build spins, for firmware that releases the write
    /// from another core or an interrupt.
    pub fn block(&self) {
        #[cfg(not(feature = "std"))]
        debug_assert!(
            self.is_ready(),
            "SaveWait::block without std on a wait that isn't over: await it instead"
        );
        self.event.wait_blocking();
        if let Some(then) = &self.then {
            then.block();
        }
    }
}

impl core::fmt::Debug for SaveWait {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SaveWait")
            .field("ready", &self.is_ready())
            .finish()
    }
}

impl Future for SaveWait {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        {
            let mut state = self.event.state.lock();
            if !state.done {
                if !state.wakers.iter().any(|waker| waker.will_wake(cx.waker())) {
                    state.wakers.push(cx.waker().clone());
                }
                return Poll::Pending;
            }
        }
        match &self.then {
            Some(then) => Pin::new(&mut SaveWait::clone(then)).poll(cx),
            None => Poll::Ready(()),
        }
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
    /// When the save finished; set before `done`. Only std has a clock for
    /// it; without std a staged write expires only by the operation task's
    /// clock ([`staged::StagedSaves::expire`]).
    #[cfg(feature = "std")]
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
        *self.slot.outcome.lock() = Outcome::Done(result);
        #[cfg(feature = "std")]
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
    #[cfg(feature = "std")]
    pub(crate) fn outlived_at(&self, now: Instant) -> bool {
        self.slot.finished_at.get().is_some_and(|finished| {
            now.saturating_duration_since(*finished) >= STAGED_WRITE_LIFETIME
        })
    }

    /// Whether a staged write waiting on this save has outlived
    /// [`STAGED_WRITE_LIFETIME`] by now. Never without std, which has no
    /// clock to tell.
    pub(crate) fn outlived(&self) -> bool {
        #[cfg(feature = "std")]
        {
            self.outlived_at(Instant::now())
        }
        #[cfg(not(feature = "std"))]
        {
            false
        }
    }

    /// Whether the save succeeded, once it has run.
    pub(crate) fn succeeded(&self) -> Option<bool> {
        match &*self.slot.outcome.lock() {
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
        let mut outcome = self.slot.outcome.lock();
        match core::mem::take(&mut *outcome) {
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
#[cfg(feature = "std")]
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
                log_debug!(directory = %parent.display(), %error, "Directory sync not supported here");
            } else {
                log_warn!(
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

/// The CRC-32 (IEEE 802.3, reflected) of `data`, which the two-slot
/// snapshot files end with.
pub(crate) fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

/// Whether a directory sync failed only because the filesystem cannot
/// synchronize a directory.
#[cfg(all(feature = "std", unix))]
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
        Err(Error::protocol(
            ErrorClass::OBJECT,
            ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
        ))
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

#[cfg(all(test, feature = "std"))]
#[path = "durable/tests.rs"]
mod tests;
