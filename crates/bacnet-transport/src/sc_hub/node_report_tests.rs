//! What this stack's node reports when this stack's hub ends or refuses its
//! connection (#2463), over real mutual TLS on loopback.
//!
//! The hub's orderly closes say 1000, which a node reads as
//! `WEBSOCKET_CLOSED_BY_PEER`, and its graceful shutdown says 1001, which a
//! node reads as `WEBSOCKET_ENDPOINT_LEAVES`. A connected node reads a
//! graceful shutdown by the Disconnect-Request that comes before the close,
//! and a refused one by the hub's NAK, which comes before the close too.
use super::deadline_test_support::*;
use super::*;
use crate::port::TransportPort;
use crate::sc::{ScActiveHub, ScDisconnectCause, ScEvent, ScTransport, WebSocketPort};
use crate::sc_tls::TlsWebSocket;
use std::time::Duration;

/// Generous: real TLS on a shared runner.
const WAIT: Duration = Duration::from_secs(10);
const NODE_VMAC: Vmac = [0x02, 0, 0, 0, 0, 0x42];
const NODE_UUID: DeviceUuid = [0x42; 16];

async fn graceful_hub(tls: &TestTls) -> ScHub {
    let timeouts = ScHubGracefulTimeouts::new(
        Duration::from_secs(1),
        Duration::from_secs(1),
        Duration::from_secs(5),
    )
    .unwrap();
    let config = tls.hub_config.clone().with_graceful_timeouts(timeouts);
    ScHub::start("127.0.0.1:0", config, [0x10; 6], [0x10; 16])
        .await
        .unwrap()
}

/// This stack's WebSocket to `hub`, upgraded but not yet connected.
async fn dial(tls: &TestTls, hub: &ScHub) -> TlsWebSocket {
    let url = format!("wss://localhost:{}", hub.local_addr().unwrap().port());
    tokio::time::timeout(WAIT, TlsWebSocket::connect(&url, tls.node.clone()))
        .await
        .expect("the dial did not end")
        .expect("the dial failed")
}

/// A started node on `hub` with `uuid`, its events past `Connected`.
async fn connected(
    tls: &TestTls,
    hub: &ScHub,
    uuid: DeviceUuid,
) -> (
    ScTransport<TlsWebSocket>,
    tokio::sync::broadcast::Receiver<ScEvent>,
) {
    let mut transport = ScTransport::new(dial(tls, hub).await, NODE_VMAC).with_device_uuid(uuid);
    let mut events = transport.connection_events();
    let _rx = tokio::time::timeout(WAIT, transport.start())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(next(&mut events).await, ScEvent::Connected { .. }));
    (transport, events)
}

async fn next(events: &mut tokio::sync::broadcast::Receiver<ScEvent>) -> ScEvent {
    tokio::time::timeout(WAIT, events.recv())
        .await
        .expect("no SC event in time")
        .unwrap()
}

async fn disconnected(events: &mut tokio::sync::broadcast::Receiver<ScEvent>) -> ScDisconnectCause {
    match next(events).await {
        ScEvent::Disconnected {
            hub: ScActiveHub::Primary,
            cause,
            ..
        } => cause,
        other => panic!("expected Disconnected, got {other:?}"),
    }
}

/// A node whose WebSocket is up but which has not connected yet gets the
/// hub's graceful shutdown as a close with status 1001.
#[tokio::test]
async fn a_graceful_hub_shutdown_reads_as_endpoint_leaves_before_connect() {
    let tls = TestTls::new();
    let mut hub = graceful_hub(&tls).await;
    let ws = dial(&tls, &hub).await;
    let node = async move {
        let error = tokio::time::timeout(WAIT, ws.recv())
            .await
            .expect("the hub's close did not arrive")
            .unwrap_err();
        // Gone, so the hub need not wait out its echo bound.
        drop(ws);
        error
    };
    let (_, error) = tokio::join!(hub.shutdown_gracefully(), node);
    let cause = ScDisconnectCause::from_error(&error);
    assert_eq!(
        cause,
        ScDisconnectCause::WebSocketClosed { close_status: 1001 },
        "{error}"
    );
    assert_eq!(
        cause.error_code(),
        Some(ErrorCode::WEBSOCKET_ENDPOINT_LEAVES)
    );
}

