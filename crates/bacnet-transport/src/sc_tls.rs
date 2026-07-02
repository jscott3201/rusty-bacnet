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
use crate::sc_frame::BACNET_SC_HUB_SUBPROTOCOL;

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
    ///
    /// Per spec AB.7.4, the `tls_config` should be configured for TLS 1.3 only:
    /// ```ignore
    /// ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
    /// ```
    pub async fn connect(
        url: &str,
        tls_config: Arc<tokio_rustls::rustls::ClientConfig>,
    ) -> Result<Self, Error> {
        let connector = tokio_tungstenite::Connector::Rustls(tls_config);

        let uri = parse_wss_uri(url)?;
        let request = tokio_tungstenite::tungstenite::ClientRequestBuilder::new(uri)
            .with_sub_protocol(BACNET_SC_HUB_SUBPROTOCOL);

        let (ws_stream, response) =
            tokio_tungstenite::connect_async_tls_with_config(request, None, false, Some(connector))
                .await
                .map_err(|e| Error::Encoding(format!("WebSocket connect failed: {e}")))?;
        verify_hub_subprotocol(&response)?;

        let (write, read) = ws_stream.split();
        Ok(Self {
            write: Mutex::new(write),
            read: Mutex::new(read),
        })
    }
}

fn parse_wss_uri(url: &str) -> Result<tokio_tungstenite::tungstenite::http::Uri, Error> {
    let uri: tokio_tungstenite::tungstenite::http::Uri = url
        .parse()
        .map_err(|e| Error::Encoding(format!("Invalid WebSocket URL: {e}")))?;

    if !uri
        .scheme_str()
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("wss"))
    {
        return Err(Error::Encoding(
            "BACnet/SC WebSocket URI must use wss scheme".into(),
        ));
    }

    if uri.host().map(str::is_empty).unwrap_or(true) {
        return Err(Error::Encoding(
            "BACnet/SC WebSocket URI must include a hub host".into(),
        ));
    }

    Ok(uri)
}

fn verify_hub_subprotocol(
    response: &tokio_tungstenite::tungstenite::handshake::client::Response,
) -> Result<(), Error> {
    let selected = response
        .headers()
        .get("Sec-WebSocket-Protocol")
        .and_then(|value| value.to_str().ok());

    if selected == Some(BACNET_SC_HUB_SUBPROTOCOL) {
        Ok(())
    } else {
        Err(Error::Encoding(format!(
            "BACnet/SC hub WebSocket subprotocol {BACNET_SC_HUB_SUBPROTOCOL} was not accepted"
        )))
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
        loop {
            // Read one message under the read lock, then drop it before
            // acquiring write (avoids read→write lock ordering deadlock).
            let msg = {
                let mut read = self.read.lock().await;
                read.next().await
            };
            match msg {
                Some(Ok(Message::Binary(data))) => return Ok(data.to_vec()),
                Some(Ok(Message::Close(_))) => {
                    return Err(Error::Encoding("WebSocket closed".into()));
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {
                    continue;
                }
                Some(Ok(_)) => {
                    // Non-binary data frames: close with 1003
                    let mut w = self.write.lock().await;
                    let _ = w
                        .send(Message::Close(Some(
                            tokio_tungstenite::tungstenite::protocol::CloseFrame {
                                code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Unsupported,
                                reason: "BACnet/SC requires binary frames".into(),
                            },
                        )))
                        .await;
                    return Err(Error::Encoding(
                        "non-binary WebSocket frame received".into(),
                    ));
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_wss_uri_accepts_secure_websocket_scheme() {
        assert!(parse_wss_uri("wss://hub.example.com:443").is_ok());
    }

    #[test]
    fn parse_wss_uri_preserves_configured_hub_authority_path_and_query() {
        let uri = parse_wss_uri("wss://hub.example.com:47808/.bacnet/sc?profile=primary").unwrap();

        assert_eq!(uri.scheme_str(), Some("wss"));
        assert_eq!(uri.host(), Some("hub.example.com"));
        assert_eq!(uri.port_u16(), Some(47808));
        assert_eq!(
            uri.path_and_query().map(|path| path.as_str()),
            Some("/.bacnet/sc?profile=primary")
        );
    }

    #[test]
    fn parse_wss_uri_rejects_plain_websocket_scheme() {
        let err = parse_wss_uri("ws://hub.example.com:80").unwrap_err();
        assert!(err.to_string().contains("wss"));
    }

    #[test]
    fn parse_wss_uri_rejects_non_websocket_scheme() {
        let err = parse_wss_uri("https://hub.example.com").unwrap_err();
        assert!(err.to_string().contains("wss"));
    }

    #[test]
    fn parse_wss_uri_rejects_missing_hub_host() {
        let err = parse_wss_uri("wss://:443").unwrap_err();
        assert!(err.to_string().contains("hub host"));
    }

    #[test]
    fn parse_wss_uri_rejects_malformed_hub_uri() {
        let err = parse_wss_uri("wss://[::1").unwrap_err();
        assert!(err.to_string().contains("Invalid WebSocket URL"));
    }

    #[test]
    fn verify_hub_subprotocol_accepts_selected_hub_protocol() {
        let response = tokio_tungstenite::tungstenite::http::Response::builder()
            .header("Sec-WebSocket-Protocol", BACNET_SC_HUB_SUBPROTOCOL)
            .body(None)
            .unwrap();

        assert!(verify_hub_subprotocol(&response).is_ok());
    }

    #[test]
    fn verify_hub_subprotocol_rejects_missing_or_wrong_protocol() {
        let missing = tokio_tungstenite::tungstenite::http::Response::builder()
            .body(None)
            .unwrap();
        assert!(verify_hub_subprotocol(&missing).is_err());

        let wrong = tokio_tungstenite::tungstenite::http::Response::builder()
            .header("Sec-WebSocket-Protocol", "dc.bsc.bacnet.org")
            .body(None)
            .unwrap();
        assert!(verify_hub_subprotocol(&wrong).is_err());
    }
}
