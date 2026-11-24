//! [`for_caller`] on a plain Tokio mutex, the lock a call waits for, with a
//! host that spawns the waiting task or keeps it for the test to poll.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::task::Waker;

use tokio::sync::{Mutex, OwnedMutexGuard};
use tokio::task::JoinHandle;

use super::*;

type Lock = Arc<Mutex<u32>>;
type Call = Pin<Box<dyn Future<Output = Option<u32>> + Send>>;

/// Poll `call` once with a waker that wakes nothing: its caller then stops
/// polling it.
fn poll_once<R>(call: &mut Pin<Box<dyn Future<Output = R> + Send>>) -> Poll<R> {
    call.as_mut().poll(&mut Context::from_waker(Waker::noop()))
}

/// Add one to the counter under the lock, counting the calls to `make`.
fn increment(lock: &Lock, makes: &Arc<AtomicUsize>) -> Call {
    let makes = Arc::clone(makes);
    Box::pin(for_caller(
        spawn_host,
        Arc::clone(lock).lock_owned(),
        move |mut count: OwnedMutexGuard<u32>| {
            makes.fetch_add(1, Ordering::SeqCst);
            *count += 1;
            *count
        },
    ))
}

/// Spawn the waiting task on the runtime.
fn spawn_host(task: CallerTask) -> Result<(), CallerTask> {
    tokio::spawn(task);
    Ok(())
}

/// A host that keeps the task for the test, with the handle it spawned.
#[derive(Clone, Default)]
struct Kept(Arc<StdMutex<Vec<JoinHandle<()>>>>);

impl Kept {
    fn host(&self) -> impl FnOnce(CallerTask) -> Result<(), CallerTask> + use<> {
        let kept = self.clone();
        move |task| {
            kept.0.lock().unwrap().push(tokio::spawn(task));
            Ok(())
        }
    }

    fn count(&self) -> usize {
        self.0.lock().unwrap().len()
    }

    /// Wait for every kept task to end.
    async fn ended(&self) {
        let handles: Vec<_> = self.0.lock().unwrap().drain(..).collect();
        for handle in handles {
            handle.await.unwrap();
        }
    }
}

#[tokio::test]
async fn nothing_in_the_way_runs_in_the_callers_poll() {
    let lock = Lock::default();
    let kept = Kept::default();
    let made = for_caller(kept.host(), Arc::clone(&lock).lock_owned(), |mut count| {
        *count += 1;
        *count
    })
    .await;
    assert_eq!(made, Some(1));
    assert_eq!(
        kept.count(),
        0,
        "no task for a call that found its way clear"
    );
}

/// A call polled once while the lock is held, then never again, takes the
/// lock in its task as it comes free and lets it go: the next taker gets it
/// without the call being polled, and the call, polled at last, answers.
#[tokio::test]
async fn a_parked_call_holds_up_nothing() {
    let lock = Lock::default();
    let makes = Arc::new(AtomicUsize::new(0));
    let held = lock.lock().await;
    let mut parked = increment(&lock, &makes);
    assert!(poll_once(&mut parked).is_pending());
    drop(held);
    assert_eq!(*lock.lock().await, 1, "the call's task made the change");
    assert_eq!(parked.await, Some(1));
    assert_eq!(makes.load(Ordering::SeqCst), 1);
}

/// A call keeps the place in the lock's queue it took in its caller's
/// poll: a waiter that queued after it gets the lock after it, and one
/// that queued before it, before it.
#[tokio::test]
async fn a_call_keeps_its_place_in_the_queue() {
    let lock = Lock::default();
    let held = lock.lock().await;
    let order = Arc::new(StdMutex::new(Vec::new()));
    let call = |n: u32| -> Pin<Box<dyn Future<Output = Option<()>> + Send>> {
        let order = Arc::clone(&order);
        Box::pin(for_caller(
            spawn_host,
            Arc::clone(&lock).lock_owned(),
            move |_count| order.lock().unwrap().push(n),
        ))
    };
    let mut first = call(0);
    assert!(poll_once(&mut first).is_pending());
    // The test's own waiter, queued between the two calls.
    let mut between: Pin<Box<dyn Future<Output = OwnedMutexGuard<u32>> + Send>> =
        Box::pin(Arc::clone(&lock).lock_owned());
    assert!(poll_once(&mut between).is_pending());
    let mut second = call(1);
    assert!(poll_once(&mut second).is_pending());
    drop(held);
    let guard = between.await;
    order.lock().unwrap().push(99);
    drop(guard);
    assert_eq!(second.await, Some(()));
    assert_eq!(first.await, Some(()));
    assert_eq!(*order.lock().unwrap(), [0, 99, 1]);
}

/// A caller gone while its task waits for the lock leaves `make` unrun,
/// and the task ends without waiting for the lock.
#[tokio::test]
async fn a_caller_gone_while_its_task_waits_gets_nothing_made() {
    let lock = Lock::default();
    let makes = Arc::new(AtomicUsize::new(0));
    let kept = Kept::default();
    let held = lock.lock().await;
    let mut parked: Call = {
        let makes = Arc::clone(&makes);
        Box::pin(for_caller(
            kept.host(),
            Arc::clone(&lock).lock_owned(),
            move |mut count| {
                makes.fetch_add(1, Ordering::SeqCst);
                *count += 1;
                *count
            },
        ))
    };
    assert!(poll_once(&mut parked).is_pending());
    assert_eq!(kept.count(), 1, "one task for the blocked call");
    drop(parked);
    tokio::time::timeout(std::time::Duration::from_secs(5), kept.ended())
        .await
        .expect("the task ends without waiting for the lock");
    drop(held);
    assert_eq!(*lock.lock().await, 0);
    assert_eq!(makes.load(Ordering::SeqCst), 0);
}

