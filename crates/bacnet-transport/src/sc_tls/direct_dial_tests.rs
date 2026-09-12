//! Dial-out matrix for direct-connection establishment (Refs #615 PR3a).
//!
//! Direct dial is WebSocket + mutual TLS to an arbitrary peer `wss` URI with
//! the direct subprotocol, reusing the node operational-certificate policy.
//! All credentials here are generated in-test with `rcgen`; no keys or
//! certificates are committed. Loopback fixtures only.

use super::{ScNodeTlsConfig, TlsWebSocket};
use crate::sc::{ScConnectError, ScWebSocketErrorKind, WebSocketPort};
use crate::sc_frame::{BACNET_SC_DIRECT_SUBPROTOCOL, BACNET_SC_HUB_SUBPROTOCOL};

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use tokio_rustls::TlsAcceptor;

type ServerResponse = tokio_tungstenite::tungstenite::handshake::server::Response;
type ServerRequest = tokio_tungstenite::tungstenite::handshake::server::Request;
type ServerError = tokio_tungstenite::tungstenite::handshake::server::ErrorResponse;

#[allow(clippy::result_large_err)]
type ServerCallback = fn(&ServerRequest, ServerResponse) -> Result<ServerResponse, ServerError>;

struct TestCa {
    params: rcgen::CertificateParams,
    key: rcgen::KeyPair,
    ca: CertificateDer<'static>,
}

impl TestCa {
    fn generate() -> Self {
        let mut params =
            rcgen::CertificateParams::new(Vec::<String>::new()).expect("empty SANs are valid");
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = params.self_signed(&key).unwrap();
        Self {
            params,
            key,
            ca: cert.der().clone(),
        }
    }

    fn issue(&self, sans: Vec<String>) -> (Vec<CertificateDer<'static>>, rcgen::KeyPair) {
        let issuer = rcgen::Issuer::from_params(&self.params, &self.key);
        let params = rcgen::CertificateParams::new(sans).unwrap();
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = params.signed_by(&key, &issuer).unwrap();
        let chain: Vec<CertificateDer<'static>> =
            CertificateDer::pem_slice_iter(cert.pem().as_bytes())
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
        (chain, key)
    }

    fn issue_expired(&self, sans: Vec<String>) -> (Vec<CertificateDer<'static>>, rcgen::KeyPair) {
        let issuer = rcgen::Issuer::from_params(&self.params, &self.key);
        let mut params = rcgen::CertificateParams::new(sans).unwrap();
        params.not_before = rcgen::date_time_ymd(2000, 1, 1);
        params.not_after = rcgen::date_time_ymd(2001, 1, 1);
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = params.signed_by(&key, &issuer).unwrap();
        let chain: Vec<CertificateDer<'static>> =
            CertificateDer::pem_slice_iter(cert.pem().as_bytes())
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
        (chain, key)
    }

    fn client_config(&self, sans: Vec<String>) -> ScNodeTlsConfig {
        let (chain, key) = self.issue(sans);
        ScNodeTlsConfig::from_der(
            vec![self.ca.clone()],
            chain,
            PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
        )
        .unwrap()
    }
}

fn server_config(
    chain: Vec<CertificateDer<'static>>,
    key: &rcgen::KeyPair,
) -> Arc<rustls::ServerConfig> {
    let key =
        rustls::pki_types::PrivateKeyDer::from_pem_slice(key.serialize_pem().as_bytes()).unwrap();
    Arc::new(
        rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
            .with_no_client_auth()
            .with_single_cert(chain, key)
            .unwrap(),
    )
}

#[allow(clippy::result_large_err)]
fn respond_with_direct(
    _: &ServerRequest,
    mut response: ServerResponse,
) -> Result<ServerResponse, ServerError> {
    response.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        BACNET_SC_DIRECT_SUBPROTOCOL.parse().unwrap(),
    );
    Ok(response)
}

#[allow(clippy::result_large_err)]
fn respond_with_hub(
    _: &ServerRequest,
    mut response: ServerResponse,
) -> Result<ServerResponse, ServerError> {
    response.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        BACNET_SC_HUB_SUBPROTOCOL.parse().unwrap(),
    );
    Ok(response)
}

#[allow(clippy::result_large_err)]
fn respond_with_none(
    _: &ServerRequest,
    response: ServerResponse,
) -> Result<ServerResponse, ServerError> {
    Ok(response)
}

