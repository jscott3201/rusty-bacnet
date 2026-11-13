//! Direct accepted-socket adapter, also used by deterministic write seams.
use futures_util::{SinkExt, StreamExt};

/// Minimal WebSocket surface used by the acceptor handshake and NPDU loop.
pub(super) trait DirectWs {
    type Read: DirectWsRead;
    async fn send_data(&mut self, data: &[u8]) -> Result<(), ()>;
    async fn send_close(&mut self) -> Result<(), ()>;
}

pub(super) enum DirectFrame {
    Binary(Vec<u8>),
    Control,
}

pub(super) trait DirectWsRead {
    /// Return one frame so ignored controls still yield a scheduling turn.
    async fn next_frame(&mut self) -> Option<Result<DirectFrame, String>>;

    /// Handshake-only filtering remains inside the caller's Connect timeout.
    async fn next_data(&mut self) -> Option<Result<Vec<u8>, String>> {
        loop {
            match self.next_frame().await? {
                Ok(DirectFrame::Binary(data)) => return Some(Ok(data)),
                Ok(DirectFrame::Control) => tokio::task::yield_now().await,
                Err(error) => return Some(Err(error)),
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
    /// Every caller ends the connection on purpose (an idle timeout or a
    /// retirement after Disconnect-Request), so the frame says 1000, as the
    /// hub's orderly closes do (#2463).
    async fn send_close(&mut self) -> Result<(), ()> {
        use tokio_tungstenite::tungstenite::protocol::{CloseFrame, frame::coding::CloseCode};
        self.send(tokio_tungstenite::tungstenite::Message::Close(Some(
            CloseFrame {
                code: CloseCode::Normal,
                reason: "".into(),
            },
        )))
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
            Ok(_) => Some(Err("non-binary".into())),
            Err(e) => Some(Err(e.to_string())),
        }
    }
}
