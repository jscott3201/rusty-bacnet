//! TLS WebSocket implementation for BACnet/SC.
//!
//! Provides [`TlsWebSocket`], a [`WebSocketPort`] backed by `tokio-tungstenite`
//! with `rustls` TLS.  This is the production WebSocket driver used by
//! [`crate::sc::ScTransport`] when connecting to a real BACnet/SC hub.

mod tls_config;
pub use tls_config::ScNodeTlsConfig;

pub(crate) mod direct_accept;
pub use direct_accept::{DirectAcceptConfig, DirectListener, DIRECT_ACCEPT_MAX_ESTABLISHED_PEERS};

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_tungstenite::tungstenite::Error as TungsteniteError;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

use bacnet_types::error::Error;

use crate::sc::{ScConnectError, ScWebSocketErrorKind, WebSocketPort};
use crate::sc_frame::{BACNET_SC_DIRECT_SUBPROTOCOL, BACNET_SC_HUB_SUBPROTOCOL};

type WsStream = WebSocketStream<tokio_rustls::client::TlsStream<TcpStream>>;

/// Turn off Nagle's algorithm on an SC TCP stream before TLS. SC traffic is
/// small WebSocket messages; with Nagle on, a message written before the
/// previous one is acknowledged waits for that ACK, which a peer with nothing
/// to send back can delay by tens to hundreds of milliseconds, depending on the
/// peer's OS (#900). A failure only costs latency, so it is logged and the
/// connection proceeds.
pub(crate) fn disable_nagle(stream: &TcpStream) {
    if let Err(error) = stream.set_nodelay(true) {
        let peer = stream.peer_addr().ok();
        tracing::debug!(%error, ?peer, "SC TCP stream keeps Nagle's algorithm");
    }
}

/// A TLS-secured WebSocket connection implementing [`WebSocketPort`].
///
/// Created via [`TlsWebSocket::connect`], which performs the TLS handshake and
/// WebSocket upgrade in one step.
pub struct TlsWebSocket {
    pub(crate) verified_leaf: [u8; 32],
    pub(crate) peer_address: std::net::SocketAddr,
    write: Mutex<futures_util::stream::SplitSink<WsStream, Message>>,
    read: Mutex<futures_util::stream::SplitStream<WsStream>>,
}

impl TlsWebSocket {
    // One physical WebSocket frame per worker turn, including controls. The
    // worker owns application/control writes; this adapter never takes write.
    pub(crate) async fn direct_frame(
        &self,
    ) -> Result<crate::sc::direct_socket::DirectFrame, Error> {
        direct_frame(&mut *self.read.lock().await).await
    }

    /// Connect to a WebSocket endpoint with TLS.
    ///
    /// `url` must be a `wss://` URL. The validated local policy supplies explicit
    /// trust and operational credentials; see [`ScNodeTlsConfig`] for the
    /// certificate-request and normal resumption limits.
    ///
    /// ```
    /// use bacnet_transport::sc_tls::{ScNodeTlsConfig, TlsWebSocket};
    /// async fn connect(url: &str, tls: ScNodeTlsConfig)
    ///     -> Result<TlsWebSocket, bacnet_types::error::Error> {
    ///     TlsWebSocket::connect(url, tls).await
    /// }
    /// ```
    ///
    /// ```compile_fail,E0308
    /// use bacnet_transport::sc_tls::TlsWebSocket;
    /// async fn raw(config: std::sync::Arc<rustls::ClientConfig>) {
    ///     let _ = TlsWebSocket::connect("wss://localhost", config).await;
    /// }
    /// ```
    pub async fn connect(url: &str, tls_config: ScNodeTlsConfig) -> Result<Self, Error> {
        boxed(|| Self::connect_with_subprotocol(url, tls_config, BACNET_SC_HUB_SUBPROTOCOL)).await
    }

    /// Dial a direct-connection peer `wss` URI with node operational credentials.
    ///
    /// This is the dial-out half of an optional direct connection: the caller
    /// supplies a peer URI (statically configured or previously discovered),
    /// and this performs the TCP dial, mutual-TLS handshake, and WebSocket
    /// upgrade offering only the direct subprotocol. Peer validation reuses
    /// the [`ScNodeTlsConfig`] policy — explicit CA trust, operational
    /// certificate checks during TLS, TLS 1.3 only, and URI-host server-name
    /// binding — exactly as for hub dial. No BACnet identity (VMAC/UUID)
    /// binding is performed here; that belongs to the later connection
    /// exchange. Discovery triggering, routing over the connection, inbound
    /// (accept-side) paths, and hub/failover interaction are out of scope.
    pub async fn connect_direct(url: &str, tls_config: ScNodeTlsConfig) -> Result<Self, Error> {
        boxed(|| Self::connect_with_subprotocol(url, tls_config, BACNET_SC_DIRECT_SUBPROTOCOL))
            .await
    }