/// Accept one loopback TLS + WebSocket peer offering `offer`, echo a single
/// binary frame, then close. Returns the dialable address.
async fn spawn_echo_peer(
    server: Arc<rustls::ServerConfig>,
    callback: ServerCallback,
) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let Ok((tcp, _)) = listener.accept().await else {
            return;
        };
        let Ok(tls) = TlsAcceptor::from(server).accept(tcp).await else {
            return;
        };
        let Ok(mut ws) = tokio_tungstenite::accept_hdr_async(tls, callback).await else {
            return;
        };
        if let Some(Ok(msg)) = ws.next().await {
            let _ = ws.send(msg).await;
        }
    });
    addr
}

async fn expect_direct_err(url: &str, config: ScNodeTlsConfig) -> bacnet_types::error::Error {
    match TlsWebSocket::connect_direct(url, config).await {
        Ok(_) => panic!("expected direct dial failure for {url}"),
        Err(err) => err,
    }
}

fn direct_url(addr: &std::net::SocketAddr) -> String {
    format!("wss://localhost:{}/.bacnet/sc", addr.port())
}

#[tokio::test]
async fn direct_dial_accepts_trusted_peer_with_direct_subprotocol() {
    let ca = TestCa::generate();
    let (server_chain, server_key) = ca.issue(vec!["localhost".into(), "127.0.0.1".into()]);
    let addr = spawn_echo_peer(
        server_config(server_chain, &server_key),
        respond_with_direct,
    )
    .await;

    let ws = tokio::time::timeout(
        Duration::from_secs(5),
        TlsWebSocket::connect_direct(&direct_url(&addr), ca.client_config(vec!["node".into()])),
    )
    .await
    .expect("direct dial timed out")
    .expect("trusted direct peer must connect");

    ws.send(b"direct hello").await.unwrap();
    let echoed = tokio::time::timeout(Duration::from_secs(5), ws.recv())
        .await
        .expect("echo timed out")
        .unwrap();
    assert_eq!(echoed, b"direct hello");
}

#[tokio::test]
async fn direct_dial_rejects_hub_subprotocol_offer() {
    let ca = TestCa::generate();
    let (server_chain, server_key) = ca.issue(vec!["localhost".into(), "127.0.0.1".into()]);
    let addr = spawn_echo_peer(server_config(server_chain, &server_key), respond_with_hub).await;

    let err = expect_direct_err(&direct_url(&addr), ca.client_config(vec!["node".into()])).await;

    // The upgrade itself rejects a subprotocol the dial did not offer, so no
    // connection is established and the failure stays typed.
    assert!(
        matches!(
            ScConnectError::from_error(&err),
            Some(ScConnectError::WebSocket {
                kind: ScWebSocketErrorKind::WebSocketHandshake,
                ..
            })
        ),
        "expected websocket-handshake error, got {err:?}"
    );
}

#[tokio::test]
async fn direct_dial_rejects_missing_subprotocol_offer() {
    let ca = TestCa::generate();
    let (server_chain, server_key) = ca.issue(vec!["localhost".into(), "127.0.0.1".into()]);
    let addr = spawn_echo_peer(server_config(server_chain, &server_key), respond_with_none).await;

    let err = expect_direct_err(&direct_url(&addr), ca.client_config(vec!["node".into()])).await;

    // The upgrade requires the server to echo an offered subprotocol, so a
    // missing offer also fails typed at the handshake with no connection.
    assert!(
        matches!(
            ScConnectError::from_error(&err),
            Some(ScConnectError::WebSocket {
                kind: ScWebSocketErrorKind::WebSocketHandshake,
                ..
            })
        ),
        "expected websocket-handshake error, got {err:?}"
    );
}

#[tokio::test]
async fn direct_dial_rejects_untrusted_peer_certificate() {
    let client_ca = TestCa::generate();
    let server_ca = TestCa::generate();
    let (server_chain, server_key) = server_ca.issue(vec!["localhost".into(), "127.0.0.1".into()]);
    let addr = spawn_echo_peer(
        server_config(server_chain, &server_key),
        respond_with_direct,
    )
    .await;

    let err = expect_direct_err(
        &direct_url(&addr),
        client_ca.client_config(vec!["node".into()]),
    )
    .await;

    assert!(
        matches!(
            ScConnectError::from_error(&err),
            Some(ScConnectError::WebSocket {
                kind: ScWebSocketErrorKind::TlsHandshake,
                ..
            })
        ),
        "expected TLS handshake error, got {err:?}"
    );
}

