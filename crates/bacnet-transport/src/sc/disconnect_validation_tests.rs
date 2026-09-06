use super::data_attribute_tests::start_transport;
use super::heartbeat_validation_tests::start_timed;
use super::*;
use crate::sc_frame::heartbeat_test_support::invalid_disconnects;
use tokio::time::timeout;

async fn recv(ws: &LoopbackWebSocket) -> Vec<u8> {
    timeout(Duration::from_secs(2), ws.recv())
        .await
        .unwrap()
        .unwrap()
}

async fn rejection_barrier(ws: &LoopbackWebSocket) {
    ws.send(&[0x08, 0, 0x33, 0x44, 0x42]).await.unwrap();
    assert_eq!(recv(ws).await, [0, 0, 0x33, 0x44, 8, 1, 0, 0, 7, 0, 7]);
}

#[tokio::test]
async fn heartbeat_explicit_destination_is_silent_at_hub_connector() {
    let (mut transport, _rx, ws) = start_transport().await;
    for destination in [[0x01; 6], [0x44; 6], [0xFF; 6]] {
        for source in [None, Some([0x22; 6])] {
            let mut wire = vec![0x0A, if source.is_some() { 12 } else { 4 }, 0x11, 0x22];
            if let Some(source) = source {
                wire.extend_from_slice(&source);
            }
            wire.extend_from_slice(&destination);
            ws.send(&wire).await.unwrap();
            // The rejection NAK is ordered after the silent local-BVLL discard.
            rejection_barrier(&ws).await;
            assert!(timeout(Duration::from_millis(10), ws.recv()).await.is_err());
        }
    }
    ws.send(&[0x0A, 0, 0x44, 0x55]).await.unwrap();
    assert_eq!(recv(&ws).await, [0x0B, 0, 0x44, 0x55]);
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn disconnect_request_payload_gets_nak_before_state_change() {
    let (mut transport, _rx, ws) = start_transport().await;
    ws.send(&[0x08, 0, 0x22, 0x33, 0x42]).await.unwrap();
    assert_eq!(recv(&ws).await, [0, 0, 0x22, 0x33, 8, 1, 0, 0, 7, 0, 7]);
    assert_eq!(
        *transport.connection_state_changes().borrow(),
        ScConnectionState::Connected
    );
    assert!(transport
        .connection()
        .unwrap()
        .lock()
        .await
        .disconnect_ack_to_send
        .is_none());
    transport.stop().await.unwrap();
}

#[test]
fn disconnect_forbidden_vmacs_do_not_mutate_public_connection() {
    // These are the malformed VMAC combinations formerly used by the three
    // legacy positive Disconnect state/ACK tests.
    for (source, destination) in [
        ([0x10; 6], [0x01; 6]),
        ([10, 20, 30, 40, 50, 60], [1, 2, 3, 4, 5, 6]),
    ] {
        for function in [ScFunction::DisconnectRequest, ScFunction::DisconnectAck] {
            for state in [
                ScConnectionState::Connected,
                ScConnectionState::Disconnecting,
            ] {
                let mut msg = ScMessage {
                    function,
                    message_id: 42,
                    originating_vmac: Some(source),
                    destination_vmac: Some(destination),
                    dest_options: Vec::new(),
                    data_options: Vec::new(),
                    payload: Bytes::new(),
                };
                for queued in [false, true] {
                    let mut conn = ScConnection::new(destination, [0; 16]);
                    conn.state = state;
                    conn.disconnect_ack_to_send = queued.then(|| {
                        let mut ack = conn.build_heartbeat_ack(0xABCD);
                        ack.function = ScFunction::DisconnectAck;
                        ack
                    });
                    let before = conn.clone();
                    assert!(conn.handle_received(&msg).is_none());
                    assert_eq!(conn.state, before.state, "{function:?}");
                    assert_eq!(
                        conn.disconnect_ack_to_send, before.disconnect_ack_to_send,
                        "{function:?}"
                    );
                    assert_eq!(conn.next_message_id, before.next_message_id);
                    // Keep each request's ID distinct from any existing queue.
                    msg.message_id += 1;
                }
            }
        }
    }
}

#[test]
fn disconnect_envelope_matrix_preserves_public_state_and_queued_ack() {
    for case in invalid_disconnects([0x01; 6]) {
        for function in [0x08, 0x09] {
            let mut wire = case.wire.clone();
            wire[0] = function;
            let msg = decode_sc_message(&wire).unwrap();
            for state in [
                ScConnectionState::Connected,
                ScConnectionState::Disconnecting,
            ] {
                for queued in [false, true] {
                    let mut conn = ScConnection::new([0x01; 6], [0; 16]);
                    conn.state = state;
                    conn.disconnect_ack_to_send =
                        queued.then(|| decode_sc_message(&[9, 0, 0xAB, 0xCD]).unwrap());
                    let before = conn.clone();
                    assert!(conn.handle_received(&msg).is_none(), "{}", case.name);
                    assert_eq!(conn.state, before.state, "{} / {function}", case.name);
                    assert_eq!(
                        conn.disconnect_ack_to_send, before.disconnect_ack_to_send,
                        "{} / {function}",
                        case.name
                    );
                    assert_eq!(conn.next_message_id, before.next_message_id);
                }
            }
        }
    }
    for function in [0x08, 0x09] {
        let mut conn = ScConnection::new([0x01; 6], [0; 16]);
        conn.state = if function == 8 {
            ScConnectionState::Connected
        } else {
            ScConnectionState::Disconnecting
        };
        let msg = decode_sc_message(&[function, 2, 0x22, 0x33, 0x9E, 0x1E]).unwrap();
        assert!(conn.handle_received(&msg).is_none());
        assert_eq!(conn.state, ScConnectionState::Disconnected);
        let expected_ack = (function == 8).then(|| decode_sc_message(&[9, 0, 0x22, 0x33]).unwrap());
        assert_eq!(conn.disconnect_ack_to_send, expected_ack);
    }
}

#[tokio::test]
async fn disconnect_envelopes_preserve_node_state_and_pending_probe() {
    let (mut transport, mut rx, ws) = start_timed().await;
    let probe = recv(&ws).await;
    assert_eq!(probe[0], 0x0A);
    for case in invalid_disconnects([0x01; 6]) {
        ws.send(&case.wire).await.unwrap();
        if let Some(nak) = case.node_nak {
            assert_eq!(recv(&ws).await, nak, "{}", case.name);
        }
        let mut ack = case.wire;
        ack[0] = 0x09;
        ws.send(&ack).await.unwrap();
        rejection_barrier(&ws).await;
        assert_eq!(
            *transport.connection_state_changes().borrow(),
            ScConnectionState::Connected,
            "{}",
            case.name
        );
        assert!(transport
            .connection()
            .unwrap()
            .lock()
            .await
            .disconnect_ack_to_send
            .is_none());
    }
    assert!(
        timeout(Duration::from_millis(350), ws.recv())
            .await
            .is_err(),
        "invalid Disconnect cleared the pending heartbeat"
    );
    ws.send(&[0x0B, 0, probe[2], probe[3]]).await.unwrap();
    let next = recv(&ws).await;
    assert_eq!(next[0], 0x0A);
    assert_ne!(&next[2..4], &probe[2..4]);
    // Valid heartbeat and relayed NPDU traffic remain serviceable after rejection.
    ws.send(&[0x0A, 0, 0x44, 0x55]).await.unwrap();
    assert_eq!(recv(&ws).await, [0x0B, 0, 0x44, 0x55]);
    ws.send(&[
        1, 8, 0x44, 0x66, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 1, 0, 0x30,
    ])
    .await
    .unwrap();
    let received = timeout(Duration::from_secs(1), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.npdu.as_ref(), &[1, 0, 0x30]);
    assert_eq!(received.source_mac.as_slice(), &[0x22; 6]);
    assert!(received.data_attributes.is_empty());
    ws.send(&[8, 2, 0x55, 0x66, 0x9E, 0x1E]).await.unwrap();
    assert_eq!(recv(&ws).await, [9, 0, 0x55, 0x66]);
    assert_eq!(
        *transport.connection_state_changes().borrow(),
        ScConnectionState::Disconnected
    );
    // Existing state/ACK behavior only: immediate node socket closure is deferred.
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn disconnect_invalid_traffic_does_not_refresh_node_liveness() {
    let (mut transport, _rx, ws) = start_timed().await;
    let mut states = transport.connection_state_changes();
    let mut messages = Vec::new();
    for case in invalid_disconnects([0x01; 6]) {
        messages.push(case.wire.clone());
        let mut ack = case.wire;
        ack[0] = 9;
        messages.push(ack);
    }
    let mut next = 0;
    let mut probe_count = 0;
    let mut interval = tokio::time::interval(Duration::from_millis(10));
    // Exercise production's std::Instant clock, not only paused Tokio time.
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
                        ScFunction::HeartbeatRequest => probe_count += 1,
                        ScFunction::Result => {}
                        other => panic!("invalid Disconnect caused {other:?}"),
                    }
                }
            }
        }
    })
    .await
    .expect("invalid Disconnect traffic kept the node connection alive");
    assert!(
        next >= messages.len(),
        "malformed Disconnect changed state before the ordinary timeout"
    );
    assert_eq!(
        probe_count, 1,
        "invalid Disconnect cleared the pending heartbeat"
    );
    transport.stop().await.unwrap();
}
