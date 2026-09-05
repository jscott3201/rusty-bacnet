use super::data_attribute_tests::{hub_accept, start_transport};
use super::*;
use crate::sc_frame::heartbeat_test_support::invalid_heartbeats;
use tokio::time::timeout;

#[tokio::test]
async fn heartbeat_request_payload_gets_nak_before_ack() {
    let (mut transport, _rx, ws) = start_transport().await;
    ws.send(&[0x0A, 0x00, 0x22, 0x33, 0x42]).await.unwrap();
    let response = timeout(Duration::from_secs(1), ws.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        response,
        [0x00, 0x00, 0x22, 0x33, 0x0A, 0x01, 0x00, 0x00, 0x07, 0x00, 0x07]
    );
    transport.stop().await.unwrap();
}

async fn start_timed() -> (
    ScTransport<LoopbackWebSocket>,
    mpsc::Receiver<ReceivedNpdu>,
    LoopbackWebSocket,
) {
    let (client, ws) = LoopbackWebSocket::pair();
    let mut transport =
        ScTransport::new(client, [0x01; 6]).with_test_heartbeat_timing_ms(100, 1000);
    let (rx, ()) = tokio::join!(transport.start(), hub_accept(&ws, [0x10; 6]));
    (transport, rx.unwrap(), ws)
}

async fn recv(ws: &LoopbackWebSocket) -> Vec<u8> {
    timeout(Duration::from_secs(2), ws.recv())
        .await
        .unwrap()
        .unwrap()
}

async fn rejection_barrier(ws: &LoopbackWebSocket) {
    ws.send(&[0x0A, 0, 0x33, 0x44, 0x42]).await.unwrap();
    assert_eq!(recv(ws).await, [0, 0, 0x33, 0x44, 0x0A, 1, 0, 0, 7, 0, 7]);
}

#[tokio::test]
async fn heartbeat_unsupported_ack_does_not_clear_pending_probe() {
    let (mut transport, _rx, ws) = start_timed().await;
    let probe = recv(&ws).await;
    assert_eq!(probe[0], 0x0A);
    ws.send(&[0x0B, 2, probe[2], probe[3], 0x5E]).await.unwrap();
    rejection_barrier(&ws).await;
    for case in invalid_heartbeats([0x01; 6]) {
        ws.send(&case.wire).await.unwrap();
        if let Some(nak) = case.nak {
            assert_eq!(recv(&ws).await, nak, "{}", case.name);
        }
        let mut ack = case.wire;
        ack[0] = 0x0B;
        ack[2..4].copy_from_slice(&probe[2..4]);
        ws.send(&ack).await.unwrap();
    }
    rejection_barrier(&ws).await;
    assert!(
        timeout(Duration::from_millis(350), ws.recv())
            .await
            .is_err(),
        "invalid ACK cleared the pending heartbeat and caused another probe"
    );
    assert_eq!(
        *transport.connection_state_changes().borrow(),
        ScConnectionState::Connected
    );

    // A matching ACK with repeated, unsupported MU-clear options completes it.
    ws.send(&[0x0B, 2, probe[2], probe[3], 0x9E, 0x1E])
        .await
        .unwrap();
    let next = recv(&ws).await;
    assert_eq!(next[0], 0x0A);
    assert_ne!(&next[2..4], &probe[2..4]);
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn heartbeat_invalid_traffic_does_not_refresh_node_liveness() {
    let (mut transport, _rx, ws) = start_timed().await;
    let mut states = transport.connection_state_changes();
    let mut messages = Vec::new();
    for case in invalid_heartbeats([0x01; 6]) {
        messages.push(case.wire.clone());
        let mut ack = case.wire;
        ack[0] = 0x0B;
        messages.push(ack);
    }
    let mut next = 0;
    let mut probe_count = 0;
    let mut interval = tokio::time::interval(Duration::from_millis(20));
    // Real time is required: production uses std::Instant for receive activity.
    timeout(Duration::from_secs(3), async {
        loop {
            tokio::select! {
                changed = states.changed() => {
                    changed.unwrap();
                    if *states.borrow_and_update() == ScConnectionState::Disconnected { break; }
                }
                _ = interval.tick() => {
                    ws.send(&messages[next % messages.len()]).await.unwrap();
                    next += 1;
                }
                response = ws.recv() => {
                    let response = decode_sc_message(&response.unwrap()).unwrap();
                    match response.function {
                        ScFunction::HeartbeatRequest => {
                            probe_count += 1;
                            // Every malformed ACK thereafter matches the live probe.
                            for message in &mut messages {
                                if message[0] == 0x0B { message[2..4].copy_from_slice(&response.message_id.to_be_bytes()); }
                            }
                        }
                        ScFunction::Result => {}
                        other => panic!("invalid heartbeat caused {other:?}"),
                    }
                }
            }
        }
    }).await.expect("invalid heartbeat traffic kept the node connection alive");
    assert!(
        next >= messages.len(),
        "all malformed cases must be exercised"
    );
    assert_eq!(
        probe_count, 1,
        "invalid traffic cleared the outstanding probe"
    );
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn heartbeat_envelopes_obey_nak_precedence_addressing_and_silence() {
    let (mut transport, _rx, ws) = start_transport().await;
    for case in invalid_heartbeats([0x01; 6]) {
        assert!(decode_sc_message(&case.wire).is_ok(), "{}", case.name);
        ws.send(&case.wire).await.unwrap();
        if let Some(nak) = case.nak {
            assert_eq!(recv(&ws).await, nak, "{}", case.name);
        }
        rejection_barrier(&ws).await;
        let mut ack = case.wire;
        ack[0] = 0x0B;
        ws.send(&ack).await.unwrap();
        rejection_barrier(&ws).await;
        assert!(
            timeout(Duration::from_millis(10), ws.recv()).await.is_err(),
            "{} caused a response loop",
            case.name
        );
    }
    // Repeated well-formed unknown MU-clear options can be ignored, and valid
    // traffic still receives exactly one normal ACK after semantic rejections.
    ws.send(&[0x0A, 2, 0x44, 0x55, 0x9E, 0x1E]).await.unwrap();
    assert_eq!(recv(&ws).await, [0x0B, 0, 0x44, 0x55]);
    transport.stop().await.unwrap();
}
