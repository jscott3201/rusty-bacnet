//! Peer-initiated WebSocket closure over real mutual TLS.
use super::deadline_capacity_tests::CountedHub;
use super::deadline_test_support::*;
use super::*;
use tokio_tungstenite::tungstenite::protocol::{frame::coding::CloseCode, CloseFrame};

fn close_frame(code: CloseCode, reason: &'static str) -> Message {
    Message::Close(Some(CloseFrame {
        code,
        reason: reason.into(),
    }))
}

async fn assert_reply(peer: &mut ClientWs, close: Message) {
    // Keep the peer's transport open and read the exact reciprocal Close before
    // any EOF/error. Sending a Close does not itself await the peer's reply.
    peer.send(close.clone()).await.unwrap();
    let received = poll_io(peer.next()).await;
    assert!(
        matches!(received, Some(Ok(ref reply)) if *reply == close),
        "expected reciprocal {close:?} before EOF, got {received:?}"
    );
}

async fn peer_close(registered: bool, code: CloseCode, reason: &'static str) {
    let tls = TestTls::new();
    let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut peer = tls.websocket(hub.address).await;
    if registered {
        peer.send(request([0x42; 6], [0x42; 16])).await.unwrap();
        assert!(matches!(poll_io(peer.next()).await,
            Some(Ok(Message::Binary(data))) if data[0] == 7));
    }
    assert_reply(&mut peer, close_frame(code, reason)).await;
    until(|| hub.active.load(Ordering::Acquire) == 0).await;
    assert!(hub.clients.lock().await.is_empty());
    // The same registration/capacity is usable again after reciprocal cleanup.
    let mut next = tls.websocket(hub.address).await;
    next.send(request([0x42; 6], [0x42; 16])).await.unwrap();
    assert!(matches!(poll_io(next.next()).await,
        Some(Ok(Message::Binary(data))) if data[0] == 7));
    hub.hub.stop().await;
    assert_eq!(hub.active.load(Ordering::Acquire), 0);
    assert_eq!(hub.hub.tasks.len(), 0);
}

#[tokio::test]
async fn registered_peer_close_echoes_code_and_reason_before_eof() {
    peer_close(true, CloseCode::Normal, "registered peer finished").await;
}

#[tokio::test]
async fn unregistered_peer_close_echoes_code_and_reason_before_eof() {
    peer_close(false, CloseCode::Away, "no Connect request").await;
}

#[tokio::test]
async fn peer_close_during_disconnect_ack_wait_replies_but_remains_forced() {
    let tls = TestTls::new();
    let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut peer = tls.websocket(hub.address).await;
    peer.send(request([0x42; 6], [0x42; 16])).await.unwrap();
    assert!(matches!(poll_io(peer.next()).await,
        Some(Ok(Message::Binary(data))) if data[0] == 7));
    let peer_side = async {
        let Some(Ok(Message::Binary(data))) = poll_io(peer.next()).await else {
            panic!("expected Disconnect-Request");
        };
        assert_eq!(
            decode_sc_message(&data).unwrap().function,
            ScFunction::DisconnectRequest
        );
        assert_reply(
            &mut peer,
            close_frame(CloseCode::Library(4001), "no Disconnect ACK"),
        )
        .await;
    };
    let (outcome, ()) = tokio::join!(hub.hub.shutdown_gracefully(), peer_side);
    assert_eq!(outcome, ScHubShutdownOutcome::Forced);
    assert_eq!(hub.active.load(Ordering::Acquire), 0);
    assert!(hub.clients.lock().await.is_empty());
    assert_eq!(hub.hub.tasks.len(), 0);
}

#[tokio::test]
async fn peer_close_cleanup_budget_covers_held_sink_and_recovers_admission() {
    use super::shutdown_blocked_tests::ControlledPeer;
    use std::time::Duration;

    for registered in [false, true] {
        let tls = TestTls::new();
        let mut hub = CountedHub::start_with_limits(
            &tls,
            ScHubHandshakeTimeouts::default(),
            ScHubAdmissionLimits {
                max_clients: 1,
                max_handshakes: 1,
            },
        )
        .await;
        let mut peer = ControlledPeer::open(&tls, &hub).await;
        if registered {
            peer.connect(0x42).await;
        }
        let closed = hub
            .clients
            .lock()
            .await
            .get(&[0x42; 6])
            .map(|c| c.closed.clone());
        let socket = Arc::downgrade(&peer.sink);
        let held = peer.sink.lock_owned().await;
        tokio::time::pause();
        peer.ws
            .send(close_frame(CloseCode::Normal, "blocked reply"))
            .await
            .unwrap();
        until(|| peer.deadline.close_started.load(Ordering::Acquire)).await;
        assert!(
            hub.clients.lock().await.is_empty(),
            "remove before sink I/O"
        );
        if let Some(closed) = closed {
            assert!(closed.load(Ordering::Acquire));
        }
        assert_eq!(hub.active.load(Ordering::Acquire), 1);
        tokio::time::advance(Duration::from_millis(4999)).await;
        assert_eq!(hub.active.load(Ordering::Acquire), 1, "cleanup ended early");
        // Tokio rounds timer deadlines up to its next millisecond tick.
        tokio::time::advance(Duration::from_millis(2)).await;
        until(|| hub.active.load(Ordering::Acquire) == 0).await;
        assert_eq!(
            socket.strong_count(),
            1,
            "cleanup retained socket after budget"
        );
        drop(held);
        assert!(socket.upgrade().is_none());
        tokio::time::resume();
        let mut next = tls.websocket(hub.address).await;
        next.send(request([0x42; 6], [0x42; 16])).await.unwrap();
        assert!(matches!(poll_io(next.next()).await,
            Some(Ok(Message::Binary(data))) if data[0] == 7));
        hub.hub.stop().await;
        assert_eq!(hub.active.load(Ordering::Acquire), 0);
        assert_eq!(hub.hub.tasks.len(), 0);
    }
}

