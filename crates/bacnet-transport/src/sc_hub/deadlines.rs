//! Absolute handshake waits. Expiry wins readiness ties.

use super::*;
use std::future::Future;
use tokio::time::{sleep_until, Instant};

pub(super) struct ConnectDeadline {
    expires: Instant,
    committed: AtomicBool,
    committed_notify: Notify,
    #[cfg(test)]
    pub(super) admission_started: AtomicBool,
    #[cfg(test)]
    pub(super) received: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    pub(super) close_started: AtomicBool,
}

impl ConnectDeadline {
    pub fn new(expires: Instant) -> Self {
        Self {
            expires,
            committed: AtomicBool::new(false),
            committed_notify: Notify::new(),
            #[cfg(test)]
            admission_started: AtomicBool::new(false),
            #[cfg(test)]
            received: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            close_started: AtomicBool::new(false),
        }
    }

    #[cfg(test)]
    pub(super) fn is_committed(&self) -> bool {
        self.committed.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(super) fn expires(&self) -> Instant {
        self.expires
    }

    /// Called only under the registry lock, before any irreversible mutation.
    pub fn commit(&self) -> bool {
        if Instant::now() >= self.expires {
            return false;
        }
        self.committed.store(true, Ordering::Release);
        self.committed_notify.notify_one();
        true
    }

    pub(super) fn expired(&self) -> bool {
        !self.committed.load(Ordering::Acquire) && Instant::now() >= self.expires
    }

    async fn outcome(&self) -> bool {
        tokio::select! {
            biased;
            _ = sleep_until(self.expires) => self.committed.load(Ordering::Acquire),
            _ = self.committed_notify.notified() => true,
        }
    }
}

pub(super) async fn serve(
    peer_addr: SocketAddr,
    hub: (Vmac, DeviceUuid),
    read: futures_util::stream::SplitStream<WebSocketStream<TlsStream>>,
    write: Arc<Mutex<WsSink>>,
    clients: Clients,
    deadline: Arc<ConnectDeadline>,
    on_heartbeat_ack: impl Fn() + Send,
) {
    let expired = {
        let handler = super::handler::run(
            peer_addr,
            hub,
            read,
            write.clone(),
            clients,
            &deadline,
            on_heartbeat_ack,
        );
        tokio::pin!(handler);
        tokio::select! {
            biased;
            committed = deadline.outcome() => {
                if committed {
                    // The registration commit permanently retires the deadline.
                    // Await the established handler without a timeout, including
                    // Connect-Accept transmission and ordinary identity cleanup.
                    handler.await;
                    false
                } else { true }
            }
            _ = &mut handler => deadline.expired(),
        }
    }; // An expired, unregistered handler is dropped before cleanup I/O.
    if expired {
        let grace = Instant::now() + std::time::Duration::from_secs(1);
        #[cfg(test)]
        deadline.close_started.store(true, Ordering::Release);
        let _ = before(grace, async {
            write.lock().await.send(Message::Close(None)).await
        })
        .await;
    }
}

pub(super) async fn before<T>(
    expires: Instant,
    operation: impl Future<Output = T>,
) -> Result<T, ()> {
    tokio::select! {
        biased;
        _ = sleep_until(expires) => Err(()),
        value = operation => {
            if Instant::now() >= expires { Err(()) } else { Ok(value) }
        }
    }
}
