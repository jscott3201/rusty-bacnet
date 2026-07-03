//! BACnet/SC WebSocket redial connector helpers.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use bacnet_types::error::Error;
use tracing::warn;

use super::failover::ActiveHub;
use super::{ScTransport, WebSocketPort};

type WebSocketConnectFuture<W> = Pin<Box<dyn Future<Output = Result<W, Error>> + Send>>;
pub(super) type WebSocketConnector<W> = Arc<dyn Fn() -> WebSocketConnectFuture<W> + Send + Sync>;

impl<W: WebSocketPort> ScTransport<W> {
    /// Set a primary WebSocket connector used for reconnect and primary-restore dials.
    ///
    /// The initial connection still uses the WebSocket passed to [`ScTransport::new`].
    /// When reconnecting after a socket-level failure, BACnet/SC needs a fresh
    /// WebSocket/TCP/TLS connection before re-running the Annex AB.3 Connect-Request
    /// handshake; this factory supplies that fresh connection without changing the
    /// pure I/O [`WebSocketPort`] trait.
    pub fn with_connector<F, Fut>(mut self, connector: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<W, Error>> + Send + 'static,
    {
        self.primary_connector = Some(Arc::new(move || Box::pin(connector())));
        self
    }

    /// Set a failover WebSocket connector used when switching to the failover hub.
    ///
    /// This avoids holding a pre-dialed failover socket idle while the primary hub is
    /// active. The connector is called at the failover decision point so the
    /// Connect-Request handshake runs on a fresh WebSocket.
    pub fn with_failover_connector<F, Fut>(mut self, connector: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<W, Error>> + Send + 'static,
    {
        self.failover_connector = Some(Arc::new(move || Box::pin(connector())));
        self
    }
}

pub(super) async fn dial_reconnect_ws<W: WebSocketPort>(
    active_hub: ActiveHub,
    primary_connector: &Option<WebSocketConnector<W>>,
    failover_connector: &Option<WebSocketConnector<W>>,
    timeout_ms: u64,
) -> Result<Option<Arc<W>>, Error> {
    let connector = match active_hub {
        ActiveHub::Primary => primary_connector.as_ref(),
        ActiveHub::Failover => failover_connector.as_ref(),
    };

    match connector {
        Some(connector) => Ok(Some(Arc::new(dial_connector(connector, timeout_ms).await?))),
        None => Ok(None),
    }
}

pub(super) async fn dial_failover_ws<W: WebSocketPort>(
    failover_connector: &Option<WebSocketConnector<W>>,
    failover_ws: &mut Option<Arc<W>>,
    timeout_ms: u64,
) -> Option<Arc<W>> {
    if let Some(connector) = failover_connector {
        match dial_connector(connector, timeout_ms).await {
            Ok(ws) => return Some(Arc::new(ws)),
            Err(e) => {
                warn!(%e, "BACnet/SC failover WebSocket redial failed");
            }
        }
    }

    failover_ws.take()
}

pub(super) async fn dial_connector<W: WebSocketPort>(
    connector: &WebSocketConnector<W>,
    timeout_ms: u64,
) -> Result<W, Error> {
    match tokio::time::timeout(Duration::from_millis(timeout_ms), connector()).await {
        Ok(result) => result,
        Err(_) => Err(Error::Timeout(Duration::from_millis(timeout_ms))),
    }
}
