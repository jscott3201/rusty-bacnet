//! Real-socket fixtures for the error codes a failed or ended hub connection
//! reports (#2438): a hub that answers the upgrade with 503, or with no
//! subprotocol or another one, and a peer that ends the WebSocket with a
//! close status, with a close frame that has none, or with no close frame.

use super::*;
use crate::port::TransportPort;
use crate::sc::{ScActiveHub, ScDisconnectCause, ScEvent, ScTransport};
use crate::sc_frame::{ScFunction, ScMessage, decode_sc_message, encode_sc_message};
use bacnet_types::enums::ErrorCode;
use bytes::{Bytes, BytesMut};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::handshake::server::Response;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

type ServerWs = WebSocketStream<tokio_rustls::server::TlsStream<TcpStream>>;

/// Generous: real TLS on a shared runner.
const WAIT: Duration = Duration::from_secs(10);

/// A loopback listener and the `wss` URL for it.
async fn listener() -> (tokio::net::TcpListener, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("wss://{}", listener.local_addr().unwrap());
    (listener, url)
}

/// The code `error` reports.
fn code_of(error: &Error) -> Option<ErrorCode> {
    ScDisconnectCause::from_error(error).error_code()
}

/// Accept one TLS connection and the WebSocket upgrade, answering with
/// `subprotocol` in Sec-WebSocket-Protocol, or with no such header.
async fn accept(
    listener: tokio::net::TcpListener,
    server: Arc<tokio_rustls::rustls::ServerConfig>,
    subprotocol: Option<&'static str>,
) -> Option<ServerWs> {
    let (tcp, _) = listener.accept().await.unwrap();
    let tls = TlsAcceptor::from(server).accept(tcp).await.unwrap();
    // The handshake callback signature is fixed by tungstenite.
    #[allow(clippy::result_large_err)]
    let answer = move |_: &_, mut response: Response| {
        if let Some(subprotocol) = subprotocol {
            response
                .headers_mut()
                .insert("Sec-WebSocket-Protocol", subprotocol.parse().unwrap());
        }
        Ok(response)
    };
    // The client fails the upgrade itself when the answer is wrong, and
    // the server may then see the connection go.
    tokio_tungstenite::accept_hdr_async(tls, answer).await.ok()
}

/// The next hub-connector event.
async fn next(events: &mut tokio::sync::broadcast::Receiver<ScEvent>) -> ScEvent {
    tokio::time::timeout(WAIT, events.recv())
        .await
        .expect("no SC event in time")
        .unwrap()
}

/// Read until the client has gone, so a close handshake can finish.
async fn drain(mut ws: ServerWs) {
    while let Some(Ok(_)) = ws.next().await {}
}

/// A hub (or a proxy before it) out of connections answers the upgrade
/// with 503, and the code says so.
#[tokio::test]
async fn a_hub_answering_the_upgrade_with_503_reports_temporary_unavailable() {
    let (node, server) = test_tls_pair();
    let (listener, url) = listener().await;
    let hub = tokio::spawn(async move {
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
            b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await
        .unwrap();
        // The client may have closed first; the 503 is already sent.
        let _ = tls.shutdown().await;
    });
    let error = match tokio::time::timeout(WAIT, TlsWebSocket::connect(&url, node)).await {
        Ok(Err(error)) => error,
        Ok(Ok(_)) => panic!("a refused upgrade cannot connect"),
        Err(_) => panic!("the dial did not end"),
    };
    hub.await.unwrap();
    assert_eq!(
        ScDisconnectCause::from_error(&error),
        ScDisconnectCause::UpgradeRefused { http_status: 503 }
    );
    assert_eq!(code_of(&error), Some(ErrorCode::HTTP_TEMPORARY_UNAVAILABLE));
}

/// An upgrade answer with no Sec-WebSocket-Protocol header is told from
/// one naming another subprotocol; each has its code.
#[tokio::test]
async fn an_upgrade_answer_without_or_with_another_subprotocol_says_which() {
    for (answer, kind, code) in [
        (
            None,
            ScWebSocketErrorKind::HubSubprotocolMissing,
            ErrorCode::HTTP_RESPONSE_MISSING_HEADER,
        ),
        (
            Some(BACNET_SC_DIRECT_SUBPROTOCOL),
            ScWebSocketErrorKind::HubSubprotocol,
            ErrorCode::HTTP_RESPONSE_VALUE_ERROR,
        ),
    ] {
        let (node, server) = test_tls_pair();
        let (listener, url) = listener().await;
        let hub = tokio::spawn(accept(listener, server, answer));
        let error = match tokio::time::timeout(WAIT, TlsWebSocket::connect(&url, node)).await {
            Ok(Err(error)) => error,
            Ok(Ok(_)) => panic!("{answer:?}: the hub subprotocol was never agreed"),
            Err(_) => panic!("{answer:?}: the dial did not end"),
        };
        drop(hub.await.unwrap());
        assert_eq!(
            ScDisconnectCause::from_error(&error),
            ScDisconnectCause::WebSocket(kind),
            "{answer:?}: {error}"
        );
        assert_eq!(code_of(&error), Some(code), "{answer:?}");
    }
}