#[tokio::test]
async fn direct_dial_rejects_self_signed_untrusted_peer() {
    let client_ca = TestCa::generate();
    let mut params =
        rcgen::CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()]).unwrap();
    params.is_ca = rcgen::IsCa::NoCa;
    let key = rcgen::KeyPair::generate().unwrap();
    let cert = params.self_signed(&key).unwrap();
    let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(cert.pem().as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let addr = spawn_echo_peer(server_config(chain, &key), respond_with_direct).await;

    let err = expect_direct_err(
        &direct_url(&addr),
        client_ca.client_config(vec!["node".into()]),
    )
    .await;

    assert!(
        matches!(
            ScConnectError::from_error(&err),
            Some(ScConnectError::WebSocket {
                kind: ScWebSocketErrorKind::TlsHandshake,
                ..
            })
        ),
        "expected TLS handshake error, got {err:?}"
    );
}

#[tokio::test]
async fn direct_dial_rejects_expired_peer_certificate() {
    let ca = TestCa::generate();
    let (server_chain, server_key) = ca.issue_expired(vec!["localhost".into(), "127.0.0.1".into()]);
    let addr = spawn_echo_peer(
        server_config(server_chain, &server_key),
        respond_with_direct,
    )
    .await;

    let err = expect_direct_err(&direct_url(&addr), ca.client_config(vec!["node".into()])).await;

    assert!(
        matches!(
            ScConnectError::from_error(&err),
            Some(ScConnectError::WebSocket {
                kind: ScWebSocketErrorKind::TlsHandshake,
                ..
            })
        ),
        "expected TLS handshake error, got {err:?}"
    );
}

#[tokio::test]
async fn direct_dial_rejects_peer_name_mismatch() {
    let ca = TestCa::generate();
    let (server_chain, server_key) = ca.issue(vec!["wrong.example".into()]);
    let addr = spawn_echo_peer(
        server_config(server_chain, &server_key),
        respond_with_direct,
    )
    .await;

    let err = expect_direct_err(&direct_url(&addr), ca.client_config(vec!["node".into()])).await;

    assert!(
        matches!(
            ScConnectError::from_error(&err),
            Some(ScConnectError::WebSocket {
                kind: ScWebSocketErrorKind::TlsHandshake,
                ..
            })
        ),
        "expected TLS handshake error, got {err:?}"
    );
}

#[tokio::test]
async fn direct_dial_rejects_malformed_uri_before_any_socket() {
    let ca = TestCa::generate();
    for (url, kind) in [
        (
            "ws://localhost:47808/.bacnet/sc",
            ScWebSocketErrorKind::UnsupportedScheme,
        ),
        (
            "https://localhost/.bacnet/sc",
            ScWebSocketErrorKind::UnsupportedScheme,
        ),
        ("wss://:443", ScWebSocketErrorKind::MissingHubHost),
        ("wss://[::1", ScWebSocketErrorKind::InvalidUrl),
        ("not-a-uri", ScWebSocketErrorKind::UnsupportedScheme),
    ] {
        let err = expect_direct_err(url, ca.client_config(vec!["node".into()])).await;
        assert!(
            matches!(
                ScConnectError::from_error(&err),
                Some(ScConnectError::WebSocket { kind: k, .. }) if *k == kind
            ),
            "url {url}: expected {kind:?}, got {err:?}"
        );
    }
}

#[test]
fn direct_subprotocol_constant_is_distinct_from_hub() {
    assert_eq!(BACNET_SC_DIRECT_SUBPROTOCOL, "dc.bsc.bacnet.org");
    assert_ne!(BACNET_SC_DIRECT_SUBPROTOCOL, BACNET_SC_HUB_SUBPROTOCOL);
}

#[test]
fn verify_direct_subprotocol_accepts_only_direct_offer() {
    let accept = tokio_tungstenite::tungstenite::http::Response::builder()
        .header("Sec-WebSocket-Protocol", BACNET_SC_DIRECT_SUBPROTOCOL)
        .body(None)
        .unwrap();
    assert!(super::verify_direct_subprotocol(&accept).is_ok());

    for offer in ["", BACNET_SC_HUB_SUBPROTOCOL, "other"] {
        let response = if offer.is_empty() {
            tokio_tungstenite::tungstenite::http::Response::builder()
                .body(None)
                .unwrap()
        } else {
            tokio_tungstenite::tungstenite::http::Response::builder()
                .header("Sec-WebSocket-Protocol", offer)
                .body(None)
                .unwrap()
        };
        let err = super::verify_direct_subprotocol(&response).unwrap_err();
        assert!(
            matches!(
                ScConnectError::from_error(&err),
                Some(ScConnectError::WebSocket {
                    kind: ScWebSocketErrorKind::DirectSubprotocol,
                    ..
                })
            ),
            "offer {offer:?} must fail direct verification: {err:?}"
        );
    }
}
