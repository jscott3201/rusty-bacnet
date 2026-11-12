//! RB-13 graceful shutdown tests: controlled loopback peers, explicit deadlines.
use super::deadline_test_support::*;
use super::*;
use crate::port_ownership::ATTEMPTS;
use std::time::Duration;

fn short_timeouts() -> ScHubGracefulTimeouts {
    ScHubGracefulTimeouts::new(
        Duration::from_secs(1),
        Duration::from_secs(1),
        Duration::from_secs(5),
    )
    .unwrap()
}

async fn start_graceful(tls: &TestTls, timeouts: ScHubGracefulTimeouts) -> ScHub {
    let config = tls.hub_config.clone().with_graceful_timeouts(timeouts);
    ScHub::start("127.0.0.1:0", config, [0x10; 6], [0x10; 16])
        .await
        .unwrap()
}

async fn register(tls: &TestTls, address: SocketAddr, id: u8) -> ClientWs {
    let mut ws = tls.websocket(address).await;
    ws.send(request([id; 6], [id; 16])).await.unwrap();
    assert!(matches!(poll_io(ws.next()).await,
        Some(Ok(Message::Binary(data))) if data[0] == 7));
    ws
}

async fn next_frame(ws: &mut ClientWs) -> Message {
    poll_io(ws.next())
        .await
        .expect("peer stream ended")
        .expect("peer transport error")
}

fn decode(data: &[u8]) -> ScMessage {
    decode_sc_message(data).unwrap()
}

/// Reciprocal peer: answers the hub Disconnect-Request, completes Close.
async fn reciprocal(ws: &mut ClientWs) {
    let frame = next_frame(ws).await;
    let Message::Binary(data) = frame else {
        panic!("expected DisconnectRequest binary, got {frame:?}");
    };
    let request = decode(&data);
    assert_eq!(request.function, ScFunction::DisconnectRequest);
    let ack = ScMessage {
        function: ScFunction::DisconnectAck,
        message_id: request.message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::new(),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &ack);
    ws.send(Message::Binary(buf.freeze())).await.unwrap();
    // AB.7.5.5: hub Closes after the Ack; flush the queued Close reply to
    // complete the handshake (an explicit second Close is SendAfterClosing).
    let frame = next_frame(ws).await;
    // An orderly close, so it says 1000 (#2463).
    assert_eq!(frame, closed_normally(), "expected Close 1000 after Ack");
    ws.flush().await.unwrap();
}

#[tokio::test]
async fn graceful_with_no_clients_is_graceful_and_rebindable() {
    let tls = TestTls::new();
    // Each run starts on a fresh port; see `rebind` for why it can lose it.
    for attempt in 1..=ATTEMPTS {
        let mut hub = start_graceful(&tls, short_timeouts()).await;
        let address = hub.local_addr().unwrap();
        assert_eq!(hub.status().await.client_count, 0);
        assert_eq!(
            hub.shutdown_gracefully().await,
            ScHubShutdownOutcome::Graceful
        );
        assert_eq!(hub.status().await.client_count, 0);
        let Some(_rebound) = rebind(address, attempt).await else {
            continue;
        };
        // Second observer sees the same completed outcome.
        assert_eq!(
            hub.shutdown_gracefully().await,
            ScHubShutdownOutcome::Graceful
        );
        return;
    }
}

#[tokio::test]
async fn graceful_reciprocal_disconnect_is_graceful() {
    let tls = TestTls::new();
    // Each run starts on a fresh port; see `rebind` for why it can lose it.
    for attempt in 1..=ATTEMPTS {
        let mut hub = start_graceful(&tls, short_timeouts()).await;
        let address = hub.local_addr().unwrap();
        let mut first = register(&tls, address, 0x42).await;
        let mut second = register(&tls, address, 0x43).await;
        assert_eq!(hub.status().await.client_count, 2);
        // Drive both peers while shutdown progresses; closing peers still read
        // as client_count until lease cleanup (no double count).
        let (outcome, _, _) = tokio::join!(
            hub.shutdown_gracefully(),
            reciprocal(&mut first),
            reciprocal(&mut second),
        );
        assert_eq!(outcome, ScHubShutdownOutcome::Graceful);
        assert_eq!(hub.status().await.client_count, 0);
        let Some(_rebound) = rebind(address, attempt).await else {
            continue;
        };
        assert_eq!(
            hub.shutdown_gracefully().await,
            ScHubShutdownOutcome::Graceful
        );
        return;
    }
}

