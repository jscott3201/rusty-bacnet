//! Storage stand-ins for the forwarder's persistence tests.

// Without std the tests that hold a save open on the writer's thread are
// left out, and the helpers only they use with them.
#![cfg_attr(not(feature = "std"), allow(dead_code))]

use super::*;
use std::future::Future;
use std::pin::pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, mpsc};
use std::task::{Context, Poll, Wake, Waker};
use std::time::Duration;

/// Storage in memory. A save can be made to fail, or to wait until the test
/// lets it go.
#[derive(Default)]
pub(super) struct MemoryPersistence {
    saved: Mutex<Option<(ObjectIdentifier, ForwarderSnapshot)>>,
    pub(super) fail: AtomicBool,
    saves: AtomicU64,
    hold: Mutex<Option<Hold>>,
}

struct Hold {
    started: mpsc::Sender<ForwarderSnapshot>,
    go: mpsc::Receiver<()>,
}

/// The test's side of a held storage: each save reports on `started` and
/// then waits for one message on `go`. Dropping `go` lets every save through.
pub(super) struct Held {
    pub(super) started: mpsc::Receiver<ForwarderSnapshot>,
    pub(super) go: mpsc::Sender<()>,
}

/// Long enough for any wait on another thread in these tests.
pub(super) const WAIT: Duration = Duration::from_secs(10);

impl MemoryPersistence {
    /// Make every later save wait for the test.
    pub(super) fn hold(&self) -> Held {
        let (started, started_rx) = mpsc::channel();
        let (go_tx, go) = mpsc::channel();
        *self.hold.lock().unwrap() = Some(Hold { started, go });
        Held {
            started: started_rx,
            go: go_tx,
        }
    }

    pub(super) fn snapshot(&self) -> ForwarderSnapshot {
        self.saved.lock().unwrap().clone().unwrap().1
    }

    /// The saved Subscribed_Recipients.
    pub(super) fn saved(&self) -> Vec<BACnetEventNotificationSubscription> {
        self.snapshot().subscribed_recipients
    }

    /// Saves that succeeded.
    pub(super) fn saves(&self) -> u64 {
        self.saves.load(Ordering::SeqCst)
    }

    /// Put `snapshot` in storage as if a forwarder had saved it.
    pub(super) fn preload(&self, forwarder: ObjectIdentifier, snapshot: ForwarderSnapshot) {
        *self.saved.lock().unwrap() = Some((forwarder, snapshot));
    }
}

impl NotificationForwarderPersistence for MemoryPersistence {
    fn load(&self, forwarder: ObjectIdentifier) -> Result<Option<ForwarderSnapshot>, Error> {
        Ok(self
            .saved
            .lock()
            .unwrap()
            .clone()
            .filter(|(oid, _)| *oid == forwarder)
            .map(|(_, snapshot)| snapshot))
    }

    fn save(&self, forwarder: ObjectIdentifier, snapshot: &ForwarderSnapshot) -> Result<(), Error> {
        if let Some(hold) = &*self.hold.lock().unwrap() {
            let _ = hold.started.send(snapshot.clone());
            let _ = hold.go.recv();
        }
        if self.fail.load(Ordering::SeqCst) {
            return Err(Error::Encoding("storage unavailable".into()));
        }
        *self.saved.lock().unwrap() = Some((forwarder, snapshot.clone()));
        self.saves.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// A forwarder kept in `storage`.
pub(super) fn persistent(storage: &Arc<MemoryPersistence>) -> NotificationForwarderObject {
    NotificationForwarderObject::with_persistence(
        1,
        "NF",
        Arc::clone(storage) as Arc<dyn NotificationForwarderPersistence>,
    )
    .unwrap()
}

struct ThreadWaker(std::thread::Thread);

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

/// Run `future` to completion on this thread, failing after [`WAIT`].
pub(super) fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut future = pin!(future);
    let deadline = std::time::Instant::now() + WAIT;
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
            return output;
        }
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        assert!(!left.is_zero(), "the future did not resolve in time");
        std::thread::park_timeout(left);
    }
}
