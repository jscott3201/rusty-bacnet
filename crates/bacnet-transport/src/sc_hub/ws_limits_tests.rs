use super::deadline_capacity_tests::CountedHub;
use super::deadline_test_support::*;
use super::ws_limits_test_support::*;
use super::*;
use crate::sc::WebSocketPort;
use crate::sc_tls::TlsWebSocket;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

#[tokio::test]
async fn initiating_websocket_rejects_oversize_declared_frame_without_body() {
    let tls = TestTls::new();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = async {
        let (tcp, _) = listener.accept().await.unwrap();
        let stream = tls.acceptor.accept(tcp).await.unwrap();
        #[allow(clippy::result_large_err)]
        let mut ws = tokio_tungstenite::accept_hdr_async(
            stream,
            |_: &_, mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                response.headers_mut().insert(
                    "Sec-WebSocket-Protocol",
                    "hub.bsc.bacnet.org".parse().unwrap(),
                );
                Ok(response)
            },
        )
        .await
        .unwrap();
        // Valid unmasked binary header, declared payload 65536; no body supplied.
        ws.get_mut()
            .write_all(&[0x82, 127, 0, 0, 0, 0, 0, 1, 0, 0])
            .await
            .unwrap();
        ws.get_mut().flush().await.unwrap();
        ws
    };
    let url = format!("wss://localhost:{}", address.port());
    let (server, node) = tokio::join!(server, TlsWebSocket::connect(&url, tls.client.clone()));
    let node = node.unwrap();
    let outcome = tokio::time::timeout(Duration::from_millis(500), node.recv()).await;
    assert!(
        matches!(outcome, Ok(Err(ref error)) if error.to_string().contains("Space limit exceeded")),
        "oversize header must fail before awaiting its body: {outcome:?}"
    );
    drop(server);
}