    // Each stage's future is boxed: the tokio-tungstenite handshake below
    // already takes about 125 KB of stack in a debug build, and unboxed, these
    // futures made this frame and connect's 51 KB rather than 24 KB (#953).
    async fn connect_with_subprotocol(
        url: &str,
        tls_config: ScNodeTlsConfig,
        subprotocol: &'static str,
    ) -> Result<Self, Error> {
        let uri = parse_wss_uri(url)?;
        let addr = tcp_addr_from_uri(&uri)?;
        let server_name = tls_server_name_from_uri(&uri)?;
        let request = tokio_tungstenite::tungstenite::ClientRequestBuilder::new(uri)
            .with_sub_protocol(subprotocol);

        let socket = boxed(|| crate::tcp_connect::connect(&addr))
            .await
            .map_err(|e| {
                ScConnectError::WebSocket {
                    kind: ScWebSocketErrorKind::TcpDial,
                    message: format!("WebSocket TCP dial to {addr} failed: {e}"),
                }
                .into_bacnet_error_with_io_kind(e.kind())
            })?;

        disable_nagle(&socket);
        let peer_address = socket.peer_addr().map_err(Error::Transport)?;
        let tls_stream = boxed(|| tls_config.into_connector().connect(server_name, socket))
            .await
            .map_err(|e| {
                ScConnectError::WebSocket {
                    kind: ScWebSocketErrorKind::TlsHandshake,
                    message: format!("WebSocket TLS handshake with {addr} failed: {e}"),
                }
                .into_bacnet_error()
            })?;

        let verified_leaf =
            direct_accept::verified_leaf_sha256(tls_stream.get_ref().1.peer_certificates())
                .ok_or_else(|| {
                    Error::Encoding("verified TLS peer has no leaf certificate".into())
                })?;
        let (ws_stream, response) = boxed(|| {
            tokio_tungstenite::client_async_with_config(
                request,
                tls_stream,
                // Public local capacities remain mutable u16 values. The adapter
                // bounds its first Vec copy; protocol layers apply current limits.
                Some(crate::sc_limits::websocket(u16::MAX as usize)),
            )
        })
        .await
        .map_err(map_websocket_upgrade_error)?;
        if subprotocol == BACNET_SC_DIRECT_SUBPROTOCOL {
            verify_direct_subprotocol(&response)?;
        } else {
            verify_hub_subprotocol(&response)?;
        }

        let (write, read) = ws_stream.split();
        Ok(Self {
            verified_leaf,
            peer_address,
            write: Mutex::new(write),
            read: Mutex::new(read),
        })
    }
}

/// Create the future `make` returns and move it to the heap, so awaiting it
/// costs the caller's state and debug-build poll frame a pointer rather than
/// the whole future (#953). For connection setup and per-connection tasks,
/// not per-frame paths.
pub(crate) fn boxed<F: std::future::Future>(make: impl FnOnce() -> F) -> std::pin::Pin<Box<F>> {
    Box::pin(make())
}

// A finite unit of receive scheduling, shared with deterministic frame tests.
async fn direct_frame<S, E>(read: &mut S) -> Result<crate::sc::direct_socket::DirectFrame, Error>
where
    S: futures_util::Stream<Item = Result<Message, E>> + Unpin,
    E: std::fmt::Display,
{
    use crate::sc::direct_socket::DirectFrame;
    match read.next().await {
        Some(Ok(Message::Binary(data))) => Ok(DirectFrame::Binary(data.to_vec())),
        Some(Ok(Message::Ping(_) | Message::Pong(_))) => Ok(DirectFrame::Control),
        Some(Err(error)) => Err(Error::Transport(std::io::Error::other(error.to_string()))),
        _ => Err(Error::Transport(std::io::Error::new(
            std::io::ErrorKind::ConnectionAborted,
            "direct WebSocket ended or carried non-binary data",
        ))),
    }
}

