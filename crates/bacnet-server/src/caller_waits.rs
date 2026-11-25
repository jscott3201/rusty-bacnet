//! An application's call that needs a lock the server's own work takes too,
//! without the wait for it in the call's task (#2772).
//!
//! Tokio's locks hand a lock that comes free to the waiter queued first,
//! whether or not anything still polls that waiter. A call left unpolled
//! while it waits (it lost a `select!`, sits in a stream nobody drives, or
//! its caller awaits something else first) would own the lock from then on,
//! and every request the server serves would queue behind it.
//!
//! [`for_caller`] runs such a call in two parts: `take`, which waits for the
//! locks the call needs (and for anything on the way, such as a staged
//! write's save), and `make`, which uses them without waiting. The call
//! polls `take` once, in its own poll. When nothing is in the way, `make`
//! runs there too, as the call always ran. Otherwise the same `take`,
//! already queued where it waits, moves to a task its host runs, so it
//! keeps its place in each queue; that task drives `take`, runs `make` and
//! hands back what `make` returns. A panic in either is raised again in the
//! caller.
//!
//! The task ends once it has handed back its answer, or once its caller has
//! gone. A caller that goes before the task holds its locks leaves `make`
//! unrun: the task looks with the locks held, and lets go of what `take`
//! held. A caller that goes after that may still have `make` run, while it
//! goes or before it would have polled the answer, and its drop does not
//! say which happened. There is one task per call that found its way
//! blocked, and none for a call that did not. The task runs in the
//! caller's tracing span.
//!
//! A call with no server to host its task, such as the application's
//! Schedule and trend ticks (#2781), runs it on the runtime it is polled on
//! (`on_runtime`) and holds the database there through a
//! [`HeldDatabase`]: once its caller has gone, that task may hold the last
//! handle, which then goes off the runtime (#1513).

use std::any::Any;
use std::future::{Future, poll_fn};
use std::ops::{Deref, DerefMut};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use bacnet_objects::database::ObjectDatabase;
use futures_util::FutureExt;
use tokio::sync::{OwnedRwLockWriteGuard, RwLock, oneshot};
use tracing::Instrument;

/// The task that waits for a call (see the [module](self) documentation),
/// for the call's host to run on its runtime.
#[doc(hidden)]
pub struct CallerTask(Pin<Box<dyn Future<Output = ()> + Send + Sync>>);

impl Future for CallerTask {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.0.as_mut().poll(cx)
    }
}

/// What a call's task hands back: what `make` returned, or the panic that
/// ended `take` or `make`.
type Answer<R> = Result<R, Box<dyn Any + Send>>;

/// Run `take`, then `make` with what it took, for a call, never leaving a
/// wait in the call's own task (see the [module](self) documentation).
///
/// `host` runs the task that waits when `take` can't finish in the call's
/// first poll; a host that hands the task back (no runtime, or a closed
/// task set) leaves the call to wait in its own task, as before. `None`
/// when the host took the task and dropped it before it answered, as a
/// runtime that shuts down does: the task is dropped only where it waits,
/// so `make` never ran, and the caller may do the call itself. `take` and
/// `make` must be `Sync`, so the call's future stays `Sync`, as the
/// server's local calls were before they waited here.
#[doc(hidden)]
pub async fn for_caller<G, R>(
    host: impl FnOnce(CallerTask) -> Result<(), CallerTask>,
    take: impl Future<Output = G> + Send + Sync + 'static,
    make: impl FnOnce(G) -> R + Send + Sync + 'static,
) -> Option<R>
where
    G: Send + 'static,
    R: Send + 'static,
{
    let mut take = Box::pin(take);
    let taken = poll_fn(|cx| {
        Poll::Ready(match take.as_mut().poll(cx) {
            Poll::Ready(taken) => Some(taken),
            Poll::Pending => None,
        })
    })
    .await;
    if let Some(taken) = taken {
        return Some(make(taken));
    }
    let (mut reply, answer) = oneshot::channel::<Answer<R>>();
    let wait = async move {
        let taken = tokio::select! {
            biased;
            () = reply.closed() => return,
            taken = AssertUnwindSafe(take).catch_unwind() => taken,
        };
        let made = match taken {
            // Checked under what `take` holds, which goes as the task ends.
            Ok(_) if reply.is_closed() => return,
            Ok(taken) => catch_unwind(AssertUnwindSafe(move || make(taken))),
            Err(panic) => Err(panic),
        };
        if let Err(Err(panic)) = reply.send(made) {
            tracing::error!(
                panic = message(&*panic),
                "A local call's work panicked after its caller went away"
            );
        }
    };
    let task = CallerTask(Box::pin(wait.in_current_span()));
    if let Err(task) = host(task) {
        task.await;
    }
    match answer.await {
        Ok(Ok(made)) => Some(made),
        Ok(Err(panic)) => resume_unwind(panic),
        Err(_) => None,
    }
}

