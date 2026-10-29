//! BACnet/SC WebSocket redial connector helpers.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use bacnet_types::error::Error;
use tracing::warn;

use super::{ScActiveHub, ScConnectError, ScDialStage, ScTransport, WebSocketPort};

tokio::task_local! {
    // The step the dial polled in this scope has reached (#1839). Shared
    // with the scope's owner, which reads it once the time is up.
    static DIAL_STAGE: Arc<AtomicU8>;
}

/// Record that the dial in progress has reached `stage`. Outside a
/// [`dial_within`] scope it does nothing.
#[cfg(feature = "sc-tls")]
pub(crate) fn enter_dial_stage(stage: ScDialStage) {
    let _ = DIAL_STAGE.try_with(|at| at.store(stage.code(), Ordering::Relaxed));
}

/// Run a hub dial (host lookup, TCP, TLS and WebSocket upgrade), bounded by
/// `timeout`.
///
/// When the time runs out, the error is [`ScConnectError::DialTimeout`],
/// naming the step the dial was in when the dialler reports its steps, as
/// the `sc-tls` feature's `TlsWebSocket::connect` does. The
/// transport bounds its own redials this way; a caller that dials the first
/// WebSocket itself can too, so that a stalled dial reads the same.
///
/// ```
/// use std::time::Duration;
/// use bacnet_transport::sc::{dial_within, ScConnectError};
///
/// # #[tokio::main(flavor = "current_thread", start_paused = true)]
/// # async fn main() {
/// let stalled = std::future::pending::<Result<(), bacnet_types::error::Error>>();
/// let error = dial_within(Duration::from_secs(1), stalled).await.unwrap_err();
/// assert!(matches!(
///     ScConnectError::from_error(&error),
///     Some(ScConnectError::DialTimeout { stage: None, .. })
/// ));
/// # }
/// ```
pub async fn dial_within<W>(
    timeout: Duration,
    dial: impl Future<Output = Result<W, Error>>,
) -> Result<W, Error> {
    let stage = Arc::new(AtomicU8::new(0));
    match tokio::time::timeout(timeout, DIAL_STAGE.scope(Arc::clone(&stage), dial)).await {
        Ok(result) => result,
        Err(_) => Err(ScConnectError::DialTimeout {
            timeout,
            stage: ScDialStage::from_code(stage.load(Ordering::Relaxed)),
        }
        .into_bacnet_error()),
    }
}

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
    active_hub: ScActiveHub,
    primary_connector: &Option<WebSocketConnector<W>>,
    failover_connector: &Option<WebSocketConnector<W>>,
    timeout_ms: u64,
) -> Result<Option<Arc<W>>, Error> {
    let connector = match active_hub {
        ScActiveHub::Primary => primary_connector.as_ref(),
        ScActiveHub::Failover => failover_connector.as_ref(),
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
    dial_within(Duration::from_millis(timeout_ms), connector()).await
}