#[tokio::test]
async fn forceful_stop_sends_no_disconnect_request() {
    let tls = TestTls::new();
    // Each run starts on a fresh port; see `rebind` for why it can lose it.
    for attempt in 1..=ATTEMPTS {
        let mut hub = start_graceful(&tls, short_timeouts()).await;
        let address = hub.local_addr().unwrap();
        let mut peer = register(&tls, address, 0x42).await;
        hub.stop().await;
        let frame = tokio::time::timeout(Duration::from_secs(2), peer.next())
            .await
            .expect("forceful close");
        match frame {
            None | Some(Err(_)) | Some(Ok(Message::Close(_))) => {}
            Some(Ok(Message::Binary(data))) => {
                let message = decode_sc_message(&data).unwrap();
                assert_ne!(
                    message.function,
                    ScFunction::DisconnectRequest,
                    "forceful stop must not send DisconnectRequest"
                );
            }
            other => panic!("unexpected forceful close frame: {other:?}"),
        }
        if rebind(address, attempt).await.is_some() {
            return;
        }
    }
}

#[tokio::test]
async fn silent_peer_times_out_forced_without_hang() {
    let tls = TestTls::new();
    // Each run starts on a fresh port; see `rebind` for why it can lose it.
    for attempt in 1..=ATTEMPTS {
        let mut hub = start_graceful(&tls, short_timeouts()).await;
        let address = hub.local_addr().unwrap();
        let mut peer = register(&tls, address, 0x42).await;
        let started = std::time::Instant::now();
        // Never answer the Disconnect-Request and never echo Close.
        let outcome = tokio::time::timeout(Duration::from_secs(10), hub.shutdown_gracefully())
            .await
            .expect("graceful hung on silent peer");
        assert_eq!(outcome, ScHubShutdownOutcome::Forced);
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "silent peer exceeded overall bound"
        );
        // Peer saw the Request but gets closed without a completed handshake.
        let frame = tokio::time::timeout(Duration::from_secs(2), peer.next()).await;
        assert!(frame.is_ok(), "silent peer left dangling");
        if rebind(address, attempt).await.is_some() {
            return;
        }
    }
}

#[tokio::test]
async fn peer_request_first_is_forced_not_graceful() {
    let tls = TestTls::new();
    // Each run starts on a fresh port; see `rebind` for why it can lose it.
    for attempt in 1..=ATTEMPTS {
        let mut hub = start_graceful(&tls, short_timeouts()).await;
        let address = hub.local_addr().unwrap();
        let mut peer = register(&tls, address, 0x42).await;
        let peer_side = async {
            // Simultaneous disconnect: peer Requests while the hub Requests.
            // This peer ignores the hub's own Request (no Ack for it) so the
            // hub's Request goes unacknowledged and the run reads forced.
            let peer_request = ScMessage {
                function: ScFunction::DisconnectRequest,
                message_id: 0x1234,
                originating_vmac: None,
                destination_vmac: None,
                dest_options: Vec::new(),
                data_options: Vec::new(),
                payload: Bytes::new(),
            };
            let mut buf = BytesMut::new();
            encode_sc_message(&mut buf, &peer_request);
            peer.send(Message::Binary(buf.freeze())).await.unwrap();
            // Hub answers our Request; hub's own Request is ignored here.
            let mut saw_ack = false;
            for _ in 0..6 {
                let frame = tokio::time::timeout(Duration::from_secs(2), peer.next())
                    .await
                    .expect("peer close")
                    .expect("stream ended")
                    .expect("transport error");
                match frame {
                    Message::Binary(data) => {
                        let message = decode(&data);
                        if message.function == ScFunction::DisconnectRequest {
                            continue;
                        }
                        assert_eq!(message.function, ScFunction::DisconnectAck);
                        assert_eq!(message.message_id, 0x1234);
                        saw_ack = true;
                    }
                    Message::Close(_) => {
                        peer.flush().await.unwrap();
                        break;
                    }
                    other => panic!("unexpected peer frame: {other:?}"),
                }
            }
            assert!(saw_ack, "hub must Ack the peer Request");
        };
        let (outcome, _) = tokio::join!(hub.shutdown_gracefully(), peer_side);
        assert_eq!(
            outcome,
            ScHubShutdownOutcome::Forced,
            "peer-closes-first without hub Ack must not read graceful"
        );
        if rebind(address, attempt).await.is_some() {
            return;
        }
    }
}

