//! TLS WebSocket implementation for BACnet/SC.
//!
//! Provides [`TlsWebSocket`], a [`WebSocketPort`] backed by `tokio-tungstenite`
//! with `rustls` TLS.  This is the production WebSocket driver used by
//! [`crate::sc::ScTransport`] when connecting to a real BACnet/SC hub.

use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use bacnet_types::error::Error;

use crate::sc::WebSocketPort;

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// A TLS-secured WebSocket connection implementing [`WebSocketPort`].
///
/// Created via [`TlsWebSocket::connect`], which performs the TLS handshake and
/// WebSocket upgrade in one step.
pub struct TlsWebSocket {
    write: Mutex<futures_util::stream::SplitSink<WsStream, Message>>,
    read: Mutex<futures_util::stream::SplitStream<WsStream>>,
}

impl TlsWebSocket {
    /// Connect to a WebSocket endpoint with TLS.
    ///
    /// `url` should be a `wss://` URL.  The provided `tls_config` is used for
    /// the underlying `rustls` TLS handshake.
    pub async fn connect(
        url: &str,
        tls_config: Arc<tokio_rustls::rustls::ClientConfig>,
    ) -> Result<Self, Error> {
        let connector = tokio_tungstenite::Connector::Rustls(tls_config);

        // Build a request that negotiates the BACnet/SC WebSocket subprotocol
        // per ASHRAE 135-2020 Annex AB.
        let uri: tokio_tungstenite::tungstenite::http::Uri = url
            .parse()
            .map_err(|e| Error::Encoding(format!("Invalid WebSocket URL: {e}")))?;
        let request = tokio_tungstenite::tungstenite::ClientRequestBuilder::new(uri)
            .with_sub_protocol("hub.bsc.bacnet.org");

        let (ws_stream, _response) =
            tokio_tungstenite::connect_async_tls_with_config(request, None, false, Some(connector))
                .await
                .map_err(|e| Error::Encoding(format!("WebSocket connect failed: {e}")))?;

        let (write, read) = ws_stream.split();
        Ok(Self {
            write: Mutex::new(write),
            read: Mutex::new(read),
        })
    }
}

impl WebSocketPort for TlsWebSocket {
    async fn send(&self, data: &[u8]) -> Result<(), Error> {
        let mut write = self.write.lock().await;
        write
            .send(Message::Binary(data.to_vec().into()))
            .await
            .map_err(|e| Error::Encoding(format!("WebSocket send failed: {e}")))
    }

    async fn recv(&self) -> Result<Vec<u8>, Error> {
        let mut read = self.read.lock().await;
        loop {
            match read.next().await {
                Some(Ok(Message::Binary(data))) => return Ok(data.to_vec()),
                Some(Ok(Message::Close(_))) => {
                    return Err(Error::Encoding("WebSocket closed".into()));
                }
                Some(Ok(_)) => continue, // skip Text, Ping, Pong, Frame
                Some(Err(e)) => {
                    return Err(Error::Encoding(format!("WebSocket recv error: {e}")));
                }
                None => {
                    return Err(Error::Encoding("WebSocket stream ended".into()));
                }
            }
        }
    }
}