fn parse_wss_uri(url: &str) -> Result<tokio_tungstenite::tungstenite::http::Uri, Error> {
    let uri: tokio_tungstenite::tungstenite::http::Uri = url.parse().map_err(|e| {
        ScConnectError::WebSocket {
            kind: ScWebSocketErrorKind::InvalidUrl,
            message: format!("Invalid WebSocket URL: {e}"),
        }
        .into_bacnet_error()
    })?;

    if !uri
        .scheme_str()
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("wss"))
    {
        return Err(ScConnectError::WebSocket {
            kind: ScWebSocketErrorKind::UnsupportedScheme,
            message: "BACnet/SC WebSocket URI must use wss scheme".into(),
        }
        .into_bacnet_error());
    }

    if uri.host().map(str::is_empty).unwrap_or(true) {
        return Err(ScConnectError::WebSocket {
            kind: ScWebSocketErrorKind::MissingHubHost,
            message: "BACnet/SC WebSocket URI must include a hub host".into(),
        }
        .into_bacnet_error());
    }

    Ok(uri)
}

fn tcp_addr_from_uri(uri: &tokio_tungstenite::tungstenite::http::Uri) -> Result<String, Error> {
    let host = uri.host().filter(|host| !host.is_empty()).ok_or_else(|| {
        ScConnectError::WebSocket {
            kind: ScWebSocketErrorKind::MissingHubHost,
            message: "BACnet/SC WebSocket URI must include a hub host".into(),
        }
        .into_bacnet_error()
    })?;
    let port = uri.port_u16().unwrap_or(443);
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    Ok(format!("{host}:{port}"))
}

fn tls_server_name_from_uri(
    uri: &tokio_tungstenite::tungstenite::http::Uri,
) -> Result<ServerName<'static>, Error> {
    let host = uri.host().filter(|host| !host.is_empty()).ok_or_else(|| {
        ScConnectError::WebSocket {
            kind: ScWebSocketErrorKind::MissingHubHost,
            message: "BACnet/SC WebSocket URI must include a hub host".into(),
        }
        .into_bacnet_error()
    })?;

    let host = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host);

    ServerName::try_from(host.to_owned()).map_err(|e| {
        ScConnectError::WebSocket {
            kind: ScWebSocketErrorKind::InvalidUrl,
            message: format!("Invalid WebSocket TLS server name: {e}"),
        }
        .into_bacnet_error()
    })
}

fn map_websocket_upgrade_error(error: TungsteniteError) -> Error {
    let kind = match &error {
        TungsteniteError::Io(_)
        | TungsteniteError::Tls(_)
        | TungsteniteError::Protocol(_)
        | TungsteniteError::Http(_)
        | TungsteniteError::HttpFormat(_) => ScWebSocketErrorKind::WebSocketHandshake,
        TungsteniteError::Url(_) => ScWebSocketErrorKind::InvalidUrl,
        _ => ScWebSocketErrorKind::WebSocketHandshake,
    };
    ScConnectError::WebSocket {
        kind,
        message: format!("WebSocket upgrade failed: {error}"),
    }
    .into_bacnet_error()
}

#[cfg(test)]
fn map_websocket_handshake_error(error: TungsteniteError) -> Error {
    let kind = match &error {
        TungsteniteError::Io(_) => ScWebSocketErrorKind::WebSocketHandshake,
        TungsteniteError::Tls(_) => ScWebSocketErrorKind::TlsHandshake,
        TungsteniteError::Protocol(_)
        | TungsteniteError::Http(_)
        | TungsteniteError::HttpFormat(_) => ScWebSocketErrorKind::WebSocketHandshake,
        TungsteniteError::Url(_) => ScWebSocketErrorKind::InvalidUrl,
        _ => ScWebSocketErrorKind::WebSocketHandshake,
    };
    ScConnectError::WebSocket {
        kind,
        message: format!("WebSocket connect failed: {error}"),
    }
    .into_bacnet_error()
}

fn verify_hub_subprotocol(
    response: &tokio_tungstenite::tungstenite::handshake::client::Response,
) -> Result<(), Error> {
    verify_selected_subprotocol(
        response,
        BACNET_SC_HUB_SUBPROTOCOL,
        "hub",
        ScWebSocketErrorKind::HubSubprotocol,
    )
}

fn verify_direct_subprotocol(
    response: &tokio_tungstenite::tungstenite::handshake::client::Response,
) -> Result<(), Error> {
    verify_selected_subprotocol(
        response,
        BACNET_SC_DIRECT_SUBPROTOCOL,
        "direct",
        ScWebSocketErrorKind::DirectSubprotocol,
    )
}