/// A caller that goes as `take` gets its lock, before the task has looked,
/// gets nothing made either: the task looks again with the lock held.
#[tokio::test]
async fn a_caller_gone_as_take_finishes_gets_nothing_made() {
    let lock = Lock::default();
    let makes = Arc::new(AtomicUsize::new(0));
    let kept = Kept::default();
    // The call itself, which `take` drops as it finishes.
    let slot: Arc<StdMutex<Option<Call>>> = Arc::default();
    let held = lock.lock().await;
    let take = {
        let (lock, slot) = (Arc::clone(&lock), Arc::clone(&slot));
        async move {
            let count = lock.lock_owned().await;
            drop(slot.lock().unwrap().take());
            count
        }
    };
    let mut call: Call = {
        let makes = Arc::clone(&makes);
        Box::pin(for_caller(kept.host(), take, move |mut count| {
            makes.fetch_add(1, Ordering::SeqCst);
            *count += 1;
            *count
        }))
    };
    assert!(poll_once(&mut call).is_pending());
    *slot.lock().unwrap() = Some(call);
    drop(held);
    kept.ended().await;
    assert!(slot.lock().unwrap().is_none(), "take dropped the call");
    assert_eq!(*lock.lock().await, 0);
    assert_eq!(makes.load(Ordering::SeqCst), 0);
}

/// A host that hands the task back leaves the call to wait in its own
/// task, as it did before.
#[tokio::test]
async fn a_host_that_refuses_leaves_the_wait_to_the_caller() {
    let lock = Lock::default();
    let held = lock.lock().await;
    let refused = Arc::new(AtomicUsize::new(0));
    let mut call: Call = {
        let refused = Arc::clone(&refused);
        Box::pin(for_caller(
            move |task| {
                refused.fetch_add(1, Ordering::SeqCst);
                Err(task)
            },
            Arc::clone(&lock).lock_owned(),
            |mut count| {
                *count += 1;
                *count
            },
        ))
    };
    assert!(poll_once(&mut call).is_pending());
    assert_eq!(refused.load(Ordering::SeqCst), 1);
    drop(held);
    assert_eq!(call.await, Some(1));
}

/// A panic in `make`, run by the task, is raised again in the caller.
#[tokio::test]
async fn a_panic_in_the_task_is_raised_in_the_caller() {
    let lock = Lock::default();
    let held = lock.lock().await;
    let mut call: Pin<Box<dyn Future<Output = Option<()>> + Send>> = Box::pin(for_caller(
        spawn_host,
        Arc::clone(&lock).lock_owned(),
        |_count| panic!("make failed"),
    ));
    assert!(poll_once(&mut call).is_pending());
    drop(held);
    let panic = AssertUnwindSafe(call).catch_unwind().await.unwrap_err();
    assert_eq!(message(&*panic), "make failed");
    assert_eq!(*lock.lock().await, 0, "the lock is free again");
}

/// A panic in `take`, once it runs in the task, is raised again in the
/// caller too, and the lock it held is free again.
#[tokio::test]
async fn a_panic_in_take_is_raised_in_the_caller() {
    let lock = Lock::default();
    let held = lock.lock().await;
    let take = {
        let lock = Arc::clone(&lock);
        async move {
            let count = lock.lock_owned().await;
            if *count == 0 {
                panic!("take failed");
            }
            count
        }
    };
    let mut call: Call = Box::pin(for_caller(spawn_host, take, |count| *count));
    assert!(poll_once(&mut call).is_pending());
    drop(held);
    let panic = AssertUnwindSafe(call).catch_unwind().await.unwrap_err();
    assert_eq!(message(&*panic), "take failed");
    assert_eq!(*lock.lock().await, 0, "the lock is free again");
}

/// A host that takes the task and then drops it unrun, as a runtime that
/// shuts down does, gets the call `None`, with `make` never run: the caller
/// may then make the call itself.
#[tokio::test]
async fn a_task_dropped_by_its_host_answers_none() {
    let lock = Lock::default();
    let makes = Arc::new(AtomicUsize::new(0));
    let held = lock.lock().await;
    let mut call: Call = {
        let makes = Arc::clone(&makes);
        Box::pin(for_caller(
            |task| {
                drop(task);
                Ok(())
            },
            Arc::clone(&lock).lock_owned(),
            move |mut count| {
                makes.fetch_add(1, Ordering::SeqCst);
                *count += 1;
                *count
            },
        ))
    };
    // The dropped task drops the answer's sender: the first poll knows.
    assert_eq!(poll_once(&mut call), Poll::Ready(None));
    drop(held);
    assert_eq!(*lock.lock().await, 0);
    assert_eq!(makes.load(Ordering::SeqCst), 0);
}