#[tokio::test]
async fn cancelled_graceful_resumes_and_completes() {
    let tls = TestTls::new();
    // Each run starts on a fresh port; see `rebind` for why it can lose it.
    for attempt in 1..=ATTEMPTS {
        let mut hub = start_graceful(&tls, short_timeouts()).await;
        let address = hub.local_addr().unwrap();
        let mut peer = register(&tls, address, 0x42).await;
        {
            let mut stopping = Box::pin(hub.shutdown_gracefully());
            assert!(futures_util::poll!(&mut stopping).is_pending());
            // Dropping the caller future must not cancel hub-owned shutdown.
        }
        assert!(
            hub.listener_task.is_some(),
            "cancelled graceful lost its join"
        );
        let (outcome, _) = tokio::join!(hub.shutdown_gracefully(), reciprocal(&mut peer));
        assert_eq!(outcome, ScHubShutdownOutcome::Graceful);
        assert!(hub.listener_task.is_none());
        if rebind(address, attempt).await.is_some() {
            return;
        }
    }
}

#[tokio::test]
async fn graceful_after_forceful_stop_is_forced() {
    let tls = TestTls::new();
    // Each run starts on a fresh port; see `rebind` for why it can lose it.
    for attempt in 1..=ATTEMPTS {
        let mut hub = start_graceful(&tls, short_timeouts()).await;
        let address = hub.local_addr().unwrap();
        let mut peer = register(&tls, address, 0x42).await;
        hub.stop().await;
        assert_eq!(
            hub.shutdown_gracefully().await,
            ScHubShutdownOutcome::Forced,
            "graceful after stop must not claim protocol success"
        );
        let _ = peer.close(None).await;
        let Some(_rebound) = rebind(address, attempt).await else {
            continue;
        };
        hub.stop().await;
        return;
    }
}

#[tokio::test]
async fn half_handshake_gets_silent_close_never_disconnect() {
    let tls = TestTls::new();
    // Each run starts on a fresh port; see `rebind` for why it can lose it.
    for attempt in 1..=ATTEMPTS {
        let mut hub = start_graceful(&tls, short_timeouts()).await;
        let address = hub.local_addr().unwrap();
        let mut ws = tls.websocket(address).await;
        let peer_side = async {
            let frame = next_frame(&mut ws).await;
            match frame {
                // Silent means no BVLC message: the close still says 1000.
                Message::Close(_) => assert_eq!(frame, closed_normally()),
                Message::Binary(data) => {
                    let message = decode(&data);
                    assert_ne!(
                        message.function,
                        ScFunction::DisconnectRequest,
                        "half-handshake must never get DisconnectRequest"
                    );
                    panic!("half-handshake got binary instead of silent Close");
                }
                other => panic!("half-handshake got {other:?}, want silent Close"),
            }
            ws.flush().await.unwrap();
        };
        let (outcome, _) = tokio::join!(hub.shutdown_gracefully(), peer_side);
        assert_eq!(outcome, ScHubShutdownOutcome::Graceful);
        if rebind(address, attempt).await.is_some() {
            return;
        }
    }
}

#[tokio::test]
async fn graceful_does_not_block_on_held_sink() {
    let tls = TestTls::new();
    // Each run starts on a fresh port; see `rebind` for why it can lose it.
    for attempt in 1..=ATTEMPTS {
        let timeouts = ScHubGracefulTimeouts::new(
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(10),
        )
        .unwrap();
        let mut hub = start_graceful(&tls, timeouts).await;
        let address = hub.local_addr().unwrap();
        let mut peer = register(&tls, address, 0x42).await;
        let sink = hub
            .clients
            .lock()
            .await
            .get(&[0x42; 6])
            .unwrap()
            .sink
            .clone();
        let held = sink.lock_owned().await;
        let outcome = tokio::time::timeout(Duration::from_secs(15), hub.shutdown_gracefully())
            .await
            .expect("drain blocked on held sink");
        assert_eq!(outcome, ScHubShutdownOutcome::Forced);
        assert!(hub.clients.lock().await.is_empty());
        drop(held);
        let _ = peer.close(None).await;
        if rebind(address, attempt).await.is_some() {
            return;
        }
    }
}