/// How the peer ended a live WebSocket: with a close status, with a close
/// frame that has none, or with no close frame at all.
#[tokio::test]
async fn a_websocket_the_peer_ends_reports_how_it_ended() {
    let away = CloseFrame {
        code: CloseCode::Away,
        reason: "hub restarting".into(),
    };
    let cases = [
        (
            Some(Some(away)),
            ScDisconnectCause::WebSocketClosed { close_status: 1001 },
            ErrorCode::WEBSOCKET_ENDPOINT_LEAVES,
        ),
        (
            Some(None),
            ScDisconnectCause::WebSocket(ScWebSocketErrorKind::Closed),
            ErrorCode::WEBSOCKET_ERROR,
        ),
        (
            None,
            ScDisconnectCause::WebSocket(ScWebSocketErrorKind::StreamEnded),
            ErrorCode::WEBSOCKET_CLOSED_ABNORMALLY,
        ),
    ];
    for (close, cause, code) in cases {
        let (node, server) = test_tls_pair();
        let (listener, url) = listener().await;
        let peer = tokio::spawn(async move {
            let mut ws = accept(listener, server, Some(BACNET_SC_HUB_SUBPROTOCOL))
                .await
                .unwrap();
            match close {
                Some(frame) => {
                    ws.close(frame).await.unwrap();
                    drain(ws).await;
                }
                // Gone without a close frame: TCP closes under TLS.
                None => drop(ws),
            }
        });
        let ws = tokio::time::timeout(WAIT, TlsWebSocket::connect(&url, node))
            .await
            .unwrap()
            .unwrap();
        let error = tokio::time::timeout(WAIT, ws.recv())
            .await
            .expect("the peer's end was not seen")
            .unwrap_err();
        drop(ws);
        peer.await.unwrap();
        assert_eq!(ScDisconnectCause::from_error(&error), cause, "{error}");
        assert_eq!(code_of(&error), Some(code), "{error}");
    }
}

/// A hub that closes a live connection with status 1001 reads as
/// `Disconnected` with that status, and its code names a peer going away.
#[tokio::test]
async fn a_hub_closing_with_a_status_reports_it_on_the_disconnect() {
    const HUB_VMAC: [u8; 6] = [0x10; 6];
    let (node, server) = test_tls_pair();
    let (listener, url) = listener().await;
    let hub = tokio::spawn(async move {
        let mut ws = accept(listener, server, Some(BACNET_SC_HUB_SUBPROTOCOL))
            .await
            .unwrap();
        let request = match ws.next().await {
            Some(Ok(Message::Binary(data))) => decode_sc_message(&data).unwrap(),
            other => panic!("expected a Connect-Request, got {other:?}"),
        };
        assert_eq!(request.function, ScFunction::ConnectRequest);
        let mut payload = Vec::with_capacity(26);
        payload.extend_from_slice(&HUB_VMAC);
        payload.extend_from_slice(&[0x33; 16]);
        payload.extend_from_slice(&1476u16.to_be_bytes());
        payload.extend_from_slice(&1476u16.to_be_bytes());
        let accept = ScMessage {
            function: ScFunction::ConnectAccept,
            message_id: request.message_id,
            originating_vmac: None,
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from(payload),
        };
        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &accept);
        ws.send(Message::Binary(buf.freeze())).await.unwrap();
        ws.close(Some(CloseFrame {
            code: CloseCode::Away,
            reason: "".into(),
        }))
        .await
        .unwrap();
        drain(ws).await;
    });
    let ws = tokio::time::timeout(WAIT, TlsWebSocket::connect(&url, node))
        .await
        .unwrap()
        .unwrap();
    let mut transport = ScTransport::new(ws, [0x02, 0, 0, 0, 0, 0x38]).with_device_uuid([0x38; 16]);
    let mut events = transport.connection_events();
    let _rx = tokio::time::timeout(WAIT, transport.start())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(next(&mut events).await, ScEvent::Connected { .. }));
    let cause = match next(&mut events).await {
        ScEvent::Disconnected {
            hub: ScActiveHub::Primary,
            cause,
            ..
        } => cause,
        other => panic!("expected Disconnected, got {other:?}"),
    };
    assert_eq!(
        cause,
        ScDisconnectCause::WebSocketClosed { close_status: 1001 }
    );
    assert_eq!(
        cause.error_code(),
        Some(ErrorCode::WEBSOCKET_ENDPOINT_LEAVES)
    );
    transport.stop().await.unwrap();
    hub.await.unwrap();
}
