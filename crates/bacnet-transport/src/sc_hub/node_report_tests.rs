//! What this stack's node reports when this stack's hub ends or refuses its
//! connection (#2463), over real mutual TLS on loopback.
//!
//! The hub's orderly closes say 1000, which a node reads as
//! `WEBSOCKET_CLOSED_BY_PEER`. A graceful shutdown of a connected node is
//! the exception: the hub's Disconnect-Request comes before the close, and
//! that is what the node reports.
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
/// hub's graceful shutdown as a close with status 1000.
#[tokio::test]
async fn a_graceful_hub_shutdown_reads_as_closed_by_peer_before_connect() {
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
        ScDisconnectCause::WebSocketClosed { close_status: 1000 },
        "{error}"
    );
    assert_eq!(
        cause.error_code(),
        Some(ErrorCode::WEBSOCKET_CLOSED_BY_PEER)
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

/// The hub's NAK to a Connect-Request names its class and code in the
/// connect error's message, not their numbers.
#[tokio::test]
async fn a_connect_nak_names_its_error_class_and_code() {
    let tls = TestTls::new();
    let mut hub = graceful_hub(&tls).await;
    // Another device holds the VMAC this node asks for.
    let mut holder = tls.websocket(hub.local_addr().unwrap()).await;
    holder.send(request(NODE_VMAC, [0x77; 16])).await.unwrap();
    assert!(matches!(poll_io(holder.next()).await,
        Some(Ok(Message::Binary(data))) if data[0] == 7));
    let mut transport =
        ScTransport::new(dial(&tls, &hub).await, NODE_VMAC).with_device_uuid(NODE_UUID);
    let error = match tokio::time::timeout(WAIT, transport.start()).await {
        Ok(Err(error)) => error,
        Ok(Ok(_)) => panic!("a duplicate VMAC cannot connect"),
        Err(_) => panic!("the connect did not end"),
    };
    let shown = error.to_string();
    assert!(
        shown.contains("error_class=COMMUNICATION error_code=NODE_DUPLICATE_VMAC"),
        "{shown}"
    );
    transport.stop().await.unwrap();
    hub.stop().await;
}