#[tokio::test]
async fn replacement_before_graceful_still_graceful_for_current() {
    let tls = TestTls::new();
    // Each run starts on a fresh port; see `rebind` for why it can lose it.
    for attempt in 1..=ATTEMPTS {
        let mut hub = start_graceful(&tls, short_timeouts()).await;
        let address = hub.local_addr().unwrap();
        let mut old = register(&tls, address, 0x42).await;
        // Same Device UUID, new VMAC replaces the old registration.
        let mut newcomer = tls.websocket(address).await;
        let mut wire = crate::sc_frame::connect_test_support::valid_connect(6, [0x43; 6]);
        wire[10..26].copy_from_slice(&[0x42; 16]);
        newcomer.send(Message::Binary(wire.into())).await.unwrap();
        assert!(matches!(poll_io(newcomer.next()).await,
            Some(Ok(Message::Binary(data))) if data[0] == 7));
        let (outcome, _) = tokio::join!(hub.shutdown_gracefully(), reciprocal(&mut newcomer));
        assert_eq!(outcome, ScHubShutdownOutcome::Graceful);
        let _ = old.close(None).await;
        if rebind(address, attempt).await.is_some() {
            return;
        }
    }
}

#[tokio::test]
async fn established_hub_restarts_same_address_config_with_independent_outcomes() {
    let tls = TestTls::new();
    let config = tls
        .hub_config
        .clone()
        .with_graceful_timeouts(short_timeouts());
    // The restart binds the first hub's port again, and another process can
    // take it while no hub holds it (#1104). Such a run goes again from the
    // first start, on a fresh port; the last run keeps the bind's error.
    for attempt in 1..=ATTEMPTS {
        let mut first = ScHub::start("127.0.0.1:0", config.clone(), [0x10; 6], [0x10; 16])
            .await
            .unwrap();
        let address = first.local_addr().unwrap();
        let mut peer = register(&tls, address, 0x42).await;
        let mut collision = tls.websocket(address).await;
        collision
            .send(request([0x10; 6], [0x43; 16]))
            .await
            .unwrap();
        assert!(
            matches!(poll_io(collision.next()).await, Some(Ok(Message::Binary(data))) if data[0] == 0)
        );
        drop(collision);
        let (outcome, ()) = tokio::join!(first.shutdown_gracefully(), reciprocal(&mut peer));
        assert_eq!(outcome, ScHubShutdownOutcome::Graceful);
        let stopped = first.status().await;
        assert!(!stopped.listening);
        assert_eq!((stopped.client_count, stopped.handshake_count), (0, 0));
        assert_eq!(
            stopped.outcomes,
            ScHubOutcomeCounts {
                vmac_collision_rejections: 1,
                ..ScHubOutcomeCounts::default()
            }
        );
        let restarted =
            ScHub::start(&address.to_string(), config.clone(), [0x10; 6], [0x10; 16]).await;
        let mut second = match restarted {
            Ok(hub) => hub,
            Err(bacnet_types::error::Error::Transport(ref err))
                if crate::port_ownership::lost_port(attempt, err) =>
            {
                continue;
            }
            Err(err) => panic!("the restart could not bind {address}: {err}"),
        };
        assert_eq!(second.local_addr(), Some(address));
        assert_eq!(
            second.status().await.outcomes,
            ScHubOutcomeCounts::default()
        );
        let mut peer = register(&tls, address, 0x42).await;
        assert_eq!(second.status().await.client_count, 1);
        let (outcome, ()) = tokio::join!(second.shutdown_gracefully(), reciprocal(&mut peer));
        assert_eq!(outcome, ScHubShutdownOutcome::Graceful);
        assert_eq!(
            second.status().await.outcomes,
            ScHubOutcomeCounts::default()
        );
        assert_eq!(first.status().await, stopped);
        return;
    }
}
