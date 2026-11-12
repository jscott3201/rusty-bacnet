//! Per-registration cleanup ownership and sticky, multi-waiter retirement.

use super::*;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, frame::coding::CloseCode};

/// The close frame for a connection the hub ends on purpose, other than by
/// shutting down: status 1000, a normal closure in RFC 6455 terms (#2463).
/// Annex AB.7.5.5 asks for the close handshake on an intentional close and
/// maps 1000 to `WEBSOCKET_CLOSED_BY_PEER`.
pub(super) fn orderly_close() -> Message {
    status_close(CloseCode::Normal)
}

/// The close frame for a connection the hub ends because it is shutting
/// down: status 1001, the server going away in RFC 6455 terms, which
/// Annex AB.7.5.5 maps to `WEBSOCKET_ENDPOINT_LEAVES` (#2463).
pub(super) fn going_away_close() -> Message {
    status_close(CloseCode::Away)
}

fn status_close(code: CloseCode) -> Message {
    Message::Close(Some(CloseFrame {
        code,
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

/// Why the dispatch ended, as far as the close frame cares.
#[derive(Clone, Copy, PartialEq, Eq)]
enum End {
    /// On purpose, or not said: an orderly close (1000) if one is sent.
    Orderly,
    /// The hub is shutting down: 1001.
    GoingAway,
    /// The peer did something wrong: the status-less close it always had.
    Fault,
}

pub(super) struct Lease {
    pub vmac: Option<Vmac>,
    peer_close_observed: bool,
    end: End,
    /// The hub ends this unregistered connection on purpose (#2463).
    hub_ends: bool,
    pub closed: Arc<AtomicBool>,
    pub notify: Arc<Notify>,
}

impl Lease {
    pub fn new() -> Self {
        Self {
            vmac: None,
            peer_close_observed: false,
            end: End::Orderly,
            hub_ends: false,
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
    /// orderly one, so it keeps the status-less frame it always had. A
    /// fault outranks a shutdown in progress.
    pub fn note_fault(&mut self) {
        self.end = End::Fault;
    }

    /// The hub's graceful shutdown ends this connection.
    pub fn note_going_away(&mut self) {
        if self.end != End::Fault {
            self.end = End::GoingAway;
        }
    }

    /// The hub ends a connection that never registered, on purpose: it
    /// refused the Connect-Request (with a NAK, or without one for a
    /// reserved VMAC), or answered a Disconnect-Request that came before
    /// any Connect. Annex AB.6.2.3 has the accepting peer close after a
    /// refusal, and AB.7.5.5 asks for the close handshake (#2463).
    pub fn note_hub_ends(&mut self) {
        self.hub_ends = true;
    }

    pub fn hub_ends(&self) -> bool {
        self.hub_ends
    }

    /// The close frame this connection ends with: 1000, or 1001 for a
    /// shutdown, unless the peer's fault ended it.
    pub fn close_message(&self) -> Message {
        match self.end {
            End::Orderly => orderly_close(),
            End::GoingAway => going_away_close(),
            End::Fault => Message::Close(None),
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
