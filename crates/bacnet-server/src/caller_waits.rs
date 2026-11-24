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
//! gone. A caller that goes before `take` holds its locks leaves `make`
//! unrun, and the task lets go of what `take` held. The check is made with
//! those locks held, so a change `make` makes was made before the caller
//! went or not at all. There is one task per call that found its way
//! blocked, and none for a call that did not.

use std::any::Any;
use std::future::{Future, poll_fn};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::FutureExt;
use tokio::sync::oneshot;

/// The task that waits for a call (see the [module](self) documentation),
/// for the call's host to run on its runtime.
#[doc(hidden)]
pub struct CallerTask(Pin<Box<dyn Future<Output = ()> + Send>>);

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
/// when the host took the task and dropped it before it answered.
#[doc(hidden)]
pub async fn for_caller<G, R>(
    host: impl FnOnce(CallerTask) -> Result<(), CallerTask>,
    take: impl Future<Output = G> + Send + 'static,
    make: impl FnOnce(G) -> R + Send + 'static,
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
    let task = CallerTask(Box::pin(async move {
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
    }));
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

#[cfg(test)]
#[path = "caller_waits_tests.rs"]
mod tests;