#[tokio::test]
async fn hub_websocket_rejects_oversize_header_and_reclaims_unregistered_slot() {
    let tls = TestTls::new();
    let hub =
        super::deadline_capacity_tests::CountedHub::start(&tls, ScHubHandshakeTimeouts::default())
            .await;
    let mut ws = tls.websocket(hub.address).await;
    assert_eq!(hub.active.load(Ordering::Acquire), 1);
    // Valid masked binary header, declared payload 5706; no body supplied.
    ws.get_mut()
        .write_all(&[0x82, 0xfe, 0x16, 0x4a, 1, 2, 3, 4])
        .await
        .unwrap();
    ws.get_mut().flush().await.unwrap();
    let result = tokio::time::timeout(Duration::from_millis(500), async {
        while hub.active.load(Ordering::Acquire) != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(
        result.is_ok(),
        "hub awaited an oversized frame body instead of releasing its slot"
    );
    assert!(hub.clients.lock().await.is_empty());
}

#[tokio::test]
async fn initiating_websocket_accepts_full_u16_capacity_with_fragmented_ping() {
    let (mut server, node) = initiating_pair().await;
    let mut message = npdu_message([0xff; 6], 61327, 4192);
    message.originating_vmac = Some([0x22; 6]);
    let wire = encoded(&message);
    assert_eq!(wire.len(), 65535); // Table 6-1 NPDU ceiling, both VMACs and encoded options
    for fragmented in [false, true] {
        let send = async {
            if fragmented {
                server
                    .get_mut()
                    .write_all(&raw_frame(2, false, &wire[..32768], false))
                    .await
                    .unwrap();
                server
                    .get_mut()
                    .write_all(&raw_frame(9, true, b"ping", false))
                    .await
                    .unwrap();
                server
                    .get_mut()
                    .write_all(&raw_frame(0, true, &wire[32768..], false))
                    .await
                    .unwrap();
            } else {
                server
                    .get_mut()
                    .write_all(&raw_frame(2, true, &wire, false))
                    .await
                    .unwrap();
            }
            server.get_mut().flush().await.unwrap();
        };
        let ((), received) = tokio::join!(send, node.recv());
        assert_eq!(received.unwrap(), wire);
    }
}

#[tokio::test]
async fn initiating_websocket_rejects_cumulative_fragment_overflow() {
    let (mut server, node) = initiating_pair().await;
    let send = async {
        for opcode in [2, 0] {
            server
                .get_mut()
                .write_all(&raw_frame(opcode, false, &vec![0x55; 32768], false))
                .await
                .unwrap();
        }
        server.get_mut().flush().await.unwrap();
    };
    let ((), result) = tokio::join!(
        send,
        tokio::time::timeout(Duration::from_secs(2), node.recv())
    );
    assert!(result
        .unwrap()
        .unwrap_err()
        .to_string()
        .contains("Space limit exceeded"));
}

#[tokio::test]
async fn hub_framing_errors_retire_registered_clients_and_preserve_healthy_peer() {
    let tls = TestTls::new();
    let hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut healthy = tls.websocket(hub.address).await;
    register(&mut healthy, [0x32; 6]).await;
    for fragments in [false, true] {
        let mut attacker = tls.websocket(hub.address).await;
        register(&mut attacker, [0x22; 6]).await;
        if fragments {
            // Both frames fit individually; incomplete message reaches 5706.
            for opcode in [2, 0] {
                attacker
                    .get_mut()
                    .write_all(&raw_frame(opcode, false, &vec![0x55; 2853], true))
                    .await
                    .unwrap();
            }
        } else {
            attacker
                .get_mut()
                .write_all(&[0x82, 0xfe, 0x16, 0x4a, 1, 2, 3, 4])
                .await
                .unwrap();
        }
        attacker.get_mut().flush().await.unwrap();
        until(|| hub.active.load(Ordering::Acquire) == 1).await;
        let map = hub.clients.lock().await;
        assert_eq!(map.len(), 1);
        assert!(!map.get(&[0x32; 6]).unwrap().closed.load(Ordering::Acquire));
        drop(map);
        heartbeat(&mut healthy, 0x1234).await;
    }
    healthy.close(None).await.unwrap();
    until(|| hub.active.load(Ordering::Acquire) == 0).await;
}

#[tokio::test]
async fn hub_accepts_inclusive_full_message_capacity_with_fragmented_ping() {
    let tls = TestTls::new();
    let hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut sender = tls.websocket(hub.address).await;
    let mut recipient = tls.websocket(hub.address).await;
    register(&mut sender, [0x22; 6]).await;
    register(&mut recipient, [0x32; 6]).await;
    // This inclusive full-message boundary allows six more optional header bytes
    // than the required workload, retaining valid source-free hub ingress.
    let message = npdu_message([0x32; 6], 1497, 4198);
    let wire = encoded(&message);
    assert_eq!(wire.len(), 5705);
    for fragmented in [false, true] {
        if fragmented {
            sender
                .get_mut()
                .write_all(&raw_frame(2, false, &wire[..2853], true))
                .await
                .unwrap();
            sender
                .get_mut()
                .write_all(&raw_frame(9, true, b"ping", true))
                .await
                .unwrap();
            sender
                .get_mut()
                .write_all(&raw_frame(0, true, &wire[2853..], true))
                .await
                .unwrap();
            sender.get_mut().flush().await.unwrap();
            assert!(
                matches!(poll_io(sender.next()).await, Some(Ok(Message::Pong(data))) if &data[..] == b"ping")
            );
        } else {
            sender.send(Message::Binary(wire.clone())).await.unwrap();
        }
        let received = receive(&mut recipient).await;
        assert_eq!(received.len(), 5705);
        assert_eq!(&received[4..10], &[0x22; 6]);
        assert_eq!(&received[10..], &wire[10..]);
    }
    sender.close(None).await.unwrap();
    recipient.close(None).await.unwrap();
    until(|| hub.active.load(Ordering::Acquire) == 0).await;
}
