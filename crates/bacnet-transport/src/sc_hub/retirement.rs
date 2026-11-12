//! Per-registration cleanup ownership and sticky, multi-waiter retirement.

use super::*;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, frame::coding::CloseCode};

/// The close frame for a connection the hub ends on purpose: status 1000,
/// a normal closure in RFC 6455 terms, so the peer can tell an orderly end
/// from a lost link (#2463). Annex AB.7.5.5 defers closing to RFC 6455,
/// where the status is optional, and maps 1000 to
/// `WEBSOCKET_CLOSED_BY_PEER`.
pub(super) fn orderly_close() -> Message {
    Message::Close(Some(CloseFrame {
        code: CloseCode::Normal,
        reason: "".into(),
    }))
}

/// The close frame for a peer that sent a data frame that is not binary:
/// status 1003, as Annex AB.7.5.3 requires.
pub(super) fn non_binary_close() -> Message {
    Message::Close(Some(CloseFrame {
        code: CloseCode::Unsupported,
        reason: "BACnet/SC requires binary frames".into(),
    }))
}

pub(super) struct Lease {
    pub vmac: Option<Vmac>,
    peer_close_observed: bool,
    /// The dispatch ended on the peer's fault, not on purpose.
    fault: bool,
    pub closed: Arc<AtomicBool>,
    pub notify: Arc<Notify>,
}

impl Lease {
    pub fn new() -> Self {
        Self {
            vmac: None,
            peer_close_observed: false,
            fault: false,
            closed: Arc::new(AtomicBool::new(false)),
            notify: Arc::new(Notify::new()),
        }
    }

    /// Tungstenite queues the reciprocal frame while reading peer Close.
    /// Preserve that fact even when the dispatch future is then retired.
    pub fn note_peer_close(&mut self) {
        self.peer_close_observed = true;
    }

    pub fn peer_close_observed(&self) -> bool {
        self.peer_close_observed
    }

    /// The dispatch ends over something the peer did wrong (a receive error,
    /// a frame type or a second Connect-Request). That close is not an
    /// orderly one, so it keeps the status-less frame it always had.
    pub fn note_fault(&mut self) {
        self.fault = true;
    }

    /// The close frame this connection ends with: the orderly 1000 unless
    /// the peer's fault ended it.
    pub fn close_message(&self) -> Message {
        if self.fault {
            Message::Close(None)
        } else {
            orderly_close()
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
        // Local retirement cleanup budget, including sink acquisition and
        // Close/flush. This is outside retirement selection: already-closed
        // state must not cancel its own cleanup. Cancellation is not rollback.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let mut sink = sink.lock().await;
            if !self.peer_close_observed {
                sink.send(self.close_message()).await?;
            }
            // A peer Close already queued its exact reciprocal frame. Another
            // send is invalid after closing and would prevent this flush.
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
