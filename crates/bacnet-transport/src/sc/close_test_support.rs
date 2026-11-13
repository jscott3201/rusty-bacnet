//! In-memory sockets that record the closes the transport asks of them,
//! for the close tests on tokio's paused clock (#2520).
use std::future::pending;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use super::{LoopbackWebSocket, WebSocketPort};
use bacnet_types::error::Error;

pub(crate) type Closes = Arc<StdMutex<Vec<Option<u16>>>>;

/// A loopback socket that records the closes asked of it. Once `hang_send`
/// or `hang_close` is set, that operation never finishes, as against a hub
/// that stopped reading.
pub(crate) struct Recorder {
    inner: LoopbackWebSocket,
    closes: Closes,
    hang_send: Arc<AtomicBool>,
    hang_close: Arc<AtomicBool>,
}

/// The test's handles on a [`Recorder`].
pub(crate) struct Probe {
    pub(crate) hub: LoopbackWebSocket,
    pub(crate) closes: Closes,
    pub(crate) hang_send: Arc<AtomicBool>,
    pub(crate) hang_close: Arc<AtomicBool>,
}

impl Probe {
    pub(crate) fn closes(&self) -> Vec<Option<u16>> {
        self.closes.lock().unwrap().clone()
    }
}

pub(crate) fn recorder() -> (Recorder, Probe) {
    let (inner, hub) = LoopbackWebSocket::pair();
    let closes = Closes::default();
    let (hang_send, hang_close) = (Arc::default(), Arc::default());
    let probe = Probe {
        hub,
        closes: closes.clone(),
        hang_send: Arc::clone(&hang_send),
        hang_close: Arc::clone(&hang_close),
    };
    let socket = Recorder {
        inner,
        closes,
        hang_send,
        hang_close,
    };
    (socket, probe)
}

impl WebSocketPort for Recorder {
    async fn send(&self, data: &[u8]) -> Result<(), Error> {
        if self.hang_send.load(Ordering::Acquire) {
            pending::<()>().await;
        }
        self.inner.send(data).await
    }

    async fn recv(&self) -> Result<Vec<u8>, Error> {
        self.inner.recv().await
    }

    async fn close(&self, status: Option<u16>) -> Result<(), Error> {
        self.closes.lock().unwrap().push(status);
        if self.hang_close.load(Ordering::Acquire) {
            pending::<()>().await;
        }
        Ok(())
    }

    fn awaits_disconnect_ack(&self) -> bool {
        true
    }
}

/// Step the paused clock 1 ms at a time until `done` holds, failing after
/// 5 s of virtual time with `what` in the message. For a close that comes
/// after a timer, such as a connect wait or the close grace.
pub(crate) async fn eventually(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "{what}: not in time"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}