/// A connected node the hub retires, because the same device connected
/// again, reads the close's 1000 on its `Disconnected` event.
#[tokio::test]
async fn a_node_the_hub_retires_reads_closed_by_peer() {
    let tls = TestTls::new();
    let mut hub = graceful_hub(&tls).await;
    let (mut transport, mut events) = connected(&tls, &hub, NODE_UUID).await;
    let mut again = tls.websocket(hub.local_addr().unwrap()).await;
    again.send(request([0x43; 6], NODE_UUID)).await.unwrap();
    let cause = disconnected(&mut events).await;
    assert_eq!(
        cause,
        ScDisconnectCause::WebSocketClosed { close_status: 1000 }
    );
    assert_eq!(
        cause.error_code(),
        Some(ErrorCode::WEBSOCKET_CLOSED_BY_PEER)
    );
    transport.stop().await.unwrap();
    hub.stop().await;
}

/// A connected node reads the hub's graceful shutdown by its
/// Disconnect-Request, which comes before the close and carries no code.
#[tokio::test]
async fn a_graceful_hub_shutdown_reads_as_a_disconnect_request_once_connected() {
    let tls = TestTls::new();
    let mut hub = graceful_hub(&tls).await;
    let (mut transport, mut events) = connected(&tls, &hub, NODE_UUID).await;
    let node = async {
        let cause = disconnected(&mut events).await;
        transport.stop().await.unwrap();
        cause
    };
    let (_, cause) = tokio::join!(hub.shutdown_gracefully(), node);
    assert_eq!(cause, ScDisconnectCause::PeerClose);
    assert_eq!(cause.error_code(), None);
}

/// Each refusal reads as it did before the hub began closing refused
/// connections with 1000 (#2463): the node reads the NAK, which comes
/// first, so its class and code surface and the close does not mask them;
/// the connect error names both. A Disconnect-Request answered before any
/// Connect reads the close.
#[tokio::test]
async fn a_refused_node_reports_the_nak_not_the_close() {
    use super::close_status_tests::{Refusal, refusing_hub};
    for refusal in Refusal::ALL {
        let tls = TestTls::new();
        let (mut hub, _incumbent) = refusing_hub(&tls).await;
        match refusal.nak() {
            Some((class, code)) => {
                let mut transport = ScTransport::new(dial(&tls, &hub).await, refusal.vmac())
                    .with_device_uuid([0x43; 16]);
                let error = match tokio::time::timeout(WAIT, transport.start()).await {
                    Ok(Err(error)) => error,
                    Ok(Ok(_)) => panic!("{refusal:?}: a refused node cannot connect"),
                    Err(_) => panic!("{refusal:?}: the connect did not end"),
                };
                let cause = ScDisconnectCause::from_error(&error);
                assert!(
                    matches!(cause, ScDisconnectCause::Nak { error_class, error_code, .. }
                        if (error_class, error_code) == (class, code)),
                    "{refusal:?}: {cause:?}"
                );
                assert_eq!(cause.error_code(), Some(code), "{refusal:?}");
                let shown = error.to_string();
                assert!(
                    shown.contains(&format!("error_class={class} error_code={code}")),
                    "{refusal:?}: {shown}"
                );
                transport.stop().await.unwrap();
            }
            None => {
                let ws = dial(&tls, &hub).await;
                let request = ScMessage {
                    function: ScFunction::DisconnectRequest,
                    message_id: 0x0102,
                    originating_vmac: None,
                    destination_vmac: None,
                    dest_options: Vec::new(),
                    data_options: Vec::new(),
                    payload: Bytes::new(),
                };
                let mut sent = BytesMut::new();
                encode_sc_message(&mut sent, &request);
                ws.send(&sent).await.unwrap();
                let ack = tokio::time::timeout(WAIT, ws.recv())
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    decode_sc_message(&ack).unwrap().function,
                    ScFunction::DisconnectAck
                );
                let error = tokio::time::timeout(WAIT, ws.recv())
                    .await
                    .expect("the hub's close did not arrive")
                    .unwrap_err();
                let cause = ScDisconnectCause::from_error(&error);
                assert_eq!(
                    cause,
                    ScDisconnectCause::WebSocketClosed { close_status: 1000 },
                    "{refusal:?}: {error}"
                );
                assert_eq!(
                    cause.error_code(),
                    Some(ErrorCode::WEBSOCKET_CLOSED_BY_PEER)
                );
            }
        }
        hub.stop().await;
    }
}

