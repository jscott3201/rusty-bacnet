//! Real-socket fixtures for the error codes a failed or ended hub connection
//! reports (#2438): a hub that answers the upgrade with 503, or with no
//! subprotocol or another one, and a peer that ends the WebSocket with a
//! close status, with a close frame that has none, or with no close frame.
//! Since #2463 also an upgrade answer with a bad WebSocket header or one
//! that is not HTTP/1.1, and a dial to an address with no route.

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

/// Dial a TLS server that reads the upgrade request and writes `answer`
/// back verbatim, and return the dial's error.
async fn upgrade_answered_with(answer: &'static [u8]) -> Error {
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
        tls.write_all(answer).await.unwrap();
        // The client may have closed first; the answer is already sent.
        let _ = tls.shutdown().await;
    });
    let error = match tokio::time::timeout(WAIT, TlsWebSocket::connect(&url, node)).await {
        Ok(Err(error)) => error,
        Ok(Ok(_)) => panic!("a bad upgrade answer cannot connect"),
        Err(_) => panic!("the dial did not end"),
    };
    hub.await.unwrap();
    error
}

/// A hub (or a proxy before it) out of connections answers the upgrade
/// with 503, and the code says so.
#[tokio::test]
async fn a_hub_answering_the_upgrade_with_503_reports_temporary_unavailable() {
    let error = upgrade_answered_with(
        b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert_eq!(
        ScDisconnectCause::from_error(&error),
        ScDisconnectCause::UpgradeRefused { http_status: 503 }
    );
    assert_eq!(code_of(&error), Some(ErrorCode::HTTP_TEMPORARY_UNAVAILABLE));
}

/// A 101 answer with a missing or wrong WebSocket header, an answer that
/// does not parse, and one whose status line holds an unusable version or
/// code each report their own code rather than the general upgrade
/// failure (#2463). tungstenite checks Upgrade, then
/// Connection, then Sec-WebSocket-Accept, so each answer gets the ones
/// before right and the one under test wrong.
#[tokio::test]
async fn a_bad_upgrade_answer_reports_a_header_or_syntax_code() {
    let header = (
        ScWebSocketErrorKind::WebSocketHeader,
        ErrorCode::HTTP_WEBSOCKET_HEADER_ERROR,
    );
    let syntax = (
        ScWebSocketErrorKind::UpgradeResponseSyntax,
        ErrorCode::HTTP_RESPONSE_SYNTAX_ERROR,
    );
    let value = (
        ScWebSocketErrorKind::UpgradeResponseValue,
        ErrorCode::HTTP_RESPONSE_VALUE_ERROR,
    );
    let cases: [(&'static [u8], _, &str); 7] = [
        // No Upgrade header (a proxy that strips hop-by-hop headers).
        (
            b"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\n\r\n",
            header,
            "Upgrade",
        ),
        // An upgrade to some other protocol.
        (
            b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: h2c\r\nConnection: Upgrade\r\n\r\n",
            header,
            "Upgrade",
        ),
        (
            b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\n",
            header,
            "Connection",
        ),
        (
            b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
              Sec-WebSocket-Accept: bm90IHRoZSBrZXk=\r\n\r\n",
            header,
            "Sec-WebSocket-Accept",
        ),
        (b"HTTP/1.1 abc Nope\r\n\r\n", syntax, "httparse"),
        (
            b"HTTP/1.0 101 Switching Protocols\r\n\r\n",
            value,
            "version",
        ),
        // Three digits, so it parses, but no status is below 100.
        (b"HTTP/1.1 099 Odd\r\n\r\n", value, "status"),
    ];
    for (answer, (kind, code), named) in cases {
        let shown = String::from_utf8_lossy(answer);
        let error = upgrade_answered_with(answer).await;
        assert_eq!(
            ScDisconnectCause::from_error(&error),
            ScDisconnectCause::WebSocket(kind),
            "{shown}: {error}"
        );
        assert_eq!(code_of(&error), Some(code), "{shown}");
        // The message keeps the library's word for the part at fault.
        assert!(error.to_string().contains(named), "{shown}: {error}");
    }
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

/// An address the OS will not route a TCP connect to, so the dial fails at
/// once with an unreachable error: Linux answers a connect to a multicast
/// address with ENETUNREACH, and macOS one to a 0.0.0.0/8 address with
/// EHOSTUNREACH. Windows has no such address known to fail the same way.
#[cfg(target_os = "linux")]
const UNROUTABLE: &str = "224.0.0.1:47808";
#[cfg(target_os = "macos")]
const UNROUTABLE: &str = "0.0.0.1:47808";

/// A dial with no route to the hub's address reports the IP table's code,
/// not the general TCP one (#2463).
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[tokio::test]
async fn a_dial_with_no_route_reports_ip_address_not_reachable() {
    let (node, _) = test_tls_pair();
    let url = format!("wss://{UNROUTABLE}");
    let error = match tokio::time::timeout(WAIT, TlsWebSocket::connect(&url, node)).await {
        Ok(Err(error)) => error,
        Ok(Ok(_)) => panic!("{UNROUTABLE} cannot connect"),
        Err(_) => panic!("the dial to {UNROUTABLE} did not fail at once"),
    };
    assert_eq!(
        ScDisconnectCause::from_error(&error),
        ScDisconnectCause::WebSocket(ScWebSocketErrorKind::AddressUnreachable),
        "{error}"
    );
    assert_eq!(code_of(&error), Some(ErrorCode::IP_ADDRESS_NOT_REACHABLE));
    assert_eq!(
        code_of(&error).unwrap().to_string(),
        "IP_ADDRESS_NOT_REACHABLE"
    );
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
    let away = CloseFrame {
        code: CloseCode::Away,
        reason: "".into(),
    };
    let cause = disconnect_after(Some(away)).await;
    assert_eq!(
        cause,
        ScDisconnectCause::WebSocketClosed { close_status: 1001 }
    );
    assert_eq!(
        cause.error_code(),
        Some(ErrorCode::WEBSOCKET_ENDPOINT_LEAVES)
    );
}

/// A hub that closes a live connection with a close frame and no status
/// reads as it did before this stack's hub began sending 1000 (#2463): a
/// status-less close and the WebSocket catch-all code. Other vendors' hubs
/// may still close that way.
#[tokio::test]
async fn a_hub_closing_without_a_status_reports_the_websocket_catch_all() {
    let cause = disconnect_after(None).await;
    assert_eq!(
        cause,
        ScDisconnectCause::WebSocket(ScWebSocketErrorKind::Closed)
    );
    assert_eq!(cause.error_code(), Some(ErrorCode::WEBSOCKET_ERROR));
}

/// Connect a node to a peer that accepts it and then closes with `close`,
/// and return the cause the node's `Disconnected` event names.
async fn disconnect_after(close: Option<CloseFrame>) -> ScDisconnectCause {
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
        ws.close(close).await.unwrap();
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
    transport.stop().await.unwrap();
    hub.await.unwrap();
    cause
}
