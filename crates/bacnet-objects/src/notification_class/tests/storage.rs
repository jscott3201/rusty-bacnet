//! Storage stand-ins for the Notification Class persistence tests (#1315).

use super::super::*;
use crate::durable::SaveWait;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

/// Storage in memory. A save can be made to fail, or to wait until the test
/// lets it go.
#[derive(Default)]
pub(super) struct MemoryPersistence {
    saved: Mutex<Option<(ObjectIdentifier, NotificationClassSnapshot)>>,
    pub(super) fail: AtomicBool,
    saves: AtomicU64,
    hold: Mutex<Option<Hold>>,
}

struct Hold {
    started: mpsc::Sender<NotificationClassSnapshot>,
    go: mpsc::Receiver<()>,
}

/// The test's side of a held storage: each save reports on `started` and
/// then waits for one message on `go`. Dropping `go` lets every save through.
pub(super) struct Held {
    pub(super) started: mpsc::Receiver<NotificationClassSnapshot>,
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

    /// What storage holds; `None` before any save.
    pub(super) fn snapshot(&self) -> Option<NotificationClassSnapshot> {
        self.saved
            .lock()
            .unwrap()
            .clone()
            .map(|(_, snapshot)| snapshot)
    }

    /// The saved Recipient_List, if storage holds one.
    pub(super) fn saved(&self) -> Option<Vec<BACnetDestination>> {
        self.snapshot().and_then(|snapshot| snapshot.recipient_list)
    }

    /// Saves that succeeded.
    pub(super) fn saves(&self) -> u64 {
        self.saves.load(Ordering::SeqCst)
    }

    /// Put `snapshot` in storage as if a class had saved it.
    pub(super) fn preload(&self, class: ObjectIdentifier, snapshot: NotificationClassSnapshot) {
        *self.saved.lock().unwrap() = Some((class, snapshot));
    }
}

impl NotificationClassPersistence for MemoryPersistence {
    fn load(&self, class: ObjectIdentifier) -> Result<Option<NotificationClassSnapshot>, Error> {
        Ok(self
            .saved
            .lock()
            .unwrap()
            .clone()
            .filter(|(oid, _)| *oid == class)
            .map(|(_, snapshot)| snapshot))
    }

    fn save(
        &self,
        class: ObjectIdentifier,
        snapshot: &NotificationClassSnapshot,
    ) -> Result<(), Error> {
        if let Some(hold) = &*self.hold.lock().unwrap() {
            let _ = hold.started.send(snapshot.clone());
            let _ = hold.go.recv();
        }
        if self.fail.load(Ordering::SeqCst) {
            return Err(Error::Encoding("storage unavailable".into()));
        }
        *self.saved.lock().unwrap() = Some((class, snapshot.clone()));
        self.saves.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// Notification Class 1, kept in `storage`.
pub(super) fn persistent(storage: &Arc<MemoryPersistence>) -> NotificationClass {
    NotificationClass::with_persistence(
        1,
        "NC",
        Arc::clone(storage) as Arc<dyn NotificationClassPersistence>,
    )
    .unwrap()
}

/// Block until `wait` is over, failing after [`WAIT`].
pub(super) fn block_on(wait: &SaveWait) {
    let (done, finished) = mpsc::channel();
    let blocking = wait.clone();
    std::thread::spawn(move || {
        blocking.block();
        let _ = done.send(());
    });
    finished
        .recv_timeout(WAIT)
        .expect("the wait did not end in time");
}
