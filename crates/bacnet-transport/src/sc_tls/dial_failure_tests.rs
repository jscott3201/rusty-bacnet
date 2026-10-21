//! Real-socket fixtures for the failures the SC connection doctor places
//! (#1839): a dial that stalls in TLS, a hub that never answers the
//! Connect-Request, and a proxy that refuses the WebSocket upgrade.

use super::*;
use crate::port::TransportPort;
use crate::sc::{dial_within, ScDialStage, ScDisconnectCause, ScTransport};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Short, since each fixture stalls on purpose and never answers.
const LIMIT: Duration = Duration::from_millis(300);

/// A loopback listener and a URL for it naming `host`.
async fn listener(host: &str) -> (tokio::net::TcpListener, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("wss://{host}:{}", listener.local_addr().unwrap().port());
    (listener, url)
}

/// TCP connects, but nothing ever answers the TLS ClientHello: the dial
/// times out in its TLS step.
#[tokio::test]
async fn a_stalled_tls_handshake_is_a_dial_timeout_in_tls() {
    let (node, _) = test_tls_pair();
    // An IP, so a lookup of localhost that tries ::1 first can't fail the
    // dial in TCP; the handshake never gets far enough to check the name.
    let (listener, url) = listener("127.0.0.1").await;
    let holder = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        std::future::pending::<()>().await;
        drop(tcp);
    });
    let error = match dial_within(LIMIT, TlsWebSocket::connect(&url, node)).await {
        Ok(_) => panic!("a stalled TLS handshake cannot connect"),
        Err(error) => error,
    };
    assert!(
        matches!(
            ScConnectError::from_error(&error),
            Some(ScConnectError::DialTimeout {
                timeout: LIMIT,
                stage: Some(ScDialStage::Tls),
            })
        ),
        "{error}"
    );
    assert!(error.to_string().contains("TLS handshake"), "{error}");
    assert_eq!(
        ScDisconnectCause::from_error(&error),
        ScDisconnectCause::DialTimeout {
            stage: Some(ScDialStage::Tls)
        }
    );
    holder.abort();
}

/// TLS and the upgrade succeed, but the hub never sends Connect-Accept: a
/// Connect-Accept timeout, not a dial timeout.
#[tokio::test]
async fn a_silent_hub_is_a_connect_accept_timeout() {
    let (node, server) = test_tls_pair();
    let (listener, url) = listener("localhost").await;
    let hub = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let tls = TlsAcceptor::from(server).accept(tcp).await.unwrap();
        // The handshake callback signature is fixed by tungstenite.
        #[allow(clippy::result_large_err)]
        let ws = tokio_tungstenite::accept_hdr_async(
            tls,
            |_: &_, mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                response.headers_mut().insert(
                    "Sec-WebSocket-Protocol",
                    BACNET_SC_HUB_SUBPROTOCOL.parse().unwrap(),
                );
                Ok(response)
            },
        )
        .await
        .unwrap();
        // Holds the WebSocket open and reads nothing.
        std::future::pending::<()>().await;
        drop(ws);
    });
    let ws = dial_within(LIMIT * 10, TlsWebSocket::connect(&url, node))
        .await
        .unwrap();
    let mut transport = ScTransport::new(ws, [0x02, 0, 0, 0, 0, 0x39])
        .with_device_uuid([0x39; 16])
        .with_connect_timeout_ms(LIMIT.as_millis() as u64);
    let error = match transport.start().await {
        Ok(_) => panic!("a silent hub cannot accept"),
        Err(error) => error,
    };
    assert!(
        matches!(error, Error::Timeout(limit) if limit == LIMIT),
        "{error}"
    );
    assert_eq!(
        ScDisconnectCause::from_error(&error),
        ScDisconnectCause::ConnectAcceptTimeout
    );
    hub.abort();
}

/// A proxy answers the upgrade with 502: the error keeps the status, and
/// the cause names it.
#[tokio::test]
async fn a_refused_upgrade_keeps_its_http_status() {
    let (node, server) = test_tls_pair();
    let (listener, url) = listener("localhost").await;
    let proxy = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut tls = TlsAcceptor::from(server).accept(tcp).await.unwrap();
        let mut request = Vec::new();
        let mut buf = [0u8; 1024];
        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = tls.read(&mut buf).await.unwrap();
            assert!(n > 0, "the upgrade request ended early");
            request.extend_from_slice(&buf[..n]);
        }
        tls.write_all(
            b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await
        .unwrap();
        // The client may have closed first; the 502 is already sent.
        let _ = tls.shutdown().await;
    });
    let error = match dial_within(LIMIT * 10, TlsWebSocket::connect(&url, node)).await {
        Ok(_) => panic!("a refused upgrade cannot connect"),
        Err(error) => error,
    };
    assert!(
        matches!(
            ScConnectError::from_error(&error),
            Some(ScConnectError::WebSocket {
                kind: ScWebSocketErrorKind::WebSocketHandshake,
                http_status: Some(502),
                tls: None,
                ..
            })
        ),
        "{error}"
    );
    assert_eq!(
        ScDisconnectCause::from_error(&error),
        ScDisconnectCause::UpgradeRefused { http_status: 502 }
    );
    proxy.await.unwrap();
}
