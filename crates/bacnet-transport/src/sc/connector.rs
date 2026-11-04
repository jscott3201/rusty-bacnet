//! BACnet/SC WebSocket redial connector helpers.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use bacnet_types::error::Error;
use tracing::warn;

#[cfg(feature = "sc-tls")]
use super::ScReconnectConfig;
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

/// The hubs a BACnet/SC node connects to: a primary and, optionally, a
/// failover hub (Annex AB.5.2), with the TLS policy and dial bound they share.
///
/// [`connect`](Self::connect) dials the hub and returns an unstarted
/// [`ScTransport`] that already knows which hub it is on, its connect
/// timeout, and how to redial each hub. Give it the node's own settings
/// (device UUID, heartbeat timing, admission limits) and start it. The
/// client's and the endpoint's SC builders both connect through it (#1946).
///
/// ```no_run
/// use bacnet_transport::sc::{ScHubDialer, ScReconnectConfig};
/// use bacnet_transport::sc_tls::ScNodeTlsConfig;
///
/// async fn connect(tls: ScNodeTlsConfig) -> Result<(), bacnet_types::error::Error> {
///     let transport = ScHubDialer::new("wss://hub.example.com/bacnet", tls)
///         .failover_url("wss://failover.example.com/bacnet")
///         .connect_timeout_ms(3_000)
///         .connect([0x02, 0, 0, 0, 0, 0x01], Some(ScReconnectConfig::default()))
///         .await?
///         .with_device_uuid([0x42; 16]);
///     let _events = transport.connection_events();
///     Ok(())
/// }
/// ```
#[cfg(feature = "sc-tls")]
#[cfg_attr(docsrs, doc(cfg(feature = "sc-tls")))]
#[derive(Clone)]
pub struct ScHubDialer {
    primary: String,
    failover: Option<String>,
    tls: crate::sc_tls::ScNodeTlsConfig,
    connect_timeout_ms: u64,
}

#[cfg(feature = "sc-tls")]
impl ScHubDialer {
    /// Dial the hub at `primary_url` with `tls`; each dial and
    /// Connect-Request is bounded by 10 seconds unless
    /// [`connect_timeout_ms`](Self::connect_timeout_ms) says otherwise.
    pub fn new(primary_url: &str, tls: crate::sc_tls::ScNodeTlsConfig) -> Self {
        Self {
            primary: primary_url.to_owned(),
            failover: None,
            tls,
            connect_timeout_ms: 10_000,
        }
    }

    /// Add a failover hub, dialled with the same TLS policy when the primary
    /// cannot be reached. [`connect`](Self::connect) then needs a reconnect
    /// configuration: without one a lost connection is never made again, so
    /// the failover hub would never be reached or left.
    ///
    /// The node starts on the failover hub when the primary cannot be
    /// dialled, and turns to it when the primary refuses the first
    /// Connect-Request or the reconnect attempts to the primary are used up.
    /// While on the failover hub it keeps trying the primary and returns to
    /// it when it can.
    pub fn failover_url(mut self, url: &str) -> Self {
        self.failover = Some(url.to_owned());
        self
    }

    /// Bound each hub dial (host lookup, TCP, TLS and WebSocket upgrade) and
    /// each Connect-Request, the first and every redial (default 10 000).
    pub fn connect_timeout_ms(mut self, ms: u64) -> Self {
        self.connect_timeout_ms = ms;
        self
    }

    /// Check the settings without any I/O: both URLs as
    /// [`TlsWebSocket::connect`](crate::sc_tls::TlsWebSocket::connect) parses
    /// them, `reconnect`'s delays, and that a failover hub comes with
    /// `reconnect`.
    pub fn validate(&self, reconnect: Option<&ScReconnectConfig>) -> Result<(), Error> {
        use crate::sc_tls::TlsWebSocket;
        if let Some(config) = reconnect {
            config.validate()?;
        }
        if self.failover.is_some() && reconnect.is_none() {
            return Err(Error::Encoding(
                "BACnet/SC failover hub requires reconnect".into(),
            ));
        }
        TlsWebSocket::validate_url(&self.primary)?;
        self.failover
            .as_deref()
            .map_or(Ok(()), TlsWebSocket::validate_url)
    }

    /// Check the settings as [`validate`](Self::validate) does, then dial the
    /// primary hub and, when that fails and a failover hub is set, the
    /// failover hub. Returns an unstarted transport for `local_vmac` over
    /// that connection, set to redial the failover hub and, with
    /// `reconnect`, the primary hub.
    ///
    /// Each dial is bounded by the connect timeout, so a blackholed or
    /// stalled hub cannot hold up the start or the turn to the failover hub.
    /// When both dials fail, the error is the primary's.
    pub async fn connect(
        self,
        local_vmac: crate::sc_frame::Vmac,
        reconnect: Option<ScReconnectConfig>,
    ) -> Result<ScTransport<crate::sc_tls::TlsWebSocket>, Error> {
        use crate::sc_tls::TlsWebSocket;
        self.validate(reconnect.as_ref())?;
        let limit = Duration::from_millis(self.connect_timeout_ms);
        let dial = |url: &str| {
            let (url, tls) = (url.to_owned(), self.tls.clone());
            async move { dial_within(limit, Box::pin(TlsWebSocket::connect(&url, tls))).await }
        };
        // An unreachable primary starts the node on the failover hub; the
        // transport restores the primary once it answers (Annex AB.5.2).
        let (ws, hub) = match dial(&self.primary).await {
            Ok(ws) => (ws, ScActiveHub::Primary),
            Err(primary) => match &self.failover {
                Some(url) => {
                    // No transport (or event stream) exists yet to report it.
                    warn!(error = %primary, "BACnet/SC primary hub dial failed; trying the failover hub");
                    match dial(url).await {
                        Ok(ws) => (ws, ScActiveHub::Failover),
                        Err(_) => return Err(primary),
                    }
                }
                None => return Err(primary),
            },
        };
        let tls = self.tls;
        let redial = |url: String| {
            let tls = tls.clone();
            move || {
                let (url, tls) = (url.clone(), tls.clone());
                async move { TlsWebSocket::connect(&url, tls).await }
            }
        };
        let mut transport = ScTransport::new(ws, local_vmac)
            .with_initial_hub(hub)
            .with_connect_timeout_ms(self.connect_timeout_ms);
        if let Some(url) = self.failover {
            transport = transport.with_failover_connector(redial(url));
        }
        if let Some(config) = reconnect {
            transport = transport
                .with_connector(redial(self.primary))
                .with_reconnect(config);
        }
        Ok(transport)
    }
}