/// A panic's text, when it has one.
fn message(panic: &(dyn Any + Send)) -> &str {
    panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("a non-text payload")
}

/// The host for a call with no server: the runtime the call is polled on,
/// which runs the task detached. The task ends once its caller has its
/// answer or has gone, and a runtime that shuts down drops it. Outside a
/// runtime the call waits in its own task, as before.
pub(crate) fn on_runtime(task: CallerTask) -> Result<(), CallerTask> {
    match tokio::runtime::Handle::try_current() {
        Ok(runtime) => {
            drop(runtime.spawn(task));
            Ok(())
        }
        Err(_) => Err(task),
    }
}

/// A strong handle on the object database that lets go through
/// [`drop_database_off_runtime`](crate::server::drop_database_off_runtime)
/// however it goes, a task dropped mid-await included (#1513, #1561).
///
/// The last handle to go drops every object, and a durable object then
/// waits for the saves it has queued: dropped on a runtime worker, that
/// wait would hold the worker for as long as storage takes. A holder that
/// can outlive every other handle, such as a call's task whose caller has
/// gone or an endpoint worker, holds the database through this.
#[doc(hidden)]
pub struct HeldDatabase(Option<Arc<RwLock<ObjectDatabase>>>);

impl HeldDatabase {
    /// Hold `db` until this is dropped.
    pub fn new(db: Arc<RwLock<ObjectDatabase>>) -> Self {
        Self(Some(db))
    }

    /// Wait for the database's write guard, which comes back with this
    /// handle. Dropped while it waits, the wait lets go of its own handle
    /// before this one goes.
    pub(crate) async fn write_owned(self) -> HeldWrite {
        let guard = Arc::clone(&*self).write_owned().await;
        HeldWrite { guard, held: self }
    }
}

impl Deref for HeldDatabase {
    type Target = Arc<RwLock<ObjectDatabase>>;

    fn deref(&self) -> &Self::Target {
        self.0.as_ref().expect("held until dropped")
    }
}

impl Drop for HeldDatabase {
    fn drop(&mut self) {
        if let Some(db) = self.0.take() {
            drop(crate::server::drop_database_off_runtime(db));
        }
    }
}

/// The database's write guard, taken through a [`HeldDatabase`], which
/// goes after the guard.
pub(crate) struct HeldWrite {
    // First: a field drops before the ones declared after it.
    guard: OwnedRwLockWriteGuard<ObjectDatabase>,
    held: HeldDatabase,
}

impl HeldWrite {
    /// The handle the guard is on, beside the guarded database.
    pub(crate) fn split(&mut self) -> (&Arc<RwLock<ObjectDatabase>>, &mut ObjectDatabase) {
        (&self.held, &mut self.guard)
    }
}

impl Deref for HeldWrite {
    type Target = ObjectDatabase;

    fn deref(&self) -> &ObjectDatabase {
        &self.guard
    }
}

impl DerefMut for HeldWrite {
    fn deref_mut(&mut self) -> &mut ObjectDatabase {
        &mut self.guard
    }
}

#[cfg(test)]
#[path = "caller_waits_tests.rs"]
mod tests;
