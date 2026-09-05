//! Per-registration cleanup ownership and sticky, multi-waiter retirement.

use super::*;

pub(super) struct Lease {
    pub vmac: Option<Vmac>,
    pub closed: Arc<AtomicBool>,
    pub notify: Arc<Notify>,
}

impl Lease {
    pub fn new() -> Self {
        Self {
            vmac: None,
            closed: Arc::new(AtomicBool::new(false)),
            notify: Arc::new(Notify::new()),
        }
    }

    pub async fn cleanup(&self, clients: &Clients, sink: &Arc<Mutex<WsSink>>) {
        self.closed.store(true, Ordering::Release);
        wake(&self.notify);
        if let Some(vmac) = self.vmac {
            let mut map = clients.lock().await;
            if map
                .get(&vmac)
                .is_some_and(|client| Arc::ptr_eq(&client.sink, sink))
            {
                map.remove(&vmac);
            }
        }
        // Local replacement cleanup budget, including sink acquisition and
        // Close/flush. This is outside retirement selection: already-closed
        // state must not cancel its own cleanup. Cancellation is not rollback.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let mut sink = sink.lock().await;
            sink.send(Message::Close(None)).await?;
            sink.flush().await
        })
        .await;
    }
}

pub(super) fn wake(notify: &Notify) {
    notify.notify_waiters();
    // Retain the existing one-waiter permit as well. All retirement consumers
    // use the closed predicate, so consuming this permit cannot lose retirement.
    notify.notify_one();
}

pub(super) async fn wait(closed: &AtomicBool, notify: &Notify) {
    loop {
        let notified = notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if closed.load(Ordering::Acquire) {
            return;
        }
        notified.await;
        // A spurious wake is not retirement; re-register before rechecking.
    }
}
