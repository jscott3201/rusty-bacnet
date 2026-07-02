use tokio::sync::{mpsc, Mutex};

use super::WebSocketPort;
use bacnet_types::error::Error;

/// In-memory loopback WebSocket for unit testing.
pub struct LoopbackWebSocket {
    rx: Mutex<mpsc::Receiver<Vec<u8>>>,
    tx: mpsc::Sender<Vec<u8>>,
}

impl LoopbackWebSocket {
    /// Create a pair of connected loopback WebSockets.
    pub fn pair() -> (Self, Self) {
        let (tx_a, rx_b) = mpsc::channel(64);
        let (tx_b, rx_a) = mpsc::channel(64);
        (
            Self {
                rx: Mutex::new(rx_a),
                tx: tx_a,
            },
            Self {
                rx: Mutex::new(rx_b),
                tx: tx_b,
            },
        )
    }
}

impl WebSocketPort for LoopbackWebSocket {
    async fn send(&self, data: &[u8]) -> Result<(), Error> {
        self.tx
            .send(data.to_vec())
            .await
            .map_err(|_| Error::Encoding("loopback ws send failed".into()))
    }

    async fn recv(&self) -> Result<Vec<u8>, Error> {
        let mut rx = self.rx.lock().await;
        rx.recv()
            .await
            .ok_or_else(|| Error::Encoding("loopback ws channel closed".into()))
    }
}
