//! Direct accepted-socket adapter, also used by deterministic write seams.
use futures_util::{SinkExt, StreamExt};

/// Minimal WebSocket surface used by the acceptor handshake and NPDU loop.
pub(super) trait DirectWs {
    type Read: DirectWsRead;
    async fn send_data(&mut self, data: &[u8]) -> Result<(), ()>;
    /// Send a close frame with `status`, or one with no status (#2520).
    async fn send_close(&mut self, status: Option<u16>) -> Result<(), ()>;
}

pub(super) enum DirectFrame {
    Binary(Vec<u8>),
    Control,
    /// A text (or other non-binary) data frame: the connection ends with a
    /// 1003 close (Annex AB.7.5.3, #2520).
    NonBinary,
}

/// Close with `status` within the close grace; the caller then drops the
/// connection (#2520). 1000 ends it in order, and no status marks the
/// peer's fault, as the hub's closes do.
pub(super) async fn close_with<W: DirectWs>(write: &mut W, status: Option<u16>) {
    let _ = tokio::time::timeout(crate::sc::CLOSE_GRACE, write.send_close(status)).await;
}

/// Answer a data frame that is not binary with a 1003 close, as the hub
/// does (Annex AB.7.5.3, #2520).
pub(super) async fn refuse_non_binary<W: DirectWs>(write: &mut W) {
    close_with(write, Some(crate::sc::CLOSE_DATA_NOT_ACCEPTED)).await;
}

/// Resolves once `signal` reads true. A sender gone without setting it
/// never raises it.
pub(super) async fn raised(signal: &mut tokio::sync::watch::Receiver<bool>) {
    while !*signal.borrow_and_update() {
        if signal.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

pub(super) trait DirectWsRead {
    /// Return one frame so ignored controls still yield a scheduling turn.
    async fn next_frame(&mut self) -> Option<Result<DirectFrame, String>>;

    /// Handshake-only filtering remains inside the caller's Connect timeout.
    /// Skips controls: the frame is binary or [`DirectFrame::NonBinary`].
    async fn next_data(&mut self) -> Option<Result<DirectFrame, String>> {
        loop {
            match self.next_frame().await? {
                Ok(DirectFrame::Control) => tokio::task::yield_now().await,
                other => return Some(other),
            }
        }
    }
}

type TlsWsStream =
    tokio_tungstenite::WebSocketStream<tokio_rustls::server::TlsStream<tokio::net::TcpStream>>;

impl DirectWs
    for futures_util::stream::SplitSink<TlsWsStream, tokio_tungstenite::tungstenite::Message>
{
    type Read = futures_util::stream::SplitStream<TlsWsStream>;
    async fn send_data(&mut self, data: &[u8]) -> Result<(), ()> {
        self.send(tokio_tungstenite::tungstenite::Message::Binary(
            data.to_vec().into(),
        ))
        .await
        .map_err(|_| ())
    }
    /// 1000 when the acceptor ends the connection on purpose (an idle
    /// timeout, a retirement, an answered Disconnect-Request), as the hub's
    /// orderly closes do (#2463), and 1003 after a non-binary frame (#2520).
    async fn send_close(&mut self, status: Option<u16>) -> Result<(), ()> {
        use tokio_tungstenite::tungstenite::protocol::CloseFrame;
        let frame = status.map(|code| CloseFrame {
            code: code.into(),
            reason: "".into(),
        });
        self.send(tokio_tungstenite::tungstenite::Message::Close(frame))
            .await
            .map_err(|_| ())
    }
}

impl<S, E> DirectWsRead for S
where
    S: futures_util::Stream<Item = Result<tokio_tungstenite::tungstenite::Message, E>> + Unpin,
    E: std::fmt::Display,
{
    async fn next_frame(&mut self) -> Option<Result<DirectFrame, String>> {
        match self.next().await? {
            Ok(tokio_tungstenite::tungstenite::Message::Binary(data)) => {
                Some(Ok(DirectFrame::Binary(data.to_vec())))
            }
            Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => None,
            Ok(
                tokio_tungstenite::tungstenite::Message::Ping(_)
                | tokio_tungstenite::tungstenite::Message::Pong(_),
            ) => Some(Ok(DirectFrame::Control)),
            Ok(_) => Some(Ok(DirectFrame::NonBinary)),
            Err(e) => Some(Err(e.to_string())),
        }
    }
}