fn verify_selected_subprotocol(
    response: &tokio_tungstenite::tungstenite::handshake::client::Response,
    expected: &str,
    role: &str,
    kind: ScWebSocketErrorKind,
) -> Result<(), Error> {
    let selected = response
        .headers()
        .get("Sec-WebSocket-Protocol")
        .and_then(|value| value.to_str().ok());

    if selected == Some(expected) {
        Ok(())
    } else {
        Err(ScConnectError::WebSocket {
            kind,
            message: format!("BACnet/SC {role} WebSocket subprotocol {expected} was not accepted"),
        }
        .into_bacnet_error())
    }
}

impl WebSocketPort for TlsWebSocket {
    async fn send(&self, data: &[u8]) -> Result<(), Error> {
        let mut write = self.write.lock().await;
        write
            .send(Message::Binary(data.to_vec().into()))
            .await
            .map_err(|e| {
                ScConnectError::WebSocket {
                    kind: ScWebSocketErrorKind::Send,
                    message: format!("WebSocket send failed: {e}"),
                }
                .into_bacnet_error()
            })
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
                    return Err(ScConnectError::WebSocket {
                        kind: ScWebSocketErrorKind::Closed,
                        message: "WebSocket closed".into(),
                    }
                    .into_bacnet_error());
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
                    return Err(ScConnectError::WebSocket {
                        kind: ScWebSocketErrorKind::NonBinaryFrame,
                        message: "non-binary WebSocket frame received".into(),
                    }
                    .into_bacnet_error());
                }
                Some(Err(e)) => {
                    return Err(ScConnectError::WebSocket {
                        kind: ScWebSocketErrorKind::Receive,
                        message: format!("WebSocket recv error: {e}"),
                    }
                    .into_bacnet_error());
                }
                None => {
                    return Err(ScConnectError::WebSocket {
                        kind: ScWebSocketErrorKind::StreamEnded,
                        message: "WebSocket stream ended".into(),
                    }
                    .into_bacnet_error());
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "sc_tls/node_tls_tests.rs"]
mod node_tls_tests;

#[cfg(test)]
#[path = "sc_tls/direct_dial_tests.rs"]
mod direct_dial_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::port_ownership::connect_loopback;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_rustls::rustls::pki_types::pem::PemObject;
    use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
    use tokio_rustls::TlsAcceptor;

