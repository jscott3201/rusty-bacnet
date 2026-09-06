//! A captured relay target owns its retirement predicate and notification.
//! Caller-selected timeouts remain outside this boundary; cancelling a send
//! does not roll back bytes buffered by tungstenite and must not cause retry.

use super::*;
use std::future::Future;
use tokio_tungstenite::tungstenite::{error::ProtocolError, Error};

impl HubRelaySink {
    pub(super) fn capture(vmac: Vmac, client: &HubClient) -> Self {
        Self {
            vmac,
            sink: client.sink.clone(),
            closed: client.closed.clone(),
            notify: client.close_notify.clone(),
        }
    }
}

// Pinned tungstenite 0.29 write contract: these errors make outbound use
// terminal. Capacity/WriteBufferFull and WouldBlock do not retire a connection.
pub(super) fn terminal(error: &Error) -> bool {
    matches!(
        error,
        Error::ConnectionClosed
            | Error::AlreadyClosed
            | Error::Protocol(ProtocolError::SendAfterClosing)
    ) || matches!(error, Error::Io(error) if error.kind() != std::io::ErrorKind::WouldBlock)
}

pub(super) trait RelayIo: Sync {
    fn send(
        &self,
        sink: &mut WsSink,
        frame: Message,
    ) -> impl Future<Output = Result<(), Error>> + Send;
}

pub(super) struct SocketIo;
impl RelayIo for SocketIo {
    async fn send(&self, sink: &mut WsSink, frame: Message) -> Result<(), Error> {
        sink.send(frame).await
    }
}

pub(super) async fn send(
    target: &HubRelaySink,
    clients: &Clients,
    frame: Message,
    io: &impl RelayIo,
) -> Result<(), Error> {
    let result = tokio::select! {
        biased;
        _ = super::retirement::wait(&target.closed, &target.notify) => return Ok(()),
        result = async {
            let mut sink = target.sink.lock().await;
            if target.closed.load(Ordering::Acquire) { return Ok(()); }
            io.send(&mut sink, frame).await
        } => result,
    };
    if result.as_ref().is_err_and(terminal) {
        // Signal the captured identity even when its VMAC has been reused.
        // Release sink ownership before acquiring the registry for removal.
        target.closed.store(true, Ordering::Release);
        super::retirement::wake(&target.notify);
        let mut map = clients.lock().await;
        if map
            .get(&target.vmac)
            .is_some_and(|client| Arc::ptr_eq(&client.sink, &target.sink))
        {
            map.remove(&target.vmac);
        }
    }
    result
}