#[tokio::test]
async fn peer_close_cleanup_cannot_remove_replacement_queued_before_it() {
    use super::shutdown_blocked_tests::ControlledPeer;

    let tls = TestTls::new();
    let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut old = ControlledPeer::open(&tls, &hub).await;
    old.connect(0x42).await;
    let socket = Arc::downgrade(&old.sink);
    let held_sink = old.sink.lock_owned().await;
    let mut next = ControlledPeer::open(&tls, &hub).await;
    let held_registry = hub.clients.lock().await;
    next.ws.send(request([0x42; 6], [0x42; 16])).await.unwrap();
    until(|| next.deadline.admission_started.load(Ordering::Acquire)).await;
    let close = close_frame(CloseCode::Normal, "old registration");
    old.ws.send(close.clone()).await.unwrap();
    until(|| old.deadline.close_started.load(Ordering::Acquire)).await;
    // FIFO registry locking commits the replacement before the old owner's
    // removal. Its identity check must preserve that new registration.
    drop(held_registry);
    assert!(matches!(poll_io(next.ws.next()).await,
        Some(Ok(Message::Binary(data))) if data[0] == 7));
    assert!(Arc::ptr_eq(
        &hub.clients.lock().await.get(&[0x42; 6]).unwrap().sink,
        &next.sink
    ));
    drop(held_sink);
    assert_eq!(poll_io(old.ws.next()).await.unwrap().unwrap(), close);
    until(|| hub.active.load(Ordering::Acquire) == 1).await;
    assert!(socket.upgrade().is_none());
    next.ws
        .send(super::retirement_tests::heartbeat_wire(25))
        .await
        .unwrap();
    assert!(matches!(poll_io(next.ws.next()).await,
        Some(Ok(Message::Binary(data))) if data[0..4] == [0x0b, 0, 0, 25]));
    hub.hub.stop().await;
    assert_eq!(hub.active.load(Ordering::Acquire), 0);
    assert_eq!(hub.hub.tasks.len(), 0);
}

#[tokio::test]
async fn cancelled_forceful_stop_joins_peer_close_cleanup_without_reply_requirement() {
    use super::shutdown_blocked_tests::ControlledPeer;

    let tls = TestTls::new();
    let mut hub = CountedHub::start(&tls, ScHubHandshakeTimeouts::default()).await;
    let mut peer = ControlledPeer::open(&tls, &hub).await;
    peer.connect(0x42).await;
    let socket = Arc::downgrade(&peer.sink);
    let held_sink = peer.sink.lock_owned().await;
    peer.ws
        .send(close_frame(CloseCode::Normal, "abort cleanup"))
        .await
        .unwrap();
    until(|| peer.deadline.close_started.load(Ordering::Acquire)).await;
    let registry = hub.clients.clone();
    let held_registry = registry.lock().await;
    assert!(held_registry.is_empty());
    {
        let mut stopping = Box::pin(hub.hub.stop());
        poll_io(async {
            loop {
                assert!(futures_util::poll!(&mut stopping).is_pending());
                if hub.active.load(Ordering::Acquire) == 0 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await;
    }
    assert!(
        hub.hub.listener_task.is_some(),
        "cancellation lost supervisor join"
    );
    assert_eq!(socket.strong_count(), 1, "aborted cleanup retained sink");
    drop(held_registry);
    poll_io(hub.hub.stop()).await;
    assert!(hub.hub.listener_task.is_none());
    assert_eq!(hub.hub.tasks.len(), 0);
    assert_eq!(
        hub.hub.shutdown_gracefully().await,
        ScHubShutdownOutcome::Forced
    );
    drop(held_sink);
    assert!(socket.upgrade().is_none());
    // Abort may forgo the reciprocal frame; it must still release transport.
    assert!(matches!(poll_io(peer.ws.next()).await, None | Some(Err(_))));
}