    #[tokio::test]
    async fn disable_nagle_sets_tcp_nodelay_on_both_ends() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dialed = connect_loopback(listener.local_addr().unwrap()).await;
        let (accepted, _) = listener.accept().await.unwrap();
        for stream in [&dialed, &accepted] {
            assert!(!stream.nodelay().unwrap(), "Nagle is on by default");
            disable_nagle(stream);
            assert!(stream.nodelay().unwrap());
        }
    }

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
        assert_eq!(
            ScConnectError::from_error(&err),
            Some(&ScConnectError::WebSocket {
                kind: ScWebSocketErrorKind::UnsupportedScheme,
                message: "BACnet/SC WebSocket URI must use wss scheme".into(),
            })
        );
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
    fn tls_server_name_from_uri_accepts_ipv6_literal() {
        let uri = parse_wss_uri("wss://[::1]:47808").unwrap();
        assert!(matches!(
            tls_server_name_from_uri(&uri).unwrap(),
            ServerName::IpAddress(_)
        ));
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
        let err = verify_hub_subprotocol(&missing).unwrap_err();
        assert!(matches!(
            ScConnectError::from_error(&err),
            Some(ScConnectError::WebSocket {
                kind: ScWebSocketErrorKind::HubSubprotocol,
                ..
            })
        ));

        let wrong = tokio_tungstenite::tungstenite::http::Response::builder()
            .header("Sec-WebSocket-Protocol", "dc.bsc.bacnet.org")
            .body(None)
            .unwrap();
        let err = verify_hub_subprotocol(&wrong).unwrap_err();
        assert!(matches!(
            ScConnectError::from_error(&err),
            Some(ScConnectError::WebSocket {
                kind: ScWebSocketErrorKind::HubSubprotocol,
                ..
            })
        ));
    }

    #[test]
    fn websocket_handshake_error_maps_to_typed_variant() {
        let err = map_websocket_handshake_error(TungsteniteError::Protocol(
            tokio_tungstenite::tungstenite::error::ProtocolError::HandshakeIncomplete,
        ));

        assert!(matches!(
            ScConnectError::from_error(&err),
            Some(ScConnectError::WebSocket {
                kind: ScWebSocketErrorKind::WebSocketHandshake,
                ..
            })
        ));
    }

    #[test]
    fn websocket_upgrade_io_error_maps_to_websocket_handshake() {
        let err = map_websocket_upgrade_error(TungsteniteError::Io(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "closed during upgrade",
        )));

        assert!(matches!(
            ScConnectError::from_error(&err),
            Some(ScConnectError::WebSocket {
                kind: ScWebSocketErrorKind::WebSocketHandshake,
                ..
            })
        ));
    }

    fn test_tls_config() -> ScNodeTlsConfig {
        test_tls_pair().0
    }

    pub(super) fn test_tls_pair() -> (ScNodeTlsConfig, Arc<tokio_rustls::rustls::ServerConfig>) {
        let _ = tokio_rustls::rustls::crypto::aws_lc_rs::default_provider().install_default();

        let mut ca_params =
            rcgen::CertificateParams::new(Vec::<String>::new()).expect("empty SANs are valid");
        ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let ca_key = rcgen::KeyPair::generate().unwrap();
        let ca_cert = ca_params.self_signed(&ca_key).unwrap();
        let ca_issuer = rcgen::Issuer::from_params(&ca_params, &ca_key);

        let server_params =
            rcgen::CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()]).unwrap();
        let server_key = rcgen::KeyPair::generate().unwrap();
        let server_cert = server_params.signed_by(&server_key, &ca_issuer).unwrap();

        let server_chain: Vec<CertificateDer<'static>> =
            CertificateDer::pem_slice_iter(server_cert.pem().as_bytes())
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
        let server_key =
            PrivateKeyDer::from_pem_slice(server_key.serialize_pem().as_bytes()).unwrap();
        let server_config = tokio_rustls::rustls::ServerConfig::builder_with_protocol_versions(&[
            &tokio_rustls::rustls::version::TLS13,
        ])
        .with_no_client_auth()
        .with_single_cert(server_chain, server_key)
        .unwrap();

        let ca_certs: Vec<CertificateDer<'static>> =
            CertificateDer::pem_slice_iter(ca_cert.pem().as_bytes())
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
        let client_key = rcgen::KeyPair::generate().unwrap();
        let client_cert = rcgen::CertificateParams::new(vec!["node".into()])
            .unwrap()
            .signed_by(&client_key, &ca_issuer)
            .unwrap();
        let client_config = ScNodeTlsConfig::from_der(
            ca_certs,
            vec![client_cert.der().clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(client_key.serialize_der()).into(),
        )
        .unwrap();

        (client_config, Arc::new(server_config))
    }

    #[tokio::test]
    async fn tls_websocket_connect_dial_failure_is_typed() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let err = match TlsWebSocket::connect(&format!("wss://{addr}"), test_tls_config()).await {
            Ok(_) => panic!("expected TCP dial failure"),
            Err(err) => err,
        };

        assert!(matches!(
            ScConnectError::from_error(&err),
            Some(ScConnectError::WebSocket {
                kind: ScWebSocketErrorKind::TcpDial,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn tls_websocket_connect_tls_failure_is_typed() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let accept_task = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
        });

        let err = match TlsWebSocket::connect(&format!("wss://{addr}"), test_tls_config()).await {
            Ok(_) => panic!("expected TLS handshake failure"),
            Err(err) => err,
        };

        assert!(matches!(
            ScConnectError::from_error(&err),
            Some(ScConnectError::WebSocket {
                kind: ScWebSocketErrorKind::TlsHandshake,
                ..
            })
        ));
        accept_task.await.unwrap();
    }

    #[tokio::test]
    async fn tls_websocket_connect_upgrade_io_failure_is_websocket_handshake() {
        let (client_config, server_config) = test_tls_pair();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let accept_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut tls = TlsAcceptor::from(server_config)
                .accept(socket)
                .await
                .unwrap();
            let mut buf = [0u8; 128];
            let _ = tokio::time::timeout(std::time::Duration::from_secs(1), tls.read(&mut buf))
                .await
                .unwrap()
                .unwrap();
            tls.shutdown().await.unwrap();
        });

        let err = match TlsWebSocket::connect(&format!("wss://{addr}"), client_config).await {
            Ok(_) => panic!("expected WebSocket upgrade failure"),
            Err(err) => err,
        };

        assert!(matches!(
            ScConnectError::from_error(&err),
            Some(ScConnectError::WebSocket {
                kind: ScWebSocketErrorKind::WebSocketHandshake,
                ..
            })
        ));
        accept_task.await.unwrap();
    }
}