/// This stack's transport will not send a reserved VMAC (it refuses one
/// before dialling), so a malformed Connect-Request goes out through its
/// WebSocket driver: the NAK arrives first, with its class and code, then
/// the hub's close with no status (#2463).
#[tokio::test]
async fn a_malformed_connect_request_reads_the_nak_then_a_close_without_status() {
    let tls = TestTls::new();
    let mut hub = graceful_hub(&tls).await;
    let ws = dial(&tls, &hub).await;
    let Message::Binary(connect) = request([0; 6], NODE_UUID) else {
        unreachable!("a Connect-Request is binary")
    };
    ws.send(&connect).await.unwrap();
    let nak = tokio::time::timeout(WAIT, ws.recv())
        .await
        .unwrap()
        .unwrap();
    match crate::sc_frame::decode_sc_bvlc_result(&decode_sc_message(&nak).unwrap()).unwrap() {
        crate::sc_frame::ScBvlcResult::Nak {
            error_class,
            error_code,
            ..
        } => assert_eq!(
            (error_class, error_code),
            (
                ErrorClass::COMMUNICATION.to_raw(),
                ErrorCode::PARAMETER_OUT_OF_RANGE.to_raw()
            )
        ),
        other => panic!("expected a NAK, got {other:?}"),
    }
    let error = tokio::time::timeout(WAIT, ws.recv())
        .await
        .expect("the hub's close did not arrive")
        .unwrap_err();
    let cause = ScDisconnectCause::from_error(&error);
    assert_eq!(
        cause,
        ScDisconnectCause::WebSocket(crate::sc::ScWebSocketErrorKind::Closed),
        "{error}"
    );
    assert_eq!(cause.error_code(), Some(ErrorCode::WEBSOCKET_ERROR));
    hub.stop().await;
}

/// A node of this stack that ends its connection on purpose reads as an
/// orderly end at the hub (#2520): one that closes with 1000 before it
/// connects is logged at debug with the status, one that stops sends its
/// Disconnect-Request first, and the hub warns about neither.
#[tokio::test]
async fn a_nodes_orderly_close_reads_as_an_orderly_end_at_the_hub() {
    let (logs, _guard) = crate::log_capture::LogCapture::start();
    let tls = TestTls::new();
    let mut hub = graceful_hub(&tls).await;
    let ws = dial(&tls, &hub).await;
    ws.close(Some(1000)).await.unwrap();
    let (mut transport, _events) = connected(&tls, &hub, NODE_UUID).await;
    transport.stop().await.unwrap();
    let logged = |message: &str| !logs.matching(message).is_empty();
    tokio::time::timeout(WAIT, async {
        while !logged("Hub: DisconnectRequest from") {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the hub did not see the stop");
    let closes = logs.matching("Hub: client ");
    assert!(
        closes
            .iter()
            .any(|close| close.fields["message"].ends_with("sent close, status Some(1000)")),
        "{closes:?}"
    );
    assert!(
        closes
            .iter()
            .all(|close| close.level == tracing::Level::DEBUG)
    );
    hub.stop().await;
    assert!(logs.warnings().is_empty(), "{:?}", logs.warnings());
}
